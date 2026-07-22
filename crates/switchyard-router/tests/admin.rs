#![cfg(unix)]

use std::{
    io::Write as _,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

use router_config::RouterConfig;
use serde_json::{Value, json};
use switchyard_router::{AdminOptions, RouterProcess};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, UnixStream},
};

fn config(version: u64) -> RouterConfig {
    serde_json::from_value(json!({
        "apiVersion": "switchyard.dev/router/v1alpha1",
        "kind": "RouterConfiguration",
        "metadata": { "deployment": "admin-test" },
        "spec": {
            "snapshot": {
                "id": "admin-snapshot",
                "version": version,
                "transitions": {
                    "http": { "strategy": "close" },
                    "https": { "strategy": "close" },
                    "websocket": { "strategy": "pin" },
                    "grpc": { "strategy": "close" },
                    "tcp": { "strategy": "close" }
                }
            }
        }
    }))
    .unwrap()
}

async fn request(path: &Path, request: Value) -> Value {
    let mut stream = UnixStream::connect(path).await.unwrap();
    let mut encoded = serde_json::to_vec(&request).unwrap();
    encoded.push(b'\n');
    stream.write_all(&encoded).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    serde_json::from_slice(&response).unwrap()
}

async fn bridged_request(path: &Path, request: Value) -> std::process::Output {
    let path = path.to_owned();
    let mut encoded = serde_json::to_vec(&request).unwrap();
    encoded.push(b'\n');
    tokio::task::spawn_blocking(move || {
        let mut child = Command::new(env!("CARGO_BIN_EXE_switchyard-router"))
            .arg("admin-client")
            .arg(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&encoded).unwrap();
        child.wait_with_output().unwrap()
    })
    .await
    .unwrap()
}

fn socket_path(prefix: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in("/private/tmp")
        .unwrap();
    let socket = directory.path().join("admin.sock");
    (directory, socket)
}

#[tokio::test]
async fn authenticates_inspects_applies_and_drains() {
    let (_directory, socket) = socket_path("sy-admin-");
    let process = RouterProcess::start(
        config(1),
        AdminOptions {
            socket_path: socket.clone(),
            token: "test-secret".into(),
        },
    )
    .await
    .unwrap();

    let unauthorized = request(
        &socket,
        json!({"token": "wrong", "operation": "current-version"}),
    )
    .await;
    assert_eq!(unauthorized["error"]["code"], "unauthorized");

    let current = request(
        &socket,
        json!({"token": "test-secret", "operation": "current-version"}),
    )
    .await;
    assert_eq!(current["result"]["version"], 1);

    let applied = request(
        &socket,
        json!({"token": "test-secret", "operation": "apply", "config": config(2)}),
    )
    .await;
    assert_eq!(applied["result"]["status"], "activated");

    let counters = request(
        &socket,
        json!({"token": "test-secret", "operation": "counters"}),
    )
    .await;
    assert_eq!(counters["result"]["activeSnapshotVersion"], 2);
    assert!(counters["result"]["adminRequests"].as_u64().unwrap() >= 4);

    let events = request(
        &socket,
        json!({"token": "test-secret", "operation": "events"}),
    )
    .await;
    assert!(!events.to_string().contains("test-secret"));

    let drained = request(
        &socket,
        json!({"token": "test-secret", "operation": "drain"}),
    )
    .await;
    assert_eq!(drained["result"]["status"], "draining");
    tokio::time::timeout(Duration::from_secs(2), process.wait())
        .await
        .expect("router did not shut down after drain")
        .unwrap();
    assert!(!socket.exists());
}

#[tokio::test]
async fn admin_client_preserves_framing_authentication_and_response() {
    let (_directory, socket) = socket_path("sy-admin-bridge-");
    let process = RouterProcess::start(
        config(7),
        AdminOptions {
            socket_path: socket.clone(),
            token: "bridge-secret".into(),
        },
    )
    .await
    .unwrap();

    let unauthorized = bridged_request(
        &socket,
        json!({"token": "wrong", "operation": "current-version"}),
    )
    .await;
    assert!(unauthorized.status.success());
    let unauthorized: Value = serde_json::from_slice(&unauthorized.stdout).unwrap();
    assert_eq!(unauthorized["error"]["code"], "unauthorized");

    let current = bridged_request(
        &socket,
        json!({"token": "bridge-secret", "operation": "current-version"}),
    )
    .await;
    assert!(current.status.success());
    let current: Value = serde_json::from_slice(&current.stdout).unwrap();
    assert_eq!(current["result"]["version"], 7);
    assert!(!current.to_string().contains("bridge-secret"));

    process.request_shutdown();
    process.wait().await.unwrap();
}

#[tokio::test]
async fn unhealthy_candidate_rolls_back_without_displacing_active_snapshot() {
    let health = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let unavailable_port = health.local_addr().unwrap().port();
    drop(health);
    let (_directory, socket) = socket_path("sy-rollback-");
    let process = RouterProcess::start(
        config(1),
        AdminOptions {
            socket_path: socket.clone(),
            token: "test-secret".into(),
        },
    )
    .await
    .unwrap();
    let mut candidate = config(2);
    candidate.spec.providers.push(
        serde_json::from_value(json!({
            "id": "unavailable",
            "endpoint": { "protocol": "http", "host": "127.0.0.1", "port": unavailable_port },
            "healthCheck": { "protocol": "http", "path": "/health", "intervalMs": 1000, "timeoutMs": 100 }
        }))
        .unwrap(),
    );

    let rejected = request(
        &socket,
        json!({"token": "test-secret", "operation": "apply", "config": candidate}),
    )
    .await;
    assert_eq!(rejected["error"]["code"], "provider_unhealthy");
    assert_eq!(rejected["error"]["status"], "rolled_back");
    assert_eq!(rejected["error"]["activeVersion"], 1);

    let current = request(
        &socket,
        json!({"token": "test-secret", "operation": "current-version"}),
    )
    .await;
    assert_eq!(current["result"]["version"], 1);
    let events = request(
        &socket,
        json!({"token": "test-secret", "operation": "events"}),
    )
    .await;
    assert!(events.to_string().contains("rolled_back"));

    process.request_shutdown();
    process.wait().await.unwrap();
}
