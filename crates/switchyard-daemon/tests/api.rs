#![cfg(unix)]

use std::{
    fs,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{Router, body::Body, http::Request};
use http_body_util::BodyExt;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use switchyard_daemon::{
    DaemonConfig,
    contract::{CommandKind, CommandResultV1, OperationStatusV1, OperationV1},
    server::{BackendOutcome, EventSink, OperationBackend, api_for_tests},
};
use tempfile::TempDir;
use tokio::sync::watch;
use tower::ServiceExt;

#[derive(Default)]
struct StubBackend;

impl OperationBackend for StubBackend {
    fn run(
        &self,
        _kind: CommandKind,
        _arguments: Vec<String>,
        mut cancellation: watch::Receiver<bool>,
        events: EventSink,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<BackendOutcome, switchyard_daemon::contract::ApiErrorV1>>
                + Send,
        >,
    > {
        Box::pin(async move {
            for kind in [
                switchyard_daemon::contract::EventKindV1::Build,
                switchyard_daemon::contract::EventKindV1::Health,
                switchyard_daemon::contract::EventKindV1::Route,
                switchyard_daemon::contract::EventKindV1::Log,
            ] {
                events.emit(kind, json!({"message": "stub"}));
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(180)) => Ok(BackendOutcome::Completed(CommandResultV1 {
                    exit_code: 0,
                    stdout: "stub output\n".into(),
                    stderr: String::new(),
                })),
                _ = async {
                    while !*cancellation.borrow() && cancellation.changed().await.is_ok() {}
                } => Ok(BackendOutcome::Cancelled(CommandResultV1 {
                    exit_code: 130,
                    stdout: String::new(),
                    stderr: String::new(),
                })),
            }
        })
    }
}

struct CountingBackend {
    active: Arc<AtomicUsize>,
    maximum: Arc<AtomicUsize>,
}

impl OperationBackend for CountingBackend {
    fn run(
        &self,
        _kind: CommandKind,
        _arguments: Vec<String>,
        mut cancellation: watch::Receiver<bool>,
        _events: EventSink,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<BackendOutcome, switchyard_daemon::contract::ApiErrorV1>>
                + Send,
        >,
    > {
        let active = self.active.clone();
        let maximum = self.maximum.clone();
        Box::pin(async move {
            let current = active.fetch_add(1, Ordering::SeqCst) + 1;
            maximum.fetch_max(current, Ordering::SeqCst);
            let cancelled = tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(180)) => false,
                _ = async { while !*cancellation.borrow() && cancellation.changed().await.is_ok() {} } => true,
            };
            active.fetch_sub(1, Ordering::SeqCst);
            let result = CommandResultV1 {
                exit_code: if cancelled { 130 } else { 0 },
                stdout: "ok\n".into(),
                stderr: String::new(),
            };
            Ok(if cancelled {
                BackendOutcome::Cancelled(result)
            } else {
                BackendOutcome::Completed(result)
            })
        })
    }
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../switchyard-planner/tests/fixtures/deployment.yaml")
}

fn second_fixture(temp: &TempDir) -> PathBuf {
    let original = fs::read_to_string(fixture()).unwrap();
    let path = temp.path().join("second.yaml");
    fs::write(
        &path,
        original.replace("name: comparison", "name: comparison-two"),
    )
    .unwrap();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../switchyard-planner/tests/fixtures/process-compose.yaml"),
        temp.path().join("process-compose.yaml"),
    )
    .unwrap();
    path
}

#[derive(Clone)]
struct TestApi {
    router: Router,
    token: String,
}

fn start_api(temp: &TempDir, backend: Arc<dyn OperationBackend>, limit: usize) -> TestApi {
    let mut config = DaemonConfig::new(temp.path().into(), "unused".into());
    config.max_heavy_operations = limit;
    let (router, token, _) = api_for_tests(config, backend).unwrap();
    TestApi { router, token }
}

async fn request(
    api: &TestApi,
    token: Option<&str>,
    method: &str,
    path: &str,
    body: Option<Value>,
    extra_headers: &[(&str, &str)],
) -> (u16, Vec<u8>) {
    let encoded = body
        .map(|body| serde_json::to_vec(&body).unwrap())
        .unwrap_or_default();
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    for (name, value) in extra_headers {
        builder = builder.header(*name, *value);
    }
    let response = api
        .router
        .clone()
        .oneshot(builder.body(Body::from(encoded)).unwrap())
        .await
        .unwrap();
    let status = response.status().as_u16();
    let body = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, body)
}

fn json_body<T: DeserializeOwned>(body: &[u8]) -> T {
    serde_json::from_slice(body).unwrap()
}

fn command_body(bundle: &Path) -> Value {
    json!({"bundle": bundle})
}

async fn wait_terminal(api: &TestApi, id: &str) -> OperationV1 {
    loop {
        let (status, body) = request(
            api,
            Some(&api.token),
            "GET",
            &format!("/api/v1/operations/{id}"),
            None,
            &[],
        )
        .await;
        assert_eq!(status, 200);
        let operation: OperationV1 = json_body(&body);
        if operation.status.terminal() {
            return operation;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn auth_and_versioned_surface_are_enforced() {
    let temp = TempDir::new().unwrap();
    let daemon = start_api(&temp, Arc::new(StubBackend), 2);
    assert_eq!(
        request(&daemon, None, "GET", "/api/v1/system/status", None, &[])
            .await
            .0,
        401
    );
    assert_eq!(
        request(
            &daemon,
            Some("wrong"),
            "GET",
            "/api/v1/system/status",
            None,
            &[]
        )
        .await
        .0,
        401
    );
    assert_eq!(
        request(
            &daemon,
            Some(&daemon.token),
            "GET",
            "/api/v1/system/status",
            None,
            &[]
        )
        .await
        .0,
        200
    );
    assert_eq!(
        request(
            &daemon,
            Some(&daemon.token),
            "GET",
            "/system/status",
            None,
            &[]
        )
        .await
        .0,
        404
    );
    let malformed = request(
        &daemon,
        Some(&daemon.token),
        "POST",
        "/api/v1/commands/validate",
        None,
        &[],
    )
    .await;
    assert_eq!(malformed.0, 400);
    let error: switchyard_daemon::contract::ApiErrorV1 = json_body(&malformed.1);
    assert_eq!(error.code, "invalid_json");
}

#[tokio::test]
async fn sse_replays_events_after_last_event_id() {
    let temp = TempDir::new().unwrap();
    let daemon = start_api(&temp, Arc::new(StubBackend), 2);
    let (status, body) = request(
        &daemon,
        Some(&daemon.token),
        "POST",
        "/api/v1/commands/validate",
        Some(command_body(&fixture())),
        &[],
    )
    .await;
    assert_eq!(status, 202);
    let operation: OperationV1 = json_body(&body);
    wait_terminal(&daemon, &operation.id).await;
    let path = format!("/api/v1/operations/{}/events", operation.id);
    let (_, all) = request(&daemon, Some(&daemon.token), "GET", &path, None, &[]).await;
    let all = String::from_utf8(all).unwrap();
    assert!(all.contains("id: 1"));
    assert!(all.contains("event: build"));
    assert!(all.contains("event: health"));
    assert!(all.contains("event: route"));
    assert!(all.contains("event: log"));
    let (_, resumed) = request(
        &daemon,
        Some(&daemon.token),
        "GET",
        &path,
        None,
        &[("Last-Event-ID", "1")],
    )
    .await;
    let resumed = String::from_utf8(resumed).unwrap();
    assert!(!resumed.contains("id: 1\n"));
    assert!(resumed.contains("id: 2"));
}

#[tokio::test]
async fn mutation_lock_global_limit_and_cancellation_work() {
    let temp = TempDir::new().unwrap();
    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let daemon = start_api(
        &temp,
        Arc::new(CountingBackend {
            active: active.clone(),
            maximum: maximum.clone(),
        }),
        1,
    );

    let bind = json!({"bundle": fixture(), "consumer": "consumer-a", "group": "base"});
    let (status, first) = request(
        &daemon,
        Some(&daemon.token),
        "POST",
        "/api/v1/commands/bind",
        Some(bind.clone()),
        &[],
    )
    .await;
    assert_eq!(status, 202);
    let first: OperationV1 = json_body(&first);
    assert_eq!(
        request(
            &daemon,
            Some(&daemon.token),
            "POST",
            "/api/v1/commands/bind",
            Some(bind),
            &[]
        )
        .await
        .0,
        409
    );
    wait_terminal(&daemon, &first.id).await;

    let second = second_fixture(&temp);
    let (_, one) = request(
        &daemon,
        Some(&daemon.token),
        "POST",
        "/api/v1/commands/apply",
        Some(command_body(&fixture())),
        &[],
    )
    .await;
    let one: OperationV1 = json_body(&one);
    let (_, two) = request(
        &daemon,
        Some(&daemon.token),
        "POST",
        "/api/v1/commands/apply",
        Some(command_body(&second)),
        &[],
    )
    .await;
    let two: OperationV1 = json_body(&two);
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(maximum.load(Ordering::SeqCst), 1);
    wait_terminal(&daemon, &one.id).await;
    wait_terminal(&daemon, &two.id).await;

    let (_, operation) = request(
        &daemon,
        Some(&daemon.token),
        "POST",
        "/api/v1/commands/apply",
        Some(command_body(&fixture())),
        &[],
    )
    .await;
    let operation: OperationV1 = json_body(&operation);
    assert_eq!(
        request(
            &daemon,
            Some(&daemon.token),
            "POST",
            &format!("/api/v1/operations/{}/cancel", operation.id),
            None,
            &[]
        )
        .await
        .0,
        202
    );
    let cancelled = wait_terminal(&daemon, &operation.id).await;
    assert_eq!(cancelled.status, OperationStatusV1::Cancelled);
}

#[tokio::test]
async fn restart_keeps_final_operation_state_in_sqlite() {
    let temp = TempDir::new().unwrap();
    let daemon = start_api(&temp, Arc::new(StubBackend), 2);
    let (_, body) = request(
        &daemon,
        Some(&daemon.token),
        "POST",
        "/api/v1/commands/validate",
        Some(command_body(&fixture())),
        &[],
    )
    .await;
    let operation: OperationV1 = json_body(&body);
    wait_terminal(&daemon, &operation.id).await;
    drop(daemon);

    let restarted = start_api(&temp, Arc::new(StubBackend), 2);
    let (status, body) = request(
        &restarted,
        Some(&restarted.token),
        "GET",
        &format!("/api/v1/operations/{}", operation.id),
        None,
        &[],
    )
    .await;
    assert_eq!(status, 200);
    let restored: OperationV1 = json_body(&body);
    assert_eq!(restored.status, OperationStatusV1::Succeeded);
    assert!(restored.result.is_none());
}
