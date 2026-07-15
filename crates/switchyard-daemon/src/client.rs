use std::{
    fmt, fs,
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream},
    os::unix::fs::PermissionsExt,
    path::Path,
    thread,
    time::Duration,
};

use serde::{Serialize, de::DeserializeOwned};

use crate::contract::{
    API_VERSION, ApiErrorV1, CommandKind, CommandRequestV1, DaemonStatusV1, DiscoveryV1,
    OperationV1,
};

/// Result of optional daemon discovery used by transparent CLI delegation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DaemonExecution {
    NotRunning,
    Completed(OperationV1),
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
        thread::sleep(Duration::from_millis(40));
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
    let address: SocketAddr = discovery
        .address
        .parse()
        .map_err(|error| ClientError::InvalidDiscovery(format!("invalid address: {error}")))?;
    let encoded = body
        .map(serde_json::to_vec)
        .transpose()
        .map_err(|error| ClientError::InvalidResponse(error.to_string()))?
        .unwrap_or_default();
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(300))?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
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
