use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    process::Command,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use rustls::{
    ClientConfig, ClientConnection, RootCertStore, StreamOwned,
    pki_types::{CertificateDer, ServerName, pem::PemObject},
};
use serde_json::json;
use switchyard_router::{
    AdminOptions, RouterProcess,
    host_gateway::{ensure_certificates, preflight},
};

#[test]
fn host_auth_precedes_config_access_but_certificate_commands_remain_tokenless() {
    let directory = tempfile::tempdir().unwrap();
    let missing_config = directory.path().join("missing.json");
    let binary = env!("CARGO_BIN_EXE_switchyard-router");

    let host = Command::new(binary)
        .args(["host", missing_config.to_str().unwrap(), "admin.socket"])
        .env_remove("SWITCHYARD_ROUTER_TOKEN")
        .output()
        .unwrap();
    let host_error = String::from_utf8_lossy(&host.stderr);
    assert!(!host.status.success());
    assert!(
        host_error.contains("SWITCHYARD_ROUTER_TOKEN"),
        "{host_error}"
    );

    for action in ["trust", "cleanup"] {
        let maintenance = Command::new(binary)
            .args(["certificates", action, missing_config.to_str().unwrap()])
            .env_remove("SWITCHYARD_ROUTER_TOKEN")
            .output()
            .unwrap();
        let maintenance_error = String::from_utf8_lossy(&maintenance.stderr);
        assert!(!maintenance.status.success());
        assert!(
            !maintenance_error.contains("SWITCHYARD_ROUTER_TOKEN"),
            "{maintenance_error}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn host_mode_terminates_https_and_routes_to_loopback() {
    let upstream = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let upstream_port = upstream.local_addr().unwrap().port();
    let gateway_port = free_port();
    let directory = tempfile::tempdir().unwrap();
    let certificate = directory.path().join("host.pem");
    let private_key = directory.path().join("host-key.pem");
    let socket = directory.path().join("admin.socket");
    let config = serde_json::from_value(json!({
        "apiVersion": "switchyard.dev/router/v1alpha1",
        "kind": "RouterConfiguration",
        "metadata": { "deployment": "host-test" },
        "spec": {
            "snapshot": {
                "id": "host-1", "version": 1,
                "transitions": {
                    "http": { "strategy": "close" }, "https": { "strategy": "close" },
                    "websocket": { "strategy": "close" }, "grpc": { "strategy": "close" },
                    "tcp": { "strategy": "close" }
                }
            },
            "listeners": [{
                "consumer": "gateway",
                "bind": { "host": "127.0.0.1", "port": gateway_port },
                "protocol": "https",
                "tls": { "certificate": certificate, "privateKey": private_key },
                "destinations": [{ "kind": "custom_domain", "slot": "ui", "domain": "ui.host-test.localhost" }]
            }],
            "providers": [{
                "id": "ui", "endpoint": { "protocol": "http", "host": "127.0.0.1", "port": upstream_port }
            }],
            "groups": [], "bindings": [],
            "routes": [{ "consumer": "gateway", "slot": "ui", "provider": "ui" }],
            "browserRoutes": [],
            "identity": { "explicitHeader": "X-Switchyard-Route", "stripBeforeForwarding": true }
        }
    }))
    .unwrap();

    preflight(&config).unwrap();
    ensure_certificates(&config).unwrap();
    let upstream_task = thread::spawn(move || {
        let (mut stream, _) = upstream.accept().unwrap();
        let mut request = [0_u8; 4096];
        let _ = stream.read(&mut request).unwrap();
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\nConnection: close\r\n\r\nhost-gateway",
            )
            .unwrap();
    });
    let process = RouterProcess::start(
        config,
        AdminOptions {
            socket_path: socket,
            token: "test-token".into(),
        },
    )
    .await
    .unwrap();

    wait_for_port(gateway_port);
    let response = tokio::task::spawn_blocking(move || {
        let mut roots = RootCertStore::empty();
        roots
            .add(CertificateDer::from_pem_file(certificate).unwrap())
            .unwrap();
        let client = Arc::new(
            ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        );
        let stream = TcpStream::connect(("127.0.0.1", gateway_port)).unwrap();
        let connection = ClientConnection::new(
            client,
            ServerName::try_from("ui.host-test.localhost").unwrap(),
        )
        .unwrap();
        let mut stream = StreamOwned::new(connection, stream);
        stream
            .write_all(
                b"GET / HTTP/1.1\r\nHost: ui.host-test.localhost\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    })
    .await
    .unwrap();
    assert!(response.contains("200 OK"), "{response}");
    assert!(response.ends_with("host-gateway"), "{response}");

    process.request_shutdown();
    process.wait().await.unwrap();
    upstream_task.join().unwrap();
}

fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn wait_for_port(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline, "gateway did not listen in time");
        thread::sleep(Duration::from_millis(10));
    }
}
