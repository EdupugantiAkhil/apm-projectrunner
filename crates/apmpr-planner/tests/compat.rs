use std::path::Path;

use apmpr_planner::{load_bundle, plan};

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
            definition_hash: "d1fff1bbb79dfc6f70612efe7d526831f14fe8a49d2d03637cceb6d12d548f27",
            resource_hash: "e7886861efdf4e19b7199d1b86fd6d0a2d7bedc8e8450bb8efbf7072e9ea404e",
            route_configs: 12,
            has_host_router: true,
        },
        Golden {
            path: "tests/compat/jas-base-deployment.yaml",
            deployment: "jas-base",
            definition_hash: "074a9af7644a7aa79c16b423fccc4850eee928869df49c9e533a976ba7ab41b1",
            resource_hash: "dd0a6e5c3cf11c31f3b04a41587018b2230de1b7748df50fb8a62cacc2f2b816",
            route_configs: 6,
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
        assert_eq!(
            serde_json::to_vec(&first).unwrap(),
            serde_json::to_vec(&second).unwrap(),
            "{} full plan output changed across identical runs",
            golden.path
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
