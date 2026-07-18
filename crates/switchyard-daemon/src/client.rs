use std::{
    fmt, fs,
    io::{self, BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    os::unix::fs::PermissionsExt,
    path::Path,
    thread,
    time::Duration,
};

use serde::{Serialize, de::DeserializeOwned};

use crate::contract::{
    API_VERSION, ApiErrorV1, CommandKind, CommandRequestV1, CreateWorktreeRequestV1,
    DaemonStatusV1, DeploymentDetailV1, DeploymentRoutesV1, DeploymentsV1, DeviceV1, DiscoveryV1,
    OperationV1, RegisterDeviceRequestV1, RegisterSourceRequestV1, RemoveWorktreeRequestV1,
    SourceV1,
};

pub fn devices(project_root: &Path) -> Result<Option<Vec<DeviceV1>>, ClientError> {
    let Some(discovery) = load_discovery(project_root)? else {
        return Ok(None);
    };
    json_request::<(), _>(&discovery, "GET", "/api/v1/devices", None).map(Some)
}

pub fn register_device(
    project_root: &Path,
    request: &RegisterDeviceRequestV1,
) -> Result<Option<DeviceV1>, ClientError> {
    let Some(discovery) = load_discovery(project_root)? else {
        return Ok(None);
    };
    json_request(&discovery, "POST", "/api/v1/devices", Some(request)).map(Some)
}

pub fn deregister_device(project_root: &Path, name: &str) -> Result<Option<()>, ClientError> {
    let Some(discovery) = load_discovery(project_root)? else {
        return Ok(None);
    };
    json_request::<(), serde_json::Value>(
        &discovery,
        "DELETE",
        &format!("/api/v1/devices/{name}"),
        None,
    )
    .map(|_| Some(()))
}

pub fn check_device(project_root: &Path, name: &str) -> Result<Option<DeviceV1>, ClientError> {
    let Some(discovery) = load_discovery(project_root)? else {
        return Ok(None);
    };
    json_request::<(), _>(
        &discovery,
        "POST",
        &format!("/api/v1/devices/{name}/check"),
        None,
    )
    .map(Some)
}

/// Lists sources through a discovered daemon, or returns `None` for one-shot fallback.
pub fn sources(project_root: &Path) -> Result<Option<Vec<SourceV1>>, ClientError> {
    let Some(discovery) = load_discovery(project_root)? else {
        return Ok(None);
    };
    json_request::<(), _>(&discovery, "GET", "/api/v1/sources", None).map(Some)
}

/// Lists deployments through a discovered daemon, or returns `None` for one-shot fallback.
pub fn deployments(project_root: &Path) -> Result<Option<DeploymentsV1>, ClientError> {
    let Some(discovery) = load_discovery(project_root)? else {
        return Ok(None);
    };
    json_request::<(), _>(&discovery, "GET", "/api/v1/deployments", None).map(Some)
}

/// Reads deployment detail through a discovered daemon, or returns `None` when no
/// daemon is reachable. A reachable daemon's API errors remain visible to callers.
pub fn deployment_detail(
    project_root: &Path,
    deployment: &str,
) -> Result<Option<DeploymentDetailV1>, ClientError> {
    let Some(discovery) = load_discovery(project_root)? else {
        return Ok(None);
    };
    json_request::<(), _>(
        &discovery,
        "GET",
        &format!("/api/v1/deployments/{deployment}"),
        None,
    )
    .map(Some)
}

/// Cancels a running operation by identifier through a discovered daemon.
pub fn cancel_operation(project_root: &Path, id: &str) -> Result<Option<OperationV1>, ClientError> {
    let Some(discovery) = load_discovery(project_root)? else {
        return Ok(None);
    };
    json_request::<(), _>(
        &discovery,
        "POST",
        &format!("/api/v1/operations/{id}/cancel"),
        None,
    )
    .map(Some)
}

/// Registers an unmanaged source through a discovered daemon.
pub fn register_source(
    project_root: &Path,
    request: &RegisterSourceRequestV1,
) -> Result<Option<SourceV1>, ClientError> {
    let Some(discovery) = load_discovery(project_root)? else {
        return Ok(None);
    };
    json_request(&discovery, "POST", "/api/v1/sources", Some(request)).map(Some)
}

/// Deregisters a source through a discovered daemon.
pub fn deregister_source(project_root: &Path, name: &str) -> Result<Option<()>, ClientError> {
    let Some(discovery) = load_discovery(project_root)? else {
        return Ok(None);
    };
    json_request::<(), serde_json::Value>(
        &discovery,
        "DELETE",
        &format!("/api/v1/sources/{name}"),
        None,
    )
    .map(|_| Some(()))
}

/// Creates a managed worktree through a discovered daemon.
pub fn create_worktree(
    project_root: &Path,
    request: &CreateWorktreeRequestV1,
) -> Result<Option<SourceV1>, ClientError> {
    let Some(discovery) = load_discovery(project_root)? else {
        return Ok(None);
    };
    json_request(&discovery, "POST", "/api/v1/worktrees", Some(request)).map(Some)
}

/// Removes a managed worktree through a discovered daemon.
pub fn remove_worktree(
    project_root: &Path,
    name: &str,
    allow_dirty: bool,
) -> Result<Option<switchyard_sources::DirtyState>, ClientError> {
    let Some(discovery) = load_discovery(project_root)? else {
        return Ok(None);
    };
    json_request(
        &discovery,
        "DELETE",
        &format!("/api/v1/worktrees/{name}"),
        Some(&RemoveWorktreeRequestV1 { allow_dirty }),
    )
    .map(Some)
}

/// Result of optional daemon discovery used by transparent CLI delegation.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum DaemonExecution {
    NotRunning,
    Completed(OperationV1),
}

/// Queries durable route versions and history through a discovered daemon.
pub fn deployment_routes(
    project_root: &Path,
    deployment: &str,
) -> Result<Option<DeploymentRoutesV1>, ClientError> {
    let Some(discovery) = load_discovery(project_root)? else {
        return Ok(None);
    };
    let path = format!("/api/v1/deployments/{deployment}/routes");
    json_request::<(), _>(&discovery, "GET", &path, None).map(Some)
}

#[derive(Debug)]
pub enum ClientError {
    Io(io::Error),
    InvalidDiscovery(String),
    InvalidResponse(String),
    Api { status: u16, error: ApiErrorV1 },
}

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::InvalidDiscovery(message) => {
                write!(formatter, "invalid daemon discovery: {message}")
            }
            Self::InvalidResponse(message) => {
                write!(formatter, "invalid daemon response: {message}")
            }
            Self::Api { status, error } => {
                write!(
                    formatter,
                    "daemon API {} (HTTP {status}): {}",
                    error.code, error.message
                )
            }
        }
    }
}

impl std::error::Error for ClientError {}
impl From<io::Error> for ClientError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

/// Reads a secure, compatible project-local discovery file.
pub fn load_discovery(project_root: &Path) -> Result<Option<DiscoveryV1>, ClientError> {
    let path = project_root.join(".switchyard/daemon.json");
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(ClientError::InvalidDiscovery(
            "daemon.json must not be accessible by group or other users".into(),
        ));
    }
    let discovery: DiscoveryV1 = serde_json::from_slice(&fs::read(path)?)
        .map_err(|error| ClientError::InvalidDiscovery(error.to_string()))?;
    if discovery.api_version != API_VERSION {
        return Err(ClientError::InvalidDiscovery(format!(
            "unsupported API version `{}`",
            discovery.api_version
        )));
    }
    let address: SocketAddr = discovery
        .address
        .parse()
        .map_err(|error| ClientError::InvalidDiscovery(format!("invalid address: {error}")))?;
    if !address.ip().is_loopback() {
        return Err(ClientError::InvalidDiscovery(
            "daemon address is not loopback-only".into(),
        ));
    }
    if discovery.token.is_empty() {
        return Err(ClientError::InvalidDiscovery("token is empty".into()));
    }
    if discovery_daemon_status(&discovery).is_none() {
        return Ok(None);
    }
    Ok(Some(discovery))
}

/// Executes through a discovered daemon, returning `NotRunning` for absent or stale discovery.
pub fn execute_if_running(
    project_root: &Path,
    kind: CommandKind,
    request: &CommandRequestV1,
) -> Result<DaemonExecution, ClientError> {
    let Some(discovery) = load_discovery(project_root)? else {
        return Ok(DaemonExecution::NotRunning);
    };
    let path = format!("/api/v1/commands/{}", kind.segment());
    let operation: OperationV1 = match json_request(&discovery, "POST", &path, Some(request)) {
        Ok(operation) => operation,
        Err(ClientError::Io(error))
            if matches!(
                error.kind(),
                io::ErrorKind::ConnectionRefused
                    | io::ErrorKind::ConnectionReset
                    | io::ErrorKind::TimedOut
                    | io::ErrorKind::NotConnected
            ) =>
        {
            return Ok(DaemonExecution::NotRunning);
        }
        Err(error) => return Err(error),
    };
    let events_path = format!("/api/v1/operations/{}/events", operation.id);
    if wait_for_terminal_event(&discovery, &events_path).is_ok() {
        let current = json_request::<(), _>(
            &discovery,
            "GET",
            &format!("/api/v1/operations/{}", operation.id),
            None,
        )?;
        return Ok(DaemonExecution::Completed(current));
    }
    let mut delay = Duration::from_millis(100);
    loop {
        let current: OperationV1 = json_request::<(), _>(
            &discovery,
            "GET",
            &format!("/api/v1/operations/{}", operation.id),
            None,
        )?;
        if current.status.terminal() {
            return Ok(DaemonExecution::Completed(current));
        }
        thread::sleep(delay);
        delay = (delay * 2).min(Duration::from_secs(1));
    }
}

pub fn daemon_status(project_root: &Path) -> Result<Option<DaemonStatusV1>, ClientError> {
    let Some(discovery) = load_discovery(project_root)? else {
        return Ok(None);
    };
    match json_request::<(), DaemonStatusV1>(&discovery, "GET", "/api/v1/system/status", None) {
        Ok(status) => Ok(Some(status)),
        Err(ClientError::Io(error)) if error.kind() == io::ErrorKind::ConnectionRefused => Ok(None),
        Err(error) => Err(error),
    }
}

pub fn daemon_stop(project_root: &Path) -> Result<bool, ClientError> {
    let Some(discovery) = load_discovery(project_root)? else {
        return Ok(false);
    };
    let _: serde_json::Value =
        json_request::<(), _>(&discovery, "POST", "/api/v1/system/shutdown", None)?;
    Ok(true)
}

fn json_request<B: Serialize, T: DeserializeOwned>(
    discovery: &DiscoveryV1,
    method: &str,
    path: &str,
    body: Option<&B>,
) -> Result<T, ClientError> {
    json_request_with_timeouts(
        discovery,
        method,
        path,
        body,
        Duration::from_millis(300),
        Duration::from_secs(30),
    )
}

fn json_request_with_timeouts<B: Serialize, T: DeserializeOwned>(
    discovery: &DiscoveryV1,
    method: &str,
    path: &str,
    body: Option<&B>,
    connect_timeout: Duration,
    read_timeout: Duration,
) -> Result<T, ClientError> {
    let address: SocketAddr = discovery
        .address
        .parse()
        .map_err(|error| ClientError::InvalidDiscovery(format!("invalid address: {error}")))?;
    let encoded = body
        .map(serde_json::to_vec)
        .transpose()
        .map_err(|error| ClientError::InvalidResponse(error.to_string()))?
        .unwrap_or_default();
    let mut stream = TcpStream::connect_timeout(&address, connect_timeout)?;
    stream.set_read_timeout(Some(read_timeout))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {}\r\nAccept: application/json\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        discovery.token,
        encoded.len()
    )?;
    stream.write_all(&encoded)?;
    stream.flush()?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| ClientError::InvalidResponse("missing HTTP header terminator".into()))?;
    let headers = String::from_utf8_lossy(&response[..header_end]);
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|status| status.parse::<u16>().ok())
        .ok_or_else(|| ClientError::InvalidResponse("missing HTTP status".into()))?;
    let mut response_body = response[header_end + 4..].to_vec();
    if headers
        .lines()
        .any(|line| line.eq_ignore_ascii_case("transfer-encoding: chunked"))
    {
        response_body = decode_chunked(&response_body)?;
    }
    if !(200..300).contains(&status) {
        let error = serde_json::from_slice(&response_body).unwrap_or_else(|_| {
            ApiErrorV1::new("http_error", String::from_utf8_lossy(&response_body))
        });
        return Err(ClientError::Api { status, error });
    }
    if response_body.is_empty() {
        return serde_json::from_slice(b"null")
            .map_err(|error| ClientError::InvalidResponse(error.to_string()));
    }
    serde_json::from_slice(&response_body)
        .map_err(|error| ClientError::InvalidResponse(error.to_string()))
}

pub(crate) fn discovery_daemon_status(discovery: &DiscoveryV1) -> Option<DaemonStatusV1> {
    json_request_with_timeouts::<(), _>(
        discovery,
        "GET",
        "/api/v1/system/status",
        None,
        Duration::from_millis(100),
        Duration::from_millis(300),
    )
    .ok()
    .filter(|status: &DaemonStatusV1| status.api_version == API_VERSION)
}

fn wait_for_terminal_event(discovery: &DiscoveryV1, path: &str) -> Result<(), ClientError> {
    let address: SocketAddr = discovery
        .address
        .parse()
        .map_err(|error| ClientError::InvalidDiscovery(format!("invalid address: {error}")))?;
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(300))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {}\r\nAccept: text/event-stream\r\nConnection: close\r\n\r\n",
        discovery.token
    )?;
    stream.flush()?;
    let mut reader = BufReader::new(stream);
    let mut headers = String::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Err(ClientError::InvalidResponse(
                "truncated SSE response headers".into(),
            ));
        }
        if line == "\r\n" {
            break;
        }
        headers.push_str(&line);
    }
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|status| status.parse::<u16>().ok())
        .ok_or_else(|| ClientError::InvalidResponse("missing HTTP status".into()))?;
    if !(200..300).contains(&status) {
        return Err(ClientError::InvalidResponse(format!(
            "SSE request returned HTTP {status}"
        )));
    }
    let chunked = headers
        .lines()
        .any(|line| line.eq_ignore_ascii_case("transfer-encoding: chunked"));
    if chunked {
        read_chunked_sse(&mut reader)
    } else {
        read_sse_lines(&mut reader)
    }
}

fn read_chunked_sse(reader: &mut impl BufRead) -> Result<(), ClientError> {
    let mut pending = String::new();
    loop {
        let mut size_line = String::new();
        reader.read_line(&mut size_line)?;
        let size = usize::from_str_radix(
            size_line.trim_end().split(';').next().unwrap_or_default(),
            16,
        )
        .map_err(|error| ClientError::InvalidResponse(error.to_string()))?;
        if size == 0 {
            return Err(ClientError::InvalidResponse(
                "SSE stream ended before a terminal operation event".into(),
            ));
        }
        let mut chunk = vec![0_u8; size];
        reader.read_exact(&mut chunk)?;
        let mut terminator = [0_u8; 2];
        reader.read_exact(&mut terminator)?;
        if terminator != *b"\r\n" {
            return Err(ClientError::InvalidResponse(
                "invalid chunk terminator".into(),
            ));
        }
        pending.push_str(&String::from_utf8_lossy(&chunk).replace('\r', ""));
        if consume_sse_records(&mut pending)? {
            return Ok(());
        }
    }
}

fn read_sse_lines(reader: &mut impl BufRead) -> Result<(), ClientError> {
    let mut pending = String::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Err(ClientError::InvalidResponse(
                "SSE stream ended before a terminal operation event".into(),
            ));
        }
        pending.push_str(&line.replace('\r', ""));
        if consume_sse_records(&mut pending)? {
            return Ok(());
        }
    }
}

fn consume_sse_records(pending: &mut String) -> Result<bool, ClientError> {
    while let Some(end) = pending.find("\n\n") {
        let record = pending[..end].to_owned();
        pending.drain(..end + 2);
        let mut event = None;
        let mut data = None;
        for line in record.lines() {
            if let Some(value) = line.strip_prefix("event:") {
                event = Some(value.trim());
            } else if let Some(value) = line.strip_prefix("data:") {
                data = Some(value.trim());
            }
        }
        if event == Some("operation") {
            let value: serde_json::Value = serde_json::from_str(data.unwrap_or_default())
                .map_err(|error| ClientError::InvalidResponse(error.to_string()))?;
            if value
                .pointer("/data/status")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|status| matches!(status, "succeeded" | "failed" | "cancelled"))
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn decode_chunked(mut input: &[u8]) -> Result<Vec<u8>, ClientError> {
    let mut output = Vec::new();
    loop {
        let line_end = input
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or_else(|| ClientError::InvalidResponse("invalid chunk header".into()))?;
        let size = usize::from_str_radix(
            String::from_utf8_lossy(&input[..line_end])
                .split(';')
                .next()
                .unwrap_or_default(),
            16,
        )
        .map_err(|error| ClientError::InvalidResponse(error.to_string()))?;
        input = &input[line_end + 2..];
        if size == 0 {
            break;
        }
        if input.len() < size + 2 {
            return Err(ClientError::InvalidResponse("truncated chunk".into()));
        }
        output.extend_from_slice(&input[..size]);
        input = &input[size + 2..];
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_parser_waits_for_a_terminal_operation_event() {
        let mut pending = concat!(
            "event: operation\n",
            "data: {\"data\":{\"status\":\"running\"}}\n\n",
            "event: log\n",
            "data: {\"data\":{\"status\":\"failed\"}}\n\n"
        )
        .to_owned();
        assert!(!consume_sse_records(&mut pending).unwrap());

        pending.push_str("event: operation\ndata: {\"data\":{\"status\":\"succeeded\"}}\n\n");
        assert!(consume_sse_records(&mut pending).unwrap());
    }
}
