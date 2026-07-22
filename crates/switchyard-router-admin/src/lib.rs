//! Typed synchronous client for the local Switchyard router administration channel.

#![cfg(unix)]

use std::{
    fmt, io,
    io::{BufRead, BufReader, Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use router_config::RouterConfig;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Default administration request timeout.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_COMMAND_OUTPUT_BYTES: u64 = 2 * 1024 * 1024;
const MANAGED_LABEL: &str = "dev.switchyard.managed";
const DEPLOYMENT_LABEL: &str = "dev.switchyard.deployment";
const INSTANCE_LABEL: &str = "dev.switchyard.instance";
const RESOURCE_HASH_LABEL: &str = "dev.switchyard.resource-hash";

/// A verified route to one router administration socket.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdminEndpoint {
    /// An owner-only socket directly reachable from the host.
    Unix(PathBuf),
    /// An owner-only socket inside an exact Switchyard-owned Docker container.
    DockerExec {
        container: String,
        socket_path: PathBuf,
        deployment: String,
        instance: String,
        resource_hash: String,
    },
}

impl AdminEndpoint {
    pub fn unix(path: impl Into<PathBuf>) -> Self {
        Self::Unix(path.into())
    }

    pub fn docker_exec(
        container: impl Into<String>,
        socket_path: impl Into<PathBuf>,
        deployment: impl Into<String>,
        instance: impl Into<String>,
        resource_hash: impl Into<String>,
    ) -> Self {
        Self::DockerExec {
            container: container.into(),
            socket_path: socket_path.into(),
            deployment: deployment.into(),
            instance: instance.into(),
            resource_hash: resource_hash.into(),
        }
    }
}

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
    Endpoint {
        code: &'static str,
        message: String,
    },
    InvalidResponse(String),
    Rejected {
        code: String,
        message: String,
        details: Value,
    },
}

impl AdminError {
    /// Stable local endpoint failure code, when the router was not safely reached.
    pub fn endpoint_code(&self) -> Option<&'static str> {
        match self {
            Self::Endpoint { code, .. } => Some(code),
            Self::Io(_) | Self::InvalidResponse(_) | Self::Rejected { .. } => None,
        }
    }

    /// Stable router rejection code, when the request reached the router.
    pub fn rejection_code(&self) -> Option<&str> {
        match self {
            Self::Rejected { code, .. } => Some(code),
            Self::Io(_) | Self::Endpoint { .. } | Self::InvalidResponse(_) => None,
        }
    }

    /// Secret-safe structured router rejection details.
    pub fn details(&self) -> Option<&Value> {
        match self {
            Self::Rejected { details, .. } => Some(details),
            Self::Io(_) | Self::Endpoint { .. } | Self::InvalidResponse(_) => None,
        }
    }
}

impl fmt::Display for AdminError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "router administration failed: {error}"),
            Self::Endpoint { code, message } => {
                write!(
                    formatter,
                    "router administration failed ({code}): {message}"
                )
            }
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
    endpoint: &AdminEndpoint,
    token: &str,
    config: &RouterConfig,
) -> Result<ApplyAcknowledgement, AdminError> {
    apply_snapshot_with_timeout(endpoint, token, config, DEFAULT_TIMEOUT)
}

/// Applies a snapshot with an explicit read/write timeout.
pub fn apply_snapshot_with_timeout(
    endpoint: &AdminEndpoint,
    token: &str,
    config: &RouterConfig,
    timeout: Duration,
) -> Result<ApplyAcknowledgement, AdminError> {
    let value = request(
        endpoint,
        &json!({"token": token, "operation": "apply", "config": config}),
        timeout,
    )?;
    serde_json::from_value(value).map_err(|error| AdminError::InvalidResponse(error.to_string()))
}

/// Returns the router's current snapshot identity.
pub fn current_snapshot(
    endpoint: &AdminEndpoint,
    token: &str,
) -> Result<SnapshotIdentity, AdminError> {
    let value = request(
        endpoint,
        &json!({"token": token, "operation": "current-version"}),
        DEFAULT_TIMEOUT,
    )?;
    serde_json::from_value(value).map_err(|error| AdminError::InvalidResponse(error.to_string()))
}

/// Returns the route-inspection response without weakening its forward compatibility.
pub fn inspect_routes(endpoint: &AdminEndpoint, token: &str) -> Result<Value, AdminError> {
    request(
        endpoint,
        &json!({"token": token, "operation": "routes"}),
        DEFAULT_TIMEOUT,
    )
}

pub fn events(endpoint: &AdminEndpoint, token: &str) -> Result<Value, AdminError> {
    request(
        endpoint,
        &json!({"token": token, "operation": "events"}),
        DEFAULT_TIMEOUT,
    )
}

fn request(
    endpoint: &AdminEndpoint,
    request: &Value,
    timeout: Duration,
) -> Result<Value, AdminError> {
    let mut encoded = serde_json::to_vec(request)
        .map_err(|error| AdminError::InvalidResponse(error.to_string()))?;
    encoded.push(b'\n');
    let response = match endpoint {
        AdminEndpoint::Unix(socket_path) => unix_request(socket_path, &encoded, timeout)?,
        AdminEndpoint::DockerExec {
            container,
            socket_path,
            deployment,
            instance,
            resource_hash,
        } => docker_exec_request(
            container,
            socket_path,
            deployment,
            instance,
            resource_hash,
            &encoded,
            timeout,
        )?,
    };
    decode_response(&response)
}

fn unix_request(
    socket_path: &Path,
    request: &[u8],
    timeout: Duration,
) -> Result<String, AdminError> {
    let mut stream = UnixStream::connect(socket_path)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    stream.write_all(request)?;
    stream.flush()?;

    let mut response = String::new();
    BufReader::new(stream).read_line(&mut response)?;
    Ok(response)
}

fn docker_exec_request(
    container: &str,
    socket_path: &Path,
    deployment: &str,
    instance: &str,
    resource_hash: &str,
    request: &[u8],
    timeout: Duration,
) -> Result<String, AdminError> {
    if container.is_empty()
        || deployment.is_empty()
        || instance.is_empty()
        || resource_hash.is_empty()
        || !socket_path.is_absolute()
    {
        return Err(AdminError::Endpoint {
            code: "invalid_endpoint",
            message: "container administration endpoint is incomplete".into(),
        });
    }
    let inspect = run_docker(
        &[
            "container",
            "inspect",
            "--format",
            "{{json .Config.Labels}}",
            container,
        ],
        None,
        timeout,
    )?;
    let labels: Value =
        serde_json::from_slice(&inspect.stdout).map_err(|error| AdminError::Endpoint {
            code: "invalid_container_inspection",
            message: error.to_string(),
        })?;
    verify_container_labels(&labels, container, deployment, instance, resource_hash)?;

    let socket = socket_path.to_str().ok_or_else(|| AdminError::Endpoint {
        code: "invalid_endpoint",
        message: "container administration socket path is not valid UTF-8".into(),
    })?;
    let output = run_docker(
        &[
            "exec",
            "--interactive",
            container,
            "/usr/local/bin/switchyard-router",
            "admin-client",
            socket,
        ],
        Some(request),
        timeout,
    )?;
    String::from_utf8(output.stdout).map_err(|error| AdminError::InvalidResponse(error.to_string()))
}

fn verify_container_labels(
    labels: &Value,
    container: &str,
    deployment: &str,
    instance: &str,
    resource_hash: &str,
) -> Result<(), AdminError> {
    for (name, expected) in [
        (MANAGED_LABEL, "true"),
        (DEPLOYMENT_LABEL, deployment),
        (INSTANCE_LABEL, instance),
        (RESOURCE_HASH_LABEL, resource_hash),
    ] {
        if labels.get(name).and_then(Value::as_str) != Some(expected) {
            return Err(AdminError::Endpoint {
                code: "container_ownership_mismatch",
                message: format!(
                    "container `{container}` does not have the expected `{name}` ownership label"
                ),
            });
        }
    }
    Ok(())
}

struct ProcessOutput {
    stdout: Vec<u8>,
}

fn run_docker(
    arguments: &[&str],
    input: Option<&[u8]>,
    timeout: Duration,
) -> Result<ProcessOutput, AdminError> {
    let mut child = Command::new("docker")
        .args(arguments)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("missing Docker stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("missing Docker stderr"))?;
    let stdout_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(MAX_COMMAND_OUTPUT_BYTES)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let stderr_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr
            .take(MAX_COMMAND_OUTPUT_BYTES)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    if let Some(input) = input {
        let write_result = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("missing Docker stdin"))?
            .write_all(input);
        if let Err(error) = write_result {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(AdminError::Io(error));
        }
    }

    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(AdminError::Endpoint {
                code: "endpoint_timeout",
                message: format!(
                    "Docker administration command exceeded {} seconds",
                    timeout.as_secs()
                ),
            });
        }
        thread::sleep(Duration::from_millis(10));
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| io::Error::other("Docker stdout reader panicked"))??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| io::Error::other("Docker stderr reader panicked"))??;
    if !status.success() {
        let detail = String::from_utf8_lossy(&stderr).trim().to_owned();
        return Err(AdminError::Endpoint {
            code: "docker_command_failed",
            message: if detail.is_empty() {
                status.to_string()
            } else {
                detail
            },
        });
    }
    Ok(ProcessOutput { stdout })
}

fn decode_response(response: &str) -> Result<Value, AdminError> {
    let value: Value = serde_json::from_str(response)
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

    #[test]
    fn container_ownership_requires_every_exact_identity_label() {
        let labels = json!({
            MANAGED_LABEL: "true",
            DEPLOYMENT_LABEL: "demo",
            INSTANCE_LABEL: "backend",
            RESOURCE_HASH_LABEL: "resource-hash",
        });
        verify_container_labels(&labels, "sidecar", "demo", "backend", "resource-hash").unwrap();

        let error =
            verify_container_labels(&labels, "sidecar", "demo", "other-backend", "resource-hash")
                .unwrap_err();
        assert_eq!(error.endpoint_code(), Some("container_ownership_mismatch"));
        assert!(!error.to_string().contains("resource-hash"));
    }
}
