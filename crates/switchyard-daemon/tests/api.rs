#![cfg(unix)]

use std::{
    fs,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
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
use switchyard_state::{
    AppliedSnapshot, LockRequest, OperationKind, OperationRecord, OperationStatus,
    RouterApplyRecord, RouterApplyStatus, StateStore, StructuredContext,
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

struct LiveBindingBackend;

struct ImmediateBackend;

struct LockLossBackend {
    cancelled: Arc<AtomicBool>,
}

impl OperationBackend for LockLossBackend {
    fn run(
        &self,
        _kind: CommandKind,
        _arguments: Vec<String>,
        _cancellation: watch::Receiver<bool>,
        _events: EventSink,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<BackendOutcome, switchyard_daemon::contract::ApiErrorV1>>
                + Send,
        >,
    > {
        Box::pin(async { unreachable!("bind uses the native backend hook") })
    }

    fn live_bind(
        &self,
        request: switchyard_daemon::contract::CommandRequestV1,
        operation_id: String,
        mut cancellation: watch::Receiver<bool>,
        _events: EventSink,
    ) -> Option<
        Pin<
            Box<
                dyn Future<Output = Result<BackendOutcome, switchyard_daemon::contract::ApiErrorV1>>
                    + Send,
            >,
        >,
    > {
        let cancelled = self.cancelled.clone();
        let binding = request.consumer.unwrap();
        Some(Box::pin(async move {
            while !*cancellation.borrow() && cancellation.changed().await.is_ok() {}
            cancelled.store(true, Ordering::SeqCst);
            let empty = || StructuredContext::new(json!({})).unwrap();
            Ok(BackendOutcome::LiveBinding {
                result: CommandResultV1 {
                    exit_code: 130,
                    stdout: String::new(),
                    stderr: "cancelled after lock loss\n".into(),
                },
                attempts: vec![RouterApplyRecord {
                    deployment: "comparison".into(),
                    router: "sidecar:backend-a".into(),
                    binding,
                    operation_id,
                    desired_version: 2,
                    desired_checksum: "candidate".into(),
                    version: 2,
                    checksum: "candidate".into(),
                    status: RouterApplyStatus::Failed,
                    observed_version: Some(1),
                    observed_checksum: Some("active".into()),
                    transition: empty(),
                    error_code: Some("cancelled_after_lock_loss".into()),
                    recorded_at: 12,
                    context: empty(),
                }],
            })
        }))
    }
}

impl OperationBackend for ImmediateBackend {
    fn run(
        &self,
        _kind: CommandKind,
        _arguments: Vec<String>,
        _cancellation: watch::Receiver<bool>,
        _events: EventSink,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<BackendOutcome, switchyard_daemon::contract::ApiErrorV1>>
                + Send,
        >,
    > {
        Box::pin(async {
            Ok(BackendOutcome::Completed(CommandResultV1 {
                exit_code: 0,
                stdout: "done\n".into(),
                stderr: String::new(),
            }))
        })
    }
}

impl OperationBackend for LiveBindingBackend {
    fn run(
        &self,
        _kind: CommandKind,
        _arguments: Vec<String>,
        _cancellation: watch::Receiver<bool>,
        _events: EventSink,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<BackendOutcome, switchyard_daemon::contract::ApiErrorV1>>
                + Send,
        >,
    > {
        Box::pin(async { unreachable!("bind uses the native backend hook") })
    }

    fn live_bind(
        &self,
        request: switchyard_daemon::contract::CommandRequestV1,
        operation_id: String,
        _cancellation: watch::Receiver<bool>,
        _events: EventSink,
    ) -> Option<
        Pin<
            Box<
                dyn Future<Output = Result<BackendOutcome, switchyard_daemon::contract::ApiErrorV1>>
                    + Send,
            >,
        >,
    > {
        let binding = request.consumer.unwrap();
        Some(Box::pin(async move {
            let empty = || StructuredContext::new(json!({})).unwrap();
            Ok(BackendOutcome::LiveBinding {
                result: CommandResultV1 {
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: "rejected\n".into(),
                },
                attempts: vec![
                    RouterApplyRecord {
                        deployment: "comparison".into(),
                        router: format!("sidecar:{binding}"),
                        binding: binding.clone(),
                        operation_id: operation_id.clone(),
                        desired_version: 2,
                        desired_checksum: "candidate".into(),
                        version: 2,
                        checksum: "candidate".into(),
                        status: RouterApplyStatus::Failed,
                        observed_version: Some(1),
                        observed_checksum: Some("active".into()),
                        transition: empty(),
                        error_code: Some("timeout".into()),
                        recorded_at: 10,
                        context: empty(),
                    },
                    RouterApplyRecord {
                        deployment: "comparison".into(),
                        router: "host-gateway".into(),
                        binding,
                        operation_id,
                        desired_version: 2,
                        desired_checksum: "candidate-host".into(),
                        version: 2,
                        checksum: "candidate-host".into(),
                        status: RouterApplyStatus::RolledBack,
                        observed_version: Some(1),
                        observed_checksum: Some("active-host".into()),
                        transition: empty(),
                        error_code: Some("provider_unhealthy".into()),
                        recorded_at: 11,
                        context: StructuredContext::new(json!({"status": "rolled_back"})).unwrap(),
                    },
                ],
            })
        }))
    }
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
    reconciliation: switchyard_state::ReconciliationReport,
}

fn start_api(temp: &TempDir, backend: Arc<dyn OperationBackend>, limit: usize) -> TestApi {
    let mut config = DaemonConfig::new(temp.path().into(), "unused".into());
    config.max_heavy_operations = limit;
    let (router, token, reconciliation) = api_for_tests(config, backend).unwrap();
    TestApi {
        router,
        token,
        reconciliation,
    }
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
async fn deployment_and_adapter_endpoints_are_authenticated_and_shape_empty_state() {
    let temp = tempfile::tempdir().unwrap();
    let api = start_api(&temp, Arc::new(ImmediateBackend), 1);

    for path in ["/api/v1/deployments", "/api/v1/adapters"] {
        let (status, _) = request(&api, None, "GET", path, None, &[]).await;
        assert_eq!(status, 401, "{path} bypassed authentication");
    }
    let (status, body) = request(
        &api,
        Some(&api.token),
        "GET",
        "/api/v1/deployments",
        None,
        &[],
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(
        json_body::<Value>(&body),
        json!({"apiVersion":"v1","deployments":[]})
    );
    let (status, body) =
        request(&api, Some(&api.token), "GET", "/api/v1/adapters", None, &[]).await;
    assert_eq!(status, 200);
    let adapters = json_body::<Value>(&body);
    let first = adapters.as_array().unwrap().first().unwrap();
    assert!(first.get("kind").is_some());
    assert!(first.get("declaration").is_some());
    assert!(first.get("configurationSchema").is_some());
}

fn definition_yaml(name: &str) -> String {
    format!(
        "apiVersion: switchyard.dev/v1alpha1\nkind: Deployment\nmetadata:\n  name: {name}\nspec: {{}}\n"
    )
}

#[tokio::test]
async fn deployment_definition_endpoints_validate_and_write_atomically() {
    let temp = tempfile::tempdir().unwrap();
    let api = start_api(&temp, Arc::new(ImmediateBackend), 1);
    let yaml = definition_yaml("demo");

    for (method, path) in [
        ("POST", "/api/v1/deployments"),
        ("GET", "/api/v1/deployments/demo/definition"),
        ("PUT", "/api/v1/deployments/demo/definition"),
    ] {
        assert_eq!(request(&api, None, method, path, None, &[]).await.0, 401);
    }

    let (status, body) = request(
        &api,
        Some(&api.token),
        "POST",
        "/api/v1/deployments",
        Some(json!({"name":"demo","yaml":yaml,"validateOnly":true})),
        &[],
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(json_body::<Value>(&body)["valid"], true);
    assert!(!temp.path().join("deployments/demo.yaml").exists());

    let (status, body) = request(
        &api,
        Some(&api.token),
        "POST",
        "/api/v1/deployments",
        Some(json!({"name":"demo","yaml":yaml})),
        &[],
    )
    .await;
    assert_eq!(status, 201);
    let created = json_body::<Value>(&body);
    assert_eq!(created["yaml"], yaml);
    assert!(Path::new(created["path"].as_str().unwrap()).is_absolute());

    let (status, body) = request(
        &api,
        Some(&api.token),
        "GET",
        "/api/v1/deployments/demo/definition",
        None,
        &[],
    )
    .await;
    assert_eq!(status, 200);
    let definition = json_body::<Value>(&body);
    let updated_yaml = format!("{}# edited\n", definition_yaml("demo"));
    let (status, body) = request(
        &api,
        Some(&api.token),
        "PUT",
        "/api/v1/deployments/demo/definition",
        Some(json!({"yaml":updated_yaml,"expectedHash":definition["hash"]})),
        &[],
    )
    .await;
    assert_eq!(status, 200);
    let updated = json_body::<Value>(&body);
    assert_eq!(updated["yaml"], updated_yaml);

    let (status, body) = request(
        &api,
        Some(&api.token),
        "PUT",
        "/api/v1/deployments/demo/definition",
        Some(json!({"yaml":"not: [valid","expectedHash":updated["hash"]})),
        &[],
    )
    .await;
    assert_eq!(status, 422);
    let error = json_body::<Value>(&body);
    assert_eq!(error["code"], "validation_failed");
    assert!(error["context"]["diagnostics"].is_array());
    assert_eq!(
        fs::read_to_string(temp.path().join("deployments/demo.yaml")).unwrap(),
        updated_yaml
    );

    let (status, body) = request(
        &api,
        Some(&api.token),
        "POST",
        "/api/v1/deployments",
        Some(json!({"name":"demo","yaml":yaml})),
        &[],
    )
    .await;
    assert_eq!(status, 409);
    assert_eq!(json_body::<Value>(&body)["code"], "deployment_exists");

    let (status, body) = request(
        &api,
        Some(&api.token),
        "PUT",
        "/api/v1/deployments/demo/definition",
        Some(json!({"yaml":yaml,"expectedHash":"stale"})),
        &[],
    )
    .await;
    assert_eq!(status, 409);
    assert_eq!(json_body::<Value>(&body)["code"], "definition_conflict");
}

#[tokio::test]
async fn definition_absence_and_validation_failures_have_stable_structured_errors() {
    let temp = tempfile::tempdir().unwrap();
    let api = start_api(&temp, Arc::new(ImmediateBackend), 1);
    let (status, body) = request(
        &api,
        Some(&api.token),
        "GET",
        "/api/v1/deployments/missing/definition",
        None,
        &[],
    )
    .await;
    assert_eq!(status, 404);
    assert_eq!(
        json_body::<Value>(&body)["code"],
        "deployment_definition_not_found"
    );

    let (status, body) = request(
        &api,
        Some(&api.token),
        "POST",
        "/api/v1/deployments",
        Some(json!({"name":"demo","yaml":"not: [valid"})),
        &[],
    )
    .await;
    assert_eq!(status, 422);
    let error = json_body::<Value>(&body);
    assert_eq!(error["code"], "validation_failed");
    assert_eq!(error["context"]["diagnostics"][0]["code"], "invalid_yaml");
    assert!(error["context"]["diagnostics"][0]["message"].is_string());
    assert!(!temp.path().join("deployments/demo.yaml").exists());

    let (status, body) = request(
        &api,
        Some(&api.token),
        "PUT",
        "/api/v1/deployments/..%2Fescape/definition",
        Some(json!({"yaml":"unused","expectedHash":"unused"})),
        &[],
    )
    .await;
    assert_eq!(status, 404);
    assert_eq!(
        json_body::<Value>(&body)["code"],
        "deployment_definition_not_found"
    );
}

#[tokio::test]
async fn deployment_list_and_detail_include_applied_manifest_and_reconciliation() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join(".switchyard/generated/demo")).unwrap();
    let (mut store, _) = StateStore::open(temp.path().join(".switchyard/state.sqlite3")).unwrap();
    let snapshot = AppliedSnapshot::from_json(json!({
        "spec": {
            "bindings": {"consumer-a": "feature"},
            "hostRouter": {"listeners": [
                {"kind": "custom_domain", "domain": "demo.localhost"}
            ]}
        }
    }))
    .unwrap();
    store
        .record_applied_snapshot("demo", "definition-1", &snapshot, 10)
        .unwrap();
    store
        .start_operation(&OperationRecord {
            id: "op-1".into(),
            deployment: "demo".into(),
            kind: OperationKind::Apply,
            status: OperationStatus::Succeeded,
            started_at: 11,
            finished_at: Some(12),
            error: None,
        })
        .unwrap();
    fs::write(
        temp.path().join(".switchyard/generated/demo/manifest.json"),
        serde_json::to_vec(&json!({
            "deployment": "demo",
            "definitionHash": "definition-1",
            "resourceHash": "resource-1",
            "sourceIdentities": {
                "consumer-a": {"path":"/work/demo","ref":"feature/x","commit":"abcdef123456","dirty":true}
            }
        }))
        .unwrap(),
    )
    .unwrap();
    drop(store);
    let api = start_api(&temp, Arc::new(ImmediateBackend), 1);

    let (status, body) = request(
        &api,
        Some(&api.token),
        "GET",
        "/api/v1/deployments",
        None,
        &[],
    )
    .await;
    assert_eq!(status, 200);
    let list = json_body::<Value>(&body);
    assert_eq!(list["deployments"][0]["name"], "demo");
    assert_eq!(list["deployments"][0]["resourceHash"], "resource-1");
    assert_eq!(
        list["deployments"][0]["lastOperation"]["status"],
        "succeeded"
    );
    assert_eq!(list["deployments"][0]["customDomains"][0], "demo.localhost");

    let (status, body) = request(
        &api,
        Some(&api.token),
        "GET",
        "/api/v1/deployments/demo",
        None,
        &[],
    )
    .await;
    assert_eq!(status, 200);
    let detail = json_body::<Value>(&body);
    assert_eq!(
        detail["sourceIdentities"]["consumer-a"]["commit"],
        "abcdef123456"
    );
    assert_eq!(detail["bindings"]["consumer-a"], "feature");
    assert_eq!(detail["reconciliation"]["deployment"], "demo");

    let (status, _) = request(
        &api,
        Some(&api.token),
        "GET",
        "/api/v1/deployments/missing",
        None,
        &[],
    )
    .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn gui_static_files_are_public_while_api_and_sse_query_tokens_stay_guarded() {
    let temp = tempfile::tempdir().unwrap();
    let dist = temp.path().join("packages/web/dist/assets");
    fs::create_dir_all(&dist).unwrap();
    fs::write(
        temp.path().join("packages/web/dist/index.html"),
        "gui-index",
    )
    .unwrap();
    fs::write(dist.join("app.js"), "gui-asset").unwrap();
    let bundle = fixture();
    let api = start_api(&temp, Arc::new(ImmediateBackend), 1);

    for path in ["/gui/", "/gui/deployments/demo", "/gui/assets/app.js"] {
        let (status, body) = request(&api, None, "GET", path, None, &[]).await;
        assert_eq!(status, 200);
        assert!(String::from_utf8(body).unwrap().starts_with("gui-"));
    }
    let (status, _) = request(&api, None, "GET", "/api/v1/system/status", None, &[]).await;
    assert_eq!(status, 401);

    let (status, body) = request(
        &api,
        Some(&api.token),
        "POST",
        "/api/v1/commands/validate",
        Some(command_body(&bundle)),
        &[],
    )
    .await;
    assert_eq!(status, 202);
    let operation: OperationV1 = json_body(&body);
    let path = format!(
        "/api/v1/operations/{}/events?access_token={}",
        operation.id, api.token
    );
    let (status, body) = request(&api, None, "GET", &path, None, &[]).await;
    assert_eq!(status, 200);
    assert!(
        String::from_utf8(body)
            .unwrap()
            .contains("event: operation")
    );
    let path = format!(
        "/api/v1/operations/{}/events?access_token=wrong",
        operation.id
    );
    let (status, _) = request(&api, None, "GET", &path, None, &[]).await;
    assert_eq!(status, 401);
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
    for (method, path) in [
        ("POST", "/api/v1/system/shutdown"),
        ("POST", "/api/v1/commands/validate"),
        ("GET", "/api/v1/operations/missing"),
        ("POST", "/api/v1/operations/missing/cancel"),
        ("GET", "/api/v1/operations/missing/events"),
        ("GET", "/api/v1/deployments/missing/routes"),
        ("GET", "/api/v1/not-found"),
        ("DELETE", "/api/v1/system/status"),
    ] {
        assert_eq!(
            request(&daemon, None, method, path, None, &[]).await.0,
            401,
            "{method} {path} bypassed authentication"
        );
    }
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
async fn terminal_operation_retention_is_bounded_and_store_remains_queryable() {
    let temp = TempDir::new().unwrap();
    let daemon = start_api(&temp, Arc::new(ImmediateBackend), 2);
    let mut ids = Vec::new();
    for _ in 0..65 {
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
        ids.push(operation.id);
    }

    let oldest = &ids[0];
    assert_eq!(
        request(
            &daemon,
            Some(&daemon.token),
            "GET",
            &format!("/api/v1/operations/{oldest}/events"),
            None,
            &[],
        )
        .await
        .0,
        404
    );
    let (status, body) = request(
        &daemon,
        Some(&daemon.token),
        "GET",
        &format!("/api/v1/operations/{oldest}"),
        None,
        &[],
    )
    .await;
    assert_eq!(status, 200);
    let stored: OperationV1 = json_body(&body);
    assert_eq!(stored.status, OperationStatusV1::Succeeded);
    assert!(stored.result.is_none());
    assert_eq!(
        request(
            &daemon,
            Some(&daemon.token),
            "GET",
            &format!("/api/v1/operations/{}/events", ids.last().unwrap()),
            None,
            &[],
        )
        .await
        .0,
        200
    );
}

#[tokio::test]
async fn lock_loss_cancels_live_binding_then_persists_its_attempts() {
    let temp = TempDir::new().unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let daemon = start_api(
        &temp,
        Arc::new(LockLossBackend {
            cancelled: cancelled.clone(),
        }),
        2,
    );
    let (status, body) = request(
        &daemon,
        Some(&daemon.token),
        "POST",
        "/api/v1/commands/bind",
        Some(json!({
            "bundle": fixture(),
            "consumer": "backend-a",
            "group": "base"
        })),
        &[],
    )
    .await;
    assert_eq!(status, 202);
    let operation: OperationV1 = json_body(&body);

    let (mut competing_store, _) =
        StateStore::open(temp.path().join(".switchyard/state.sqlite3")).unwrap();
    let future_now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
        + 60_000;
    competing_store
        .acquire_lock(&LockRequest {
            deployment: "comparison",
            owner: "replacement-daemon",
            pid: 999,
            process_started_at: future_now,
            token: "replacement-token",
            now: future_now,
            ttl_millis: 15_000,
        })
        .unwrap();
    drop(competing_store);

    let terminal = tokio::time::timeout(
        Duration::from_secs(7),
        wait_terminal(&daemon, &operation.id),
    )
    .await
    .expect("heartbeat should detect the replaced lease");
    assert_eq!(terminal.status, OperationStatusV1::Failed);
    assert_eq!(
        terminal.error.as_ref().map(|error| error.code.as_str()),
        Some("operation_lock_lost")
    );
    assert!(cancelled.load(Ordering::SeqCst));

    let (_, body) = request(
        &daemon,
        Some(&daemon.token),
        "GET",
        "/api/v1/deployments/comparison/routes",
        None,
        &[],
    )
    .await;
    let routes: switchyard_daemon::contract::DeploymentRoutesV1 = json_body(&body);
    assert!(routes.history.iter().any(|attempt| {
        attempt.operation_id.as_deref() == Some(operation.id.as_str())
            && attempt.activation_status == "rejected"
    }));
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

#[tokio::test]
async fn failed_live_binding_versions_and_rollback_history_survive_restart() {
    let temp = TempDir::new().unwrap();
    let daemon = start_api(&temp, Arc::new(LiveBindingBackend), 2);
    let body = json!({"bundle": fixture(), "consumer": "backend-a", "group": "base", "transition": {"strategy": "drain", "timeoutMs": 2500}});
    let (status, operation) = request(
        &daemon,
        Some(&daemon.token),
        "POST",
        "/api/v1/commands/bind",
        Some(body),
        &[],
    )
    .await;
    assert_eq!(status, 202);
    let operation: OperationV1 = json_body(&operation);
    assert_eq!(
        wait_terminal(&daemon, &operation.id).await.status,
        OperationStatusV1::Failed
    );
    let (status, body) = request(
        &daemon,
        Some(&daemon.token),
        "GET",
        "/api/v1/deployments/comparison/routes",
        None,
        &[],
    )
    .await;
    assert_eq!(status, 200);
    let routes: switchyard_daemon::contract::DeploymentRoutesV1 = json_body(&body);
    assert_eq!(routes.bindings.len(), 2);
    let sidecar = routes
        .bindings
        .iter()
        .find(|binding| binding.router.starts_with("sidecar:"))
        .unwrap();
    assert_eq!(sidecar.desired_version, Some(2));
    assert_eq!(sidecar.current_version, Some(1));
    assert_eq!(sidecar.observed_version, Some(1));
    assert_eq!(sidecar.status, "failed");
    let host = routes
        .bindings
        .iter()
        .find(|binding| binding.router == "host-gateway")
        .unwrap();
    assert_eq!(host.current_version, Some(1));
    assert_eq!(host.desired_version, Some(2));
    assert!(
        routes
            .history
            .iter()
            .any(|entry| entry.activation_status == "rolled_back")
    );
    drop(daemon);

    let restarted = start_api(&temp, Arc::new(LiveBindingBackend), 2);
    let (_, body) = request(
        &restarted,
        Some(&restarted.token),
        "GET",
        "/api/v1/deployments/comparison/routes",
        None,
        &[],
    )
    .await;
    let restored: switchyard_daemon::contract::DeploymentRoutesV1 = json_body(&body);
    assert_eq!(restored.bindings, routes.bindings);
    assert_eq!(restored.history, routes.history);
}

#[tokio::test]
async fn applied_domains_bindings_and_deleted_database_recovery_survive_daemon_restart() {
    let temp = TempDir::new().unwrap();
    let bundle =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/routing-matrix/deployment.yaml");
    let authored = switchyard_planner::load_bundle(&bundle).unwrap();
    let plan = switchyard_planner::plan(&authored).unwrap();
    switchyard_planner::write_plan(temp.path(), &plan).unwrap();
    let daemon = start_api(&temp, Arc::new(StubBackend), 2);
    let (_, operation) = request(
        &daemon,
        Some(&daemon.token),
        "POST",
        "/api/v1/commands/apply",
        Some(command_body(&bundle)),
        &[],
    )
    .await;
    let operation: OperationV1 = json_body(&operation);
    assert_eq!(
        wait_terminal(&daemon, &operation.id).await.status,
        OperationStatusV1::Succeeded
    );
    drop(daemon);

    let (store, _) =
        switchyard_state::StateStore::open(temp.path().join(".switchyard/state.sqlite3")).unwrap();
    let applied = store.applied_snapshot("routing-matrix").unwrap().unwrap();
    assert!(
        applied
            .snapshot
            .as_json()
            .contains("routing-matrix.localhost")
    );
    assert!(applied.snapshot.as_json().contains("bindings"));
    drop(store);
    let restarted = start_api(&temp, Arc::new(StubBackend), 2);
    assert!(
        restarted
            .reconciliation
            .deployments
            .iter()
            .any(|entry| entry.deployment == "routing-matrix")
    );
    drop(restarted);

    fs::remove_file(temp.path().join(".switchyard/state.sqlite3")).unwrap();
    let recovered = start_api(&temp, Arc::new(StubBackend), 2);
    let deployment = recovered
        .reconciliation
        .deployments
        .iter()
        .find(|entry| entry.deployment == "routing-matrix")
        .unwrap();
    assert!(
        deployment
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == switchyard_state::DriftCode::AppliedStateMissing)
    );
}

fn init_git_repository(path: &Path) {
    fs::create_dir_all(path).unwrap();
    for arguments in [
        vec!["init", "-b", "main"],
        vec!["config", "user.email", "tests@switchyard.invalid"],
        vec!["config", "user.name", "Switchyard Tests"],
    ] {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    }
    fs::write(path.join("tracked"), "initial\n").unwrap();
    for arguments in [vec!["add", "tracked"], vec!["commit", "-m", "initial"]] {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    }
}

#[tokio::test]
async fn source_and_worktree_endpoints_enforce_auth_validation_and_non_destructive_errors() {
    let temp = TempDir::new().unwrap();
    let repository = temp.path().join("repository");
    init_git_repository(&repository);
    let daemon = start_api(&temp, Arc::new(StubBackend), 2);
    assert_eq!(
        request(&daemon, None, "GET", "/api/v1/sources", None, &[])
            .await
            .0,
        401
    );
    let (status, body) = request(
        &daemon,
        Some(&daemon.token),
        "POST",
        "/api/v1/sources",
        Some(json!({"name":"repo","path":repository})),
        &[],
    )
    .await;
    assert_eq!(status, 201, "{}", String::from_utf8_lossy(&body));
    assert_eq!(
        request(
            &daemon,
            Some(&daemon.token),
            "POST",
            "/api/v1/sources",
            Some(json!({"name":"missing","path":temp.path().join("missing")})),
            &[]
        )
        .await
        .0,
        400
    );
    let (status, body) = request(
        &daemon,
        Some(&daemon.token),
        "POST",
        "/api/v1/worktrees",
        Some(json!({"repository":"repo","ref":"HEAD","name":"feature"})),
        &[],
    )
    .await;
    assert_eq!(status, 201, "{}", String::from_utf8_lossy(&body));
    let created: switchyard_daemon::contract::SourceV1 = json_body(&body);
    fs::write(created.source.path.join("untracked"), "dirty\n").unwrap();
    let (status, body) = request(
        &daemon,
        Some(&daemon.token),
        "DELETE",
        "/api/v1/worktrees/feature",
        Some(json!({"allowDirty":false})),
        &[],
    )
    .await;
    assert_eq!(status, 409);
    let error: switchyard_daemon::contract::ApiErrorV1 = json_body(&body);
    assert_eq!(error.code, "source_dirty");
    assert_eq!(
        request(
            &daemon,
            Some(&daemon.token),
            "DELETE",
            "/api/v1/worktrees/feature",
            Some(json!({"allowDirty":true})),
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
            "DELETE",
            "/api/v1/sources/feature",
            None,
            &[]
        )
        .await
        .0,
        204
    );
    let (status, body) = request(
        &daemon,
        Some(&daemon.token),
        "DELETE",
        "/api/v1/worktrees/repo",
        Some(json!({"allowDirty":true})),
        &[],
    )
    .await;
    assert_eq!(status, 400);
    let error: switchyard_daemon::contract::ApiErrorV1 = json_body(&body);
    assert_eq!(error.code, "source_unmanaged");
    let (status, body) = request(
        &daemon,
        Some(&daemon.token),
        "GET",
        "/api/v1/worktrees?repository=repo",
        None,
        &[],
    )
    .await;
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    let worktrees: Vec<switchyard_daemon::contract::WorktreeV1> = json_body(&body);
    assert_eq!(worktrees.len(), 1);
}
