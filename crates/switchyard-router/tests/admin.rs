#![cfg(unix)]

use std::{path::Path, time::Duration};

use router_config::RouterConfig;
use serde_json::{Value, json};
use switchyard_router::{AdminOptions, RouterProcess};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
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

#[tokio::test]
async fn authenticates_inspects_applies_and_drains() {
    let socket = std::env::temp_dir().join(format!(
        "switchyard-router-admin-{}-{}.sock",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
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
