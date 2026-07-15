use std::fs;

use router_config::{
    BindingId, BrowserIdentity, BrowserIdentitySource, GatewayExposure, GatewayExposureMode,
    RouterConfig, ValidationCode,
};

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
fn compat_host_router_with_lan_exposure_remains_readable() {
    // This fixture is a Phase-7 compatibility golden. Regenerate it only as part
    // of a deliberate, versioned router schema change; update the fixture and
    // this assertion in the same commit so unversioned breaking changes fail CI.
    let source = include_str!("compat/host-router-lan.json");
    let config: RouterConfig =
        serde_json::from_str(source).expect("compat host router should parse");
    config
        .validate()
        .expect("compat host router should validate");
    assert_eq!(config.spec.exposure_mode(), GatewayExposureMode::Lan);
    assert!(config.spec.lan_exposure_acknowledged());
    assert!(
        config
            .spec
            .exposure
            .as_ref()
            .is_some_and(|exposure| exposure.publish_tailscale)
    );

    let encoded = serde_json::to_string_pretty(&config).expect("config should serialize");
    let reparsed: RouterConfig =
        serde_json::from_str(&encoded).expect("serialized compat router should parse");
    assert_eq!(reparsed, config);
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
fn explicit_header_route_values_follow_the_public_contract() {
    let valid = [
        "a".to_owned(),
        "0".to_owned(),
        "route.with_all:_-09".to_owned(),
        format!("a{}", "z".repeat(127)),
    ];
    for value in valid {
        let mut config = fixture("valid/routing-matrix.json");
        config.spec.browser_routes[0].identity = BrowserIdentity::ExplicitHeader {
            value: BindingId::from(value.as_str()),
        };
        config.validate().unwrap_or_else(|errors| {
            panic!("valid explicit-header route `{value}` was rejected: {errors:?}")
        });
    }

    let invalid = [
        "".to_owned(),
        "Uppercase".to_owned(),
        "-starts-with-punctuation".to_owned(),
        "contains/slash".to_owned(),
        "nonascii-é".to_owned(),
        format!("a{}", "z".repeat(128)),
    ];
    for value in invalid {
        let mut config = fixture("valid/routing-matrix.json");
        config.spec.browser_routes[0].identity = BrowserIdentity::ExplicitHeader {
            value: BindingId::from(value.as_str()),
        };
        let errors = config.validate().unwrap_err();
        assert!(
            errors.iter().any(|error| {
                error.code == ValidationCode::InvalidIdentifier
                    && error.path == "spec.browserRoutes[0].identity.value"
            }),
            "{value}: {errors:?}"
        );
    }
}

#[test]
fn provider_identity_header_opt_in_defaults_false_and_round_trips() {
    let mut config = fixture("valid/routing-matrix.json");
    assert!(
        config
            .spec
            .providers
            .iter()
            .all(|provider| !provider.receive_identity_header)
    );

    config.spec.providers[0].receive_identity_header = true;
    let encoded = serde_json::to_string(&config).unwrap();
    let decoded: RouterConfig = serde_json::from_str(&encoded).unwrap();
    assert!(decoded.spec.providers[0].receive_identity_header);
    assert!(
        decoded.spec.providers[1..]
            .iter()
            .all(|provider| !provider.receive_identity_header)
    );
}

#[test]
fn lan_exposure_defaults_to_loopback_and_round_trips_when_acknowledged() {
    let mut config = fixture("valid/routing-matrix.json");
    assert_eq!(config.spec.exposure_mode(), GatewayExposureMode::Loopback);

    config.spec.exposure = Some(GatewayExposure {
        mode: GatewayExposureMode::Lan,
        acknowledge_lan_exposure_risk: true,
        publish_tailscale: true,
    });
    config.spec.listeners[0].bind.host = "0.0.0.0".parse().unwrap();
    for provider in &mut config.spec.providers {
        provider.endpoint.host = "127.0.0.1".into();
    }
    let encoded = serde_json::to_string(&config).unwrap();
    assert!(encoded.contains("acknowledgeLanExposureRisk"));
    assert!(encoded.contains("publishTailscale"));
    let decoded: RouterConfig = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, config);
    decoded
        .validate()
        .expect("acknowledged LAN exposure is valid");
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
        (
            "invalid/lan-bind-without-opt-in.json",
            ValidationCode::LanExposureNotEnabled,
        ),
        (
            "invalid/lan-opt-in-without-acknowledgement.json",
            ValidationCode::LanExposureRiskNotAcknowledged,
        ),
        (
            "invalid/tailscale-publication-without-lan.json",
            ValidationCode::TailscalePublicationRequiresLanExposure,
        ),
        (
            "invalid/lan-non-loopback-provider.json",
            ValidationCode::UnsafeLanProvider,
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

#[test]
fn listener_invariants_are_validated_before_runtime_construction() {
    let mut config = fixture("valid/routing-matrix.json");
    let listener = &mut config.spec.listeners[0];
    listener.bind.port = 0;
    listener.destinations.clear();
    listener.protocol = router_config::Protocol::Http;
    listener.proxy_identity = Some(router_config::BindingId::from("profile"));
    listener.proxy_authentication = Some(router_config::ProxyAuthentication {
        scheme: router_config::ProxyAuthenticationScheme::Basic,
        credential_file: std::path::PathBuf::new(),
    });

    let errors = config.validate().expect_err("invalid listener must fail");
    let invalid = errors
        .iter()
        .filter(|error| error.code == ValidationCode::InvalidListener)
        .count();
    assert!(invalid >= 4, "{errors:?}");
}
