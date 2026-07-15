use std::{fs, path::Path};

use switchyard_planner::{
    DiagnosticCode, ManagedProfile, PublishedUpstream, UiRoute, load_bundle, plan,
    plan_with_binding, write_plan,
};

fn bundle() -> switchyard_planner::Bundle {
    load_bundle(Path::new("tests/fixtures/deployment.yaml")).expect("fixture should load")
}

#[test]
fn compose_and_manifest_are_deterministic_and_owned() {
    let bundle = bundle();
    let first = plan(&bundle).expect("fixture should plan");
    let second = plan(&bundle).expect("fixture should plan again");

    assert_eq!(first.compose_yaml, second.compose_yaml);
    assert_eq!(first.manifest_json, second.manifest_json);
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
