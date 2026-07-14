use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use router_config::RouterConfig;
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
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
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
