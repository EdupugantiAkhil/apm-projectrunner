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
    extract::{Path as AxumPath, State, rejection::JsonRejection},
    http::{HeaderMap, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response, Sse, sse::Event},
    routing::{get, post},
};
use futures_util::{Stream, stream};
use serde_json::{Value, json};
use switchyard_state::{
    GeneratedManifest, LockRequest, OperationKind, OperationRecord, OperationStatus,
    ReconciliationInput, ReconciliationReport, StateError, StateStore, StructuredContext,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    net::TcpListener,
    process::Command,
    sync::{Notify, Semaphore, watch},
};

use crate::contract::{
    API_VERSION, ApiErrorV1, CommandKind, CommandRequestV1, CommandResultV1, DaemonStatusV1,
    DiscoveryV1, EventKindV1, EventV1, OperationStatusV1, OperationV1,
};

const LOCK_TTL_MILLIS: i64 = 15_000;
const EVENT_CAPACITY: usize = 2_048;

/// Configuration for one daemon instance.
#[derive(Clone, Debug)]
pub struct DaemonConfig {
    pub project_root: PathBuf,
    pub bind: SocketAddr,
    pub max_heavy_operations: usize,
    pub cli_program: PathBuf,
}

impl DaemonConfig {
    pub fn new(project_root: PathBuf, cli_program: PathBuf) -> Self {
        Self {
            project_root,
            bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            max_heavy_operations: 2,
            cli_program,
        }
    }
}

/// Outcome produced by an operation backend.
#[derive(Clone, Debug)]
pub enum BackendOutcome {
    Completed(CommandResultV1),
    Cancelled(CommandResultV1),
}

/// Injectable, Docker-free operation backend used by the server and integration tests.
pub trait OperationBackend: Send + Sync + 'static {
    fn run(
        &self,
        kind: CommandKind,
        arguments: Vec<String>,
        cancellation: watch::Receiver<bool>,
        events: EventSink,
    ) -> Pin<Box<dyn Future<Output = Result<BackendOutcome, ApiErrorV1>> + Send>>;
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
    ) -> Pin<Box<dyn Future<Output = Result<BackendOutcome, ApiErrorV1>> + Send>> {
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
    heavy: Semaphore,
    backend: Arc<dyn OperationBackend>,
    shutdown: watch::Sender<bool>,
    active_notify: Notify,
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
    let prepared = prepare(config.clone(), backend)?;
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
    let resources = observe_docker().unwrap_or_default();
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
        heavy: Semaphore::new(config.max_heavy_operations),
        backend,
        shutdown: shutdown.clone(),
        active_notify: Notify::new(),
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
#[doc(hidden)]
pub fn api_for_tests(
    config: DaemonConfig,
    backend: Arc<dyn OperationBackend>,
) -> Result<(Router, String, ReconciliationReport), DaemonError> {
    let prepared = prepare(config, backend)?;
    Ok((
        routes(prepared.inner),
        prepared.token,
        prepared.reconciliation,
    ))
}

fn routes(inner: Arc<Inner>) -> Router {
    Router::new()
        .route("/api/v1/system/status", get(system_status))
        .route("/api/v1/system/shutdown", post(system_shutdown))
        .route("/api/v1/commands/{kind}", post(start_command))
        .route("/api/v1/operations/{id}", get(get_operation))
        .route("/api/v1/operations/{id}/cancel", post(cancel_operation))
        .route("/api/v1/operations/{id}/events", get(operation_events))
        .fallback(api_not_found)
        .method_not_allowed_fallback(api_method_not_allowed)
        .with_state(inner.clone())
        .layer(middleware::from_fn_with_state(inner, auth_middleware))
}

async fn auth_middleware(
    State(inner): State<Arc<Inner>>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    if let Err(response) = authenticate(&inner, request.headers()) {
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

async fn system_status(State(inner): State<Arc<Inner>>, headers: HeaderMap) -> Response {
    if let Err(response) = authenticate(&inner, &headers) {
        return response;
    }
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

async fn system_shutdown(State(inner): State<Arc<Inner>>, headers: HeaderMap) -> Response {
    if let Err(response) = authenticate(&inner, &headers) {
        return response;
    }
    inner.shutdown.send_replace(true);
    StatusCode::ACCEPTED.into_response()
}

async fn start_command(
    State(inner): State<Arc<Inner>>,
    AxumPath(segment): AxumPath<String>,
    headers: HeaderMap,
    payload: Result<Json<CommandRequestV1>, JsonRejection>,
) -> Response {
    if let Err(response) = authenticate(&inner, &headers) {
        return response;
    }
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
    match begin_operation(inner, kind, deployment, arguments).await {
        Ok(operation) => (StatusCode::ACCEPTED, Json(operation)).into_response(),
        Err((status, error)) => (status, Json(error)).into_response(),
    }
}

async fn begin_operation(
    inner: Arc<Inner>,
    kind: CommandKind,
    deployment: String,
    arguments: Vec<String>,
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
        cancellation_rx,
        events,
        lock,
    ));
    Ok(operation)
}

async fn execute_operation(
    inner: Arc<Inner>,
    id: String,
    kind: CommandKind,
    arguments: Vec<String>,
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
    let mut work = inner.backend.run(kind, arguments, cancellation, sink);
    let mut heartbeat = tokio::time::interval(Duration::from_secs(5));
    let outcome = loop {
        tokio::select! {
            outcome = &mut work => break outcome,
            _ = heartbeat.tick(), if lock.is_some() => {
                let result = inner.store.lock().unwrap_or_else(|error| error.into_inner())
                    .heartbeat_lock(lock.as_mut().expect("guarded"), now_millis(), LOCK_TTL_MILLIS);
                if let Err(error) = result {
                    break Err(ApiErrorV1::new(error.code(), error.to_string()));
                }
            }
        }
    };
    drop(permit);
    match outcome {
        Ok(BackendOutcome::Completed(result)) if result.exit_code == 0 => finish_operation(
            &inner,
            &id,
            OperationStatusV1::Succeeded,
            Some(result),
            None,
            &events,
            lock,
        ),
        Ok(BackendOutcome::Completed(result)) => finish_operation(
            &inner,
            &id,
            OperationStatusV1::Failed,
            Some(result),
            Some(ApiErrorV1::new(
                "command_failed",
                "switchyard command failed",
            )),
            &events,
            lock,
        ),
        Ok(BackendOutcome::Cancelled(result)) => finish_operation(
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
    inner.active_notify.notify_waiters();
}

async fn get_operation(
    State(inner): State<Arc<Inner>>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = authenticate(&inner, &headers) {
        return response;
    }
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
    headers: HeaderMap,
) -> Response {
    if let Err(response) = authenticate(&inner, &headers) {
        return response;
    }
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
    if let Err(response) = authenticate(&inner, &headers) {
        return response;
    }
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

fn authenticate(inner: &Inner, headers: &HeaderMap) -> Result<(), Response> {
    let presented = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default();
    if !constant_time_eq(presented.as_bytes(), inner.token.as_bytes()) {
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
    address.ip().is_loopback()
        && std::net::TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok()
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
            prepare(config, Arc::new(Unused)),
            Err(DaemonError::InvalidConfiguration(_))
        ));
    }
}
