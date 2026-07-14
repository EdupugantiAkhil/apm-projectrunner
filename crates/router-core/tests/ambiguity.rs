use router_config::{RouterConfig, ValidationCode};
use router_core::RouteEngine;

#[test]
fn duplicate_route_keys_fail_closed() {
    let mut config: RouterConfig = serde_json::from_str(include_str!(
        "../../router-config/tests/fixtures/valid/routing-matrix.json"
    ))
    .unwrap();
    config.spec.routes.push(config.spec.routes[0].clone());

    let error = match RouteEngine::new(config) {
        Ok(_) => panic!("ambiguous route was accepted"),
        Err(error) => error,
    };
    let router_core::ApplyError::Invalid(errors) = error else {
        panic!("expected validation errors");
    };
    assert!(
        errors
            .iter()
            .any(|error| error.code == ValidationCode::AmbiguousRoute)
    );
}
