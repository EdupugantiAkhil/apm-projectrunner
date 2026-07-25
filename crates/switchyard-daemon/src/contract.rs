//! Framework-neutral version 1 control-plane API contract.

use std::{
    collections::BTreeMap,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use switchyard_state::DeploymentReconciliation;

/// Stable URL prefix for this contract generation.
pub const API_V1_PREFIX: &str = "/api/v1";
/// Contract identifier carried in discovery and response bodies.
pub const API_VERSION: &str = "v1";

/// An existing CLI operation exposed through the daemon.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommandKind {
    Validate,
    Plan,
    Apply,
    Bind,
    Status,
    Routes,
    Logs,
    Open,
    Down,
    Cleanup,
    RunAction,
}

impl CommandKind {
    /// API path segment for this command.
    pub const fn segment(self) -> &'static str {
        match self {
            Self::Validate => "validate",
            Self::Plan => "plan",
            Self::Apply => "apply",
            Self::Bind => "bind",
            Self::Status => "status",
            Self::Routes => "routes",
            Self::Logs => "logs",
            Self::Open => "open",
            Self::Down => "down",
            Self::Cleanup => "cleanup",
            Self::RunAction => "run-action",
        }
    }

    /// Whether the command changes deployment state.
    pub const fn mutating(self) -> bool {
        matches!(
            self,
            Self::Apply | Self::Bind | Self::Open | Self::Down | Self::Cleanup
        )
    }

    /// Whether the command consumes a global heavy-operation permit.
    pub const fn heavy(self) -> bool {
        matches!(self, Self::Apply)
    }
}

/// Version 1 command request. Unused optional fields must be omitted.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandRequestV1 {
    pub bundle: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consumer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition: Option<TransitionPolicyV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui: Option<String>,
    #[serde(default)]
    pub routes: bool,
    #[serde(default)]
    pub confirmed: bool,
}

/// Existing-connection behavior requested for a live binding change.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "strategy",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum TransitionPolicyV1 {
    Close,
    Drain { timeout_ms: u64 },
    Pin,
}

impl CommandRequestV1 {
    /// Converts a typed request to the existing script-stable CLI argument surface.
    pub fn arguments(&self, kind: CommandKind) -> Result<Vec<String>, ApiErrorV1> {
        let bundle = self.bundle.to_string_lossy().into_owned();
        let required = |value: &Option<String>, field: &'static str| {
            value
                .clone()
                .ok_or_else(|| ApiErrorV1::new("invalid_request", format!("`{field}` is required")))
        };
        let arguments = match kind {
            CommandKind::Validate => vec!["validate".into(), bundle],
            CommandKind::Plan => vec!["plan".into(), bundle],
            CommandKind::Apply => vec!["up".into(), bundle],
            CommandKind::Bind => {
                let mut args = vec![
                    "bind".into(),
                    bundle,
                    required(&self.consumer, "consumer")?,
                    required(&self.group, "group")?,
                ];
                match self.transition {
                    None => {}
                    Some(TransitionPolicyV1::Close) => {
                        args.extend(["--transition".into(), "close".into()])
                    }
                    Some(TransitionPolicyV1::Pin) => {
                        args.extend(["--transition".into(), "pin".into()])
                    }
                    Some(TransitionPolicyV1::Drain { timeout_ms }) => args.extend([
                        "--transition".into(),
                        "drain".into(),
                        "--drain-timeout-ms".into(),
                        timeout_ms.to_string(),
                    ]),
                }
                args
            }
            CommandKind::Status => {
                let mut args = vec!["status".into(), bundle];
                if self.routes {
                    args.push("--routes".into());
                }
                args
            }
            CommandKind::Routes => vec!["routes".into(), bundle],
            CommandKind::Logs => {
                let mut args = vec!["logs".into(), bundle];
                if let Some(target) = &self.target {
                    args.push(target.clone());
                }
                args
            }
            CommandKind::Open => vec!["open".into(), bundle, required(&self.ui, "ui")?],
            CommandKind::Down => vec!["down".into(), bundle],
            CommandKind::Cleanup => {
                let mut args = vec!["cleanup".into(), bundle];
                if self.confirmed {
                    args.push("--yes".into());
                }
                args
            }
            CommandKind::RunAction => {
                return Err(ApiErrorV1::new(
                    "invalid_request",
                    "run actions use the dedicated run-action endpoint",
                ));
            }
        };
        Ok(arguments)
    }
}

/// Durable operation state returned by create, inspect, and cancellation endpoints.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationV1 {
    pub api_version: String,
    pub id: String,
    pub deployment: String,
    pub kind: CommandKind,
    /// True when the operation stops or deletes runtime state.
    pub destructive: bool,
    pub status: OperationStatusV1,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub error: Option<ApiErrorV1>,
    /// Present while this daemon still retains the script-compatible command result.
    pub result: Option<CommandResultV1>,
}

/// One newest-first page of durable operation records.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationsV1 {
    pub api_version: String,
    pub operations: Vec<OperationV1>,
    /// Stable operation ID to pass as `cursor` for the next older page.
    pub next_cursor: Option<String>,
}

/// Versioned terminal/active operation states.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatusV1 {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl OperationStatusV1 {
    pub const fn terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

/// Captured script-compatible output from the existing CLI implementation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandResultV1 {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Stable machine-readable API failure envelope.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiErrorV1 {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<Value>,
}

impl ApiErrorV1 {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            context: None,
        }
    }
}

/// Authenticated daemon health response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonStatusV1 {
    pub api_version: String,
    pub instance_id: String,
    pub pid: u32,
    pub active_operations: usize,
    pub max_heavy_operations: usize,
}

/// Identity of the project folder owned by this daemon.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectV1 {
    pub api_version: String,
    pub name: String,
    pub root: PathBuf,
    pub registered: bool,
}

/// Structured or shell body of one project run action.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum RunActionDefinitionV1 {
    Structured {
        command: switchyard_run_actions::StructuredCommand,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        overlays: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        variation: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        set: Vec<String>,
    },
    Shell {
        command: String,
    },
}

/// One named project run action.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunActionV1 {
    pub name: String,
    pub description: Option<String>,
    #[serde(flatten)]
    pub definition: RunActionDefinitionV1,
}

/// Project run-action listing and project-local shell acknowledgement state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunActionsV1 {
    pub api_version: String,
    pub actions: Vec<RunActionV1>,
    pub shell_notice_acknowledged: bool,
}

/// Create or replacement payload. Shell-shaped payloads are represented so the server can
/// reject browser shell authoring with a stable domain error rather than a JSON parse error.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum PutRunActionRequestV1 {
    Structured {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        command: switchyard_run_actions::StructuredCommand,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        overlays: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        variation: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        set: Vec<String>,
    },
    Shell {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        command: String,
    },
}

/// Preview-or-execute request for one existing run action.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecuteRunActionRequestV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle: Option<PathBuf>,
    #[serde(default)]
    pub confirmed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview_hash: Option<String>,
    #[serde(default)]
    pub acknowledge_shell_warning: bool,
}

/// Target context displayed before a run action executes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum RunActionTargetV1 {
    Deployment { name: String, bundle: PathBuf },
    ProjectShellContext { root: PathBuf },
}

/// Exact process payload displayed before a run action executes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum RunActionExecutionV1 {
    Structured { argv: Vec<String> },
    Shell { command: String },
}

/// Hash-bound confirmation preview. The hash must be returned unchanged to execute.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunActionPreviewV1 {
    pub api_version: String,
    pub name: String,
    pub description: String,
    pub target: RunActionTargetV1,
    pub execution: RunActionExecutionV1,
    pub shell_notice_acknowledged: bool,
    pub shell_acknowledgement_required: bool,
    pub preview_hash: String,
}

/// Latest operation fields shown beside a deployment list entry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentOperationSummaryV1 {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
}

/// Compact deployment state for the GUI rail.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentSummaryV1 {
    pub name: String,
    pub definition_hash: Option<String>,
    pub resource_hash: Option<String>,
    pub applied_at: Option<i64>,
    pub last_operation: Option<DeploymentOperationSummaryV1>,
    pub custom_domains: Vec<String>,
    pub bindings: Value,
    pub gateway_exposure: Option<GatewayExposureV1>,
    pub mdns_publication: Option<MdnsPublicationV1>,
    pub tailscale_publication: Option<TailscalePublicationV1>,
}

/// Effective host-gateway listener exposure for deployment inspection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayExposureV1 {
    pub mode: router_config::GatewayExposureMode,
    pub exposed_addresses: Vec<SocketAddr>,
}

/// CLI-owned mDNS publisher state and its most recent LAN preflight report.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MdnsPublicationV1 {
    pub publications: Vec<MdnsPublishedNameV1>,
    pub checks: Vec<MdnsCheckV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MdnsPublishedNameV1 {
    pub name: String,
    pub address: IpAddr,
    pub pid: u32,
    pub status: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MdnsCheckV1 {
    pub name: String,
    pub outcome: String,
    pub detail: String,
}

/// CLI-owned, advisory tailnet reachability record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TailscalePublicationV1 {
    pub scope: String,
    pub names: Vec<String>,
    pub addresses: Vec<String>,
    pub ports: Vec<u16>,
    pub checks: Vec<MdnsCheckV1>,
}

/// Versioned deployment-list response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentsV1 {
    pub api_version: String,
    pub deployments: Vec<DeploymentSummaryV1>,
}

/// Authored deployment definition returned to editors.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentDefinitionV1 {
    pub api_version: String,
    pub name: String,
    pub path: PathBuf,
    pub yaml: String,
    pub hash: String,
}

/// Create or validate-only request for an authored deployment definition.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateDeploymentRequestV1 {
    pub name: String,
    pub yaml: String,
    #[serde(default)]
    pub validate_only: bool,
}

/// Optimistic replacement request for an authored deployment definition.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateDeploymentDefinitionRequestV1 {
    pub yaml: String,
    pub expected_hash: String,
}

/// Successful validation, with a planner-derived resource preview for builders.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentValidationV1 {
    pub api_version: String,
    pub name: String,
    pub valid: bool,
    pub diagnostics: Vec<switchyard_planner::Diagnostic>,
    pub preview: Value,
}

/// Applied deployment state plus the daemon's live reconciliation projection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentDetailV1 {
    pub api_version: String,
    pub deployment: String,
    pub definition_hash: Option<String>,
    pub resource_hash: Option<String>,
    pub applied_at: Option<i64>,
    pub snapshot: Option<Value>,
    pub manifest: Option<Value>,
    pub source_identities: Value,
    pub reconciliation: DeploymentReconciliation,
    pub resources: Vec<switchyard_state::OwnedResourceObservation>,
    pub custom_domains: Vec<String>,
    pub bindings: Value,
    pub gateway_exposure: Option<GatewayExposureV1>,
    pub mdns_publication: Option<MdnsPublicationV1>,
    pub tailscale_publication: Option<TailscalePublicationV1>,
}

/// Project-local daemon discovery document. Its containing file is mode 0600.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryV1 {
    pub api_version: String,
    pub address: String,
    pub token: String,
    pub pid: u32,
}

/// One resumable SSE record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventV1 {
    pub id: u64,
    pub operation_id: String,
    pub kind: EventKindV1,
    pub timestamp: i64,
    pub data: Value,
}

/// Stable SSE event names shared by operation, build, health, route, and log observers.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKindV1 {
    Operation,
    Build,
    Health,
    Route,
    Log,
}

/// Desired/applied/observed version state for one router binding.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterBindingV1 {
    pub router: String,
    pub binding: String,
    pub desired_version: Option<i64>,
    pub desired_checksum: Option<String>,
    pub current_version: Option<i64>,
    pub current_checksum: Option<String>,
    pub previous_version: Option<i64>,
    pub previous_checksum: Option<String>,
    pub observed_version: Option<i64>,
    pub observed_checksum: Option<String>,
    pub status: String,
    pub transition: Value,
    pub last_error_code: Option<String>,
    pub updated_at: i64,
}

/// One immutable route apply, rejection, or rollback history record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteHistoryV1 {
    pub sequence: i64,
    pub router: Option<String>,
    pub binding: Option<String>,
    pub operation_id: Option<String>,
    pub version: i64,
    pub checksum: String,
    pub activation_status: String,
    pub recorded_at: i64,
    pub context: Value,
}

/// Route version visibility and append-only history for one deployment.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentRoutesV1 {
    pub api_version: String,
    pub deployment: String,
    pub bindings: Vec<RouterBindingV1>,
    pub history: Vec<RouteHistoryV1>,
}

/// Origin identity required to disambiguate profiles with the same name.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum ProfileOriginV1 {
    Project,
    ImportedFromSource {
        source: String,
        commit: Option<String>,
    },
    DiscoveredInSource {
        source: String,
        commit: Option<String>,
    },
}

/// Trust state derived by the shared profile operations layer.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileTrustV1 {
    Trusted,
    Imported,
    Changed,
    NotImported,
}

/// Execution adapter family used by one profile service.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileAdapterKindV1 {
    Container,
    Script,
    ProcessCompose,
}

/// One service declared by a startup profile.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileServiceV1 {
    pub name: String,
    pub adapter_kind: ProfileAdapterKindV1,
}

/// One project-local, imported, or source-local startup profile.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileV1 {
    pub api_version: String,
    pub name: String,
    pub deployment: String,
    pub origin: ProfileOriginV1,
    pub trust: ProfileTrustV1,
    pub shadowed: bool,
    pub services: Vec<ProfileServiceV1>,
}

/// Source manifest diagnostic retained without hiding valid profiles from other sources.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSourceErrorV1 {
    pub source: String,
    pub message: String,
}

/// Project-wide profile library response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfilesV1 {
    pub api_version: String,
    pub profiles: Vec<ProfileV1>,
    pub source_errors: Vec<ProfileSourceErrorV1>,
}

/// Flat origin kind used in profile-selection query parameters.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileOriginKindV1 {
    Project,
    ImportedFromSource,
    DiscoveredInSource,
}

/// Query used to select one profile definition from the project-wide library.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileSelectorV1 {
    pub deployment: String,
    pub origin: ProfileOriginKindV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// Expanded planner block for one selected startup profile.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileDefinitionV1 {
    pub api_version: String,
    pub name: String,
    pub deployment: String,
    pub origin: ProfileOriginV1,
    pub trust: ProfileTrustV1,
    pub definition: Value,
}

/// Verbatim manifest that must be reviewed before importing source-local content.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileManifestReviewV1 {
    pub api_version: String,
    pub source: String,
    pub manifest: String,
    pub review_hash: String,
}

/// Request to validate a selected profile against a registered checkout.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidateProfileRequestV1 {
    pub deployment: String,
    pub origin: ProfileOriginV1,
    pub checkout: String,
    #[serde(default)]
    pub target_deployment: Option<String>,
    #[serde(default)]
    pub instance_name: Option<String>,
    #[serde(default)]
    pub device: Option<String>,
    #[serde(default)]
    pub parameters: Option<BTreeMap<String, String>>,
}

/// Profile service topology shown before an instance is authored.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileServicePreviewV1 {
    pub name: String,
    pub ports: Vec<u16>,
    pub volumes: Vec<ProfileVolumePreviewV1>,
}

/// Profile volume topology shown before an instance is authored.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileVolumePreviewV1 {
    pub name: String,
    pub target: PathBuf,
    pub read_only: bool,
}

/// Planner-derived expansion report for profile/checkout validation.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileValidationV1 {
    pub api_version: String,
    pub name: String,
    pub deployment: String,
    pub checkout: String,
    pub valid: bool,
    pub expanded_services: Vec<String>,
    pub services: Vec<ProfileServicePreviewV1>,
    pub diagnostics: Vec<switchyard_planner::Diagnostic>,
    pub error: Option<String>,
    pub draft: Option<String>,
}

/// Import request proving which verbatim source manifest the user reviewed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportProfileRequestV1 {
    pub source: String,
    pub reviewed_manifest_hash: String,
}

/// Request to register an existing path without taking ownership.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegisterSourceRequestV1 {
    pub name: String,
    pub path: PathBuf,
}

/// Request to register a remote machine reachable through the user's SSH configuration.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegisterDeviceRequestV1 {
    pub name: String,
    pub host: String,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    pub user: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<PathBuf>,
}

const fn default_ssh_port() -> u16 {
    22
}

/// Device origin. `local` is implicit and cannot be removed.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceKindV1 {
    Local,
    Ssh,
}

/// SSH reachability derived independently from runtime eligibility.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceReachabilityV1 {
    Unchecked,
    Reachable,
    Unreachable,
    AuthFailed,
}

/// Whether Switchyard can execute containers on a device.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceEligibilityV1 {
    Eligible,
    Ineligible,
}

/// One authored instance currently placed on a device.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevicePlacementV1 {
    pub deployment: String,
    pub instance: String,
}

/// Device registration, separate reachability and eligibility, and authored placements.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceV1 {
    pub name: String,
    pub kind: DeviceKindV1,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub identity_file: Option<PathBuf>,
    pub created_at: Option<i64>,
    pub last_checked_at: Option<i64>,
    pub last_check_status: switchyard_state::DeviceCheckStatus,
    pub last_check_detail: Option<String>,
    pub reachability: DeviceReachabilityV1,
    pub eligibility: DeviceEligibilityV1,
    pub eligibility_reason: String,
    pub placed_instances: Vec<DevicePlacementV1>,
}

/// Request to create a managed linked worktree.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateWorktreeRequestV1 {
    pub repository: String,
    pub r#ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Explicit destructive confirmation for dirty managed-source removal.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoveWorktreeRequestV1 {
    #[serde(default)]
    pub allow_dirty: bool,
}

/// Registered source with live-derived identity and Git state.
pub type SourceV1 = switchyard_sources::RegisteredSourceInspection;

/// Live worktree inspection entry.
pub type WorktreeV1 = switchyard_sources::WorktreeInspection;

impl EventKindV1 {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Operation => "operation",
            Self::Build => "build",
            Self::Health => "health",
            Self::Route => "route",
            Self::Log => "log",
        }
    }
}
