//! Typed synchronous client for the local Switchyard router administration channel.

#![cfg(unix)]

use std::{
    fmt, io,
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::Duration,
};

use router_config::RouterConfig;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Default administration request timeout.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Router-side snapshot identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotIdentity {
    pub id: String,
    pub version: u64,
    pub checksum: String,
}

/// Successful apply acknowledgement returned by the router.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyAcknowledgement {
    pub version: u64,
    pub checksum: String,
    pub status: ActivationStatus,
}

/// Router activation outcome.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivationStatus {
    Activated,
    RejectedStale,
}

/// Administration-channel failure. Rejection details never contain the request token.
#[derive(Debug)]
pub enum AdminError {
    Io(io::Error),
    InvalidResponse(String),
    Rejected {
        code: String,
        message: String,
        details: Value,
    },
}

impl AdminError {
    /// Stable router rejection code, when the request reached the router.
    pub fn rejection_code(&self) -> Option<&str> {
        match self {
            Self::Rejected { code, .. } => Some(code),
            Self::Io(_) | Self::InvalidResponse(_) => None,
        }
    }

    /// Secret-safe structured router rejection details.
    pub fn details(&self) -> Option<&Value> {
        match self {
            Self::Rejected { details, .. } => Some(details),
            Self::Io(_) | Self::InvalidResponse(_) => None,
        }
    }
}

impl fmt::Display for AdminError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "router administration failed: {error}"),
            Self::InvalidResponse(message) => {
                write!(formatter, "router returned an invalid response: {message}")
            }
            Self::Rejected { code, message, .. } => {
                write!(formatter, "router rejected {code}: {message}")
            }
        }
    }
}

impl std::error::Error for AdminError {}

impl From<io::Error> for AdminError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Applies a complete immutable snapshot and decodes its acknowledgement.
pub fn apply_snapshot(
    socket_path: &Path,
    token: &str,
    config: &RouterConfig,
) -> Result<ApplyAcknowledgement, AdminError> {
    apply_snapshot_with_timeout(socket_path, token, config, DEFAULT_TIMEOUT)
}

/// Applies a snapshot with an explicit read/write timeout.
pub fn apply_snapshot_with_timeout(
    socket_path: &Path,
    token: &str,
    config: &RouterConfig,
    timeout: Duration,
) -> Result<ApplyAcknowledgement, AdminError> {
    let value = request(
        socket_path,
        &json!({"token": token, "operation": "apply", "config": config}),
        timeout,
    )?;
    serde_json::from_value(value).map_err(|error| AdminError::InvalidResponse(error.to_string()))
}

/// Returns the router's current snapshot identity.
pub fn current_snapshot(socket_path: &Path, token: &str) -> Result<SnapshotIdentity, AdminError> {
    let value = request(
        socket_path,
        &json!({"token": token, "operation": "current-version"}),
        DEFAULT_TIMEOUT,
    )?;
    serde_json::from_value(value).map_err(|error| AdminError::InvalidResponse(error.to_string()))
}

/// Returns the route-inspection response without weakening its forward compatibility.
pub fn inspect_routes(socket_path: &Path, token: &str) -> Result<Value, AdminError> {
    request(
        socket_path,
        &json!({"token": token, "operation": "routes"}),
        DEFAULT_TIMEOUT,
    )
}

fn request(socket_path: &Path, request: &Value, timeout: Duration) -> Result<Value, AdminError> {
    let mut stream = UnixStream::connect(socket_path)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    serde_json::to_writer(&mut stream, request)
        .map_err(|error| AdminError::InvalidResponse(error.to_string()))?;
    stream.write_all(b"\n")?;
    stream.flush()?;

    let mut response = String::new();
    BufReader::new(stream).read_line(&mut response)?;
    let value: Value = serde_json::from_str(&response)
        .map_err(|error| AdminError::InvalidResponse(error.to_string()))?;
    if value.get("ok").and_then(Value::as_bool) == Some(false) {
        let error = value.get("error").cloned().unwrap_or(Value::Null);
        return Err(AdminError::Rejected {
            code: error
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned(),
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("router rejected the request")
                .to_owned(),
            details: error,
        });
    }
    value
        .get("result")
        .cloned()
        .ok_or_else(|| AdminError::InvalidResponse("missing result".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejection_display_does_not_include_details() {
        let error = AdminError::Rejected {
            code: "stale_snapshot".into(),
            message: "snapshot is old".into(),
            details: json!({"token": "must-not-render"}),
        };
        assert_eq!(
            error.to_string(),
            "router rejected stale_snapshot: snapshot is old"
        );
    }
}
