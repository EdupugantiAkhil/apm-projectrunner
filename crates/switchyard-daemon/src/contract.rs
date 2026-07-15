//! Framework-neutral version 1 control-plane API contract.

use std::{
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
    pub status: OperationStatusV1,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub error: Option<ApiErrorV1>,
    /// Present while this daemon still retains the script-compatible command result.
    pub result: Option<CommandResultV1>,
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

/// Request to register an existing path without taking ownership.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegisterSourceRequestV1 {
    pub name: String,
    pub path: PathBuf,
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
