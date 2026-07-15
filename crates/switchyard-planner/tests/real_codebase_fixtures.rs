use std::{collections::BTreeSet, fs, path::Path};

use switchyard_planner::{OverlayOptions, load_bundle, plan, plan_with_overlays};

fn repository_path(relative: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

#[test]
fn legacy_workspace_fixture_expands_through_generic_planner_contracts() {
    let deployment = repository_path("examples/jas-base/deployment.yaml");
    let bundle = load_bundle(&deployment).expect("legacy-shape fixture should load");
    let generated = plan(&bundle).expect("legacy-shape fixture should plan");
    let compose: serde_json::Value =
        serde_yaml::from_str(&generated.compose_yaml).expect("Compose should be YAML");
    let services = compose["services"]
        .as_object()
        .expect("Compose services should be a map");

    let expected = BTreeSet::from([
        "jas-base--ai-feature--suite",
        "jas-base--ai-main--suite",
        "jas-base--db-main--document-store",
        "jas-base--db-main--initialize-schema",
        "jas-base--db-main--kv-store",
        "jas-base--fixture-image--builder",
        "jas-base--jas-feature--service",
        "jas-base--jas-feature--service--app",
        "jas-base--jas-feature--service--router",
        "jas-base--jas-main--service",
        "jas-base--jas-main--service--app",
        "jas-base--jas-main--service--router",
        "jas-base--ui-a--app",
        "jas-base--ui-a--app--app",
        "jas-base--ui-a--app--router",
        "jas-base--ui-b--app",
        "jas-base--ui-b--app--app",
        "jas-base--ui-b--app--router",
    ]);
    assert_eq!(
        services.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        expected
    );

    let init = &services["jas-base--db-main--initialize-schema"];
    assert!(
        init.get("restart").is_none(),
        "task services must not restart"
    );
    assert_eq!(
        init["depends_on"]["jas-base--db-main--kv-store"]["condition"],
        "service_healthy"
    );
    assert_eq!(
        services["jas-base--jas-main--service--app"]["depends_on"]["jas-base--db-main--initialize-schema"]
            ["condition"],
        "service_completed_successfully"
    );
    assert_eq!(compose["volumes"].as_object().unwrap().len(), 2);

    let main_group = &bundle.spec.groups["ai-main"].providers;
    let feature_group = &bundle.spec.groups["ai-feature"];
    assert_eq!(main_group.len(), 5);
    assert_eq!(feature_group.extends.as_deref(), Some("ai-main"));
    assert_eq!(feature_group.providers.len(), 5);
    assert_eq!(bundle.spec.bindings["jas-main"], "ai-feature");
    assert_eq!(bundle.spec.bindings["jas-feature"], "ai-main");
    assert_eq!(bundle.spec.bindings["ui-a"], "ai-feature");
    assert_eq!(bundle.spec.bindings["ui-b"], "ai-main");
    assert_ne!(
        bundle.spec.sources["monorepo-main"].path,
        bundle.spec.sources["jas-feature-worktree"].path
    );
    assert_eq!(bundle.spec.routes["ui-a"]["java"], "jas-main/service");
    assert_eq!(bundle.spec.routes["ui-b"]["java"], "jas-feature/service");

    for consumer in ["jas-main", "jas-feature"] {
        let routes: serde_json::Value = serde_json::from_str(&generated.route_configs[consumer])
            .expect("sidecar route configuration should be JSON");
        let destinations = routes["spec"]["listeners"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|listener| listener["destinations"].as_array().unwrap())
            .map(|destination| destination["slot"].as_str().unwrap())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            destinations,
            BTreeSet::from([
                "audit",
                "catalog",
                "database-document",
                "database-kv",
                "reports",
                "scheduler",
                "search",
            ])
        );
    }

    for consumer in ["ui-a", "ui-b"] {
        let routes: serde_json::Value = serde_json::from_str(&generated.route_configs[consumer])
            .expect("UI sidecar route configuration should be JSON");
        let destinations = routes["spec"]["listeners"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|listener| listener["destinations"].as_array().unwrap())
            .map(|destination| destination["slot"].as_str().unwrap())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            destinations,
            BTreeSet::from(["audit", "catalog", "java", "reports", "scheduler", "search",])
        );
    }

    let host: serde_json::Value = serde_json::from_str(
        generated
            .host_router_config
            .as_deref()
            .expect("custom domains require a host router"),
    )
    .unwrap();
    let domains = host["spec"]["listeners"][0]["destinations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|destination| destination["domain"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        domains,
        BTreeSet::from(["ui-a.jas-base.localhost", "ui-b.jas-base.localhost"])
    );

    for source in ["main", "feature"] {
        let process_file = repository_path(&format!(
            "examples/jas-base/sources/{source}/process-compose.yaml"
        ));
        let process: serde_json::Value =
            serde_yaml::from_str(&fs::read_to_string(process_file).unwrap()).unwrap();
        assert_eq!(process["processes"].as_object().unwrap().len(), 5);
        assert_eq!(
            process["processes"]["audit"]["depends_on"]["scheduler"]["condition"],
            "process_healthy"
        );
    }

    for block in bundle.spec.blocks.values() {
        for service in block.services.values() {
            assert!(service.hooks.prepare.is_empty());
            assert!(service.hooks.post_ready.is_empty());
            assert!(service.hooks.stop.is_empty());
            assert!(service.hooks.cleanup.is_empty());
        }
    }
}

#[test]
fn unrelated_fixture_bundles_use_the_same_deterministic_planning_path() {
    for relative in [
        "examples/jas-base/deployment.yaml",
        "examples/routing-matrix/deployment.yaml",
    ] {
        let bundle = load_bundle(&repository_path(relative)).expect("fixture should load");
        let first = plan(&bundle).expect("fixture should plan");
        let second = plan(&bundle).expect("fixture should plan again");
        assert_eq!(first.deployment, second.deployment);
        assert_eq!(first.definition_hash, second.definition_hash);
        assert_eq!(first.resource_hash, second.resource_hash);
        assert_eq!(first.compose_yaml, second.compose_yaml);
        assert_eq!(first.route_configs, second.route_configs);
        assert!(!first.compose_yaml.is_empty());
    }
}

#[test]
fn overlays_create_disjoint_deterministic_variation_plans() {
    let mut bundle = load_bundle(&repository_path("examples/jas-base/deployment.yaml")).unwrap();
    // Stable custom-domain listeners are intentionally singleton host resources. The
    // variation proof covers the otherwise identical container topology without making
    // the test depend on plans already generated in the developer's workspace.
    bundle.spec.host_router = None;
    bundle.spec.host_upstreams.clear();
    bundle.spec.ui_routes.clear();
    let main_overlay = repository_path("examples/jas-base/overlays/main.yaml");
    let feature_overlay = repository_path("examples/jas-base/overlays/feature.yaml");
    let main = plan_with_overlays(
        &bundle,
        &OverlayOptions {
            overlays: vec![main_overlay],
            variation: Some("main".into()),
            set: Default::default(),
        },
    )
    .expect("main variation should plan");
    let feature = plan_with_overlays(
        &bundle,
        &OverlayOptions {
            overlays: vec![feature_overlay],
            variation: Some("feature".into()),
            set: Default::default(),
        },
    )
    .expect("feature variation should plan");
    assert_eq!(main.deployment, "jas-base--main");
    assert_eq!(feature.deployment, "jas-base--feature");
    assert_ne!(main.compose_project, feature.compose_project);
    assert_ne!(main.artifact_dir, feature.artifact_dir);
    assert_ne!(main.resource_hash, feature.resource_hash);
    assert!(!main.injected_files.is_empty());
    assert!(!feature.injected_files.is_empty());
}

#[test]
fn production_crate_identifiers_do_not_name_the_legacy_fixture() {
    let source_root = repository_path("crates");
    let mut files = Vec::new();
    collect_rust_sources(&source_root, &mut files);
    let mut violations = Vec::new();
    for path in files {
        let source = fs::read_to_string(&path).unwrap();
        for identifier in rust_identifiers(&source) {
            if identifier.to_ascii_lowercase().contains("jas") {
                violations.push(format!("{}: {identifier}", path.display()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "fixture-specific identifiers entered production crate source:\n{}",
        violations.join("\n")
    );
}

fn collect_rust_sources(directory: &Path, files: &mut Vec<std::path::PathBuf>) {
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_rust_sources(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs")
            && path
                .components()
                .any(|component| component.as_os_str() == "src")
        {
            files.push(path);
        }
    }
}

fn rust_identifiers(source: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    let mut identifiers = Vec::new();
    let mut index = 0;
    let mut block_depth = 0_u32;
    while index < bytes.len() {
        if block_depth > 0 {
            if bytes[index..].starts_with(b"/*") {
                block_depth += 1;
                index += 2;
            } else if bytes[index..].starts_with(b"*/") {
                block_depth -= 1;
                index += 2;
            } else {
                index += 1;
            }
        } else if bytes[index..].starts_with(b"//") {
            index += bytes[index..]
                .iter()
                .position(|byte| *byte == b'\n')
                .unwrap_or(bytes.len() - index);
        } else if bytes[index..].starts_with(b"/*") {
            block_depth = 1;
            index += 2;
        } else if bytes[index] == b'"'
            || bytes[index] == b'\''
                && bytes
                    .get(index + 1)
                    .is_some_and(|byte| !byte.is_ascii_alphabetic() && *byte != b'_')
        {
            index = skip_quoted(bytes, index, bytes[index]);
        } else if bytes[index].is_ascii_alphabetic() || bytes[index] == b'_' {
            let start = index;
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            identifiers.push(source[start..index].to_owned());
        } else {
            index += 1;
        }
    }
    identifiers
}

fn skip_quoted(bytes: &[u8], mut index: usize, quote: u8) -> usize {
    index += 1;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index = (index + 2).min(bytes.len());
        } else if bytes[index] == quote {
            return index + 1;
        } else {
            index += 1;
        }
    }
    index
}
