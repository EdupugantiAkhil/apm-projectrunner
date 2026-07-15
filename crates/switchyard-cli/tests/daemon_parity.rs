#![cfg(unix)]

use std::{fs, path::Path, process::Command, sync::Arc, time::Duration};

use axum::{Router, body::Body, http::Request};
use http_body_util::BodyExt;
use serde::de::DeserializeOwned;
use serde_json::json;
use switchyard_daemon::{
    DaemonConfig,
    contract::OperationV1,
    server::{CliBackend, api_for_tests},
};
use tempfile::TempDir;
use tower::ServiceExt;

fn prepare_project() -> TempDir {
    let temp = TempDir::new().unwrap();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../switchyard-planner/tests/fixtures");
    fs::copy(
        fixture.join("deployment.yaml"),
        temp.path().join("deployment.yaml"),
    )
    .unwrap();
    fs::copy(
        fixture.join("process-compose.yaml"),
        temp.path().join("process-compose.yaml"),
    )
    .unwrap();
    temp
}

async fn api_request<T: DeserializeOwned>(
    router: &Router,
    token: &str,
    method: &str,
    uri: &str,
    body: serde_json::Value,
) -> T {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success(), "{}", response.status());
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

#[tokio::test]
async fn no_daemon_fallback_and_api_backend_have_identical_output() {
    let temp = prepare_project();
    let binary = env!("CARGO_BIN_EXE_switchyard");
    let direct = Command::new(binary)
        .current_dir(temp.path())
        .args(["validate", "deployment.yaml"])
        .output()
        .unwrap();
    assert!(direct.status.success());
    assert!(!temp.path().join(".switchyard/daemon.json").exists());

    let config = DaemonConfig::new(temp.path().into(), binary.into());
    let backend = Arc::new(CliBackend::new(binary.into(), temp.path().into()));
    let (router, token, _) = api_for_tests(config, backend).unwrap();
    let started: OperationV1 = api_request(
        &router,
        &token,
        "POST",
        "/api/v1/commands/validate",
        json!({"bundle": "deployment.yaml"}),
    )
    .await;
    let completed = loop {
        let operation: OperationV1 = api_request(
            &router,
            &token,
            "GET",
            &format!("/api/v1/operations/{}", started.id),
            serde_json::Value::Null,
        )
        .await;
        if operation.status.terminal() {
            break operation;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let result = completed.result.unwrap();
    assert_eq!(result.exit_code, direct.status.code().unwrap());
    assert_eq!(result.stdout.as_bytes(), direct.stdout);
    assert_eq!(result.stderr.as_bytes(), direct.stderr);

    let direct_error = Command::new(binary)
        .current_dir(temp.path())
        .args(["validate", "missing.yaml"])
        .output()
        .unwrap();
    let started: OperationV1 = api_request(
        &router,
        &token,
        "POST",
        "/api/v1/commands/validate",
        json!({"bundle": "missing.yaml"}),
    )
    .await;
    let failed = loop {
        let operation: OperationV1 = api_request(
            &router,
            &token,
            "GET",
            &format!("/api/v1/operations/{}", started.id),
            serde_json::Value::Null,
        )
        .await;
        if operation.status.terminal() {
            break operation;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let result = failed.result.unwrap();
    assert_eq!(result.exit_code, direct_error.status.code().unwrap());
    assert_eq!(result.stdout.as_bytes(), direct_error.stdout);
    assert_eq!(result.stderr.as_bytes(), direct_error.stderr);
}
