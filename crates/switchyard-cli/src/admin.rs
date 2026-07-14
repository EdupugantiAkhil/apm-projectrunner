#![cfg(unix)]

use std::{
    fmt, io,
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::Duration,
};

use router_config::RouterConfig;
use serde_json::{Value, json};

#[derive(Debug)]
pub enum AdminError {
    Io(io::Error),
    InvalidResponse(String),
    Rejected { code: String, message: String },
}

impl fmt::Display for AdminError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "router administration failed: {error}"),
            Self::InvalidResponse(message) => {
                write!(formatter, "router returned an invalid response: {message}")
            }
            Self::Rejected { code, message } => {
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

pub fn apply_snapshot(
    socket_path: &Path,
    token: &str,
    config: &RouterConfig,
) -> Result<Value, AdminError> {
    let body = json!({
        "token": token,
        "operation": "apply",
        "config": config,
    });
    request(socket_path, &body)
}

pub fn inspect_routes(socket_path: &Path, token: &str) -> Result<Value, AdminError> {
    request(socket_path, &json!({"token": token, "operation": "routes"}))
}

pub fn current_version(socket_path: &Path, token: &str) -> Result<u64, AdminError> {
    let result = request(
        socket_path,
        &json!({"token": token, "operation": "current-version"}),
    )?;
    result["version"]
        .as_u64()
        .ok_or_else(|| AdminError::InvalidResponse("current version is missing".into()))
}

fn request(socket_path: &Path, request: &Value) -> Result<Value, AdminError> {
    let mut stream = UnixStream::connect(socket_path)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    serde_json::to_writer(&mut stream, request)
        .map_err(|error| AdminError::InvalidResponse(error.to_string()))?;
    stream.write_all(b"\n")?;
    stream.flush()?;

    let mut response = String::new();
    BufReader::new(stream).read_line(&mut response)?;
    let value: Value = serde_json::from_str(&response)
        .map_err(|error| AdminError::InvalidResponse(error.to_string()))?;
    if value.get("ok").and_then(Value::as_bool) == Some(false) {
        let error = &value["error"];
        return Err(AdminError::Rejected {
            code: error["code"].as_str().unwrap_or("unknown").to_owned(),
            message: error["message"]
                .as_str()
                .unwrap_or("router rejected the request")
                .to_owned(),
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
    fn rejection_display_does_not_include_token() {
        let error = AdminError::Rejected {
            code: "stale_snapshot".into(),
            message: "snapshot is old".into(),
        };
        assert_eq!(
            error.to_string(),
            "router rejected stale_snapshot: snapshot is old"
        );
    }
}
