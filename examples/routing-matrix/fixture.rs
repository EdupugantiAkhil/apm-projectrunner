//! Tiny dependency-free HTTP fixture used for every routing-matrix application role.

use std::{
    env, fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

const SERVICES: [(&str, u16); 5] = [
    ("catalog", 8001),
    ("search", 8002),
    ("reports", 8003),
    ("scheduler", 8004),
    ("audit", 8005),
];

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("routing-fixture: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    match arguments.as_slice() {
        [mode, listen, service, provider] if mode == "provider" => serve(
            listen,
            Role::Provider {
                service: service.clone(),
                provider: provider.clone(),
            },
        ),
        [mode, listen, service] if mode == "provider-instance" => {
            let instance = env::var("SWITCHYARD_INSTANCE")
                .map_err(|_| "SWITCHYARD_INSTANCE must be set".to_owned())?;
            let suite = instance
                .strip_suffix(&format!("-{service}"))
                .ok_or_else(|| format!("instance {instance} must end with -{service}"))?;
            serve(
                listen,
                Role::Provider {
                    service: service.clone(),
                    provider: format!("{suite}/{service}"),
                },
            )
        }
        [mode, listen, backend, state_file] if mode == "backend" => serve(
            listen,
            Role::Backend {
                backend: backend.clone(),
                counter: Arc::new(Mutex::new(Counter::load(state_file)?)),
            },
        ),
        [mode, listen, state_file] if mode == "backend-instance" => serve(
            listen,
            Role::Backend {
                backend: env::var("SWITCHYARD_INSTANCE")
                    .map_err(|_| "SWITCHYARD_INSTANCE must be set".to_owned())?,
                counter: Arc::new(Mutex::new(Counter::load(state_file)?)),
            },
        ),
        [mode, addresses @ ..] if mode == "probe" && !addresses.is_empty() => {
            for address in addresses {
                request_identity(address)?;
            }
            Ok(())
        }
        [mode, socket, token, config] if mode == "admin-apply" => {
            admin_apply(socket, token, config)
        }
        [mode] if mode == "hold" => loop {
            thread::park();
        },
        _ => Err("usage: routing-fixture provider <listen> <service> <provider> | provider-instance <listen> <service> | backend <listen> <backend> <state-file> | backend-instance <listen> <state-file> | probe <address>... | admin-apply <socket> <token> <config> | hold".into()),
    }
}

fn admin_apply(socket: &str, token: &str, config: &str) -> Result<(), String> {
    let config = fs::read_to_string(config).map_err(|error| format!("read {config}: {error}"))?;
    let mut stream = UnixStream::connect(socket)
        .map_err(|error| format!("connect administration socket {socket}: {error}"))?;
    write!(
        stream,
        "{{\"token\":{},\"operation\":\"apply\",\"config\":{config}}}\n",
        json(token)
    )
    .map_err(|error| format!("write administration request: {error}"))?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|error| format!("read administration response: {error}"))?;
    if response.contains("\"ok\":true") {
        Ok(())
    } else {
        Err(format!("route apply was rejected: {}", response.trim()))
    }
}

enum Role {
    Provider {
        service: String,
        provider: String,
    },
    Backend {
        backend: String,
        counter: Arc<Mutex<Counter>>,
    },
}

impl Clone for Role {
    fn clone(&self) -> Self {
        match self {
            Self::Provider { service, provider } => Self::Provider {
                service: service.clone(),
                provider: provider.clone(),
            },
            Self::Backend { backend, counter } => Self::Backend {
                backend: backend.clone(),
                counter: Arc::clone(counter),
            },
        }
    }
}

fn serve(address: &str, role: Role) -> Result<(), String> {
    let listener = TcpListener::bind(address).map_err(|error| format!("bind {address}: {error}"))?;
    for connection in listener.incoming() {
        match connection {
            Ok(stream) => {
                let role = role.clone();
                thread::spawn(move || {
                    if let Err(error) = handle(stream, &role) {
                        eprintln!("routing-fixture: request failed: {error}");
                    }
                });
            }
            Err(error) => eprintln!("routing-fixture: accept failed: {error}"),
        }
    }
    Ok(())
}

fn handle(mut stream: TcpStream, role: &Role) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|error| error.to_string())?;
    let mut request = [0_u8; 4096];
    let size = stream.read(&mut request).map_err(|error| error.to_string())?;
    let request = String::from_utf8_lossy(&request[..size]);
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");

    let (status, body) = match path {
        "/health" => (200, "{\"status\":\"ok\"}".to_owned()),
        "/identity" => match identity(role) {
            Ok(body) => (200, body),
            Err(error) => (
                502,
                format!("{{\"code\":\"dependency_failed\",\"message\":{}}}", json(&error)),
            ),
        },
        _ => (404, "{\"code\":\"not_found\"}".to_owned()),
    };
    let reason = if status == 200 { "OK" } else if status == 404 { "Not Found" } else { "Bad Gateway" };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .map_err(|error| error.to_string())
}

fn identity(role: &Role) -> Result<String, String> {
    match role {
        Role::Provider { service, provider } => Ok(format!(
            "{{\"service\":{},\"provider\":{}}}",
            json(service),
            json(provider)
        )),
        Role::Backend { backend, counter } => {
            let mut services = Vec::with_capacity(SERVICES.len());
            for (service, port) in SERVICES {
                let body = request_identity(&format!("127.0.0.1:{port}"))?;
                services.push(format!("{}:{body}", json(service)));
            }
            let request_count = counter
                .lock()
                .map_err(|_| "counter lock is poisoned".to_owned())?
                .increment()?;
            Ok(format!(
                "{{\"backend\":{},\"requestCount\":{request_count},\"services\":{{{}}}}}",
                json(backend),
                services.join(",")
            ))
        }
    }
}

fn request_identity(address: &str) -> Result<String, String> {
    let mut stream = TcpStream::connect_timeout(
        &address
            .parse()
            .map_err(|error| format!("invalid address {address}: {error}"))?,
        Duration::from_secs(2),
    )
    .map_err(|error| format!("connect {address}: {error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|error| error.to_string())?;
    write!(
        stream,
        "GET /identity HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .map_err(|error| error.to_string())?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|error| format!("read {address}: {error}"))?;
    let (headers, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| format!("invalid HTTP response from {address}"))?;
    if !headers.starts_with("HTTP/1.1 200") {
        return Err(format!("non-success response from {address}: {headers}"));
    }
    Ok(body.to_owned())
}

struct Counter {
    path: PathBuf,
    value: u64,
}

impl Counter {
    fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref().to_owned();
        let value = match fs::read_to_string(&path) {
            Ok(value) => value
                .trim()
                .parse()
                .map_err(|error| format!("read {}: {error}", path.display()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(format!("read {}: {error}", path.display())),
        };
        Ok(Self { path, value })
    }

    fn increment(&mut self) -> Result<u64, String> {
        self.value = self.value.saturating_add(1);
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create {}: {error}", parent.display()))?;
        }
        fs::write(&self.path, self.value.to_string())
            .map_err(|error| format!("write {}: {error}", self.path.display()))?;
        Ok(self.value)
    }
}

fn json(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len() + 2);
    encoded.push('"');
    for character in value.chars() {
        match character {
            '"' => encoded.push_str("\\\""),
            '\\' => encoded.push_str("\\\\"),
            '\n' => encoded.push_str("\\n"),
            '\r' => encoded.push_str("\\r"),
            '\t' => encoded.push_str("\\t"),
            character if character.is_control() => {
                encoded.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => encoded.push(character),
        }
    }
    encoded.push('"');
    encoded
}
