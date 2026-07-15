//! Framework-neutral version 1 control-plane API contract.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui: Option<String>,
    #[serde(default)]
    pub routes: bool,
    #[serde(default)]
    pub confirmed: bool,
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
            CommandKind::Bind => vec![
                "bind".into(),
                bundle,
                required(&self.consumer, "consumer")?,
                required(&self.group, "group")?,
            ],
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
