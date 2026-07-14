use std::{fs, path::Path};

use switchyard_planner::{DiagnosticCode, load_bundle, plan, plan_with_binding, write_plan};

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
