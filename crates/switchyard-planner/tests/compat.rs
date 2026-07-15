use std::path::Path;

use switchyard_planner::{load_bundle, plan};

struct Golden {
    path: &'static str,
    deployment: &'static str,
    definition_hash: &'static str,
    resource_hash: &'static str,
    route_configs: usize,
    has_host_router: bool,
}

#[test]
fn current_example_deployments_remain_schema_compatible_and_deterministic() {
    // These fixtures pin today's accepted deployment schema. Regenerate the
    // copied YAML and the hashes below only for a deliberate, versioned schema
    // change; the intended flow is to update the compat fixture from the new
    // user-facing definition, run this test once to inspect the reported hashes,
    // then review the fixture and hash diff together.
    let goldens = [
        Golden {
            path: "tests/compat/routing-matrix-deployment.yaml",
            deployment: "routing-matrix",
            definition_hash: "59a88df224114e8ddf8a677ac85ca6ef6b30815d44ab8330ef6363e1e1b58fbf",
            resource_hash: "7ea572a56dc173d8a78672bc06bfe9a9cfa3db93e702b1efd981adf346eaa16c",
            route_configs: 2,
            has_host_router: true,
        },
        Golden {
            path: "tests/compat/jas-base-deployment.yaml",
            deployment: "jas-base",
            definition_hash: "cb347e7eba40bfad79be344bce4aefb98463d625c9de74cab36a8c603fbdda17",
            resource_hash: "13fcb8edf4cefee8b310ebf19dee553c6618c650021581cfd07ceb05655e86ef",
            route_configs: 4,
            has_host_router: true,
        },
    ];

    for golden in goldens {
        let bundle = load_bundle(Path::new(golden.path)).expect("compat fixture should load");
        let first = plan(&bundle).expect("compat fixture should plan");
        let second = plan(&bundle).expect("compat fixture should plan deterministically");
        assert_eq!(first.deployment, golden.deployment);
        assert_eq!(
            first.definition_hash, golden.definition_hash,
            "{} definition hash changed to {}",
            golden.path, first.definition_hash
        );
        assert_eq!(
            first.resource_hash, golden.resource_hash,
            "{} resource hash changed to {}",
            golden.path, first.resource_hash
        );
        assert_eq!(first.compose_yaml, second.compose_yaml);
        assert_eq!(first.route_configs, second.route_configs);
        assert_eq!(first.definition_hash, second.definition_hash);
        assert_eq!(first.resource_hash, second.resource_hash);
        assert_eq!(first.route_configs.len(), golden.route_configs);
        assert_eq!(first.host_router_config.is_some(), golden.has_host_router);
        for config in first.route_configs.values() {
            let router: router_config::RouterConfig =
                serde_json::from_str(config).expect("generated router config should parse");
            router
                .validate()
                .expect("generated router config should validate");
        }
        if let Some(config) = &first.host_router_config {
            let router: router_config::RouterConfig =
                serde_json::from_str(config).expect("host router config should parse");
            router
                .validate()
                .expect("host router config should validate");
        }
    }
}
