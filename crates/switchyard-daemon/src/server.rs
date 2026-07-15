use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    convert::Infallible,
    fs::{self, OpenOptions},
    future::Future,
    io::{self, Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    pin::Pin,
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query, State, rejection::JsonRejection},
    http::{HeaderMap, Request, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Redirect, Response, Sse, sse::Event},
    routing::{delete, get, post},
};
use futures_util::{Stream, stream};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use switchyard_sources::{SourceError, SourceManager};
use switchyard_state::{
    AppliedSnapshot, GeneratedManifest, LockRequest, OperationKind, OperationRecord,
    OperationStatus, ReconciliationInput, ReconciliationReport, RouterApplyRecord,
    RouterApplyStatus, StateError, StateStore, StructuredContext,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    net::TcpListener,
    process::Command,
    sync::{Notify, Semaphore, watch},
};

use crate::contract::{
    API_VERSION, ApiErrorV1, CommandKind, CommandRequestV1, CommandResultV1,
    CreateDeploymentRequestV1, CreateWorktreeRequestV1, DaemonStatusV1, DeploymentDefinitionV1,
    DeploymentDetailV1, DeploymentOperationSummaryV1, DeploymentRoutesV1, DeploymentSummaryV1,
    DeploymentValidationV1, DeploymentsV1, DiscoveryV1, EventKindV1, EventV1, GatewayExposureV1,
    OperationStatusV1, OperationV1, RegisterSourceRequestV1, RemoveWorktreeRequestV1,
    RouteHistoryV1, RouterBindingV1, TransitionPolicyV1, UpdateDeploymentDefinitionRequestV1,
};

const LOCK_TTL_MILLIS: i64 = 15_000;
const EVENT_CAPACITY: usize = 2_048;
const TERMINAL_OPERATION_RETENTION: usize = 64;

/// Configuration for one daemon instance.
#[derive(Clone, Debug)]
pub struct DaemonConfig {
    pub project_root: PathBuf,
    pub bind: SocketAddr,
    pub max_heavy_operations: usize,
    pub cli_program: PathBuf,
    pub gui_dist: PathBuf,
}

impl DaemonConfig {
    pub fn new(project_root: PathBuf, cli_program: PathBuf) -> Self {
        let gui_dist = project_root.join("packages/web/dist");
        Self {
            project_root,
            bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            max_heavy_operations: 2,
            cli_program,
            gui_dist,
        }
    }
}

/// Outcome produced by an operation backend.
#[derive(Clone, Debug)]
pub enum BackendOutcome {
    Completed(CommandResultV1),
    Cancelled(CommandResultV1),
    LiveBinding {
        result: CommandResultV1,
        attempts: Vec<RouterApplyRecord>,
    },
}

/// Boxed asynchronous operation result used by injectable daemon backends.
pub type BackendFuture = Pin<Box<dyn Future<Output = Result<BackendOutcome, ApiErrorV1>> + Send>>;

/// Injectable, Docker-free operation backend used by the server and integration tests.
pub trait OperationBackend: Send + Sync + 'static {
    fn run(
        &self,
        kind: CommandKind,
        arguments: Vec<String>,
        cancellation: watch::Receiver<bool>,
        events: EventSink,
    ) -> BackendFuture;

    /// Native daemon-owned live bind path. Test backends may retain the command path.
    fn live_bind(
        &self,
        _request: CommandRequestV1,
        _operation_id: String,
        _cancellation: watch::Receiver<bool>,
        _events: EventSink,
    ) -> Option<BackendFuture> {
        None
    }
}

/// Event emitter passed to operation backends.
#[derive(Clone)]
pub struct EventSink {
    operation_id: String,
    log: Arc<EventLog>,
}

impl EventSink {
    pub fn emit(&self, kind: EventKindV1, data: Value) {
        self.log.emit(&self.operation_id, kind, data);
    }
}

/// Backend that executes the existing `switchyard` command implementation.
#[derive(Clone, Debug)]
pub struct CliBackend {
    program: PathBuf,
    project_root: PathBuf,
}

impl CliBackend {
    pub fn new(program: PathBuf, project_root: PathBuf) -> Self {
        Self {
            program,
            project_root,
        }
    }
}

impl OperationBackend for CliBackend {
    fn run(
        &self,
        kind: CommandKind,
        arguments: Vec<String>,
        mut cancellation: watch::Receiver<bool>,
        events: EventSink,
    ) -> BackendFuture {
        let program = self.program.clone();
        let project_root = self.project_root.clone();
        Box::pin(async move {
            let mut child = Command::new(&program)
                .args(arguments)
                .current_dir(project_root)
                .env("SWITCHYARD_BYPASS_DAEMON", "1")
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .map_err(|error| {
                    ApiErrorV1::new(
                        "operation_spawn_failed",
                        format!("could not start switchyard command: {error}"),
                    )
                })?;
            let stdout = child.stdout.take().expect("piped stdout");
            let stderr = child.stderr.take().expect("piped stderr");
            let stdout_events = events.clone();
            let stdout_task = tokio::spawn(read_output(stdout, false, kind, stdout_events));
            let stderr_task = tokio::spawn(read_output(stderr, true, kind, events));
            let (status, cancelled) = tokio::select! {
                status = child.wait() => (status, false),
                changed = cancellation.changed() => {
                    if changed.is_ok() && *cancellation.borrow() {
                        let _ = child.kill().await;
                    }
                    (child.wait().await, true)
                }
            };
            let status = status.map_err(|error| {
                ApiErrorV1::new(
                    "operation_wait_failed",
                    format!("command wait failed: {error}"),
                )
            })?;
            let stdout = stdout_task
                .await
                .map_err(|error| ApiErrorV1::new("output_task_failed", error.to_string()))??;
            let stderr = stderr_task
                .await
                .map_err(|error| ApiErrorV1::new("output_task_failed", error.to_string()))??;
            let result = CommandResultV1 {
                exit_code: status.code().unwrap_or(if cancelled { 130 } else { 1 }),
                stdout,
                stderr,
            };
            Ok(if cancelled {
                BackendOutcome::Cancelled(result)
            } else {
                BackendOutcome::Completed(result)
            })
        })
    }

    fn live_bind(
        &self,
        request: CommandRequestV1,
        operation_id: String,
        cancellation: watch::Receiver<bool>,
        events: EventSink,
    ) -> Option<BackendFuture> {
        let project_root = self.project_root.clone();
        Some(Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                apply_live_binding(&project_root, request, &operation_id, cancellation, events)
            })
            .await
            .map_err(|error| ApiErrorV1::new("router_apply_task_failed", error.to_string()))?
        }))
    }
}

struct RouterTarget {
    id: String,
    socket: PathBuf,
    old: router_config::RouterConfig,
    candidate: router_config::RouterConfig,
}

trait RouterAdmin {
    fn current_snapshot(
        &self,
        socket: &Path,
        token: &str,
    ) -> Result<switchyard_router_admin::SnapshotIdentity, switchyard_router_admin::AdminError>;

    fn apply_snapshot(
        &self,
        socket: &Path,
        token: &str,
        config: &router_config::RouterConfig,
    ) -> Result<switchyard_router_admin::ApplyAcknowledgement, switchyard_router_admin::AdminError>;
}

struct LocalRouterAdmin;

impl RouterAdmin for LocalRouterAdmin {
    fn current_snapshot(
        &self,
        socket: &Path,
        token: &str,
    ) -> Result<switchyard_router_admin::SnapshotIdentity, switchyard_router_admin::AdminError>
    {
        switchyard_router_admin::current_snapshot(socket, token)
    }

    fn apply_snapshot(
        &self,
        socket: &Path,
        token: &str,
        config: &router_config::RouterConfig,
    ) -> Result<switchyard_router_admin::ApplyAcknowledgement, switchyard_router_admin::AdminError>
    {
        switchyard_router_admin::apply_snapshot(socket, token, config)
    }
}

fn apply_live_binding(
    project_root: &Path,
    request: CommandRequestV1,
    operation_id: &str,
    cancellation: watch::Receiver<bool>,
    events: EventSink,
) -> Result<BackendOutcome, ApiErrorV1> {
    apply_live_binding_with_admin(
        project_root,
        request,
        operation_id,
        cancellation,
        events,
        &LocalRouterAdmin,
    )
}

fn apply_live_binding_with_admin(
    project_root: &Path,
    request: CommandRequestV1,
    operation_id: &str,
    cancellation: watch::Receiver<bool>,
    events: EventSink,
    admin: &impl RouterAdmin,
) -> Result<BackendOutcome, ApiErrorV1> {
    let consumer = request
        .consumer
        .as_deref()
        .ok_or_else(|| ApiErrorV1::new("invalid_request", "`consumer` is required"))?;
    let group = request
        .group
        .as_deref()
        .ok_or_else(|| ApiErrorV1::new("invalid_request", "`group` is required"))?;
    let bundle_path = if request.bundle.is_absolute() {
        request.bundle.clone()
    } else {
        project_root.join(&request.bundle)
    };
    let authored = switchyard_planner::load_bundle(&bundle_path)
        .map_err(|error| ApiErrorV1::new("bundle_load_failed", error.to_string()))?;
    let authored_plan = switchyard_planner::plan(&authored)
        .map_err(|errors| ApiErrorV1::new("plan_failed", format_diagnostics(errors)))?;
    let resolved_path = project_root
        .join(&authored_plan.artifact_dir)
        .join("resolved-deployment.yaml");
    let base = if resolved_path.is_file() {
        let manifest_path = project_root
            .join(&authored_plan.artifact_dir)
            .join("manifest.json");
        let manifest: Value =
            serde_json::from_slice(&fs::read(&manifest_path).map_err(|error| {
                ApiErrorV1::new("generated_manifest_missing", error.to_string())
            })?)
            .map_err(|error| ApiErrorV1::new("generated_manifest_invalid", error.to_string()))?;
        if manifest["deployment"].as_str() != Some(authored_plan.deployment.as_str())
            || manifest["resourceHash"].as_str() != Some(authored_plan.resource_hash.as_str())
        {
            return Err(ApiErrorV1::new(
                "generated_state_drift",
                "generated bind state does not match this deployment; reconcile drift before binding",
            ));
        }
        switchyard_planner::load_bundle(&resolved_path)
            .map_err(|error| ApiErrorV1::new("resolved_state_invalid", error.to_string()))?
    } else {
        authored
    };
    let mut plan = switchyard_planner::plan_with_binding(&base, consumer, group)
        .map_err(|errors| ApiErrorV1::new("plan_failed", format_diagnostics(errors)))?;
    let token = std::env::var("SWITCHYARD_ROUTER_TOKEN").map_err(|_| {
        ApiErrorV1::new(
            "router_token_missing",
            "SWITCHYARD_ROUTER_TOKEN must be set for bind",
        )
    })?;
    let transition = request.transition;
    let route_dir = project_root
        .join(&authored_plan.artifact_dir)
        .join("routes");
    let old_sidecar: router_config::RouterConfig = serde_json::from_slice(
        &fs::read(route_dir.join(format!("{consumer}.json")))
            .map_err(|error| ApiErrorV1::new("active_snapshot_missing", error.to_string()))?,
    )
    .map_err(|error| ApiErrorV1::new("active_snapshot_invalid", error.to_string()))?;
    let mut candidate_sidecar: router_config::RouterConfig = serde_json::from_str(
        plan.route_configs
            .get(consumer)
            .ok_or_else(|| ApiErrorV1::new("router_missing", "consumer has no sidecar router"))?,
    )
    .map_err(|error| ApiErrorV1::new("candidate_snapshot_invalid", error.to_string()))?;
    set_transition(&mut candidate_sidecar, transition);
    let sidecar = plan
        .sidecars
        .get(consumer)
        .ok_or_else(|| ApiErrorV1::new("router_missing", "consumer has no sidecar router"))?;
    let mut targets = vec![RouterTarget {
        id: format!("sidecar:{consumer}"),
        socket: project_root.join(&sidecar.admin_socket),
        old: old_sidecar,
        candidate: candidate_sidecar,
    }];

    let host_dir = project_root.join(".switchyard/run").join(&plan.deployment);
    let host_socket = host_dir.join("host.socket");
    let host_config = host_dir.join("host-router.json");
    if let Some(encoded) = &plan.host_router_config {
        let mut candidate: router_config::RouterConfig = serde_json::from_str(encoded)
            .map_err(|error| ApiErrorV1::new("candidate_snapshot_invalid", error.to_string()))?;
        let old = match fs::read(&host_config) {
            Ok(encoded) => serde_json::from_slice(&encoded)
                .map_err(|error| ApiErrorV1::new("active_snapshot_invalid", error.to_string()))?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => candidate.clone(),
            Err(error) => {
                return Err(ApiErrorV1::new(
                    "active_snapshot_missing",
                    error.to_string(),
                ));
            }
        };
        if host_config.is_file() {
            for provider in &mut candidate.spec.providers {
                if let Some(active) = old
                    .spec
                    .providers
                    .iter()
                    .find(|active| active.id == provider.id)
                {
                    provider.endpoint = active.endpoint.clone();
                }
            }
        }
        set_transition(&mut candidate, transition);
        targets.push(RouterTarget {
            id: "host-gateway".into(),
            socket: host_socket,
            old,
            candidate,
        });
    }

    let transition_context = StructuredContext::new(
        serde_json::to_value(&targets[0].candidate.spec.snapshot.transitions)
            .unwrap_or(Value::Null),
    )
    .map_err(|error| ApiErrorV1::new(error.code(), error.to_string()))?;
    let mut attempts = Vec::new();
    let mut applied: Vec<(usize, switchyard_router_admin::SnapshotIdentity)> = Vec::new();
    let mut acknowledgements = Vec::new();
    let mut failure = None;
    for (index, target) in targets.iter_mut().enumerate() {
        if *cancellation.borrow() {
            failure = Some("operation was cancelled".to_owned());
            break;
        }
        let observed = match admin.current_snapshot(&target.socket, &token) {
            Ok(observed) => observed,
            Err(error) => {
                failure = Some(error.to_string());
                attempts.push(router_attempt(
                    &plan.deployment,
                    target,
                    consumer,
                    operation_id,
                    target.candidate.spec.snapshot.version,
                    snapshot_checksum(&target.candidate),
                    RouterApplyStatus::Failed,
                    None,
                    transition_context.clone(),
                    Some("router_unreachable"),
                    json!({"message": error.to_string()}),
                )?);
                break;
            }
        };
        let version = observed.version.checked_add(1).ok_or_else(|| {
            ApiErrorV1::new(
                "snapshot_version_exhausted",
                "router snapshot version is exhausted",
            )
        })?;
        target.candidate.spec.snapshot.version = version;
        target.candidate.spec.snapshot.id = router_config::RouteSnapshotId::new(format!(
            "{}-bind-{version}",
            target.id.replace(':', "-")
        ));
        let checksum = snapshot_checksum(&target.candidate);
        events.emit(
            EventKindV1::Route,
            json!({"router": target.id, "binding": consumer, "desiredVersion": version, "status": "pending"}),
        );
        match admin.apply_snapshot(&target.socket, &token, &target.candidate) {
            Ok(ack)
                if ack.version == version
                    && ack.checksum == checksum
                    && ack.status == switchyard_router_admin::ActivationStatus::Activated =>
            {
                attempts.push(router_attempt(
                    &plan.deployment,
                    target,
                    consumer,
                    operation_id,
                    version,
                    checksum,
                    RouterApplyStatus::Active,
                    Some((observed.version, observed.checksum.clone())),
                    transition_context.clone(),
                    None,
                    json!({"acknowledgement": ack}),
                )?);
                acknowledgements.push(json!({"router": target.id, "acknowledgement": ack}));
                applied.push((index, observed));
            }
            Ok(ack) => {
                failure = Some(format!(
                    "router {} returned a non-activating acknowledgement",
                    target.id
                ));
                attempts.push(router_attempt(
                    &plan.deployment,
                    target,
                    consumer,
                    operation_id,
                    version,
                    checksum,
                    RouterApplyStatus::Failed,
                    Some((observed.version, observed.checksum)),
                    transition_context.clone(),
                    Some("acknowledgement_mismatch"),
                    json!({"acknowledgement": ack}),
                )?);
                break;
            }
            Err(error) => {
                let rolled_back = error.rejection_code() == Some("provider_unhealthy");
                failure = Some(error.to_string());
                attempts.push(router_attempt(
                    &plan.deployment,
                    target,
                    consumer,
                    operation_id,
                    version,
                    checksum,
                    if rolled_back {
                        RouterApplyStatus::RolledBack
                    } else {
                        RouterApplyStatus::Failed
                    },
                    Some((observed.version, observed.checksum)),
                    transition_context.clone(),
                    error.rejection_code().or(Some("router_apply_failed")),
                    error
                        .details()
                        .cloned()
                        .unwrap_or_else(|| json!({"message": error.to_string()})),
                )?);
                break;
            }
        }
    }

    if failure.is_some() {
        rollback_applied_targets(
            &plan.deployment,
            consumer,
            operation_id,
            &token,
            transition,
            &transition_context,
            &mut targets,
            applied,
            &mut attempts,
            admin,
        );
        let message = failure.expect("checked");
        return Ok(BackendOutcome::LiveBinding {
            result: CommandResultV1 {
                exit_code: 1,
                stdout: String::new(),
                stderr: format!("switchyard: {message}\n"),
            },
            attempts,
        });
    }

    let sidecar_config = &targets[0].candidate;
    plan.route_configs.insert(
        consumer.to_owned(),
        serde_json::to_string_pretty(sidecar_config)
            .map_err(|error| ApiErrorV1::new("snapshot_encode_failed", error.to_string()))?,
    );
    if let Some(target) = targets.iter().find(|target| target.id == "host-gateway") {
        plan.host_router_config = Some(
            serde_json::to_string_pretty(&target.candidate)
                .map_err(|error| ApiErrorV1::new("snapshot_encode_failed", error.to_string()))?,
        );
        fs::write(
            host_dir.join("host-router.json"),
            plan.host_router_config.as_deref().unwrap(),
        )
        .map_err(|error| ApiErrorV1::new("host_snapshot_write_failed", error.to_string()))?;
    }
    switchyard_planner::write_plan(project_root, &plan)
        .map_err(|error| ApiErrorV1::new("artifact_write_failed", error.to_string()))?;
    events.emit(
        EventKindV1::Route,
        json!({"binding": consumer, "status": "active"}),
    );
    let mut stdout = format!(
        "router acknowledgement: {}\n",
        serde_json::to_string(
            acknowledgements
                .first()
                .map(|entry| &entry["acknowledgement"])
                .unwrap_or(&Value::Null)
        )
        .unwrap_or_else(|_| "null".into())
    );
    for acknowledgement in acknowledgements.iter().skip(1) {
        stdout.push_str(&format!(
            "router acknowledgement ({}): {}\n",
            acknowledgement["router"].as_str().unwrap_or("additional"),
            serde_json::to_string(&acknowledgement["acknowledgement"])
                .unwrap_or_else(|_| "null".into())
        ));
    }
    stdout.push_str(&format!("bound `{consumer}` to `{group}`\n"));
    Ok(BackendOutcome::LiveBinding {
        result: CommandResultV1 {
            exit_code: 0,
            stdout,
            stderr: String::new(),
        },
        attempts,
    })
}

#[allow(clippy::too_many_arguments)]
fn rollback_applied_targets(
    deployment: &str,
    consumer: &str,
    operation_id: &str,
    token: &str,
    transition: Option<TransitionPolicyV1>,
    transition_context: &StructuredContext,
    targets: &mut [RouterTarget],
    applied: Vec<(usize, switchyard_router_admin::SnapshotIdentity)>,
    attempts: &mut Vec<RouterApplyRecord>,
    admin: &impl RouterAdmin,
) {
    for (index, previously_observed) in applied.into_iter().rev() {
        let target = &mut targets[index];
        let observed = match admin.current_snapshot(&target.socket, token) {
            Ok(observed) => observed,
            Err(error) => {
                attempts.push(rollback_attempt(
                    deployment,
                    target,
                    consumer,
                    operation_id,
                    target.candidate.spec.snapshot.version,
                    snapshot_checksum(&target.candidate),
                    RouterApplyStatus::Failed,
                    Some((previously_observed.version, previously_observed.checksum)),
                    transition_context.clone(),
                    Some("rollback_observation_failed"),
                    json!({"message": error.to_string()}),
                ));
                continue;
            }
        };
        let Some(rollback_version) = observed.version.checked_add(1) else {
            attempts.push(rollback_attempt(
                deployment,
                target,
                consumer,
                operation_id,
                observed.version,
                observed.checksum.clone(),
                RouterApplyStatus::Failed,
                Some((observed.version, observed.checksum)),
                transition_context.clone(),
                Some("rollback_version_exhausted"),
                json!({"message": "router snapshot version is exhausted"}),
            ));
            continue;
        };
        target.old.spec.snapshot.version = rollback_version;
        target.old.spec.snapshot.id = router_config::RouteSnapshotId::new(format!(
            "{}-rollback-{rollback_version}",
            target.id.replace(':', "-")
        ));
        set_transition(&mut target.old, transition);
        let checksum = snapshot_checksum(&target.old);
        match admin.apply_snapshot(&target.socket, token, &target.old) {
            Ok(ack)
                if ack.version == rollback_version
                    && ack.checksum == checksum
                    && ack.status == switchyard_router_admin::ActivationStatus::Activated =>
            {
                attempts.push(rollback_attempt(
                    deployment,
                    target,
                    consumer,
                    operation_id,
                    rollback_version,
                    checksum,
                    RouterApplyStatus::RolledBack,
                    Some((observed.version, observed.checksum.clone())),
                    transition_context.clone(),
                    None,
                    json!({"reason": "peer_router_failed"}),
                ));
            }
            Ok(ack) => attempts.push(rollback_attempt(
                deployment,
                target,
                consumer,
                operation_id,
                rollback_version,
                checksum,
                RouterApplyStatus::Failed,
                Some((observed.version, observed.checksum)),
                transition_context.clone(),
                Some("rollback_acknowledgement_mismatch"),
                json!({"acknowledgement": ack}),
            )),
            Err(error) => attempts.push(rollback_attempt(
                deployment,
                target,
                consumer,
                operation_id,
                rollback_version,
                checksum,
                RouterApplyStatus::Failed,
                Some((observed.version, observed.checksum)),
                transition_context.clone(),
                Some("rollback_failed"),
                json!({"message": error.to_string()}),
            )),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn rollback_attempt(
    deployment: &str,
    target: &RouterTarget,
    binding: &str,
    operation_id: &str,
    version: u64,
    checksum: String,
    status: RouterApplyStatus,
    observed: Option<(u64, String)>,
    transition: StructuredContext,
    error_code: Option<&str>,
    context: Value,
) -> RouterApplyRecord {
    let fallback_checksum = checksum.clone();
    let fallback_observed = observed.clone();
    let fallback_transition = transition.clone();
    router_attempt(
        deployment,
        target,
        binding,
        operation_id,
        version,
        checksum,
        status,
        observed,
        transition,
        error_code,
        context,
    )
    .unwrap_or_else(|error| RouterApplyRecord {
        deployment: deployment.into(),
        router: target.id.clone(),
        binding: binding.into(),
        operation_id: operation_id.into(),
        desired_version: i64::try_from(target.candidate.spec.snapshot.version).unwrap_or(i64::MAX),
        desired_checksum: snapshot_checksum(&target.candidate),
        version: i64::try_from(version).unwrap_or(i64::MAX),
        checksum: fallback_checksum,
        status: RouterApplyStatus::Failed,
        observed_version: fallback_observed
            .as_ref()
            .map(|(version, _)| i64::try_from(*version).unwrap_or(i64::MAX)),
        observed_checksum: fallback_observed.map(|(_, checksum)| checksum),
        transition: fallback_transition,
        error_code: Some(
            error_code
                .unwrap_or("rollback_attempt_record_failed")
                .into(),
        ),
        recorded_at: now_millis(),
        context: StructuredContext::new(json!({
            "recordingErrorCode": error.code,
            "message": error.message
        }))
        .unwrap_or_else(|_| StructuredContext::new(json!({})).expect("empty context is valid")),
    })
}

#[allow(clippy::too_many_arguments)]
fn router_attempt(
    deployment: &str,
    target: &RouterTarget,
    binding: &str,
    operation_id: &str,
    version: u64,
    checksum: String,
    status: RouterApplyStatus,
    observed: Option<(u64, String)>,
    transition: StructuredContext,
    error_code: Option<&str>,
    context: Value,
) -> Result<RouterApplyRecord, ApiErrorV1> {
    Ok(RouterApplyRecord {
        deployment: deployment.into(),
        router: target.id.clone(),
        binding: binding.into(),
        operation_id: operation_id.into(),
        desired_version: i64::try_from(target.candidate.spec.snapshot.version).map_err(|_| {
            ApiErrorV1::new("snapshot_version_exhausted", "version exceeds SQLite range")
        })?,
        desired_checksum: snapshot_checksum(&target.candidate),
        version: i64::try_from(version).map_err(|_| {
            ApiErrorV1::new("snapshot_version_exhausted", "version exceeds SQLite range")
        })?,
        checksum,
        status,
        observed_version: observed
            .as_ref()
            .map(|(version, _)| *version)
            .map(i64::try_from)
            .transpose()
            .map_err(|_| {
                ApiErrorV1::new("snapshot_version_exhausted", "version exceeds SQLite range")
            })?,
        observed_checksum: observed.map(|(_, checksum)| checksum),
        transition,
        error_code: error_code.map(str::to_owned),
        recorded_at: now_millis(),
        context: StructuredContext::new(context)
            .map_err(|error| ApiErrorV1::new(error.code(), error.to_string()))?,
    })
}

fn snapshot_checksum(config: &router_config::RouterConfig) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(config).expect("router configuration serializes"))
    )
}

fn set_transition(
    config: &mut router_config::RouterConfig,
    transition: Option<TransitionPolicyV1>,
) {
    let Some(transition) = transition else { return };
    let policy = match transition {
        TransitionPolicyV1::Close => router_config::ConnectionTransitionPolicy::Close,
        TransitionPolicyV1::Drain { timeout_ms } => {
            router_config::ConnectionTransitionPolicy::Drain { timeout_ms }
        }
        TransitionPolicyV1::Pin => router_config::ConnectionTransitionPolicy::Pin,
    };
    config.spec.snapshot.transitions = router_config::ConnectionTransitionPolicies {
        http: policy.clone(),
        https: policy.clone(),
        websocket: policy.clone(),
        grpc: policy.clone(),
        tcp: policy,
    };
}

fn format_diagnostics(diagnostics: Vec<switchyard_planner::Diagnostic>) -> String {
    diagnostics
        .into_iter()
        .map(|diagnostic| diagnostic.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

async fn read_output<R>(
    reader: R,
    stderr: bool,
    kind: CommandKind,
    events: EventSink,
) -> Result<String, ApiErrorV1>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut lines = BufReader::new(reader).lines();
    let mut captured = String::new();
    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|error| ApiErrorV1::new("output_read_failed", error.to_string()))?
    {
        captured.push_str(&line);
        captured.push('\n');
        let event_kind = match kind {
            CommandKind::Apply => EventKindV1::Build,
            CommandKind::Status => EventKindV1::Health,
            CommandKind::Bind | CommandKind::Routes => EventKindV1::Route,
            _ => EventKindV1::Log,
        };
        events.emit(
            event_kind,
            json!({"line": redact_event_line(&line), "stderr": stderr}),
        );
    }
    Ok(captured)
}

fn redact_event_line(line: &str) -> &str {
    let normalized = line.to_ascii_lowercase();
    if [
        "authorization",
        "password",
        "secret",
        "token",
        "private_key",
    ]
    .iter()
    .any(|word| normalized.contains(word))
    {
        "[REDACTED]"
    } else {
        line
    }
}

struct EventLog {
    events: Mutex<VecDeque<EventV1>>,
    next_id: Mutex<u64>,
    notify: Notify,
    terminal: Mutex<bool>,
}

impl EventLog {
    fn new() -> Self {
        Self {
            events: Mutex::new(VecDeque::with_capacity(EVENT_CAPACITY)),
            next_id: Mutex::new(1),
            notify: Notify::new(),
            terminal: Mutex::new(false),
        }
    }

    fn emit(&self, operation_id: &str, kind: EventKindV1, data: Value) {
        let id = {
            let mut next = self
                .next_id
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let id = *next;
            *next = next.saturating_add(1);
            id
        };
        let mut events = self
            .events
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if events.len() == EVENT_CAPACITY {
            events.pop_front();
        }
        events.push_back(EventV1 {
            id,
            operation_id: operation_id.into(),
            kind,
            timestamp: now_millis(),
            data,
        });
        drop(events);
        self.notify.notify_waiters();
    }

    fn finish(&self) {
        *self
            .terminal
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = true;
        self.notify.notify_waiters();
    }

    async fn next_after(&self, after: u64) -> Option<EventV1> {
        loop {
            let notified = self.notify.notified();
            if let Some(event) = self
                .events
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .iter()
                .find(|event| event.id > after)
                .cloned()
            {
                return Some(event);
            }
            if *self
                .terminal
                .lock()
                .unwrap_or_else(|error| error.into_inner())
            {
                return None;
            }
            notified.await;
        }
    }
}

struct RuntimeOperation {
    operation: OperationV1,
    cancellation: watch::Sender<bool>,
    events: Arc<EventLog>,
}

struct Inner {
    config: DaemonConfig,
    instance_id: String,
    token: String,
    store: Mutex<StateStore>,
    operations: Mutex<HashMap<String, RuntimeOperation>>,
    terminal_operations: Mutex<VecDeque<String>>,
    heavy: Semaphore,
    backend: Arc<dyn OperationBackend>,
    shutdown: watch::Sender<bool>,
    active_notify: Notify,
    reconciliation: ReconciliationReport,
}

/// A running daemon useful for embedding and integration tests.
pub struct RunningDaemon {
    pub address: SocketAddr,
    pub token: String,
    pub reconciliation: ReconciliationReport,
    pub(crate) shutdown: watch::Sender<bool>,
    pub(crate) task: tokio::task::JoinHandle<Result<(), DaemonError>>,
}

impl RunningDaemon {
    pub async fn shutdown(self) -> Result<(), DaemonError> {
        self.shutdown.send_replace(true);
        self.task
            .await
            .map_err(|error| DaemonError::Message(error.to_string()))?
    }
}

#[derive(Debug)]
pub enum DaemonError {
    Io(io::Error),
    State(StateError),
    InvalidConfiguration(String),
    Message(String),
}

impl std::fmt::Display for DaemonError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::State(error) => error.fmt(formatter),
            Self::InvalidConfiguration(message) | Self::Message(message) => message.fmt(formatter),
        }
    }
}

impl std::error::Error for DaemonError {}
impl From<io::Error> for DaemonError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}
impl From<StateError> for DaemonError {
    fn from(value: StateError) -> Self {
        Self::State(value)
    }
}

/// Starts a daemon using the real CLI backend.
pub async fn start(config: DaemonConfig) -> Result<RunningDaemon, DaemonError> {
    let backend = Arc::new(CliBackend::new(
        config.cli_program.clone(),
        config.project_root.clone(),
    ));
    start_with_backend(config, backend).await
}

/// Starts a daemon with an injectable backend.
pub async fn start_with_backend(
    config: DaemonConfig,
    backend: Arc<dyn OperationBackend>,
) -> Result<RunningDaemon, DaemonError> {
    let discovery_path = config.project_root.join(".switchyard/daemon.json");
    if discovery_listener_is_active(&discovery_path) {
        return Err(DaemonError::InvalidConfiguration(
            "a daemon discovered for this project is already listening".into(),
        ));
    }
    let prepared = prepare(
        config.clone(),
        backend,
        observe_docker().unwrap_or_default(),
    )?;
    let Prepared {
        inner,
        token,
        reconciliation,
        shutdown,
        shutdown_rx,
    } = prepared;
    let listener = TcpListener::bind(config.bind).await?;
    let address = listener.local_addr()?;
    if !address.ip().is_loopback() {
        return Err(DaemonError::InvalidConfiguration(
            "resolved daemon listener is not loopback-only".into(),
        ));
    }
    write_discovery(
        &discovery_path,
        &DiscoveryV1 {
            api_version: API_VERSION.into(),
            address: address.to_string(),
            token: token.clone(),
            pid: std::process::id(),
        },
    )?;
    let app = routes(inner.clone());
    let discovery_token = token.clone();
    let task = tokio::spawn(async move {
        let mut shutdown_rx = shutdown_rx;
        let shutdown_inner = inner.clone();
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                while !*shutdown_rx.borrow() && shutdown_rx.changed().await.is_ok() {}
                request_cancellation(&shutdown_inner);
            })
            .await
            .map_err(DaemonError::Io)?;
        cancel_and_wait(&inner).await;
        remove_discovery_if_owned(&discovery_path, &discovery_token)?;
        Ok(())
    });
    Ok(RunningDaemon {
        address,
        token,
        reconciliation,
        shutdown,
        task,
    })
}

struct Prepared {
    inner: Arc<Inner>,
    token: String,
    reconciliation: ReconciliationReport,
    shutdown: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
}

fn prepare(
    config: DaemonConfig,
    backend: Arc<dyn OperationBackend>,
    resources: Vec<switchyard_state::OwnedResourceObservation>,
) -> Result<Prepared, DaemonError> {
    if !config.bind.ip().is_loopback() {
        return Err(DaemonError::InvalidConfiguration(
            "daemon listener must use a loopback address".into(),
        ));
    }
    if config.max_heavy_operations == 0 {
        return Err(DaemonError::InvalidConfiguration(
            "heavy-operation concurrency must be positive".into(),
        ));
    }
    let state_dir = config.project_root.join(".switchyard");
    fs::create_dir_all(&state_dir)?;
    let (mut store, _) = StateStore::open(state_dir.join("state.sqlite3"))?;
    let now = now_millis();
    store.recover_abandoned_operations(now)?;
    let manifests = GeneratedManifest::load_generated(&state_dir.join("generated"))?;
    let reconciliation = store.reconcile(
        &ReconciliationInput {
            manifests,
            resources,
        },
        now,
    )?;
    let token = random_hex(32)?;
    let instance_id = random_hex(16)?;
    let (shutdown, shutdown_rx) = watch::channel(false);
    let inner = Arc::new(Inner {
        config: config.clone(),
        instance_id,
        token: token.clone(),
        store: Mutex::new(store),
        operations: Mutex::new(HashMap::new()),
        terminal_operations: Mutex::new(VecDeque::new()),
        heavy: Semaphore::new(config.max_heavy_operations),
        backend,
        shutdown: shutdown.clone(),
        active_notify: Notify::new(),
        reconciliation: reconciliation.clone(),
    });
    Ok(Prepared {
        inner,
        token,
        reconciliation,
        shutdown,
        shutdown_rx,
    })
}

/// In-memory API instance for transport-independent tests in socket-restricted environments.
///
/// Startup reconciliation sees no runtime resources: tests stay hermetic even when a
/// real Docker Engine with Switchyard-labeled resources is present on the host.
#[doc(hidden)]
pub fn api_for_tests(
    config: DaemonConfig,
    backend: Arc<dyn OperationBackend>,
) -> Result<(Router, String, ReconciliationReport), DaemonError> {
    let prepared = prepare(config, backend, Vec::new())?;
    Ok((
        routes(prepared.inner),
        prepared.token,
        prepared.reconciliation,
    ))
}

fn routes(inner: Arc<Inner>) -> Router {
    Router::new()
        .route("/gui", get(|| async { Redirect::permanent("/gui/") }))
        .route(
            "/gui/",
            get(|State(inner)| async move {
                serve_gui(State(inner), AxumPath("index.html".into())).await
            }),
        )
        .route("/gui/{*path}", get(serve_gui))
        .route("/api/v1/system/status", get(system_status))
        .route("/api/v1/system/shutdown", post(system_shutdown))
        .route("/api/v1/commands/{kind}", post(start_command))
        .route("/api/v1/operations/{id}", get(get_operation))
        .route("/api/v1/operations/{id}/cancel", post(cancel_operation))
        .route("/api/v1/operations/{id}/events", get(operation_events))
        .route(
            "/api/v1/deployments/{deployment}/routes",
            get(deployment_routes),
        )
        .route(
            "/api/v1/deployments",
            get(list_deployments).post(create_deployment),
        )
        .route(
            "/api/v1/deployments/{deployment}/definition",
            get(deployment_definition).put(update_deployment_definition),
        )
        .route("/api/v1/deployments/{deployment}", get(deployment_detail))
        .route("/api/v1/adapters", get(list_adapters))
        .route("/api/v1/sources", get(list_sources).post(register_source))
        .route("/api/v1/sources/{name}", delete(deregister_source))
        .route(
            "/api/v1/worktrees",
            get(list_worktrees).post(create_worktree),
        )
        .route("/api/v1/worktrees/{name}", delete(remove_worktree))
        .fallback(api_not_found)
        .method_not_allowed_fallback(api_method_not_allowed)
        .with_state(inner.clone())
        .layer(middleware::from_fn_with_state(inner, auth_middleware))
}

async fn serve_gui(State(inner): State<Arc<Inner>>, AxumPath(path): AxumPath<String>) -> Response {
    let relative = Path::new(&path);
    let safe = relative
        .components()
        .all(|component| matches!(component, std::path::Component::Normal(_)));
    let requested = if safe {
        inner.config.gui_dist.join(relative)
    } else {
        inner.config.gui_dist.join("index.html")
    };
    let file = if requested.is_file() {
        requested
    } else {
        inner.config.gui_dist.join("index.html")
    };
    match tokio::fs::read(&file).await {
        Ok(contents) => {
            let content_type = match file.extension().and_then(|value| value.to_str()) {
                Some("css") => "text/css; charset=utf-8",
                Some("js") => "text/javascript; charset=utf-8",
                Some("json") => "application/json",
                Some("svg") => "image/svg+xml",
                Some("png") => "image/png",
                Some("ico") => "image/x-icon",
                _ => "text/html; charset=utf-8",
            };
            ([(header::CONTENT_TYPE, content_type)], contents).into_response()
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => api_error(
            StatusCode::NOT_FOUND,
            "gui_not_built",
            "GUI assets are unavailable; run `npm run build` in packages/web",
        ),
        Err(error) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "gui_read_failed",
            &error.to_string(),
        ),
    }
}

async fn list_adapters() -> Response {
    Json(switchyard_adapters::built_in_registry().list()).into_response()
}

fn snapshot_fields(snapshot: Option<&Value>) -> (Vec<String>, Value) {
    let bindings = snapshot
        .and_then(|value| value.pointer("/spec/bindings"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let mut domains = Vec::new();
    fn collect_domains(value: &Value, domains: &mut Vec<String>) {
        match value {
            Value::Object(object) => {
                if object.get("kind").and_then(Value::as_str) == Some("custom_domain") {
                    if let Some(domain) = object.get("domain").and_then(Value::as_str) {
                        domains.push(domain.to_owned());
                    }
                }
                for child in object.values() {
                    collect_domains(child, domains);
                }
            }
            Value::Array(array) => {
                for child in array {
                    collect_domains(child, domains);
                }
            }
            _ => {}
        }
    }
    if let Some(snapshot) = snapshot {
        collect_domains(snapshot, &mut domains);
    }
    domains.sort();
    domains.dedup();
    (domains, bindings)
}

fn snapshot_gateway_exposure(snapshot: Option<&Value>) -> Option<GatewayExposureV1> {
    let config: router_config::RouterConfig =
        serde_json::from_value(snapshot?.pointer("/spec/hostRouter")?.clone()).ok()?;
    let interfaces = if config.spec.needs_interface_enumeration() {
        local_ip_address::list_afinet_netifas()
            .ok()?
            .into_iter()
            .map(|(_, address)| address)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let summary = config.spec.exposure_summary(&interfaces);
    Some(GatewayExposureV1 {
        mode: summary.mode,
        exposed_addresses: summary.exposed_addresses,
    })
}

fn read_manifest(root: &Path, deployment: &str) -> Option<Value> {
    let path = root
        .join(".switchyard/generated")
        .join(deployment)
        .join("manifest.json");
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn definition_hash(yaml: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(yaml.as_bytes());
    format!("{:x}", digest.finalize())
}

fn valid_deployment_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 63
        && name.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit() && index > 0
                || byte == b'-' && index > 0
        })
        && !name.ends_with('-')
}

fn definition_path(inner: &Inner, deployment: &str) -> PathBuf {
    let recorded = inner
        .store
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .deployments()
        .ok()
        .and_then(|entries| {
            entries
                .into_iter()
                .find(|entry| entry.deployment == deployment)
        })
        .and_then(|entry| entry.snapshot_json)
        .and_then(|snapshot| serde_json::from_str::<Value>(&snapshot).ok())
        .and_then(|snapshot| {
            [
                "/definitionPath",
                "/definition/path",
                "/metadata/definitionPath",
            ]
            .into_iter()
            .find_map(|pointer| {
                snapshot
                    .pointer(pointer)
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
            })
        });
    recorded
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                inner.config.project_root.join(path)
            }
        })
        .unwrap_or_else(|| {
            inner
                .config
                .project_root
                .join("deployments")
                .join(format!("{deployment}.yaml"))
        })
}

fn validation_error(message: impl Into<String>, diagnostics: Value) -> Response {
    let mut error = ApiErrorV1::new("validation_failed", message);
    error.context = Some(json!({"diagnostics": diagnostics}));
    (StatusCode::UNPROCESSABLE_ENTITY, Json(error)).into_response()
}

fn validate_definition(
    root: &Path,
    name: &str,
    yaml: &str,
) -> Result<DeploymentValidationV1, Response> {
    if !valid_deployment_name(name) {
        return Err(validation_error(
            "deployment definition is invalid",
            json!([{"code":"invalid_name","path":"metadata.name","message":"name must be a lowercase DNS label (letters, digits, and hyphens)"}]),
        ));
    }
    let directory = root.join("deployments");
    if let Err(error) = fs::create_dir_all(&directory) {
        return Err(api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "definition_write_failed",
            &error.to_string(),
        ));
    }
    let temporary = directory.join(format!(
        ".{name}.validate-{}-{}.yaml",
        std::process::id(),
        random_hex(6).unwrap_or_else(|_| "fallback".into())
    ));
    if let Err(error) = fs::write(&temporary, yaml) {
        return Err(api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "definition_write_failed",
            &error.to_string(),
        ));
    }
    let loaded = switchyard_planner::load_bundle(&temporary);
    let result = match loaded {
        Err(error) => Err(validation_error(
            "deployment definition is invalid",
            json!([{"code":"invalid_yaml","path":"$","message":error.to_string()}]),
        )),
        Ok(bundle) => match switchyard_planner::plan(&bundle) {
            Err(diagnostics) => Err(validation_error(
                "deployment definition is invalid",
                serde_json::to_value(diagnostics).unwrap_or_else(|_| json!([])),
            )),
            Ok(plan) if plan.deployment != name => Err(validation_error(
                "deployment definition name does not match the requested name",
                json!([{"code":"invalid_name","path":"metadata.name","message":format!("expected `{name}`, found `{}`", plan.deployment)}]),
            )),
            Ok(plan) => {
                let resolved = serde_json::to_value(&bundle).unwrap_or_else(|_| json!({}));
                let blocks = resolved.pointer("/spec/blocks").and_then(Value::as_object);
                let instances = resolved
                    .pointer("/spec/instances")
                    .and_then(Value::as_array);
                let expanded_service_count = instances.map_or(0, |instances| {
                    instances
                        .iter()
                        .map(|instance| {
                            instance
                                .get("block")
                                .and_then(Value::as_str)
                                .and_then(|block| blocks.and_then(|blocks| blocks.get(block)))
                                .and_then(|block| block.get("services"))
                                .and_then(Value::as_object)
                                .map_or(0, serde_json::Map::len)
                        })
                        .sum::<usize>()
                });
                Ok(DeploymentValidationV1 {
                    api_version: API_VERSION.into(),
                    name: name.into(),
                    valid: true,
                    diagnostics: Vec::new(),
                    preview: json!({
                        "expandedServiceCount": expanded_service_count,
                        "composeYaml": plan.compose_yaml,
                        "manifest": serde_json::from_str::<Value>(&plan.manifest_json).unwrap_or_else(|_| json!({})),
                        "routes": plan.route_configs.keys().collect::<Vec<_>>(),
                        "definition": resolved,
                    }),
                })
            }
        },
    };
    let _ = fs::remove_file(temporary);
    result
}

async fn deployment_definition(
    State(inner): State<Arc<Inner>>,
    AxumPath(deployment): AxumPath<String>,
) -> Response {
    if !valid_deployment_name(&deployment) {
        return api_error(
            StatusCode::NOT_FOUND,
            "deployment_definition_not_found",
            "deployment definition not found",
        );
    }
    blocking_source_response(move || {
        let path = definition_path(&inner, &deployment);
        match fs::read_to_string(&path) {
            Ok(yaml) => Json(DeploymentDefinitionV1 {
                api_version: API_VERSION.into(),
                name: deployment,
                path: fs::canonicalize(&path).unwrap_or(path),
                hash: definition_hash(&yaml),
                yaml,
            })
            .into_response(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => api_error(
                StatusCode::NOT_FOUND,
                "deployment_definition_not_found",
                "deployment definition not found",
            ),
            Err(error) => api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "definition_read_failed",
                &error.to_string(),
            ),
        }
    })
    .await
}

async fn create_deployment(
    State(inner): State<Arc<Inner>>,
    payload: Result<Json<CreateDeploymentRequestV1>, JsonRejection>,
) -> Response {
    let request = match payload {
        Ok(Json(request)) => request,
        Err(error) => {
            return api_error(StatusCode::BAD_REQUEST, "invalid_json", &error.body_text());
        }
    };
    blocking_source_response(move || create_deployment_blocking(&inner, request)).await
}

fn create_deployment_blocking(inner: &Inner, request: CreateDeploymentRequestV1) -> Response {
    let validation =
        match validate_definition(&inner.config.project_root, &request.name, &request.yaml) {
            Ok(validation) => validation,
            Err(response) => return response,
        };
    if request.validate_only {
        return Json(validation).into_response();
    }
    let target = inner
        .config
        .project_root
        .join("deployments")
        .join(format!("{}.yaml", request.name));
    let temporary = target.with_extension(format!(
        "yaml.tmp-{}-{}",
        std::process::id(),
        random_hex(6).unwrap_or_else(|_| "fallback".into())
    ));
    if let Err(error) = fs::write(&temporary, request.yaml.as_bytes()) {
        return api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "definition_write_failed",
            &error.to_string(),
        );
    }
    match fs::hard_link(&temporary, &target) {
        Ok(()) => {
            let _ = fs::remove_file(&temporary);
            (
                StatusCode::CREATED,
                Json(DeploymentDefinitionV1 {
                    api_version: API_VERSION.into(),
                    name: request.name,
                    path: fs::canonicalize(&target).unwrap_or(target),
                    hash: definition_hash(&request.yaml),
                    yaml: request.yaml,
                }),
            )
                .into_response()
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let _ = fs::remove_file(temporary);
            api_error(
                StatusCode::CONFLICT,
                "deployment_exists",
                "deployment definition already exists",
            )
        }
        Err(error) => {
            let _ = fs::remove_file(temporary);
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "definition_write_failed",
                &error.to_string(),
            )
        }
    }
}

async fn update_deployment_definition(
    State(inner): State<Arc<Inner>>,
    AxumPath(deployment): AxumPath<String>,
    payload: Result<Json<UpdateDeploymentDefinitionRequestV1>, JsonRejection>,
) -> Response {
    let request = match payload {
        Ok(Json(request)) => request,
        Err(error) => {
            return api_error(StatusCode::BAD_REQUEST, "invalid_json", &error.body_text());
        }
    };
    blocking_source_response(move || {
        update_deployment_definition_blocking(&inner, &deployment, request)
    })
    .await
}

fn update_deployment_definition_blocking(
    inner: &Inner,
    deployment: &str,
    request: UpdateDeploymentDefinitionRequestV1,
) -> Response {
    if !valid_deployment_name(deployment) {
        return api_error(
            StatusCode::NOT_FOUND,
            "deployment_definition_not_found",
            "deployment definition not found",
        );
    }
    let path = definition_path(inner, deployment);
    let current = match fs::read_to_string(&path) {
        Ok(current) => current,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return api_error(
                StatusCode::NOT_FOUND,
                "deployment_definition_not_found",
                "deployment definition not found",
            );
        }
        Err(error) => {
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "definition_read_failed",
                &error.to_string(),
            );
        }
    };
    if definition_hash(&current) != request.expected_hash {
        return api_error(
            StatusCode::CONFLICT,
            "definition_conflict",
            "deployment definition changed since it was loaded",
        );
    }
    if let Err(response) =
        validate_definition(&inner.config.project_root, deployment, &request.yaml)
    {
        return response;
    }
    match fs::read_to_string(&path) {
        Ok(latest) if definition_hash(&latest) != request.expected_hash => {
            return api_error(
                StatusCode::CONFLICT,
                "definition_conflict",
                "deployment definition changed since it was loaded",
            );
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return api_error(
                StatusCode::NOT_FOUND,
                "deployment_definition_not_found",
                "deployment definition not found",
            );
        }
        Err(error) => {
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "definition_read_failed",
                &error.to_string(),
            );
        }
    }
    let temporary = path.with_extension(format!(
        "yaml.tmp-{}-{}",
        std::process::id(),
        random_hex(6).unwrap_or_else(|_| "fallback".into())
    ));
    if let Err(error) = fs::write(&temporary, request.yaml.as_bytes()) {
        return api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "definition_write_failed",
            &error.to_string(),
        );
    }
    if let Err(error) = fs::rename(&temporary, &path) {
        let _ = fs::remove_file(temporary);
        return api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "definition_write_failed",
            &error.to_string(),
        );
    }
    Json(DeploymentDefinitionV1 {
        api_version: API_VERSION.into(),
        name: deployment.to_owned(),
        path: fs::canonicalize(&path).unwrap_or(path),
        hash: definition_hash(&request.yaml),
        yaml: request.yaml,
    })
    .into_response()
}

async fn list_deployments(State(inner): State<Arc<Inner>>) -> Response {
    let stored = match inner
        .store
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .deployments()
    {
        Ok(stored) => stored,
        Err(error) => {
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                error.code(),
                &error.to_string(),
            );
        }
    };
    let deployments = stored
        .into_iter()
        .map(|stored| {
            let snapshot = stored
                .snapshot_json
                .as_deref()
                .and_then(|value| serde_json::from_str::<Value>(value).ok());
            let manifest = read_manifest(&inner.config.project_root, &stored.deployment);
            let (custom_domains, bindings) = snapshot_fields(snapshot.as_ref());
            let gateway_exposure = snapshot_gateway_exposure(snapshot.as_ref());
            DeploymentSummaryV1 {
                name: stored.deployment,
                definition_hash: stored.definition_hash,
                resource_hash: manifest
                    .as_ref()
                    .and_then(|value| value.get("resourceHash"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                applied_at: stored.applied_at,
                last_operation: stored.last_operation.map(|operation| {
                    DeploymentOperationSummaryV1 {
                        id: operation.id,
                        kind: operation.kind,
                        status: operation.status,
                        started_at: operation.started_at,
                        finished_at: operation.finished_at,
                    }
                }),
                custom_domains,
                bindings,
                gateway_exposure,
            }
        })
        .collect();
    Json(DeploymentsV1 {
        api_version: API_VERSION.into(),
        deployments,
    })
    .into_response()
}

async fn deployment_detail(
    State(inner): State<Arc<Inner>>,
    AxumPath(deployment): AxumPath<String>,
) -> Response {
    let stored = match inner
        .store
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .deployments()
    {
        Ok(deployments) => deployments
            .into_iter()
            .find(|entry| entry.deployment == deployment),
        Err(error) => {
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                error.code(),
                &error.to_string(),
            );
        }
    };
    let Some(stored) = stored else {
        return api_error(
            StatusCode::NOT_FOUND,
            "deployment_not_found",
            "deployment not found",
        );
    };
    let snapshot = stored
        .snapshot_json
        .as_deref()
        .and_then(|value| serde_json::from_str::<Value>(value).ok());
    let manifest = read_manifest(&inner.config.project_root, &deployment);
    let source_identities = manifest
        .as_ref()
        .and_then(|value| value.get("sourceIdentities"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let resource_hash = manifest
        .as_ref()
        .and_then(|value| value.get("resourceHash"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let reconciliation = inner
        .reconciliation
        .deployments
        .iter()
        .find(|entry| entry.deployment == deployment)
        .cloned()
        .unwrap_or_else(|| switchyard_state::DeploymentReconciliation {
            deployment: deployment.clone(),
            diagnostics: Vec::new(),
        });
    let resources = match inner
        .store
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .active_resources(&deployment)
    {
        Ok(resources) => resources,
        Err(error) => {
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                error.code(),
                &error.to_string(),
            );
        }
    };
    let (custom_domains, bindings) = snapshot_fields(snapshot.as_ref());
    let gateway_exposure = snapshot_gateway_exposure(snapshot.as_ref());
    Json(DeploymentDetailV1 {
        api_version: API_VERSION.into(),
        deployment,
        definition_hash: stored.definition_hash,
        resource_hash,
        applied_at: stored.applied_at,
        snapshot,
        manifest,
        source_identities,
        reconciliation,
        resources,
        custom_domains,
        bindings,
        gateway_exposure,
    })
    .into_response()
}

/// Runs Git-subprocess and SQLite source work on the blocking pool so a slow
/// repository operation cannot stall the async workers serving other requests.
async fn blocking_source_response<F>(task: F) -> Response
where
    F: FnOnce() -> Response + Send + 'static,
{
    match tokio::task::spawn_blocking(task).await {
        Ok(response) => response,
        Err(error) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "source_task_failed",
            &error.to_string(),
        ),
    }
}

async fn list_sources(State(inner): State<Arc<Inner>>) -> Response {
    blocking_source_response(move || {
        let store = inner
            .store
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        match SourceManager::new(&inner.config.project_root).list(&store) {
            Ok(sources) => Json(sources).into_response(),
            Err(error) => source_api_error(error),
        }
    })
    .await
}

async fn register_source(
    State(inner): State<Arc<Inner>>,
    payload: Result<Json<RegisterSourceRequestV1>, JsonRejection>,
) -> Response {
    let Json(request) = match payload {
        Ok(payload) => payload,
        Err(error) => {
            return api_error(StatusCode::BAD_REQUEST, "invalid_json", &error.body_text());
        }
    };
    blocking_source_response(move || {
        let store = inner
            .store
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let manager = SourceManager::new(&inner.config.project_root);
        match manager.register_unmanaged(&store, &request.name, &request.path) {
            Ok(source) => {
                let inspection = manager.inspect(&source.path, source.requested_ref.as_deref());
                (
                    StatusCode::CREATED,
                    Json(switchyard_sources::RegisteredSourceInspection { source, inspection }),
                )
                    .into_response()
            }
            Err(error) => source_api_error(error),
        }
    })
    .await
}

async fn deregister_source(
    State(inner): State<Arc<Inner>>,
    AxumPath(name): AxumPath<String>,
) -> Response {
    blocking_source_response(move || {
        let store = inner
            .store
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        match SourceManager::new(&inner.config.project_root).deregister(&store, &name) {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(error) => source_api_error(error),
        }
    })
    .await
}

#[derive(serde::Deserialize)]
struct WorktreeQuery {
    repository: String,
}

async fn list_worktrees(
    State(inner): State<Arc<Inner>>,
    Query(query): Query<WorktreeQuery>,
) -> Response {
    blocking_source_response(move || {
        let store = inner
            .store
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let repository = match store.source(&query.repository) {
            Ok(Some(source)) => source.repository_path.unwrap_or(source.path),
            Ok(None) => {
                return api_error(
                    StatusCode::NOT_FOUND,
                    "repository_unregistered",
                    &format!("repository source `{}` is not registered", query.repository),
                );
            }
            Err(error) => {
                return api_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    error.code(),
                    &error.to_string(),
                );
            }
        };
        match SourceManager::new(&inner.config.project_root).worktrees(&repository) {
            Ok(worktrees) => Json(worktrees).into_response(),
            Err(error) => source_api_error(error),
        }
    })
    .await
}

async fn create_worktree(
    State(inner): State<Arc<Inner>>,
    payload: Result<Json<CreateWorktreeRequestV1>, JsonRejection>,
) -> Response {
    let Json(request) = match payload {
        Ok(payload) => payload,
        Err(error) => {
            return api_error(StatusCode::BAD_REQUEST, "invalid_json", &error.body_text());
        }
    };
    if request.r#ref.is_empty() {
        return api_error(
            StatusCode::BAD_REQUEST,
            "invalid_source_ref",
            "worktree ref cannot be empty",
        );
    }
    let name = request
        .name
        .unwrap_or_else(|| sanitize_source_name(&request.r#ref));
    if name.is_empty() {
        return api_error(
            StatusCode::BAD_REQUEST,
            "invalid_source_name",
            "worktree name cannot be empty",
        );
    }
    blocking_source_response(move || {
        let store = inner
            .store
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let manager = SourceManager::new(&inner.config.project_root);
        match manager.create_worktree(
            &store,
            &request.repository,
            &request.r#ref,
            &name,
            request.path.as_deref(),
        ) {
            Ok(source) => {
                let inspection = manager.inspect(&source.path, source.requested_ref.as_deref());
                (
                    StatusCode::CREATED,
                    Json(switchyard_sources::RegisteredSourceInspection { source, inspection }),
                )
                    .into_response()
            }
            Err(error) => source_api_error(error),
        }
    })
    .await
}

async fn remove_worktree(
    State(inner): State<Arc<Inner>>,
    AxumPath(name): AxumPath<String>,
    payload: Result<Json<RemoveWorktreeRequestV1>, JsonRejection>,
) -> Response {
    let request = match payload {
        Ok(Json(request)) => request,
        Err(error) => {
            return api_error(StatusCode::BAD_REQUEST, "invalid_json", &error.body_text());
        }
    };
    blocking_source_response(move || {
        let store = inner
            .store
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        match SourceManager::new(&inner.config.project_root).remove(
            &store,
            &name,
            request.allow_dirty,
        ) {
            Ok(changes) => Json(changes).into_response(),
            Err(error) => source_api_error(error),
        }
    })
    .await
}

fn sanitize_source_name(reference: &str) -> String {
    reference
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn source_api_error(error: SourceError) -> Response {
    let status = match error.code() {
        "source_not_found" | "repository_unregistered" => StatusCode::NOT_FOUND,
        "source_already_registered"
        | "source_target_exists"
        | "source_dirty"
        | "source_managed_exists" => StatusCode::CONFLICT,
        "source_io" | "state_io" | "state_sqlite" | "git_unavailable" => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
        _ => StatusCode::BAD_REQUEST,
    };
    api_error(status, error.code(), &error.to_string())
}

async fn deployment_routes(
    State(inner): State<Arc<Inner>>,
    AxumPath(deployment): AxumPath<String>,
) -> Response {
    let store = inner
        .store
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let bindings = match store.router_bindings(&deployment) {
        Ok(bindings) => bindings,
        Err(error) => {
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                error.code(),
                &error.to_string(),
            );
        }
    };
    let history = match store.route_history(&deployment) {
        Ok(history) => history,
        Err(error) => {
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                error.code(),
                &error.to_string(),
            );
        }
    };
    let bindings = bindings
        .into_iter()
        .map(|binding| RouterBindingV1 {
            router: binding.router,
            binding: binding.binding,
            desired_version: binding.desired_version,
            desired_checksum: binding.desired_checksum,
            current_version: binding.current_version,
            current_checksum: binding.current_checksum,
            previous_version: binding.previous_version,
            previous_checksum: binding.previous_checksum,
            observed_version: binding.observed_version,
            observed_checksum: binding.observed_checksum,
            status: binding.status,
            transition: serde_json::from_str(&binding.transition_json).unwrap_or(Value::Null),
            last_error_code: binding.last_error_code,
            updated_at: binding.updated_at,
        })
        .collect();
    let history = history
        .into_iter()
        .map(|entry| RouteHistoryV1 {
            sequence: entry.sequence,
            router: entry.router,
            binding: entry.binding,
            operation_id: entry.operation_id,
            version: entry.version,
            checksum: entry.checksum,
            activation_status: entry.activation_status,
            recorded_at: entry.recorded_at,
            context: serde_json::from_str(&entry.context_json).unwrap_or(Value::Null),
        })
        .collect();
    Json(DeploymentRoutesV1 {
        api_version: API_VERSION.into(),
        deployment,
        bindings,
        history,
    })
    .into_response()
}

async fn auth_middleware(
    State(inner): State<Arc<Inner>>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let path = request.uri().path();
    if path == "/gui" || path.starts_with("/gui/") {
        return next.run(request).await;
    }
    let query_token = (path.starts_with("/api/v1/operations/") && path.ends_with("/events"))
        .then(|| request.uri().query())
        .flatten()
        .and_then(|query| {
            query.split('&').find_map(|field| {
                let (name, value) = field.split_once('=')?;
                (name == "access_token").then_some(value)
            })
        });
    if let Err(response) = authenticate(&inner, request.headers(), query_token) {
        return response;
    }
    next.run(request).await
}

async fn api_not_found() -> Response {
    api_error(
        StatusCode::NOT_FOUND,
        "api_not_found",
        "API route not found",
    )
}

async fn api_method_not_allowed() -> Response {
    api_error(
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
        "HTTP method is not supported for this API route",
    )
}

async fn system_status(State(inner): State<Arc<Inner>>) -> Response {
    let operations = inner
        .operations
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    Json(DaemonStatusV1 {
        api_version: API_VERSION.into(),
        instance_id: inner.instance_id.clone(),
        pid: std::process::id(),
        active_operations: operations
            .values()
            .filter(|operation| !operation.operation.status.terminal())
            .count(),
        max_heavy_operations: inner.config.max_heavy_operations,
    })
    .into_response()
}

async fn system_shutdown(State(inner): State<Arc<Inner>>) -> Response {
    inner.shutdown.send_replace(true);
    StatusCode::ACCEPTED.into_response()
}

async fn start_command(
    State(inner): State<Arc<Inner>>,
    AxumPath(segment): AxumPath<String>,
    payload: Result<Json<CommandRequestV1>, JsonRejection>,
) -> Response {
    let request = match payload {
        Ok(Json(request)) => request,
        Err(error) => {
            return api_error(StatusCode::BAD_REQUEST, "invalid_json", &error.body_text());
        }
    };
    let Some(kind) = parse_kind(&segment) else {
        return api_error(
            StatusCode::NOT_FOUND,
            "unknown_command",
            "unknown API command",
        );
    };
    let arguments = match request.arguments(kind) {
        Ok(arguments) => arguments,
        Err(error) => return (StatusCode::BAD_REQUEST, Json(error)).into_response(),
    };
    let bundle_path = if request.bundle.is_absolute() {
        request.bundle.clone()
    } else {
        inner.config.project_root.join(&request.bundle)
    };
    let deployment = deployment_id(&bundle_path);
    match begin_operation(inner, kind, deployment, arguments, request).await {
        Ok(operation) => (StatusCode::ACCEPTED, Json(operation)).into_response(),
        Err((status, error)) => (status, Json(error)).into_response(),
    }
}

async fn begin_operation(
    inner: Arc<Inner>,
    kind: CommandKind,
    deployment: String,
    arguments: Vec<String>,
    request: CommandRequestV1,
) -> Result<OperationV1, (StatusCode, ApiErrorV1)> {
    let id = format!(
        "op-{}-{}",
        now_millis(),
        random_hex(8).map_err(internal_error)?
    );
    let lock_token = random_hex(16).map_err(internal_error)?;
    let started_at = now_millis();
    let lock = if kind.mutating() {
        let request = LockRequest {
            deployment: &deployment,
            owner: &inner.instance_id,
            pid: std::process::id(),
            process_started_at: started_at,
            token: &lock_token,
            now: started_at,
            ttl_millis: LOCK_TTL_MILLIS,
        };
        match inner
            .store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .acquire_lock(&request)
        {
            Ok(lock) => Some(lock),
            Err(StateError::LockContended { .. }) => {
                return Err((
                    StatusCode::CONFLICT,
                    ApiErrorV1::new(
                        "operation_lock_contended",
                        format!("deployment `{deployment}` already has a mutation in progress"),
                    ),
                ));
            }
            Err(error) => return Err(internal_error(error)),
        }
    } else {
        None
    };
    let operation = OperationV1 {
        api_version: API_VERSION.into(),
        id: id.clone(),
        deployment: deployment.clone(),
        kind,
        status: OperationStatusV1::Pending,
        started_at,
        finished_at: None,
        error: None,
        result: None,
    };
    if let Err(error) = inner
        .store
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .start_operation(&OperationRecord {
            id: id.clone(),
            deployment,
            kind: state_kind(kind),
            status: OperationStatus::Pending,
            started_at,
            finished_at: None,
            error: None,
        })
    {
        if let Some(lock) = lock {
            let _ = inner
                .store
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .release_lock(lock);
        }
        return Err(internal_error(error));
    }
    let (cancellation, cancellation_rx) = watch::channel(false);
    let events = Arc::new(EventLog::new());
    events.emit(&id, EventKindV1::Operation, json!({"status": "pending"}));
    inner
        .operations
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(
            id.clone(),
            RuntimeOperation {
                operation: operation.clone(),
                cancellation,
                events: events.clone(),
            },
        );
    tokio::spawn(execute_operation(
        inner,
        id,
        kind,
        arguments,
        request,
        cancellation_rx,
        events,
        lock,
    ));
    Ok(operation)
}

#[allow(clippy::too_many_arguments)]
async fn execute_operation(
    inner: Arc<Inner>,
    id: String,
    kind: CommandKind,
    arguments: Vec<String>,
    request: CommandRequestV1,
    cancellation: watch::Receiver<bool>,
    events: Arc<EventLog>,
    mut lock: Option<switchyard_state::OperationLock>,
) {
    let permit = if kind.heavy() {
        tokio::select! {
            permit = inner.heavy.acquire() => permit.ok(),
            _ = cancellation_wait(cancellation.clone()) => None,
        }
    } else {
        None
    };
    if kind.heavy() && permit.is_none() {
        finish_operation(
            &inner,
            &id,
            OperationStatusV1::Cancelled,
            None,
            Some(ApiErrorV1::new(
                "operation_cancelled",
                "operation was cancelled",
            )),
            &events,
            lock.take(),
        );
        return;
    }
    set_running(&inner, &id, &events);
    let sink = EventSink {
        operation_id: id.clone(),
        log: events.clone(),
    };
    let persistence_request = request.clone();
    let mut work = if kind == CommandKind::Bind {
        inner
            .backend
            .live_bind(request, id.clone(), cancellation.clone(), sink.clone())
            .unwrap_or_else(|| inner.backend.run(kind, arguments, cancellation, sink))
    } else {
        inner.backend.run(kind, arguments, cancellation, sink)
    };
    let mut heartbeat = tokio::time::interval(Duration::from_secs(5));
    let mut lock_lost = None;
    let mut outcome = loop {
        tokio::select! {
            outcome = &mut work => break outcome,
            _ = heartbeat.tick(), if lock.is_some() => {
                let result = inner.store.lock().unwrap_or_else(|error| error.into_inner())
                    .heartbeat_lock(lock.as_mut().expect("guarded"), now_millis(), LOCK_TTL_MILLIS);
                if let Err(error) = result {
                    lock_lost = Some(ApiErrorV1::new(error.code(), error.to_string()));
                    if let Some(operation) = inner.operations.lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()).get(&id)
                    {
                        operation.cancellation.send_replace(true);
                    }
                    break work.await;
                }
            }
        }
    };
    drop(permit);
    let successful_apply = lock_lost.is_none()
        && (matches!(
            &outcome,
            Ok(BackendOutcome::Completed(result)) if result.exit_code == 0
        ) || matches!(
            &outcome,
            Ok(BackendOutcome::LiveBinding { result, .. }) if result.exit_code == 0
        ));
    if successful_apply && matches!(kind, CommandKind::Apply | CommandKind::Bind) {
        if let Err(error) = persist_applied_snapshot(&inner, &persistence_request) {
            outcome = Err(error);
        }
    }
    match outcome {
        Ok(BackendOutcome::LiveBinding { result, attempts }) => {
            let persistence = {
                let mut store = inner
                    .store
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                attempts
                    .iter()
                    .try_for_each(|attempt| store.record_router_apply(attempt))
            };
            let error = lock_lost.or_else(|| {
                persistence
                    .err()
                    .map(|error| ApiErrorV1::new(error.code(), error.to_string()))
            });
            if let Some(error) = error {
                finish_operation(
                    &inner,
                    &id,
                    OperationStatusV1::Failed,
                    Some(result),
                    Some(error),
                    &events,
                    lock,
                );
            } else if result.exit_code == 0 {
                finish_operation(
                    &inner,
                    &id,
                    OperationStatusV1::Succeeded,
                    Some(result),
                    None,
                    &events,
                    lock,
                );
            } else {
                finish_operation(
                    &inner,
                    &id,
                    OperationStatusV1::Failed,
                    Some(result),
                    Some(ApiErrorV1::new(
                        "router_apply_failed",
                        "live binding change failed",
                    )),
                    &events,
                    lock,
                );
            }
        }
        Ok(BackendOutcome::Completed(result)) if result.exit_code == 0 && lock_lost.is_none() => {
            finish_operation(
                &inner,
                &id,
                OperationStatusV1::Succeeded,
                Some(result),
                None,
                &events,
                lock,
            )
        }
        Ok(BackendOutcome::Completed(result)) => finish_operation(
            &inner,
            &id,
            OperationStatusV1::Failed,
            Some(result),
            lock_lost.or_else(|| {
                Some(ApiErrorV1::new(
                    "command_failed",
                    "switchyard command failed",
                ))
            }),
            &events,
            lock,
        ),
        Ok(BackendOutcome::Cancelled(result)) if lock_lost.is_none() => finish_operation(
            &inner,
            &id,
            OperationStatusV1::Cancelled,
            Some(result),
            Some(ApiErrorV1::new(
                "operation_cancelled",
                "operation was cancelled",
            )),
            &events,
            lock,
        ),
        Ok(BackendOutcome::Cancelled(result)) => finish_operation(
            &inner,
            &id,
            OperationStatusV1::Failed,
            Some(result),
            lock_lost,
            &events,
            lock,
        ),
        Err(error) => finish_operation(
            &inner,
            &id,
            OperationStatusV1::Failed,
            None,
            Some(error),
            &events,
            lock,
        ),
    }
}

fn persist_applied_snapshot(inner: &Inner, request: &CommandRequestV1) -> Result<(), ApiErrorV1> {
    let bundle_path = if request.bundle.is_absolute() {
        request.bundle.clone()
    } else {
        inner.config.project_root.join(&request.bundle)
    };
    let bundle = switchyard_planner::load_bundle(&bundle_path)
        .map_err(|error| ApiErrorV1::new("bundle_load_failed", error.to_string()))?;
    let authored_plan = switchyard_planner::plan(&bundle)
        .map_err(|errors| ApiErrorV1::new("plan_failed", format_diagnostics(errors)))?;
    let resolved_path = inner
        .config
        .project_root
        .join(&authored_plan.artifact_dir)
        .join("resolved-deployment.yaml");
    let resolved = switchyard_planner::load_bundle(&resolved_path)
        .map_err(|error| ApiErrorV1::new("resolved_state_invalid", error.to_string()))?;
    let applied_plan = switchyard_planner::plan(&resolved)
        .map_err(|errors| ApiErrorV1::new("plan_failed", format_diagnostics(errors)))?;
    let snapshot = AppliedSnapshot::from_json(
        serde_json::to_value(&resolved)
            .map_err(|error| ApiErrorV1::new("snapshot_encode_failed", error.to_string()))?,
    )
    .map_err(|error| ApiErrorV1::new(error.code(), error.to_string()))?;
    inner
        .store
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .record_applied_snapshot(
            &applied_plan.deployment,
            &applied_plan.definition_hash,
            &snapshot,
            now_millis(),
        )
        .map_err(|error| ApiErrorV1::new(error.code(), error.to_string()))
}

async fn cancellation_wait(mut cancellation: watch::Receiver<bool>) {
    while !*cancellation.borrow() && cancellation.changed().await.is_ok() {}
}

fn set_running(inner: &Inner, id: &str, events: &EventLog) {
    if let Some(runtime) = inner
        .operations
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get_mut(id)
    {
        runtime.operation.status = OperationStatusV1::Running;
    }
    let _ = inner
        .store
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .update_operation(id, OperationStatus::Running, None, None);
    events.emit(id, EventKindV1::Operation, json!({"status": "running"}));
}

fn finish_operation(
    inner: &Inner,
    id: &str,
    status: OperationStatusV1,
    result: Option<CommandResultV1>,
    error: Option<ApiErrorV1>,
    events: &EventLog,
    lock: Option<switchyard_state::OperationLock>,
) {
    let finished_at = now_millis();
    if let Some(runtime) = inner
        .operations
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get_mut(id)
    {
        runtime.operation.status = status;
        runtime.operation.finished_at = Some(finished_at);
        runtime.operation.result = result;
        runtime.operation.error = error.clone();
    }
    let state_error = error.as_ref().and_then(|error| {
        StructuredContext::new(error.context.clone().unwrap_or_else(|| json!({})))
            .ok()
            .map(|context| switchyard_state::OperationError {
                code: error.code.clone(),
                context,
            })
    });
    let state_status = match status {
        OperationStatusV1::Pending => OperationStatus::Pending,
        OperationStatusV1::Running => OperationStatus::Running,
        OperationStatusV1::Succeeded => OperationStatus::Succeeded,
        OperationStatusV1::Failed => OperationStatus::Failed,
        OperationStatusV1::Cancelled => OperationStatus::Cancelled,
    };
    let store = inner
        .store
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let _ = store.update_operation(id, state_status, Some(finished_at), state_error.as_ref());
    if let Some(lock) = lock {
        let _ = store.release_lock(lock);
    }
    drop(store);
    events.emit(
        id,
        EventKindV1::Operation,
        json!({"status": status, "error": error}),
    );
    events.finish();
    retain_terminal_operation(inner, id);
    inner.active_notify.notify_waiters();
}

fn retain_terminal_operation(inner: &Inner, id: &str) {
    let evicted = {
        let mut terminal = inner
            .terminal_operations
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        terminal.push_back(id.to_owned());
        (terminal.len() > TERMINAL_OPERATION_RETENTION)
            .then(|| terminal.pop_front())
            .flatten()
    };
    if let Some(evicted) = evicted {
        inner
            .operations
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&evicted);
    }
}

async fn get_operation(
    State(inner): State<Arc<Inner>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    if let Some(operation) = inner
        .operations
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&id)
        .map(|runtime| runtime.operation.clone())
    {
        return Json(operation).into_response();
    }
    let stored = inner
        .store
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .operation(&id);
    match stored {
        Ok(Some(operation)) => match operation_from_stored(operation) {
            Ok(operation) => Json(operation).into_response(),
            Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response(),
        },
        Ok(None) => api_error(
            StatusCode::NOT_FOUND,
            "operation_not_found",
            "operation not found",
        ),
        Err(error) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            error.code(),
            &error.to_string(),
        ),
    }
}

async fn cancel_operation(
    State(inner): State<Arc<Inner>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let mut operations = inner
        .operations
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let Some(operation) = operations.get_mut(&id) else {
        return api_error(
            StatusCode::NOT_FOUND,
            "operation_not_found",
            "operation not found",
        );
    };
    if operation.operation.status.terminal() {
        return (
            StatusCode::CONFLICT,
            Json(ApiErrorV1::new(
                "operation_terminal",
                "operation has already finished",
            )),
        )
            .into_response();
    }
    operation.cancellation.send_replace(true);
    (StatusCode::ACCEPTED, Json(operation.operation.clone())).into_response()
}

async fn operation_events(
    State(inner): State<Arc<Inner>>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
) -> Response {
    let last_id = match headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
    {
        Some(value) => match value.parse::<u64>() {
            Ok(value) => value,
            Err(_) => {
                return api_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_last_event_id",
                    "Last-Event-ID must be an unsigned integer",
                );
            }
        },
        None => 0,
    };
    let Some(log) = inner
        .operations
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&id)
        .map(|operation| operation.events.clone())
    else {
        return api_error(
            StatusCode::NOT_FOUND,
            "operation_not_found",
            "operation event stream not found",
        );
    };
    let events = event_stream(log, last_id);
    Sse::new(events).into_response()
}

fn event_stream(
    log: Arc<EventLog>,
    last_id: u64,
) -> impl Stream<Item = Result<Event, Infallible>> + Send {
    stream::unfold((log, last_id), |(log, cursor)| async move {
        let event = log.next_after(cursor).await?;
        let next = event.id;
        let encoded = serde_json::to_string(&event).expect("event contract serializes");
        Some((
            Ok(Event::default()
                .id(event.id.to_string())
                .event(event.kind.as_str())
                .data(encoded)),
            (log, next),
        ))
    })
}

fn authenticate(
    inner: &Inner,
    headers: &HeaderMap,
    query_token: Option<&str>,
) -> Result<(), Response> {
    let header_token = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let authorized = header_token
        .into_iter()
        .chain(query_token)
        .any(|presented| constant_time_eq(presented.as_bytes(), inner.token.as_bytes()));
    if !authorized {
        return Err(api_error(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "missing or invalid local authentication token",
        ));
    }
    Ok(())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let max = left.len().max(right.len());
    let mut difference = left.len() ^ right.len();
    for index in 0..max {
        difference |= usize::from(*left.get(index).unwrap_or(&0) ^ *right.get(index).unwrap_or(&0));
    }
    difference == 0
}

fn api_error(status: StatusCode, code: &str, message: &str) -> Response {
    (status, Json(ApiErrorV1::new(code, message))).into_response()
}

fn internal_error(error: impl std::fmt::Display) -> (StatusCode, ApiErrorV1) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        ApiErrorV1::new("internal_error", error.to_string()),
    )
}

fn parse_kind(value: &str) -> Option<CommandKind> {
    [
        CommandKind::Validate,
        CommandKind::Plan,
        CommandKind::Apply,
        CommandKind::Bind,
        CommandKind::Status,
        CommandKind::Routes,
        CommandKind::Logs,
        CommandKind::Open,
        CommandKind::Down,
        CommandKind::Cleanup,
    ]
    .into_iter()
    .find(|kind| kind.segment() == value)
}

fn state_kind(kind: CommandKind) -> OperationKind {
    match kind {
        CommandKind::Apply => OperationKind::Apply,
        CommandKind::Bind => OperationKind::Bind,
        CommandKind::Down => OperationKind::Stop,
        CommandKind::Cleanup => OperationKind::Cleanup,
        other => OperationKind::Other(other.segment().into()),
    }
}

fn deployment_id(bundle_path: &Path) -> String {
    switchyard_planner::load_bundle(bundle_path)
        .ok()
        .and_then(|bundle| switchyard_planner::plan(&bundle).ok())
        .map_or_else(
            || {
                let path = fs::canonicalize(bundle_path).unwrap_or_else(|_| bundle_path.into());
                format!("invalid-bundle:{}", path.display())
            },
            |plan| plan.deployment,
        )
}

fn operation_from_stored(
    stored: switchyard_state::StoredOperation,
) -> Result<OperationV1, ApiErrorV1> {
    let kind = parse_kind(if stored.kind == "start" {
        "apply"
    } else {
        &stored.kind
    })
    .ok_or_else(|| ApiErrorV1::new("stored_operation_invalid", "unknown stored operation kind"))?;
    let status = match stored.status.as_str() {
        "pending" => OperationStatusV1::Pending,
        "running" => OperationStatusV1::Running,
        "succeeded" => OperationStatusV1::Succeeded,
        "failed" => OperationStatusV1::Failed,
        "cancelled" => OperationStatusV1::Cancelled,
        _ => {
            return Err(ApiErrorV1::new(
                "stored_operation_invalid",
                "unknown stored operation status",
            ));
        }
    };
    let error = stored.error_code.map(|code| ApiErrorV1 {
        code,
        message: "operation recorded a terminal error".into(),
        context: stored
            .error_context_json
            .and_then(|value| serde_json::from_str(&value).ok()),
    });
    Ok(OperationV1 {
        api_version: API_VERSION.into(),
        id: stored.id,
        deployment: stored.deployment,
        kind,
        status,
        started_at: stored.started_at,
        finished_at: stored.finished_at,
        error,
        result: None,
    })
}

async fn cancel_and_wait(inner: &Inner) {
    request_cancellation(inner);
    loop {
        let notified = inner.active_notify.notified();
        let active = inner
            .operations
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .any(|operation| !operation.operation.status.terminal());
        if !active {
            break;
        }
        notified.await;
    }
}

fn request_cancellation(inner: &Inner) {
    let operations = inner
        .operations
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    for operation in operations
        .values()
        .filter(|operation| !operation.operation.status.terminal())
    {
        operation.cancellation.send_replace(true);
    }
}

fn write_discovery(path: &Path, discovery: &DiscoveryV1) -> Result<(), DaemonError> {
    let temporary = path.with_extension(format!("json.tmp-{}", std::process::id()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&temporary)?;
    serde_json::to_writer(&mut file, discovery)
        .map_err(|error| DaemonError::Message(error.to_string()))?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn discovery_listener_is_active(path: &Path) -> bool {
    let Ok(encoded) = fs::read(path) else {
        return false;
    };
    let Ok(discovery) = serde_json::from_slice::<DiscoveryV1>(&encoded) else {
        return false;
    };
    let Ok(address) = discovery.address.parse::<SocketAddr>() else {
        return false;
    };
    if !address.ip().is_loopback() {
        return false;
    }
    crate::client::discovery_daemon_status(&discovery).is_some()
}

fn remove_discovery_if_owned(path: &Path, token: &str) -> Result<(), DaemonError> {
    let encoded = match fs::read(path) {
        Ok(encoded) => encoded,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let owned = serde_json::from_slice::<DiscoveryV1>(&encoded)
        .is_ok_and(|discovery| constant_time_eq(discovery.token.as_bytes(), token.as_bytes()));
    if owned {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn random_hex(bytes: usize) -> io::Result<String> {
    let mut random = vec![0_u8; bytes];
    OpenOptions::new()
        .read(true)
        .open("/dev/urandom")?
        .read_exact(&mut random)?;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes * 2);
    for byte in random {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(encoded)
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

fn observe_docker() -> Result<Vec<switchyard_state::OwnedResourceObservation>, io::Error> {
    let mut observations = Vec::new();
    for (kind, noun, supports_all) in [
        (switchyard_state::ResourceKind::Container, "container", true),
        (switchyard_state::ResourceKind::Image, "image", true),
        (switchyard_state::ResourceKind::Network, "network", false),
        (switchyard_state::ResourceKind::Volume, "volume", false),
    ] {
        let mut arguments = vec![noun, "list"];
        if supports_all {
            arguments.push("--all");
        }
        arguments.extend(["--filter", "label=dev.switchyard.managed=true", "--quiet"]);
        let output = std::process::Command::new("docker")
            .args(arguments)
            .output()?;
        if !output.status.success() {
            continue;
        }
        for id in String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|id| !id.is_empty())
        {
            let inspected = std::process::Command::new("docker")
                .args([noun, "inspect", id])
                .output()?;
            if !inspected.status.success() {
                continue;
            }
            let values: Vec<Value> = serde_json::from_slice(&inspected.stdout).unwrap_or_default();
            let Some(value) = values.first() else {
                continue;
            };
            let labels_value = value
                .pointer("/Config/Labels")
                .or_else(|| value.get("Labels"));
            let labels = labels_value
                .and_then(Value::as_object)
                .map(|labels| {
                    labels
                        .iter()
                        .filter_map(|(key, value)| {
                            value.as_str().map(|value| (key.clone(), value.into()))
                        })
                        .collect::<BTreeMap<_, _>>()
                })
                .unwrap_or_default();
            observations.push(switchyard_state::OwnedResourceObservation {
                kind,
                id: value
                    .get("Id")
                    .or_else(|| value.get("ID"))
                    .and_then(Value::as_str)
                    .unwrap_or(id)
                    .into(),
                name: value
                    .get("Name")
                    .and_then(Value::as_str)
                    .unwrap_or(id)
                    .trim_start_matches('/')
                    .into(),
                labels,
                state: value
                    .pointer("/State/Status")
                    .and_then(Value::as_str)
                    .map(Into::into),
            });
        }
    }
    Ok(observations)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FailingRollbackAdmin {
        applied: Mutex<Vec<String>>,
    }

    impl RouterAdmin for FailingRollbackAdmin {
        fn current_snapshot(
            &self,
            socket: &Path,
            _token: &str,
        ) -> Result<switchyard_router_admin::SnapshotIdentity, switchyard_router_admin::AdminError>
        {
            if socket == Path::new("second.socket") {
                return Err(switchyard_router_admin::AdminError::InvalidResponse(
                    "observation unavailable".into(),
                ));
            }
            Ok(switchyard_router_admin::SnapshotIdentity {
                id: "active".into(),
                version: 2,
                checksum: "active-checksum".into(),
            })
        }

        fn apply_snapshot(
            &self,
            socket: &Path,
            _token: &str,
            config: &router_config::RouterConfig,
        ) -> Result<
            switchyard_router_admin::ApplyAcknowledgement,
            switchyard_router_admin::AdminError,
        > {
            self.applied
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(socket.to_string_lossy().into_owned());
            Ok(switchyard_router_admin::ApplyAcknowledgement {
                version: config.spec.snapshot.version,
                checksum: snapshot_checksum(config),
                status: switchyard_router_admin::ActivationStatus::Activated,
            })
        }
    }

    #[test]
    fn rollback_observation_failure_is_recorded_and_remaining_targets_continue() {
        let config: router_config::RouterConfig = serde_json::from_str(include_str!(
            "../../router-config/tests/fixtures/valid/v1alpha1-minimal.json"
        ))
        .unwrap();
        let target = |id: &str, socket: &str| {
            let mut candidate = config.clone();
            candidate.spec.snapshot.version = 2;
            RouterTarget {
                id: id.into(),
                socket: socket.into(),
                old: config.clone(),
                candidate,
            }
        };
        let mut targets = vec![
            target("first", "first.socket"),
            target("second", "second.socket"),
        ];
        let observed = |id: &str| switchyard_router_admin::SnapshotIdentity {
            id: id.into(),
            version: 1,
            checksum: format!("{id}-checksum"),
        };
        let admin = FailingRollbackAdmin {
            applied: Mutex::new(Vec::new()),
        };
        let mut attempts = Vec::new();
        rollback_applied_targets(
            "demo",
            "backend",
            "operation",
            "token",
            None,
            &StructuredContext::new(json!({})).unwrap(),
            &mut targets,
            vec![(0, observed("first")), (1, observed("second"))],
            &mut attempts,
            &admin,
        );

        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0].router, "second");
        assert_eq!(
            attempts[0].error_code.as_deref(),
            Some("rollback_observation_failed")
        );
        assert_eq!(attempts[1].router, "first");
        assert_eq!(attempts[1].status, RouterApplyStatus::RolledBack);
        assert_eq!(
            *admin
                .applied
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
            vec!["first.socket"]
        );
    }

    #[test]
    fn discovery_is_private_and_token_comparison_handles_different_lengths() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("daemon.json");
        write_discovery(
            &path,
            &DiscoveryV1 {
                api_version: API_VERSION.into(),
                address: "127.0.0.1:1234".into(),
                token: "not-logged".into(),
                pid: 1,
            },
        )
        .unwrap();
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(constant_time_eq(b"same", b"same"));
        assert!(!constant_time_eq(b"short", b"longer"));
    }

    #[test]
    fn non_loopback_binding_is_refused_before_listener_startup() {
        let temp = tempfile::TempDir::new().unwrap();
        let config = DaemonConfig {
            project_root: temp.path().into(),
            bind: "0.0.0.0:0".parse().unwrap(),
            max_heavy_operations: 1,
            cli_program: "unused".into(),
            gui_dist: temp.path().join("dist"),
        };
        struct Unused;
        impl OperationBackend for Unused {
            fn run(
                &self,
                _kind: CommandKind,
                _arguments: Vec<String>,
                _cancellation: watch::Receiver<bool>,
                _events: EventSink,
            ) -> Pin<Box<dyn Future<Output = Result<BackendOutcome, ApiErrorV1>> + Send>>
            {
                unreachable!()
            }
        }
        assert!(matches!(
            prepare(config, Arc::new(Unused), Vec::new()),
            Err(DaemonError::InvalidConfiguration(_))
        ));
    }
}
