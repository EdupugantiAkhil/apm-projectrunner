use std::{fs, path::Path};

use switchyard_planner::{
    ChangeImpact, DiagnosticCode, ManagedProfile, OverlayOptions, PlanningDevice,
    PublishedUpstream, classify_changes, load_bundle, load_bundle_from_str, parse_dotenv, plan,
    plan_with_binding, plan_with_devices, plan_with_overlays, write_plan,
};

fn bundle() -> switchyard_planner::Bundle {
    load_bundle(Path::new("tests/fixtures/deployment.yaml")).expect("fixture should load")
}

fn devices() -> std::collections::BTreeMap<String, PlanningDevice> {
    std::collections::BTreeMap::from([(
        "builder".into(),
        PlanningDevice {
            user: "akhil".into(),
            host: "example-host".into(),
            port: 22,
            identity_file: Some("/keys/build".into()),
        },
    )])
}

#[test]
fn v1alpha1_loader_error_names_the_migration_command() {
    let directory = tempfile::tempdir().unwrap();
    let error = load_bundle_from_str(
        "apiVersion: switchyard.dev/v1alpha1\nkind: Deployment\nmetadata: { name: demo }\nspec: {}\n",
        &directory.path().join("deployment.yaml"),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("run `switchyard migrate`"));
    assert!(error.contains("switchyard.dev/v1alpha2"));
}

#[test]
fn group_member_instance_service_reference_resolves_service_ambiguity() {
    let mut deployment = bundle();
    let provider = deployment.spec.blocks.get_mut("provider").unwrap();
    provider
        .services
        .insert("alternate".into(), provider.services["api"].clone());

    let generated = plan(&deployment).expect("explicit service should resolve the group member");
    let routes: serde_json::Value =
        serde_json::from_str(&generated.route_configs["consumer-a"]).unwrap();
    assert_eq!(
        routes["spec"]["transparentProxy"]["members"],
        serde_json::json!([{
            "component": "provider-main/api",
            "host": "comparison--provider-main--api"
        }])
    );
}

#[test]
fn instance_device_defaults_to_local_and_requires_registration() {
    let mut deployment = bundle();
    assert!(
        deployment
            .spec
            .instances
            .iter()
            .all(|instance| instance.device.is_none())
    );
    let omitted = serde_yaml::to_value(&deployment.spec.instances[0]).unwrap();
    assert!(omitted.get("device").is_none());
    deployment.spec.instances[0].device = Some("local".into());
    let explicit = serde_yaml::to_value(&deployment.spec.instances[0]).unwrap();
    assert_eq!(explicit["device"], "local");
    plan(&deployment).expect("an explicit local device should plan");

    deployment.spec.instances[0].device = Some("builder".into());
    let diagnostics = plan(&deployment).expect_err("remote placement requires registration");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.path == "spec.instances[0].device"
            && diagnostic.message.contains("unregistered device `builder`")
    }));
}

#[test]
fn remote_non_container_execution_is_rejected() {
    let mut deployment = bundle();
    deployment.spec.instances[3].device = Some("builder".into());
    let diagnostics = plan_with_devices(&deployment, &devices()).unwrap_err();
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.path.ends_with("services.processes.execution")
            && diagnostic
                .message
                .contains("only supports container execution")
    }));
}

#[test]
fn remote_provider_is_partitioned_and_routed_by_device_host() {
    let mut deployment = bundle();
    deployment.spec.instances[0].device = Some("builder".into());
    let generated = plan_with_devices(&deployment, &devices()).unwrap();
    let remote = &generated.remote_projects["builder"];
    assert_eq!(remote.compose_project, "sy--comparison-builder");
    assert_eq!(remote.compose_file, Path::new("compose.builder.yaml"));
    assert_eq!(remote.services, ["comparison--provider-main--api"]);
    let remote_compose: serde_yaml::Value = serde_yaml::from_str(&remote.compose_yaml).unwrap();
    let network = "sy--comparison-builder--private";
    assert_eq!(remote_compose["networks"][network]["name"], network);
    assert_eq!(
        remote_compose["networks"][network]["labels"]["dev.switchyard.managed"],
        "true"
    );
    assert_eq!(
        remote_compose["networks"][network]["labels"]["dev.switchyard.deployment"],
        "comparison"
    );
    assert_eq!(
        remote_compose["networks"][network]["labels"]["dev.switchyard.device"],
        "builder"
    );
    assert_eq!(
        remote_compose["networks"][network]["labels"]["dev.switchyard.resource-hash"],
        generated.resource_hash
    );
    let service = "comparison--provider-main--api";
    assert_eq!(remote_compose["services"][service]["networks"][0], network);
    assert_eq!(remote_compose["services"][service]["ports"][0], "8080:8080");
    assert_eq!(
        remote_compose["services"][service]["labels"]["dev.switchyard.instance"],
        "provider-main"
    );
    assert_eq!(
        remote_compose["services"][service]["labels"]["dev.switchyard.service"],
        "api"
    );
    let volume = &remote_compose["volumes"]["comparison--provider-main--data"];
    assert_eq!(volume["labels"]["dev.switchyard.managed"], "true");
    assert_eq!(volume["labels"]["dev.switchyard.deployment"], "comparison");
    assert_eq!(volume["labels"]["dev.switchyard.device"], "builder");
    assert_eq!(volume["labels"]["dev.switchyard.instance"], "provider-main");
    assert_eq!(volume["labels"]["dev.switchyard.service"], "api");
    assert_eq!(
        volume["labels"]["dev.switchyard.resource-hash"],
        generated.resource_hash
    );
    assert!(
        !generated
            .compose_yaml
            .contains("comparison--provider-main--api:")
    );
    let route: serde_json::Value =
        serde_json::from_str(&generated.route_configs["consumer-a"]).unwrap();
    assert_eq!(
        route["spec"]["transparentProxy"]["members"][0]["host"],
        "example-host"
    );
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
    let live = plan_with_binding(&base_bundle, "consumer-a", "feature").unwrap();
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
    let sidecar_dependencies = compose["services"]["comparison--consumer-a--router"]["depends_on"]
        .as_object()
        .unwrap();
    assert_eq!(
        sidecar_dependencies["comparison--consumer-a--namespace"]["condition"],
        "service_started"
    );
    assert!(
        !sidecar_dependencies.contains_key("comparison--provider-main--api--app"),
        "transparent routing must tolerate providers that start later"
    );
    for (service, expected_component) in [
        ("comparison--consumer-a--namespace", "namespace"),
        ("comparison--consumer-a--api--app", "api"),
        ("comparison--consumer-a--router", "router"),
        ("comparison--provider-main--namespace", "namespace"),
        ("comparison--provider-main--api--app", "api"),
        ("comparison--provider-main--router", "router"),
    ] {
        assert_eq!(
            compose["services"][service]["labels"]["dev.switchyard.instance"],
            if service.contains("consumer-a") {
                "consumer-a"
            } else {
                "provider-main"
            }
        );
        assert_eq!(
            compose["services"][service]["labels"]["dev.switchyard.service"],
            expected_component
        );
    }
    assert_eq!(
        compose["services"]["comparison--consumer-a--router"]["cap_add"][0],
        "NET_ADMIN"
    );
    assert_eq!(first.sidecars.len(), 3);
    assert_eq!(first.route_configs.len(), 3);
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
        let namespace = format!("comparison--{consumer}--namespace");
        assert!(
            plan.compose_yaml
                .contains(&format!("network_mode: service:{namespace}"))
        );
        assert!(
            plan.compose_yaml
                .contains(&format!("comparison--{consumer}--router"))
        );
        assert!(plan.route_configs[consumer].contains("\"port\": 65535"));
    }
}

#[test]
fn group_routes_without_any_provides_consumes_or_declared_ports() {
    let mut deployment = bundle();
    for block in deployment.spec.blocks.values_mut() {
        for service in block.services.values_mut() {
            service.publish.clear();
            service.probe = None;
        }
    }

    let generated = plan(&deployment).expect("group membership alone should route");
    for consumer in ["consumer-a", "consumer-b"] {
        let config: serde_json::Value =
            serde_json::from_str(&generated.route_configs[consumer]).unwrap();
        assert_eq!(config["spec"]["listeners"], serde_json::json!([]));
        assert_eq!(config["spec"]["providers"], serde_json::json!([]));
        assert_eq!(
            config["spec"]["transparentProxy"]["members"][0]["component"],
            "provider-main/api"
        );
        assert_eq!(config["spec"]["transparentProxy"]["port"], 65_535);
    }
}

#[test]
fn disabled_group_member_is_omitted_without_losing_priority_position() {
    let mut deployment = bundle();
    deployment
        .spec
        .instances
        .retain(|instance| matches!(instance.name.as_str(), "provider-main" | "consumer-a"));
    let mut backup = deployment.spec.instances[0].clone();
    backup.name = "provider-backup".into();
    deployment.spec.instances.push(backup);
    let group = deployment.spec.groups.get_mut("base").unwrap();
    group.instances = vec!["provider-main/api".into(), "provider-backup/api".into()];
    deployment.spec.groups.remove("feature");
    deployment
        .spec
        .bindings
        .retain(|consumer, _| consumer == "consumer-a");

    let enabled = plan(&deployment).expect("both members should plan");
    deployment.spec.groups.get_mut("base").unwrap().disabled = vec!["provider-main".into()];
    let generated = plan(&deployment).expect("disabled member should be ignored");
    assert_eq!(generated.resource_hash, enabled.resource_hash);
    assert_eq!(generated.compose_yaml, enabled.compose_yaml);
    assert_ne!(
        generated.route_configs["consumer-a"],
        enabled.route_configs["consumer-a"]
    );
    let config: serde_json::Value =
        serde_json::from_str(&generated.route_configs["consumer-a"]).unwrap();
    assert_eq!(
        config["spec"]["transparentProxy"]["members"],
        serde_json::json!([{
            "component": "provider-backup/api",
            "host": "comparison--provider-backup--api"
        }])
    );
    let resolved: serde_json::Value =
        serde_yaml::from_str(&generated.resolved_deployment_yaml).unwrap();
    assert_eq!(
        resolved["spec"]["groups"]["base"]["disabled"],
        serde_json::json!(["provider-main"])
    );
}

#[test]
fn disabled_entry_must_name_a_resolved_group_member() {
    let mut deployment = bundle();
    deployment.spec.groups.get_mut("base").unwrap().disabled = vec!["not-a-member".into()];

    let errors = plan(&deployment).expect_err("unknown disabled member should fail");
    assert!(errors.iter().any(|error| {
        error.path == "spec.groups.base.disabled[0]"
            && error.code == DiagnosticCode::MissingReference
    }));
}

#[test]
fn disabled_membership_is_local_and_not_inherited() {
    let mut deployment = bundle();
    deployment.spec.groups.get_mut("base").unwrap().disabled = vec!["provider-main".into()];
    deployment
        .spec
        .bindings
        .insert("consumer-a".into(), "feature".into());
    deployment.spec.bindings.remove("consumer-b");

    let generated = plan(&deployment).expect("child group should reactivate inherited member");
    let config: serde_json::Value =
        serde_json::from_str(&generated.route_configs["consumer-a"]).unwrap();
    assert_eq!(
        config["spec"]["transparentProxy"]["members"][0]["component"],
        "provider-main/api"
    );
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

fn routing_matrix_bundle() -> switchyard_planner::Bundle {
    load_bundle(Path::new("tests/compat/routing-matrix-deployment.yaml"))
        .expect("routing fixture should load")
}

fn jas_base_bundle() -> switchyard_planner::Bundle {
    load_bundle(Path::new("tests/compat/jas-base-deployment.yaml"))
        .expect("JAS fixture should load")
}

fn planned_host_router(deployment: &switchyard_planner::Bundle) -> router_config::RouterConfig {
    let generated = plan(deployment).expect("addressed fixture should plan");
    serde_json::from_str(generated.host_router_config.as_ref().unwrap()).unwrap()
}

#[test]
fn group_address_generates_its_domain_destination_and_origin_route() {
    let host = planned_host_router(&jas_base_bundle());
    assert!(host.spec.listeners.iter().any(|listener| {
        listener.destinations.iter().any(|destination| {
            matches!(
                destination,
                router_config::ListenerDestination::CustomDomain { slot, domain }
                    if slot.as_str() == "ui-b-domain" && domain == "ai-main.jas-base.localhost"
            )
        })
    }));
    assert!(host.spec.browser_routes.iter().any(|route| {
        matches!(
            &route.identity,
            router_config::BrowserIdentity::Origin { origin }
                if origin == "http://ai-main.jas-base.localhost:18081"
        ) && route.destination.as_str() == "browser-java"
            && route.provider.as_str() == "jas-feature"
    }));
}

#[test]
fn instance_address_generates_its_domain_destination_and_origin_route() {
    let host = planned_host_router(&routing_matrix_bundle());
    assert!(host.spec.listeners.iter().any(|listener| {
        listener.destinations.iter().any(|destination| {
            matches!(
                destination,
                router_config::ListenerDestination::CustomDomain { slot, domain }
                    if slot.as_str() == "ui-3-domain"
                        && domain == "ui-3.routing-matrix.localhost"
            )
        })
    }));
    assert!(host.spec.browser_routes.iter().any(|route| {
        matches!(
            &route.identity,
            router_config::BrowserIdentity::Origin { origin }
                if origin == "http://ui-3.routing-matrix.localhost:18080"
        ) && route.destination.as_str() == "browser-backend"
            && route.provider.as_str() == "backend-1"
    }));
}

#[test]
fn group_address_rejects_a_group_without_an_addressed_member() {
    let mut deployment = jas_base_bundle();
    deployment
        .spec
        .groups
        .get_mut("ai-main")
        .unwrap()
        .instances
        .retain(|member| member != "ui-b/app");

    let errors = plan(&deployment).expect_err("a group address needs an addressed member");
    assert!(errors.iter().any(|error| {
        error.path == "spec.groups.ai-main.address"
            && error
                .message
                .contains("exactly one active member with its own address")
            && error.message.contains("candidates: none")
    }));
}

#[test]
fn group_address_rejects_a_group_with_two_addressed_members() {
    let mut deployment = jas_base_bundle();
    deployment
        .spec
        .groups
        .get_mut("ai-main")
        .unwrap()
        .instances
        .push("ui-a/app".into());

    let errors = plan(&deployment).expect_err("a group address may not guess between members");
    assert!(errors.iter().any(|error| {
        error.path == "spec.groups.ai-main.address"
            && error.message.contains("ui-a")
            && error.message.contains("ui-b")
    }));
}

#[test]
fn duplicate_addresses_are_case_insensitive_and_ignore_a_trailing_dot() {
    let mut deployment = jas_base_bundle();
    let index = deployment
        .spec
        .instances
        .iter()
        .position(|instance| instance.name == "ui-a")
        .unwrap();
    deployment.spec.instances[index].address = Some("AI-MAIN.JAS-BASE.LOCALHOST.".into());

    let errors = plan(&deployment).expect_err("addresses are case-insensitively unique");
    assert!(errors.iter().any(|error| {
        error.code == DiagnosticCode::DuplicateName
            && error.path == "spec.groups.ai-main.address"
            && error
                .message
                .contains(&format!("spec.instances[{index}].address"))
    }));
}

#[test]
fn invalid_address_hostname_is_rejected_at_the_authored_field() {
    let mut deployment = routing_matrix_bundle();
    deployment.spec.instances[3].address = Some("not_a_hostname".into());

    let errors = plan(&deployment).expect_err("addresses must be plausible hostnames");
    assert!(errors.iter().any(|error| {
        error.code == DiagnosticCode::InvalidPath
            && error.path == "spec.instances[3].address"
            && error.message == "address must be a plausible hostname"
    }));
}

#[test]
fn authored_domain_for_a_different_slot_conflicts_with_generated_address() {
    let mut deployment = jas_base_bundle();
    deployment.spec.host_router.as_mut().unwrap().spec.listeners[0]
        .destinations
        .push(router_config::ListenerDestination::CustomDomain {
            slot: router_config::RouteSlotId::from("ui-a-domain"),
            domain: "ai-main.jas-base.localhost".into(),
        });

    let errors = plan(&deployment).expect_err("authored and generated domain slots must agree");
    assert!(errors.iter().any(|error| {
        error.code == DiagnosticCode::ListenerConflict
            && error.path == "spec.groups.ai-main.address"
            && error.message.contains("does not route to `ui-b`")
    }));
}

#[test]
fn authored_origin_for_a_different_provider_conflicts_with_generated_address() {
    let mut deployment = jas_base_bundle();
    deployment
        .spec
        .host_router
        .as_mut()
        .unwrap()
        .spec
        .browser_routes
        .push(router_config::BrowserRoute {
            identity: router_config::BrowserIdentity::Origin {
                origin: "http://ai-main.jas-base.localhost:18081".into(),
            },
            destination: router_config::RouteSlotId::from("browser-java"),
            provider: router_config::ComponentId::from("jas-main"),
        });

    let errors = plan(&deployment).expect_err("authored and generated origins must agree");
    assert!(errors.iter().any(|error| {
        error.code == DiagnosticCode::ListenerConflict
            && error.path == "spec.groups.ai-main.address"
            && error
                .message
                .contains("routes destination `browser-java` to `jas-main`")
            && error.message.contains("instead of `jas-feature`")
    }));
}

#[test]
fn identical_authored_domain_and_origin_merge_without_duplicates() {
    let mut deployment = jas_base_bundle();
    let router = &mut deployment.spec.host_router.as_mut().unwrap().spec;
    router.listeners[0]
        .destinations
        .push(router_config::ListenerDestination::CustomDomain {
            slot: router_config::RouteSlotId::from("ui-b-domain"),
            domain: "ai-main.jas-base.localhost".into(),
        });
    router.browser_routes.push(router_config::BrowserRoute {
        identity: router_config::BrowserIdentity::Origin {
            origin: "http://ai-main.jas-base.localhost:18081".into(),
        },
        destination: router_config::RouteSlotId::from("browser-java"),
        provider: router_config::ComponentId::from("jas-feature"),
    });

    let host = planned_host_router(&deployment);
    let domains = host
        .spec
        .listeners
        .iter()
        .flat_map(|listener| &listener.destinations)
        .filter(|destination| {
            matches!(
                destination,
                router_config::ListenerDestination::CustomDomain { domain, .. }
                    if domain == "ai-main.jas-base.localhost"
            )
        })
        .count();
    let origins = host
        .spec
        .browser_routes
        .iter()
        .filter(|route| {
            matches!(
                &route.identity,
                router_config::BrowserIdentity::Origin { origin }
                    if origin == "http://ai-main.jas-base.localhost:18081"
            ) && route.destination.as_str() == "browser-java"
        })
        .count();
    assert_eq!(domains, 1);
    assert_eq!(origins, 1);
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

    let mut remote_bundle = bundle.clone();
    remote_bundle.spec.instances[0].device = Some("builder".into());
    let remote = plan_with_devices(&remote_bundle, &devices()).unwrap();
    assert_eq!(
        remote.host_upstreams["backend"].remote_address.as_deref(),
        Some("example-host:8080")
    );
    let remote_host_config: router_config::RouterConfig =
        serde_json::from_str(remote.host_router_config.as_ref().unwrap()).unwrap();
    assert_eq!(
        remote_host_config.spec.providers[0].endpoint.host,
        "example-host"
    );
    assert_eq!(remote_host_config.spec.providers[0].endpoint.port, 8080);

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
        .instances = vec!["missing/api".into()];
    let consumer_block = bundle
        .spec
        .blocks
        .get_mut("consumer")
        .expect("consumer block exists");
    let api = consumer_block.services.get_mut("api").expect("api exists");
    api.depends_on.insert("api".into(), Default::default());

    let errors = plan(&bundle).expect_err("invalid bundle should fail before generation");
    for expected in [
        DiagnosticCode::MissingVariable,
        DiagnosticCode::DependencyCycle,
        DiagnosticCode::MissingReference,
    ] {
        assert!(
            errors.iter().any(|error| error.code == expected),
            "missing {expected:?}: {errors:#?}"
        );
    }
}

#[test]
fn missing_source_paths_plan_without_writing_and_are_created_by_up() {
    let mut bundle = bundle();
    bundle
        .spec
        .sources
        .get_mut("app")
        .expect("source exists")
        .path = "does-not-exist".into();
    let generated = plan(&bundle).expect("a missing authored worktree is materialized by up");
    assert!(generated.source_identities.values().all(|identity| {
        identity.commit.is_none() && identity.repository.is_none() && identity.dirty.is_none()
    }));
    assert!(!Path::new(".switchyard/generated/comparison").exists());
}

#[test]
fn generated_route_configuration_matches_router_contract() {
    let plan = plan(&bundle()).expect("fixture should plan");
    let mut actual: serde_json::Value =
        serde_json::from_str(&plan.route_configs["consumer-a"]).unwrap();
    assert_eq!(
        actual["spec"]["transparentProxy"]["members"][0]["component"],
        "provider-main/api"
    );
    actual["spec"]
        .as_object_mut()
        .unwrap()
        .remove("transparentProxy");
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("golden/consumer-a-router.json")).unwrap();
    assert_eq!(actual, expected);
    for config in plan.route_configs.values() {
        let value: serde_json::Value = serde_json::from_str(config).expect("config is JSON");
        assert_eq!(value["kind"], "RouterConfiguration");
        if !value["spec"]["listeners"]
            .as_array()
            .is_some_and(Vec::is_empty)
        {
            assert_eq!(value["spec"]["listeners"][0]["bind"]["host"], "127.0.0.1");
            assert_eq!(value["spec"]["providers"][0]["endpoint"]["port"], 8080);
        }
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
fn sources_require_a_declared_repository_and_nonempty_ref() {
    let mut bundle = bundle();
    let source = bundle
        .spec
        .sources
        .get_mut("app")
        .expect("fixture declares the app source");
    source.repository = "missing".into();
    let errors = plan(&bundle).expect_err("source without a declared repository");
    assert!(
        errors.iter().any(
            |diagnostic| diagnostic.code == DiagnosticCode::MissingReference
                && diagnostic.path == "spec.sources.app.repository"
        ),
        "expected a MissingReference diagnostic for spec.sources.app.repository: {errors:?}"
    );

    let mut empty_ref_bundle = crate::bundle();
    let source = empty_ref_bundle
        .spec
        .sources
        .get_mut("app")
        .expect("fixture declares the app source");
    source.r#ref = String::new();
    let errors = plan(&empty_ref_bundle).expect_err("source with an empty ref");
    assert!(
        errors.iter().any(
            |diagnostic| diagnostic.code == DiagnosticCode::MissingReference
                && diagnostic.path == "spec.sources.app.ref"
        ),
        "expected a MissingReference diagnostic for spec.sources.app.ref: {errors:?}"
    );
}

#[test]
fn repositories_require_one_origin_and_source_paths_stay_project_local_and_distinct() {
    let mut deployment = bundle();
    let repository = deployment.spec.repositories.get_mut("fixture").unwrap();
    repository.clone = Some("../../../../outside".into());
    let diagnostics = plan(&deployment).unwrap_err();
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.path == "spec.repositories.fixture" && diagnostic.message.contains("exactly one")
    }));

    let mut deployment = bundle();
    deployment.spec.sources.get_mut("app").unwrap().path = "../../../../../outside".into();
    let diagnostics = plan(&deployment).unwrap_err();
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.path == "spec.sources.app.path"
            && diagnostic.message.contains("escapes project directory")
    }));

    let mut deployment = bundle();
    deployment
        .spec
        .sources
        .insert("duplicate".into(), deployment.spec.sources["app"].clone());
    let diagnostics = plan(&deployment).unwrap_err();
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.path == "spec.sources.duplicate.path"
            && diagnostic.message.contains("already used")
    }));
}

#[test]
fn declared_lifecycle_hooks_are_rejected_not_silently_ignored() {
    // The reserved per-service `hooks` field was removed in Phase 7 because the
    // runtime never executed it; initialization belongs to `lifecycle: task`
    // script services. Declaring it must fail loudly, not parse into a no-op.
    let fixture = fs::read_to_string("tests/fixtures/deployment.yaml").unwrap();
    let with_hooks = fixture.replacen(
        "      services:\n        api:\n",
        "      services:\n        api:\n          hooks: { postReady: [[\"true\"]] }\n",
        1,
    );
    assert_ne!(
        fixture, with_hooks,
        "fixture shape changed; update this test"
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("deployment.yaml");
    fs::write(&path, with_hooks).unwrap();
    let error = load_bundle(&path).expect_err("hooks must be rejected");
    assert!(
        error.to_string().contains("hooks"),
        "error should name the removed field: {error}"
    );
}
