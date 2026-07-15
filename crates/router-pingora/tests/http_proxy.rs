use std::{
    fs::OpenOptions,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use router_config::{ProxyAuthentication, ProxyAuthenticationScheme, RouterConfig};
use router_core::RouteEngine;
use router_pingora::{DataPlaneEvent, HttpDataPlane, ProxyOptions};
use serde_json::json;

struct TestUpstream {
    address: SocketAddr,
    healthy: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}

impl TestUpstream {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let healthy = Arc::new(AtomicBool::new(true));
        let stop = Arc::new(AtomicBool::new(false));
        let healthy_in_thread = healthy.clone();
        let stop_in_thread = stop.clone();
        let join = thread::spawn(move || {
            while !stop_in_thread.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => handle_connection(stream, &healthy_in_thread),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("upstream accept failed: {error}"),
                }
            }
        });
        Self {
            address,
            healthy,
            stop,
            join: Some(join),
        }
    }
}

impl Drop for TestUpstream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            join.join().unwrap();
        }
    }
}

fn handle_connection(mut stream: TcpStream, healthy: &AtomicBool) {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut request = Vec::new();
    let mut byte = [0_u8; 1];
    while !request.ends_with(b"\r\n\r\n") {
        if stream.read_exact(&mut byte).is_err() {
            return;
        }
        request.push(byte[0]);
    }
    let request_text = String::from_utf8_lossy(&request);
    if request_text.starts_with("GET /health ") {
        let status = if healthy.load(Ordering::Relaxed) {
            "200 OK"
        } else {
            "503 Service Unavailable"
        };
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
    } else if request_text
        .to_ascii_lowercase()
        .contains("upgrade: websocket")
    {
        stream
            .write_all(
                b"HTTP/1.1 101 Switching Protocols\r\nConnection: upgrade\r\nUpgrade: websocket\r\n\r\n",
            )
            .unwrap();
        let mut payload = [0_u8; 4];
        stream.read_exact(&mut payload).unwrap();
        stream.write_all(&payload).unwrap();
    } else {
        let body = request_text.as_bytes();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(body).unwrap();
    }
}

fn unused_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

struct CredentialFile(PathBuf);

impl CredentialFile {
    fn create(port: u16) -> Self {
        let path = std::env::temp_dir().join(format!(
            "switchyard-proxy-auth-{}-{port}",
            std::process::id()
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options
            .open(&path)
            .unwrap()
            .write_all(b"test-token\n")
            .unwrap();
        Self(path)
    }
}

impl Drop for CredentialFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn config(proxy_port: u16, upstream_port: u16) -> RouterConfig {
    serde_json::from_value(json!({
        "apiVersion": "switchyard.dev/router/v1alpha1",
        "kind": "RouterConfiguration",
        "metadata": { "deployment": "proxy-test" },
        "spec": {
            "snapshot": {
                "id": "proxy-test-1",
                "version": 1,
                "transitions": {
                    "http": { "strategy": "close" },
                    "https": { "strategy": "close" },
                    "websocket": { "strategy": "pin" },
                    "grpc": { "strategy": "close" },
                    "tcp": { "strategy": "close" }
                }
            },
            "listeners": [{
                "consumer": "test-client",
                "bind": { "host": "127.0.0.1", "port": proxy_port },
                "protocol": "websocket",
                "destinations": [{ "kind": "loopback", "slot": "api" }]
            }],
            "providers": [{
                "id": "test-upstream",
                "endpoint": { "protocol": "websocket", "host": "127.0.0.1", "port": upstream_port },
                "healthCheck": { "protocol": "http", "path": "/health", "intervalMs": 1000, "timeoutMs": 500 }
            }],
            "routes": [{ "consumer": "test-client", "slot": "api", "provider": "test-upstream" }],
            "identity": { "explicitHeader": "X-Switchyard-Route", "stripBeforeForwarding": true }
        }
    }))
    .unwrap()
}

fn browser_config(proxy_port: u16, upstream_port: u16) -> RouterConfig {
    serde_json::from_value(json!({
        "apiVersion": "switchyard.dev/router/v1alpha1",
        "kind": "RouterConfiguration",
        "metadata": { "deployment": "browser-test" },
        "spec": {
            "snapshot": {
                "id": "browser-test-1",
                "version": 1,
                "transitions": {
                    "http": { "strategy": "close" },
                    "https": { "strategy": "close" },
                    "websocket": { "strategy": "pin" },
                    "grpc": { "strategy": "close" },
                    "tcp": { "strategy": "close" }
                }
            },
            "listeners": [{
                "bind": { "host": "127.0.0.1", "port": proxy_port },
                "protocol": "http",
                "destinations": [{
                    "kind": "legacy_localhost",
                    "slot": "browser-backend",
                    "host": "localhost"
                }]
            }],
            "providers": [
                {
                    "id": "backend-one",
                    "endpoint": { "protocol": "http", "host": "127.0.0.1", "port": upstream_port }
                },
                {
                    "id": "backend-two",
                    "endpoint": { "protocol": "http", "host": "127.0.0.1", "port": upstream_port }
                }
            ],
            "browserRoutes": [
                {
                    "identity": { "source": "origin", "origin": "https://ui-one.test" },
                    "destination": "browser-backend",
                    "provider": "backend-one"
                },
                {
                    "identity": { "source": "explicit_header", "value": "tab-one" },
                    "destination": "browser-backend",
                    "provider": "backend-one"
                },
                {
                    "identity": { "source": "explicit_header", "value": "tab-two" },
                    "destination": "browser-backend",
                    "provider": "backend-two"
                }
            ],
            "identity": { "explicitHeader": "X-Switchyard-Route", "stripBeforeForwarding": true }
        }
    }))
    .unwrap()
}

fn request(address: SocketAddr, request: &[u8]) -> Vec<u8> {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream.write_all(request).unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    response
}

#[test]
fn proxies_http_and_websocket_and_rejects_unhealthy_provider() {
    let upstream = TestUpstream::start();
    let proxy_port = unused_port();
    let config = config(proxy_port, upstream.address.port());
    let engine = Arc::new(RouteEngine::new(config.clone()).unwrap());
    let running = HttpDataPlane::new(
        engine,
        config.spec.listeners.clone(),
        config.spec.identity.clone(),
        ProxyOptions::default(),
    )
    .unwrap()
    .spawn()
    .unwrap();
    let telemetry = running.telemetry();
    assert!(running.wait_ready(Duration::from_secs(2)));
    let proxy = SocketAddr::from(([127, 0, 0, 1], proxy_port));

    let response = request(
        proxy,
        b"GET /echo HTTP/1.1\r\nHost: localhost\r\nX-Switchyard-Route: secret\r\nConnection: close\r\n\r\n",
    );
    let response = String::from_utf8(response).unwrap().to_ascii_lowercase();
    assert!(response.starts_with("http/1.1 200"));
    assert!(response.contains("x-forwarded-host: localhost"));
    assert!(response.contains("x-forwarded-proto: http"));
    assert!(response.contains("x-forwarded-for:"));
    assert!(!response.contains("x-switchyard-route"));

    let mut websocket = TcpStream::connect(proxy).unwrap();
    websocket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    websocket
        .write_all(b"GET /ws HTTP/1.1\r\nHost: localhost\r\nConnection: upgrade\r\nUpgrade: websocket\r\n\r\n")
        .unwrap();
    let mut handshake = [0_u8; 89];
    let count = websocket.read(&mut handshake).unwrap();
    assert!(String::from_utf8_lossy(&handshake[..count]).starts_with("HTTP/1.1 101"));
    websocket.write_all(b"ping").unwrap();
    let mut echoed = [0_u8; 4];
    websocket.read_exact(&mut echoed).unwrap();
    assert_eq!(&echoed, b"ping");

    upstream.healthy.store(false, Ordering::Relaxed);
    let response = request(
        proxy,
        b"GET /echo HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    );
    let response = String::from_utf8(response).unwrap();
    assert!(response.starts_with("HTTP/1.1 503"));
    assert!(response.contains("\"code\":\"provider_unhealthy\""));

    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while telemetry.metrics().active_requests != 0 && std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(telemetry.metrics().requests, 3);
    assert_eq!(telemetry.metrics().errors, 1);
    assert_eq!(telemetry.metrics().active_requests, 0);
    assert!(telemetry.events().iter().any(|event| matches!(
        event,
        DataPlaneEvent::Rejection {
            status: 503,
            code
        } if code == "provider_unhealthy"
    )));

    running.shutdown();
}

#[test]
fn browser_routes_enforce_origin_and_answer_cors_preflight() {
    let upstream = TestUpstream::start();
    let proxy_port = unused_port();
    let config = browser_config(proxy_port, upstream.address.port());
    let engine = Arc::new(RouteEngine::new(config.clone()).unwrap());
    let running = HttpDataPlane::new(
        engine,
        config.spec.listeners.clone(),
        config.spec.identity.clone(),
        ProxyOptions::default(),
    )
    .unwrap()
    .spawn()
    .unwrap();
    assert!(running.wait_ready(Duration::from_secs(2)));
    let proxy = SocketAddr::from(([127, 0, 0, 1], proxy_port));

    let response = String::from_utf8(request(
        proxy,
        b"GET /echo HTTP/1.1\r\nHost: localhost\r\nOrigin: https://ui-one.test\r\nConnection: close\r\n\r\n",
    ))
    .unwrap()
    .to_ascii_lowercase();
    assert!(response.starts_with("http/1.1 200"));
    assert!(response.contains("access-control-allow-origin: https://ui-one.test"));
    assert!(!response.contains("access-control-allow-origin: *"));
    assert!(response.contains("vary: origin"));

    let response = String::from_utf8(request(
        proxy,
        b"OPTIONS /echo HTTP/1.1\r\nHost: localhost\r\nOrigin: https://ui-one.test\r\nAccess-Control-Request-Method: POST\r\nAccess-Control-Request-Headers: X-Demo\r\nAccess-Control-Request-Private-Network: true\r\nConnection: close\r\n\r\n",
    ))
    .unwrap()
    .to_ascii_lowercase();
    assert!(response.starts_with("http/1.1 204"));
    assert!(response.contains("access-control-allow-origin: https://ui-one.test"));
    assert!(response.contains("access-control-allow-methods: post"));
    assert!(response.contains("access-control-allow-headers: x-demo"));
    assert!(response.contains("access-control-allow-private-network: true"));
    assert!(response.contains(
        "vary: origin, access-control-request-method, access-control-request-headers, access-control-request-private-network"
    ));

    let response = String::from_utf8(request(
        proxy,
        b"OPTIONS /echo HTTP/1.1\r\nHost: localhost\r\nOrigin: https://ui-one.test\r\nAccess-Control-Request-Method: POST GET\r\nConnection: close\r\n\r\n",
    ))
    .unwrap();
    assert!(response.starts_with("HTTP/1.1 400"));
    assert!(response.contains("\"code\":\"invalid_cors_preflight\""));

    let response = String::from_utf8(request(
        proxy,
        b"GET /echo HTTP/1.1\r\nHost: localhost\r\nOrigin: https://unknown.test\r\nConnection: close\r\n\r\n",
    ))
    .unwrap();
    assert!(response.starts_with("HTTP/1.1 403"));
    assert!(response.contains("\"code\":\"disallowed_origin\""));
    assert!(response.contains("origin:https://ui-one.test"));
    assert!(
        !response
            .to_ascii_lowercase()
            .contains("access-control-allow-origin")
    );

    let response = String::from_utf8(request(
        proxy,
        b"GET /echo HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    ))
    .unwrap();
    assert!(response.starts_with("HTTP/1.1 400"));
    assert!(response.contains("\"code\":\"missing_route_identity\""));

    let response = String::from_utf8(request(
        proxy,
        b"GET /echo HTTP/1.1\r\nHost: localhost\r\nOrigin: https://ui-one.test\r\nX-Switchyard-Route: tab-one\r\nConnection: close\r\n\r\n",
    ))
    .unwrap()
    .to_ascii_lowercase();
    assert!(response.starts_with("http/1.1 200"));
    assert!(!response.contains("x-switchyard-route"));

    let response = String::from_utf8(request(
        proxy,
        b"GET /echo HTTP/1.1\r\nHost: localhost\r\nOrigin: https://unknown.test\r\nX-Switchyard-Route: tab-one\r\nConnection: close\r\n\r\n",
    ))
    .unwrap()
    .to_ascii_lowercase();
    assert!(response.starts_with("http/1.1 200"));
    assert!(!response.contains("access-control-allow-origin"));
    assert!(!response.contains("x-switchyard-route"));

    let response = String::from_utf8(request(
        proxy,
        b"OPTIONS /echo HTTP/1.1\r\nHost: localhost\r\nOrigin: https://unknown.test\r\nX-Switchyard-Route: tab-one\r\nAccess-Control-Request-Method: GET\r\nConnection: close\r\n\r\n",
    ))
    .unwrap()
    .to_ascii_lowercase();
    assert!(response.starts_with("http/1.1 403"));
    assert!(!response.contains("access-control-allow-origin"));

    let response = String::from_utf8(request(
        proxy,
        b"GET /echo HTTP/1.1\r\nHost: localhost\r\nOrigin: https://ui-one.test\r\nX-Switchyard-Route: unknown\r\nConnection: close\r\n\r\n",
    ))
    .unwrap();
    assert!(response.starts_with("HTTP/1.1 403"));
    assert!(response.contains("\"code\":\"unknown_route_identity\""));

    let response = String::from_utf8(request(
        proxy,
        b"GET /echo HTTP/1.1\r\nHost: localhost\r\nOrigin: https://ui-one.test\r\nX-Switchyard-Route: tab-two\r\nConnection: close\r\n\r\n",
    ))
    .unwrap();
    assert!(response.starts_with("HTTP/1.1 400"));
    assert!(response.contains("\"code\":\"conflicting_route_identity\""));

    running.shutdown();
}

#[test]
fn identity_header_preservation_requires_selected_provider_opt_in() {
    let upstream = TestUpstream::start();
    let proxy_port = unused_port();
    let mut config = browser_config(proxy_port, upstream.address.port());
    config.spec.identity.strip_before_forwarding = false;
    config.spec.providers[0].receive_identity_header = true;
    let engine = Arc::new(RouteEngine::new(config.clone()).unwrap());
    let running = HttpDataPlane::new(
        engine,
        config.spec.listeners.clone(),
        config.spec.identity.clone(),
        ProxyOptions::default(),
    )
    .unwrap()
    .spawn()
    .unwrap();
    assert!(running.wait_ready(Duration::from_secs(2)));
    let proxy = SocketAddr::from(([127, 0, 0, 1], proxy_port));

    let opted_in = String::from_utf8(request(
        proxy,
        b"GET /echo HTTP/1.1\r\nHost: localhost\r\nX-Switchyard-Route: tab-one\r\nConnection: close\r\n\r\n",
    ))
    .unwrap()
    .to_ascii_lowercase();
    assert!(opted_in.contains("x-switchyard-route: tab-one"));

    let not_opted_in = String::from_utf8(request(
        proxy,
        b"GET /echo HTTP/1.1\r\nHost: localhost\r\nX-Switchyard-Route: tab-two\r\nConnection: close\r\n\r\n",
    ))
    .unwrap()
    .to_ascii_lowercase();
    assert!(!not_opted_in.contains("x-switchyard-route"));

    running.shutdown();
}

#[test]
fn explicit_identity_is_rejected_on_non_loopback_listener() {
    let upstream = TestUpstream::start();
    let proxy_port = unused_port();
    let mut config = browser_config(proxy_port, upstream.address.port());
    config.spec.listeners[0].bind.host = "0.0.0.0".parse().unwrap();
    // Even with acknowledged LAN exposure, the explicit identity header must stay
    // untrusted on non-loopback listeners.
    config.spec.exposure = Some(router_config::GatewayExposure {
        mode: router_config::GatewayExposureMode::Lan,
        acknowledge_lan_exposure_risk: true,
    });
    let engine = Arc::new(RouteEngine::new(config.clone()).unwrap());
    let running = HttpDataPlane::new(
        engine,
        config.spec.listeners.clone(),
        config.spec.identity.clone(),
        ProxyOptions::default(),
    )
    .unwrap()
    .spawn()
    .unwrap();
    assert!(running.wait_ready(Duration::from_secs(2)));

    let response = String::from_utf8(request(
        SocketAddr::from(([127, 0, 0, 1], proxy_port)),
        b"GET /echo HTTP/1.1\r\nHost: localhost\r\nOrigin: https://ui-one.test\r\nX-Switchyard-Route: tab-one\r\nConnection: close\r\n\r\n",
    ))
    .unwrap()
    .to_ascii_lowercase();
    assert!(response.starts_with("http/1.1 403"));
    assert!(response.contains("\"code\":\"untrusted_identity_header\""));
    assert!(response.contains("access-control-allow-origin: https://ui-one.test"));

    running.shutdown();
}

#[test]
fn managed_profile_listener_requires_and_strips_proxy_credentials() {
    let upstream = TestUpstream::start();
    let proxy_port = unused_port();
    let credential = CredentialFile::create(proxy_port);
    let mut config = browser_config(proxy_port, upstream.address.port());
    config.spec.listeners[0].proxy_authentication = Some(ProxyAuthentication {
        scheme: ProxyAuthenticationScheme::Basic,
        credential_file: credential.0.clone(),
    });
    let engine = Arc::new(RouteEngine::new(config.clone()).unwrap());
    let running = HttpDataPlane::new(
        engine,
        config.spec.listeners.clone(),
        config.spec.identity.clone(),
        ProxyOptions::default(),
    )
    .unwrap()
    .spawn()
    .unwrap();
    assert!(running.wait_ready(Duration::from_secs(2)));
    let proxy = SocketAddr::from(([127, 0, 0, 1], proxy_port));

    let response = String::from_utf8(request(
        proxy,
        b"GET /echo HTTP/1.1\r\nHost: localhost\r\nOrigin: https://ui-one.test\r\nConnection: close\r\n\r\n",
    ))
    .unwrap()
    .to_ascii_lowercase();
    assert!(response.starts_with("http/1.1 407"));
    assert!(response.contains("proxy-authenticate: basic realm=\"switchyard\""));

    let response = String::from_utf8(request(
        proxy,
        b"GET /echo HTTP/1.1\r\nHost: localhost\r\nOrigin: https://ui-one.test\r\nProxy-Authorization: Basic wrong\r\nConnection: close\r\n\r\n",
    ))
    .unwrap();
    assert!(response.starts_with("HTTP/1.1 407"));
    assert!(!response.contains("test-token"));

    let response = String::from_utf8(request(
        proxy,
        b"GET /echo HTTP/1.1\r\nHost: localhost\r\nOrigin: https://ui-one.test\r\nProxy-Authorization: Basic c3dpdGNoeWFyZDp0ZXN0LXRva2Vu\r\nConnection: close\r\n\r\n",
    ))
    .unwrap()
    .to_ascii_lowercase();
    assert!(response.starts_with("http/1.1 200"));
    assert!(!response.contains("proxy-authorization"));

    running.shutdown();
}
