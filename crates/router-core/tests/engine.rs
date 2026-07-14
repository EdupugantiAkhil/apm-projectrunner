use std::sync::Arc;

use router_config::{
    BindingId, BrowserIdentity, BrowserRoute, ComponentId, InstanceId, RouteSlotId, RouterConfig,
};
use router_core::{ActivationStatus, BrowserLookup, LookupError, RouteEngine};

fn config() -> RouterConfig {
    serde_json::from_str(include_str!(
        "../../router-config/tests/fixtures/valid/routing-matrix.json"
    ))
    .unwrap()
}

#[test]
fn compiles_groups_direct_routes_and_browser_precedence() {
    let mut value = config();
    value.spec.routes.push(router_config::Route {
        consumer: InstanceId::from("backend-1"),
        slot: RouteSlotId::from("catalog"),
        provider: router_config::ComponentId::from("services-main/catalog"),
    });
    let engine = RouteEngine::new(value).unwrap();
    let snapshot = engine.snapshot();
    let backend = InstanceId::from("backend-1");
    let catalog = RouteSlotId::from("catalog");
    assert_eq!(
        snapshot
            .lookup_consumer(&backend, &catalog)
            .unwrap()
            .provider
            .as_str(),
        "services-main/catalog"
    );

    let destination = RouteSlotId::from("browser-backend");
    let target = snapshot
        .lookup_browser(BrowserLookup {
            destination: &destination,
            explicit_header: None,
            origin: Some("https://ui-2.comparison.localhost"),
            proxy_listener: None,
        })
        .unwrap();
    assert_eq!(target.provider.as_str(), "backend-2");

    assert_eq!(
        snapshot.lookup_browser(BrowserLookup {
            destination: &destination,
            explicit_header: Some("unknown"),
            origin: Some("https://ui-2.comparison.localhost"),
            proxy_listener: None,
        }),
        Err(LookupError::UnknownIdentity)
    );
}

#[test]
fn atomically_applies_newer_snapshots_and_rejects_stale_versions() {
    let mut first = config();
    first.spec.snapshot.version = 4;
    let engine = RouteEngine::new(first).unwrap();

    let mut newer = config();
    newer.spec.snapshot.version = 5;
    let ack = engine.apply(newer).unwrap();
    assert_eq!(ack.status, ActivationStatus::Activated);
    assert_eq!(engine.snapshot().version(), 5);
    assert_eq!(engine.previous_snapshot().unwrap().version(), 4);

    let mut stale = config();
    stale.spec.snapshot.version = 3;
    let ack = engine.apply(stale).unwrap();
    assert_eq!(ack.status, ActivationStatus::RejectedStale);
    assert_eq!(engine.snapshot().version(), 5);
}

#[test]
fn invalid_apply_leaves_active_snapshot_unchanged() {
    let engine = RouteEngine::new(config()).unwrap();
    let before = engine.snapshot().checksum().to_owned();
    let mut invalid = config();
    invalid.spec.snapshot.version += 1;
    invalid.spec.providers.clear();
    assert!(engine.apply(invalid).is_err());
    assert_eq!(engine.snapshot().checksum(), before);
}

#[test]
fn lookups_never_observe_partial_group_reload() {
    let engine = Arc::new(RouteEngine::new(config()).unwrap());
    let mut threads = Vec::new();
    for _ in 0..4 {
        let engine = Arc::clone(&engine);
        threads.push(std::thread::spawn(move || {
            let consumer = InstanceId::from("backend-1");
            for _ in 0..2_000 {
                let snapshot = engine.snapshot();
                for slot in ["catalog", "search", "reports", "scheduler", "audit"] {
                    assert!(
                        snapshot
                            .lookup_consumer(&consumer, &RouteSlotId::from(slot))
                            .is_some()
                    );
                }
            }
        }));
    }

    for version in 2..100 {
        let mut next = config();
        next.spec.snapshot.version = version;
        assert_eq!(
            engine.apply(next).unwrap().status,
            ActivationStatus::Activated
        );
    }
    for thread in threads {
        thread.join().unwrap();
    }
}

#[test]
fn explicit_header_identity_can_be_compiled() {
    let mut value = config();
    value.spec.browser_routes[0].identity = BrowserIdentity::ExplicitHeader {
        value: BindingId::from("tab-one"),
    };
    let engine = RouteEngine::new(value).unwrap();
    let destination = RouteSlotId::from("browser-backend");
    assert_eq!(
        engine
            .snapshot()
            .lookup_browser(BrowserLookup {
                destination: &destination,
                explicit_header: Some("tab-one"),
                origin: None,
                proxy_listener: None,
            })
            .unwrap()
            .provider
            .as_str(),
        "backend-1"
    );
}

#[test]
fn header_and_origin_must_select_the_same_provider() {
    let mut value = config();
    value.spec.browser_routes.push(BrowserRoute {
        identity: BrowserIdentity::ExplicitHeader {
            value: BindingId::from("tab-two"),
        },
        destination: RouteSlotId::from("browser-backend"),
        provider: ComponentId::from("backend-2"),
    });
    let engine = RouteEngine::new(value).unwrap();
    let destination = RouteSlotId::from("browser-backend");

    assert_eq!(
        engine.snapshot().lookup_browser(BrowserLookup {
            destination: &destination,
            explicit_header: Some("tab-two"),
            origin: Some("https://ui-1.comparison.localhost"),
            proxy_listener: None,
        }),
        Err(LookupError::ConflictingIdentity)
    );
    assert_eq!(
        engine
            .snapshot()
            .lookup_browser(BrowserLookup {
                destination: &destination,
                explicit_header: Some("tab-two"),
                origin: Some("https://ui-2.comparison.localhost"),
                proxy_listener: None,
            })
            .unwrap()
            .provider
            .as_str(),
        "backend-2"
    );
    assert_eq!(
        engine
            .snapshot()
            .lookup_browser(BrowserLookup {
                destination: &destination,
                explicit_header: Some("tab-two"),
                origin: Some("https://unknown.example"),
                proxy_listener: None,
            })
            .unwrap()
            .provider
            .as_str(),
        "backend-2"
    );
}

#[test]
fn origin_matching_is_exact_and_scoped_to_the_destination() {
    let engine = RouteEngine::new(config()).unwrap();
    let snapshot = engine.snapshot();
    let destination = RouteSlotId::from("browser-backend");
    let other_destination = RouteSlotId::from("catalog");

    assert_eq!(
        snapshot.lookup_browser(BrowserLookup {
            destination: &destination,
            explicit_header: None,
            origin: Some("https://UI-2.comparison.localhost"),
            proxy_listener: None,
        }),
        Err(LookupError::UnknownIdentity)
    );
    assert_eq!(
        snapshot.lookup_browser(BrowserLookup {
            destination: &other_destination,
            explicit_header: None,
            origin: Some("https://ui-2.comparison.localhost"),
            proxy_listener: None,
        }),
        Err(LookupError::UnknownIdentity)
    );
    assert_eq!(snapshot.browser_candidates(&other_destination), []);
    assert!(
        snapshot
            .browser_candidates(&destination)
            .iter()
            .any(|candidate| candidate.identity == "origin:https://ui-2.comparison.localhost")
    );
}
