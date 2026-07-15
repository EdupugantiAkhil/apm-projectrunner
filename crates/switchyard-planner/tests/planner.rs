use std::{fs, path::Path};

use switchyard_planner::{
    ChangeImpact, DiagnosticCode, ManagedProfile, OverlayOptions, PublishedUpstream, UiRoute,
    classify_changes, load_bundle, parse_dotenv, plan, plan_with_binding, plan_with_overlays,
    write_plan,
};

fn bundle() -> switchyard_planner::Bundle {
    load_bundle(Path::new("tests/fixtures/deployment.yaml")).expect("fixture should load")
}

fn write_overlay(directory: &Path, name: &str, body: &str) -> std::path::PathBuf {
    let path = directory.join(name);
    fs::write(&path, body).unwrap();
    path
}

#[test]
fn strict_dotenv_parser_has_no_shell_semantics() {
    let values = parse_dotenv("# comment\nPLAIN=value\nSHELL=$(touch /tmp/never)\nEMPTY=\n")
        .expect("strict dotenv should parse literals");
    assert_eq!(values["SHELL"], "$(touch /tmp/never)");
    assert_eq!(values["EMPTY"], "");
    assert!(parse_dotenv("export BAD=value").is_err());
    assert!(parse_dotenv("MISSING").is_err());
    assert!(parse_dotenv("DUP=one\nDUP=two").is_err());
}

#[test]
fn overlays_resolve_in_order_trace_shadows_and_materialize_files() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("values.env"), "FROM_FILE=first\n").unwrap();
    let first = write_overlay(
        directory.path(),
        "first.yaml",
        r#"
apiVersion: switchyard.dev/v1alpha1
kind: Overlay
metadata: { name: first }
spec:
  selectors: { instances: { names: [consumer-a] } }
  environment:
    envFiles: [values.env]
    set: { STATIC_VALUE: overlay-one, REMOVE_ME: inherited }
  parameters: { LOG_LEVEL: overlay }
  routes: { search: provider-main/api }
  variables: { enabled: "true" }
  files:
    - content: "enabled=${overlay.variables.enabled}\ncommand=$(touch /tmp/never)\n"
      target: /runtime/config/app.conf
      template: true
      mode: "0640"
"#,
    );
    let second = write_overlay(
        directory.path(),
        "second.yaml",
        r#"
apiVersion: switchyard.dev/v1alpha1
kind: Overlay
metadata: { name: second }
spec:
  selectors: { instances: { names: [consumer-a] } }
  environment:
    set: { STATIC_VALUE: overlay-two }
    unset: [REMOVE_ME]
  routes:
    search: { provider: provider-main/api, replace: true }
  files:
    - content: "replacement=${instance.name}/${deployment.name}/${parameters.LOG_LEVEL}\n"
      target: /runtime/config/app.conf
      template: true
      replace: true
"#,
    );
    let options = OverlayOptions {
        overlays: vec![first, second],
        variation: None,
        set: Default::default(),
    };
    let plan = plan_with_overlays(&bundle(), &options).expect("ordered overlays should resolve");
    let resolved: serde_json::Value = serde_yaml::from_str(&plan.resolved_deployment_yaml).unwrap();
    let consumer = resolved["spec"]["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|value| value["name"] == "consumer-a")
        .unwrap();
    assert_eq!(consumer["environment"]["STATIC_VALUE"], "overlay-two");
    assert_eq!(consumer["parameters"]["LOG_LEVEL"], "debug");
    assert!(consumer["environment"].get("REMOVE_ME").is_none());
    assert!(plan.compose_yaml.contains("STATIC_VALUE: overlay-two"));
    assert!(plan.compose_yaml.contains(":/runtime/config/app.conf:ro"));
    let trace = plan
        .origins
        .iter()
        .find(|trace| {
            trace.instance == "consumer-a"
                && trace.category == "environment"
                && trace.key == "STATIC_VALUE"
        })
        .unwrap();
    assert_eq!(trace.value, "overlay-two");
    assert!(
        trace
            .shadowed
            .iter()
            .any(|origin| origin.value == "overlay-one")
    );
    assert_eq!(plan.injected_files.len(), 1);
    assert_eq!(plan.injected_files[0].mode, 0o644);
    let workspace = tempfile::tempdir().unwrap();
    let output = write_plan(workspace.path(), &plan).unwrap();
    let materialized = output.join(&plan.injected_files[0].relative_path);
    assert!(materialized.is_file());
    let content = fs::read_to_string(materialized).unwrap();
    assert_eq!(content, "replacement=consumer-a/comparison/debug\n");
    assert!(!Path::new("/tmp/never").exists());
}

#[test]
fn overlay_validation_rejects_conflicts_selectors_templates_and_traversal() {
    let directory = tempfile::tempdir().unwrap();
    let missing = write_overlay(
        directory.path(),
        "missing.yaml",
        r#"
apiVersion: switchyard.dev/v1alpha1
kind: Overlay
metadata: { name: missing }
spec:
  selectors: { instances: { names: [misspelled] } }
  files: [{ content: x, target: /runtime/../escape }]
"#,
    );
    let errors = plan_with_overlays(
        &bundle(),
        &OverlayOptions {
            overlays: vec![missing],
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.code == DiagnosticCode::SelectorNoMatch)
    );
    assert!(
        errors
            .iter()
            .any(|error| error.code == DiagnosticCode::InvalidPath)
    );

    let optional = write_overlay(
        directory.path(),
        "optional.yaml",
        r#"
apiVersion: switchyard.dev/v1alpha1
kind: Overlay
metadata: { name: optional }
spec:
  selectors: { optional: true, instances: { names: [missing] } }
  environment: { set: { UNUSED: value } }
"#,
    );
    plan_with_overlays(
        &bundle(),
        &OverlayOptions {
            overlays: vec![optional],
            ..Default::default()
        },
    )
    .expect("optional selector is a no-op");

    let unknown = write_overlay(
        directory.path(),
        "unknown.yaml",
        r#"
apiVersion: switchyard.dev/v1alpha1
kind: Overlay
metadata: { name: unknown }
spec:
  selectors: { instances: { names: [consumer-a] } }
  files: [{ content: "${unknown.expression}", target: /runtime/config/value, template: true }]
"#,
    );
    let errors = plan_with_overlays(
        &bundle(),
        &OverlayOptions {
            overlays: vec![unknown],
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.code == DiagnosticCode::MissingVariable)
    );
}

#[test]
fn secrets_are_redacted_and_variations_are_disjoint() {
    let directory = tempfile::tempdir().unwrap();
    let secret = write_overlay(
        directory.path(),
        "secret.yaml",
        r#"
apiVersion: switchyard.dev/v1alpha1
kind: Overlay
metadata: { name: secret }
spec:
  selectors: { instances: { names: [consumer-a] } }
  environment:
    set:
      API_TOKEN: { environmentVariable: SUPER_SECRET_TOKEN }
"#,
    );
    let first = plan_with_overlays(
        &bundle(),
        &OverlayOptions {
            overlays: vec![secret.clone()],
            variation: Some("one".into()),
            set: Default::default(),
        },
    )
    .unwrap();
    let second = plan_with_overlays(
        &bundle(),
        &OverlayOptions {
            overlays: vec![secret],
            variation: Some("two".into()),
            set: Default::default(),
        },
    )
    .unwrap();
    for preview in [&first.resolved_deployment_yaml, &first.manifest_json] {
        assert!(!preview.contains("literal-secret-value"));
        assert!(preview.contains("«secret: SUPER_SECRET_TOKEN»"));
    }
    assert!(!first.compose_yaml.contains("SUPER_SECRET_TOKEN"));
    assert!(first.compose_yaml.contains("SWITCHYARD_OVERLAY_SECRET_"));
    assert_ne!(first.deployment, second.deployment);
    assert_ne!(first.compose_project, second.compose_project);
    assert_ne!(first.resource_hash, second.resource_hash);
    let workspace = tempfile::tempdir().unwrap();
    let one = write_plan(workspace.path(), &first).unwrap();
    let two = write_plan(workspace.path(), &second).unwrap();
    assert_ne!(one, two);
    assert!(one.join("manifest.json").is_file() && two.join("manifest.json").is_file());
}

#[test]
fn change_preview_distinguishes_live_restart_and_rebuild() {
    let workspace = tempfile::tempdir().unwrap();
    let base_bundle = bundle();
    let base = plan(&base_bundle).unwrap();
    write_plan(workspace.path(), &base).unwrap();

    let directory = tempfile::tempdir().unwrap();
    let route = write_overlay(
        directory.path(),
        "route.yaml",
        r#"
apiVersion: switchyard.dev/v1alpha1
kind: Overlay
metadata: { name: route }
spec:
  selectors: { instances: { names: [consumer-a] } }
  routes: { search: provider-main/api }
"#,
    );
    let live = plan_with_overlays(
        &base_bundle,
        &OverlayOptions {
            overlays: vec![route],
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        classify_changes(workspace.path(), &live)
            .unwrap()
            .iter()
            .all(|change| change.impact == ChangeImpact::Live)
    );

    let environment = write_overlay(
        directory.path(),
        "environment.yaml",
        r#"
apiVersion: switchyard.dev/v1alpha1
kind: Overlay
metadata: { name: environment }
spec:
  selectors: { instances: { names: [consumer-a] } }
  environment: { set: { ADDED: value } }
"#,
    );
    let restart = plan_with_overlays(
        &base_bundle,
        &OverlayOptions {
            overlays: vec![environment],
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        classify_changes(workspace.path(), &restart)
            .unwrap()
            .iter()
            .any(|change| change.impact == ChangeImpact::Restart)
    );

    let mut rebuilt_bundle = base_bundle;
    if let switchyard_planner::Execution::Container { image, .. } = &mut rebuilt_bundle
        .spec
        .blocks
        .get_mut("provider")
        .unwrap()
        .services
        .get_mut("api")
        .unwrap()
        .execution
    {
        *image = Some("example/provider:2".into());
    }
    let rebuilt = plan(&rebuilt_bundle).unwrap();
    assert!(
        classify_changes(workspace.path(), &rebuilt)
            .unwrap()
            .iter()
            .any(|change| change.impact == ChangeImpact::Rebuild)
    );
}

#[test]
fn compose_and_manifest_are_deterministic_and_owned() {
    let bundle = bundle();
    let first = plan(&bundle).expect("fixture should plan");
    let second = plan(&bundle).expect("fixture should plan again");

    assert_eq!(first.compose_yaml, second.compose_yaml);
    assert_eq!(first.manifest_json, second.manifest_json);
    assert!(
        !first
            .resolved_deployment_yaml
            .contains("resolvedOverlayFiles")
    );
    assert!(!first.manifest_json.contains("\"origins\""));
    assert!(!first.manifest_json.contains("\"injectedFiles\""));
    assert!(!first.has_overrides);
    assert_eq!(
        first.artifact_dir,
        Path::new(".switchyard/generated/comparison")
    );
    assert!(first.compose_yaml.contains("driver: bridge"));
    assert!(!first.compose_yaml.contains("external: true"));
    assert!(first.compose_yaml.contains("127.0.0.1::8080"));
    assert!(!first.compose_yaml.contains("published:"));
    assert!(first.compose_yaml.contains("dev.switchyard.resource-hash"));
    assert!(first.compose_yaml.contains("process-compose"));
    assert!(!first.compose_yaml.contains("set-a-real-token"));
    assert!(
        first
            .compose_yaml
            .contains("/routes/consumer-a.json:/config/consumer-a.json:ro")
    );
    let compose: serde_json::Value =
        serde_yaml::from_str(&first.compose_yaml).expect("generated Compose should parse");
    assert_eq!(
        compose["services"]["comparison--consumer-a--api--router"]["depends_on"]["comparison--provider-main--api"]
            ["condition"],
        "service_healthy"
    );
    assert_eq!(first.sidecars.len(), 2);
    assert_eq!(first.route_configs.len(), 2);
    assert_ne!(first.definition_hash, "");
    assert_ne!(first.resource_hash, "");
    assert_eq!(first.source_identities.len(), bundle.spec.instances.len());
    let manifest: serde_json::Value = serde_json::from_str(&first.manifest_json).unwrap();
    assert_eq!(
        manifest["sourceIdentities"]["consumer-a"]["path"],
        first.source_identities["consumer-a"].path
    );
}

#[test]
fn identical_loopback_ports_are_isolated_by_consumer_namespace() {
    let plan = plan(&bundle()).expect("fixture should plan");
    for consumer in ["consumer-a", "consumer-b"] {
        let namespace = format!("comparison--{consumer}--api");
        assert!(
            plan.compose_yaml
                .contains(&format!("network_mode: service:{namespace}"))
        );
        assert!(
            plan.compose_yaml
                .contains(&format!("comparison--{consumer}--api--router"))
        );
        assert!(plan.route_configs[consumer].contains("\"port\": 8001"));
    }
}

#[test]
fn binding_changes_routes_without_changing_resources() {
    let bundle = bundle();
    let base = plan(&bundle).expect("base should plan");
    let changed =
        plan_with_binding(&bundle, "consumer-a", "feature").expect("binding override should plan");
    assert_eq!(base.resource_hash, changed.resource_hash);
    assert_ne!(base.definition_hash, changed.definition_hash);
    assert_eq!(base.compose_yaml, changed.compose_yaml);
    let resolved: serde_json::Value =
        serde_yaml::from_str(&changed.resolved_deployment_yaml).unwrap();
    assert_eq!(resolved["spec"]["bindings"]["consumer-a"], "feature");
}

#[test]
fn backend_group_invariant_requires_duplicate_instances_for_different_groups() {
    let mut bundle = bundle();
    bundle
        .spec
        .blocks
        .get_mut("consumer")
        .unwrap()
        .services
        .get_mut("api")
        .unwrap()
        .publish = vec![3000];
    bundle.spec.host_router = Some(
        serde_json::from_value(serde_json::json!({
            "apiVersion": "switchyard.dev/router/v1alpha1",
            "kind": "RouterConfiguration",
            "metadata": { "deployment": "comparison" },
            "spec": {
                "snapshot": {
                    "id": "host-topology", "version": 1,
                    "transitions": {
                        "http": { "strategy": "close" }, "https": { "strategy": "close" },
                        "websocket": { "strategy": "pin" }, "grpc": { "strategy": "drain", "timeoutMs": 1000 },
                        "tcp": { "strategy": "close" }
                    }
                },
                "listeners": [{
                    "bind": { "host": "127.0.0.1", "port": 10081 }, "protocol": "http",
                    "destinations": [{ "kind": "legacy_localhost", "slot": "backend", "host": "localhost" }]
                }],
                "providers": [
                    { "id": "backend-a", "endpoint": { "protocol": "http", "host": "127.0.0.1", "port": 0 } },
                    { "id": "backend-b", "endpoint": { "protocol": "http", "host": "127.0.0.1", "port": 0 } }
                ],
                "groups": [], "bindings": [], "routes": [],
                "browserRoutes": [
                    { "identity": { "source": "origin", "origin": "http://ui-a.localhost" }, "destination": "backend", "provider": "backend-a" },
                    { "identity": { "source": "origin", "origin": "http://ui-b.localhost" }, "destination": "backend", "provider": "backend-b" }
                ],
                "identity": { "explicitHeader": "X-Switchyard-Route", "stripBeforeForwarding": true }
            }
        }))
        .unwrap(),
    );
    bundle.spec.host_upstreams.insert(
        "backend-a".into(),
        PublishedUpstream {
            instance: "consumer-a".into(),
            service: "api".into(),
            port: 3000,
        },
    );
    bundle.spec.host_upstreams.insert(
        "backend-b".into(),
        PublishedUpstream {
            instance: "consumer-b".into(),
            service: "api".into(),
            port: 3000,
        },
    );
    bundle.spec.ui_routes.insert(
        "provider-main".into(),
        UiRoute {
            origin: "http://ui-a.localhost".into(),
            backend: "consumer-a".into(),
            downstream_group: "base".into(),
        },
    );
    bundle.spec.ui_routes.insert(
        "suite-main".into(),
        UiRoute {
            origin: "http://ui-b.localhost".into(),
            backend: "consumer-b".into(),
            downstream_group: "feature".into(),
        },
    );

    plan(&bundle).expect("two backend instances from one source may select different groups");

    bundle.spec.ui_routes.get_mut("suite-main").unwrap().backend = "consumer-a".into();
    bundle
        .spec
        .host_upstreams
        .get_mut("backend-b")
        .unwrap()
        .instance = "consumer-a".into();
    let errors = plan(&bundle).expect_err("one backend cannot satisfy two group requirements");
    let invariant = errors
        .iter()
        .find(|error| error.code == DiagnosticCode::BackendGroupInvariant)
        .expect("invariant diagnostic should be explicit");
    assert!(invariant.message.contains("duplicate the backend instance"));
    assert!(invariant.message.contains("per-request downstream context"));
}

#[test]
fn writes_recovery_artifacts_under_generated_directory() {
    let plan = plan(&bundle()).expect("fixture should plan");
    let workspace = tempfile::tempdir().expect("temporary workspace");
    let output = write_plan(workspace.path(), &plan).expect("artifacts should write");
    assert!(output.join("compose.yaml").is_file());
    assert!(output.join("resolved-deployment.yaml").is_file());
    assert!(output.join("manifest.json").is_file());
    assert!(output.join("routes/consumer-a.json").is_file());
}

#[test]
fn writes_deterministic_credential_free_managed_profile_metadata() {
    let mut bundle = bundle();
    bundle.spec.managed_profiles.insert(
        "consumer-a".into(),
        ManagedProfile {
            route: "consumer-a".into(),
            start_url: "http://consumer-a.comparison.localhost:10081".into(),
        },
    );
    bundle.spec.host_router = Some(
        serde_json::from_value(serde_json::json!({
            "apiVersion": "switchyard.dev/router/v1alpha1",
            "kind": "RouterConfiguration",
            "metadata": { "deployment": "comparison" },
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
                    "bind": { "host": "127.0.0.1", "port": 10081 }, "protocol": "http",
                    "destinations": [
                        { "kind": "legacy_localhost", "slot": "backend", "host": "localhost" },
                        { "kind": "custom_domain", "slot": "ui-start", "domain": "consumer-a.comparison.localhost" }
                    ]
                }],
                "providers": [{ "id": "backend", "endpoint": { "protocol": "http", "host": "127.0.0.1", "port": 0 } }],
                "groups": [], "bindings": [], "routes": [],
                "browserRoutes": [
                    {
                        "identity": { "source": "proxy_listener", "listener": "consumer-a" },
                        "destination": "backend", "provider": "backend"
                    },
                    {
                        "identity": { "source": "proxy_listener", "listener": "consumer-a" },
                        "destination": "ui-start", "provider": "backend"
                    }
                ],
                "identity": { "explicitHeader": "X-Switchyard-Route", "stripBeforeForwarding": true }
            }
        }))
        .unwrap(),
    );
    bundle.spec.host_upstreams.insert(
        "backend".into(),
        PublishedUpstream {
            instance: "provider-main".into(),
            service: "api".into(),
            port: 8080,
        },
    );
    let first = plan(&bundle).expect("managed profile should plan");
    let second = plan(&bundle).expect("managed profile should be deterministic");
    assert_eq!(first.managed_profiles, second.managed_profiles);
    let profile = &first.managed_profiles["consumer-a"];
    assert_eq!(profile.proxy_address.split(':').next(), Some("127.0.0.1"));
    assert!(!serde_json::to_string(profile).unwrap().contains("token"));
    let host_config: router_config::RouterConfig =
        serde_json::from_str(first.host_router_config.as_ref().unwrap()).unwrap();
    let proxy = host_config
        .spec
        .listeners
        .iter()
        .find(|listener| {
            listener
                .proxy_identity
                .as_ref()
                .is_some_and(|value| value.as_str() == "consumer-a")
        })
        .unwrap();
    assert_eq!(
        format!("{}:{}", proxy.bind.host, proxy.bind.port),
        profile.proxy_address
    );
    assert!(proxy.proxy_authentication.is_some());
    assert_eq!(proxy.destinations.len(), 2);
    assert!(proxy.destinations.iter().all(|destination| {
        matches!(
            destination,
            router_config::ListenerDestination::ProxyTarget { port: 10081, .. }
        )
    }));
    assert_eq!(first.host_upstreams["backend"].container_port, 8080);
    assert_eq!(
        first.host_upstreams["backend"].compose_service,
        "comparison--provider-main--api"
    );

    let workspace = tempfile::tempdir().unwrap();
    let output = write_plan(workspace.path(), &first).unwrap();
    let artifact = output.join("managed-profiles/consumer-a.json");
    assert!(output.join("host-router.json").is_file());
    let written: serde_json::Value = serde_json::from_slice(&fs::read(artifact).unwrap()).unwrap();
    assert_eq!(written["route"], "consumer-a");
    assert_eq!(written["startUrl"], profile.start_url);

    let mut invalid_mapping = bundle.clone();
    invalid_mapping
        .spec
        .host_upstreams
        .get_mut("backend")
        .unwrap()
        .port = 8081;
    let errors = plan(&invalid_mapping).expect_err("unpublished upstream port must fail");
    assert!(errors.iter().any(|error| {
        error.code == DiagnosticCode::MissingReference
            && error.path == "spec.hostUpstreams.backend.port"
    }));

    let mut ambiguous = bundle.clone();
    let mut duplicate = ambiguous.spec.host_router.as_ref().unwrap().spec.listeners[0].clone();
    duplicate.bind.port += 1;
    ambiguous
        .spec
        .host_router
        .as_mut()
        .unwrap()
        .spec
        .listeners
        .push(duplicate);
    let errors = plan(&ambiguous).expect_err("ambiguous profile destination must fail");
    assert!(errors.iter().any(|error| {
        error.path == "spec.managedProfiles.consumer-a.route"
            && error.message.contains("expected exactly one")
    }));

    bundle
        .spec
        .managed_profiles
        .get_mut("consumer-a")
        .unwrap()
        .start_url = "https://consumer-a.comparison.localhost".into();
    let errors = plan(&bundle).expect_err("managed proxy HTTPS must fail closed");
    assert!(errors.iter().any(|error| {
        error.code == DiagnosticCode::InvalidPath && error.path.ends_with(".startUrl")
    }));
}

#[test]
fn reports_required_variables_cycles_conflicts_and_missing_providers_together() {
    let mut bundle = bundle();
    let consumer = bundle
        .spec
        .instances
        .iter_mut()
        .find(|instance| instance.name == "consumer-a")
        .expect("consumer exists");
    consumer.parameters.clear();
    bundle
        .spec
        .groups
        .get_mut("base")
        .expect("group exists")
        .providers
        .insert("search".into(), "missing/api".into());
    let consumer_block = bundle
        .spec
        .blocks
        .get_mut("consumer")
        .expect("consumer block exists");
    let api = consumer_block.services.get_mut("api").expect("api exists");
    api.depends_on.insert("api".into(), Default::default());
    api.consumes
        .insert("duplicate".into(), api.consumes["search"].clone());

    let errors = plan(&bundle).expect_err("invalid bundle should fail before generation");
    for expected in [
        DiagnosticCode::MissingVariable,
        DiagnosticCode::DependencyCycle,
        DiagnosticCode::ListenerConflict,
        DiagnosticCode::MissingProvider,
    ] {
        assert!(
            errors.iter().any(|error| error.code == expected),
            "missing {expected:?}: {errors:#?}"
        );
    }
}

#[test]
fn rejects_source_paths_before_writing_any_artifact() {
    let mut bundle = bundle();
    bundle
        .spec
        .sources
        .get_mut("app")
        .expect("source exists")
        .path = "does-not-exist".into();
    let errors = plan(&bundle).expect_err("bad source should fail");
    assert!(
        errors
            .iter()
            .any(|error| error.code == DiagnosticCode::InvalidPath)
    );
    assert!(!Path::new(".switchyard/generated/comparison").exists());
}

#[test]
fn generated_route_configuration_matches_router_contract() {
    let plan = plan(&bundle()).expect("fixture should plan");
    assert_eq!(
        plan.route_configs["consumer-a"],
        include_str!("golden/consumer-a-router.json").trim()
    );
    for config in plan.route_configs.values() {
        let value: serde_json::Value = serde_json::from_str(config).expect("config is JSON");
        assert_eq!(value["kind"], "RouterConfiguration");
        assert_eq!(value["spec"]["listeners"][0]["bind"]["host"], "127.0.0.1");
        assert_eq!(value["spec"]["providers"][0]["endpoint"]["port"], 8080);
        let router: router_config::RouterConfig =
            serde_json::from_str(config).expect("config matches router schema");
        router.validate().expect("generated route config validates");
    }
}

#[test]
fn fixture_file_does_not_need_generated_state() {
    let yaml = fs::read_to_string("tests/fixtures/deployment.yaml").expect("fixture is readable");
    assert!(!yaml.contains(".switchyard/generated"));
}

#[test]
fn parallel_deployments_have_disjoint_names_and_dynamic_loopback_ports() {
    let first = plan(&bundle()).unwrap();
    let mut second_bundle = bundle();
    second_bundle.metadata.name = "comparison-two".into();
    let second = plan(&second_bundle).unwrap();
    assert_ne!(first.compose_project, second.compose_project);
    assert!(!first.compose_yaml.contains("comparison-two"));
    assert!(!second.compose_yaml.contains("sy-comparison-private"));
    assert!(first.compose_yaml.contains("127.0.0.1::8080"));
    assert!(second.compose_yaml.contains("127.0.0.1::8080"));

    let first_manifest: serde_json::Value = serde_json::from_str(&first.manifest_json).unwrap();
    let second_manifest: serde_json::Value = serde_json::from_str(&second.manifest_json).unwrap();
    assert_ne!(first_manifest["network"], second_manifest["network"]);
    assert_ne!(
        first_manifest["ownershipLabels"],
        second_manifest["ownershipLabels"]
    );
    let first_compose: serde_json::Value = serde_yaml::from_str(&first.compose_yaml).unwrap();
    let second_compose: serde_json::Value = serde_yaml::from_str(&second.compose_yaml).unwrap();
    let first_volumes = first_compose["volumes"]
        .as_object()
        .unwrap()
        .values()
        .map(|volume| volume["name"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    let second_volumes = second_compose["volumes"]
        .as_object()
        .unwrap()
        .values()
        .map(|volume| volume["name"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(first_volumes.is_disjoint(&second_volumes));
}

#[test]
fn worktree_sources_still_require_repository_and_ref_through_adapters() {
    let mut bundle = bundle();
    let source = bundle
        .spec
        .sources
        .get_mut("app")
        .expect("fixture declares the app source");
    source.r#type = switchyard_planner::SourceType::Worktree;
    source.repository = None;
    source.r#ref = None;
    let errors = plan(&bundle).expect_err("worktree source without repository and ref");
    assert!(
        errors
            .iter()
            .any(|diagnostic| diagnostic.code == DiagnosticCode::InvalidPath
                && diagnostic.path == "spec.sources.app"),
        "expected an InvalidPath diagnostic for spec.sources.app: {errors:?}"
    );

    let mut empty_ref_bundle = crate::bundle();
    let source = empty_ref_bundle
        .spec
        .sources
        .get_mut("app")
        .expect("fixture declares the app source");
    source.r#type = switchyard_planner::SourceType::Worktree;
    source.repository = Some(".".into());
    source.r#ref = Some(String::new());
    let errors = plan(&empty_ref_bundle).expect_err("worktree source with an empty ref");
    assert!(
        errors
            .iter()
            .any(|diagnostic| diagnostic.code == DiagnosticCode::InvalidPath
                && diagnostic.path == "spec.sources.app"),
        "expected an InvalidPath diagnostic for spec.sources.app: {errors:?}"
    );
}
