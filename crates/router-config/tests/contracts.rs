use std::fs;

use router_config::{BrowserIdentitySource, RouterConfig, ValidationCode};

fn fixture(path: &str) -> RouterConfig {
    let contents =
        fs::read_to_string(format!("tests/fixtures/{path}")).expect("fixture should be readable");
    serde_json::from_str(&contents).expect("fixture should match the schema")
}

#[test]
fn routing_matrix_round_trips_and_validates() {
    let config = fixture("valid/routing-matrix.json");
    config.validate().expect("routing matrix should be valid");

    let json = serde_json::to_string(&config).expect("configuration should serialize");
    let decoded: RouterConfig = serde_json::from_str(&json).expect("round trip should decode");
    assert_eq!(decoded, config);
    assert_eq!(decoded.api_version, router_config::API_VERSION);
}

#[test]
fn v1alpha1_minimal_document_remains_compatible() {
    let config: RouterConfig =
        serde_json::from_str(include_str!("fixtures/valid/v1alpha1-minimal.json"))
            .expect("v1alpha1 defaults should remain readable");
    assert!(config.spec.listeners.is_empty());
    config.validate().expect("minimal document should be valid");
}

#[test]
fn browser_identity_precedence_is_fixed() {
    assert_eq!(
        BrowserIdentitySource::PRECEDENCE,
        [
            BrowserIdentitySource::ExplicitHeader,
            BrowserIdentitySource::Origin,
            BrowserIdentitySource::ProxyListener,
        ]
    );
}

#[test]
fn invalid_fixtures_report_stable_codes() {
    let cases = [
        (
            "invalid/duplicate-listeners.json",
            ValidationCode::DuplicateListener,
        ),
        (
            "invalid/missing-provider.json",
            ValidationCode::MissingProvider,
        ),
        (
            "invalid/incompatible-protocols.json",
            ValidationCode::IncompatibleProtocol,
        ),
        (
            "invalid/incomplete-group.json",
            ValidationCode::IncompleteGroup,
        ),
    ];

    for (path, expected) in cases {
        let errors = fixture(path).validate().expect_err(path);
        assert!(
            errors.iter().any(|error| error.code == expected),
            "{path}: {errors:?}"
        );
        let encoded = serde_json::to_value(&errors).expect("diagnostics should serialize");
        assert!(
            encoded
                .as_array()
                .is_some_and(|items| items.iter().all(|item| {
                    item.get("code").is_some()
                        && item.get("path").is_some()
                        && item.get("message").is_some()
                }))
        );
    }
}
