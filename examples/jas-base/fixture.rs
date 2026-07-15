//! Dependency-free HTTP applications for the generic legacy-workspace fixture.

use std::{
    env, fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command, ExitCode},
    thread,
    time::{Duration, Instant},
};

const AI_SERVICES: [(&str, u16); 5] = [
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
            eprintln!("jas-base fixture: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    match arguments.as_slice() {
        [mode, listen, service, state] if mode == "store" => serve(
            listen,
            Role::Store {
                service: service.clone(),
                state: state.into(),
            },
        ),
        [mode, listen, service] if mode == "ai-service" => serve(
            listen,
            Role::Ai {
                service: service.clone(),
            },
        ),
        [mode, listen] if mode == "jas" => serve(listen, Role::Jas),
        [mode, listen] if mode == "ui" => serve(listen, Role::Ui),
        [mode, file] if mode == "suite" => supervise(file),
        [mode, addresses @ ..] if mode == "initialize" && !addresses.is_empty() => {
            for address in addresses {
                initialize(address)?;
            }
            Ok(())
        }
        [mode, addresses @ ..] if mode == "probe" && !addresses.is_empty() => {
            for address in addresses {
                request_json(address, "GET", "/health")?;
            }
            Ok(())
        }
        [mode] if mode == "hold" => loop {
            thread::park();
        },
        _ => Err("usage: jas-base-fixture store <listen> <service> <state> | ai-service <listen> <service> | jas <listen> | ui <listen> | suite <process-compose.yaml> | initialize <address>... | probe <address>... | hold".into()),
    }
}

#[derive(Clone)]
enum Role {
    Store { service: String, state: PathBuf },
    Ai { service: String },
    Jas,
    Ui,
}

fn serve(address: &str, role: Role) -> Result<(), String> {
    let listener = TcpListener::bind(address).map_err(|error| format!("bind {address}: {error}"))?;
    for connection in listener.incoming() {
        match connection {
            Ok(stream) => {
                let role = role.clone();
                thread::spawn(move || {
                    if let Err(error) = handle(stream, &role) {
                        eprintln!("jas-base fixture request failed: {error}");
                    }
                });
            }
            Err(error) => eprintln!("jas-base fixture accept failed: {error}"),
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
    let first = request.lines().next().unwrap_or("GET / HTTP/1.1");
    let mut words = first.split_whitespace();
    let method = words.next().unwrap_or("GET");
    let path = words.next().unwrap_or("/");

    let (status, body) = match (method, path) {
        (_, "/health") => (200, "{\"status\":\"ok\"}".to_owned()),
        ("POST", "/initialize") => match role {
            Role::Store { service, state } => {
                let count = increment_state(state)?;
                (200, store_identity(service, state, count))
            }
            _ => (404, "{\"error\":\"not a store\"}".to_owned()),
        },
        (_, "/identity") => match identity(role) {
            Ok(body) => (200, body),
            Err(error) => (502, format!("{{\"error\":{}}}", json(&error))),
        },
        _ => (404, "{\"error\":\"not found\"}".to_owned()),
    };
    let reason = if status == 200 { "OK" } else { "Error" };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .map_err(|error| error.to_string())
}

fn identity(role: &Role) -> Result<String, String> {
    match role {
        Role::Store { service, state } => {
            let count = read_state(state)?;
            Ok(store_identity(service, state, count))
        }
        Role::Ai { service } => Ok(common_identity(service)),
        Role::Jas => {
            let kv = request_json("127.0.0.1:9101", "GET", "/identity")?;
            let document = request_json("127.0.0.1:9102", "GET", "/identity")?;
            let ai = request_ai()?;
            Ok(format!(
                "{{{},\"selectedProviders\":{{\"database\":{{\"kv\":{kv},\"document\":{document}}},\"python\":{ai}}}}}",
                common_fields("jas-service")
            ))
        }
        Role::Ui => {
            let java = request_json("127.0.0.1:10081", "GET", "/identity")?;
            let ai = request_ai()?;
            Ok(format!(
                "{{{},\"selectedProviders\":{{\"java\":{java},\"python\":{ai}}}}}",
                common_fields("ui")
            ))
        }
    }
}

fn request_ai() -> Result<String, String> {
    let mut entries = Vec::new();
    for (service, port) in AI_SERVICES {
        entries.push(format!(
            "{}:{}",
            json(service),
            request_json(&format!("127.0.0.1:{port}"), "GET", "/identity")?
        ));
    }
    Ok(format!("{{{}}}", entries.join(",")))
}

fn common_identity(service: &str) -> String {
    format!("{{{}}}", common_fields(service))
}

fn common_fields(service: &str) -> String {
    format!(
        "\"deployment\":{},\"instance\":{},\"service\":{},\"source\":{}",
        json(&required_env("SWITCHYARD_DEPLOYMENT")),
        json(&required_env("SWITCHYARD_INSTANCE")),
        json(service),
        json(&required_env("FIXTURE_SOURCE"))
    )
}

fn store_identity(service: &str, state: &PathBuf, count: u64) -> String {
    format!(
        "{{{},\"initialized\":{},\"initializationCount\":{count}}}",
        common_fields(service),
        state.exists()
    )
}

fn required_env(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| format!("missing:{name}"))
}

fn read_state(path: &PathBuf) -> Result<u64, String> {
    match fs::read_to_string(path) {
        Ok(value) => value
            .trim()
            .parse()
            .map_err(|error| format!("parse {}: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(format!("read {}: {error}", path.display())),
    }
}

fn increment_state(path: &PathBuf) -> Result<u64, String> {
    let count = read_state(path)? + 1;
    fs::write(path, format!("{count}\n"))
        .map_err(|error| format!("write {}: {error}", path.display()))?;
    Ok(count)
}

fn initialize(address: &str) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match request_json(address, "POST", "/initialize") {
            Ok(_) => return Ok(()),
            Err(error) if Instant::now() < deadline => {
                eprintln!("waiting to initialize {address}: {error}");
                thread::sleep(Duration::from_millis(200));
            }
            Err(error) => return Err(error),
        }
    }
}

fn request_json(address: &str, method: &str, path: &str) -> Result<String, String> {
    let mut stream = TcpStream::connect(address).map_err(|error| format!("connect {address}: {error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|error| error.to_string())?;
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
    )
    .map_err(|error| error.to_string())?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|error| error.to_string())?;
    let (head, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| format!("invalid HTTP response from {address}"))?;
    if !head.starts_with("HTTP/1.1 200") {
        return Err(format!("request to {address}{path} failed: {head}"));
    }
    Ok(body.to_owned())
}

fn supervise(file: &str) -> Result<(), String> {
    let configuration = fs::read_to_string(file).map_err(|error| format!("read {file}: {error}"))?;
    for (service, _) in AI_SERVICES {
        if !configuration.contains(&format!("  {service}:")) {
            return Err(format!("{file} does not declare {service}"));
        }
    }
    let mut children = Vec::new();
    for (service, port) in AI_SERVICES {
        let child = Command::new("/usr/local/bin/jas-base-fixture")
            .args(["ai-service", &format!("0.0.0.0:{port}"), service])
            .spawn()
            .map_err(|error| format!("start {service}: {error}"))?;
        children.push(child);
        wait_ready(port, &mut children)?;
    }
    wait_for_child(children)
}

fn wait_ready(port: u16, children: &mut [Child]) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if request_json(&format!("127.0.0.1:{port}"), "GET", "/health").is_ok() {
            return Ok(());
        }
        for child in children.iter_mut() {
            if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
                return Err(format!("suite child exited before readiness: {status}"));
            }
        }
        if Instant::now() >= deadline {
            return Err(format!("timed out waiting for port {port}"));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn wait_for_child(mut children: Vec<Child>) -> Result<(), String> {
    loop {
        for child in &mut children {
            if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
                return Err(format!("suite child exited: {status}"));
            }
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn json(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => {
                output.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => output.push(character),
        }
    }
    output.push('"');
    output
}
