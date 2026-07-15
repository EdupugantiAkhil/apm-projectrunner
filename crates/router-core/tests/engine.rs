use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

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
#[ignore = "duration-based reliability test; run via scripts/reliability.sh"]
fn reload_storm_preserves_group_atomicity_and_version_order() {
    let duration = std::env::var("SWITCHYARD_RELOAD_STORM_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(30));
    let mut initial = config();
    initial.spec.snapshot.version = 1;
    let engine = Arc::new(RouteEngine::new(initial).unwrap());
    let stop = Arc::new(AtomicBool::new(false));
    let partial_groups = Arc::new(AtomicUsize::new(0));
    let stale_acceptances = Arc::new(AtomicUsize::new(0));
    let version_regressions = Arc::new(AtomicUsize::new(0));
    let highest_seen = Arc::new(AtomicU64::new(1));

    let mut threads = Vec::new();
    let workers = std::env::var("SWITCHYARD_CONCURRENCY")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(8);
    for _ in 0..workers {
        let engine = Arc::clone(&engine);
        let stop = Arc::clone(&stop);
        let partial_groups = Arc::clone(&partial_groups);
        let version_regressions = Arc::clone(&version_regressions);
        let highest_seen = Arc::clone(&highest_seen);
        threads.push(std::thread::spawn(move || {
            let consumer = InstanceId::from("backend-1");
            let slots = [
                RouteSlotId::from("catalog"),
                RouteSlotId::from("search"),
                RouteSlotId::from("reports"),
                RouteSlotId::from("scheduler"),
            ];
            // Monotonicity is only guaranteed per observer: between one thread's
            // `snapshot()` and a shared fetch_max, another thread can legitimately
            // observe a newer version, so a global high-water check would produce
            // false regressions. Track the last version seen by this thread.
            let mut last_version = 0_u64;
            while !stop.load(Ordering::Relaxed) {
                let snapshot = engine.snapshot();
                let version = snapshot.version();
                if version < last_version {
                    version_regressions.fetch_add(1, Ordering::Relaxed);
                }
                last_version = version;
                highest_seen.fetch_max(version, Ordering::SeqCst);
                let providers = slots
                    .iter()
                    .map(|slot| {
                        snapshot
                            .lookup_consumer(&consumer, slot)
                            .map(|route| route.provider.to_string())
                    })
                    .collect::<Vec<_>>();
                let all_main = providers.iter().all(|provider| {
                    provider
                        .as_deref()
                        .is_some_and(|provider| provider.starts_with("services-main/"))
                });
                let all_feature = providers.iter().all(|provider| {
                    provider
                        .as_deref()
                        .is_some_and(|provider| provider.starts_with("services-feature/"))
                });
                if !(all_main || all_feature) {
                    partial_groups.fetch_add(1, Ordering::Relaxed);
                }
                assert_eq!(
                    snapshot
                        .lookup_consumer(&consumer, &RouteSlotId::from("audit"))
                        .unwrap()
                        .provider
                        .as_str(),
                    "services-shared/audit"
                );
            }
        }));
    }

    let deadline = Instant::now() + duration;
    let mut version = 2_u64;
    while Instant::now() < deadline {
        let mut next = config();
        next.spec.snapshot.version = version;
        let group = if version % 2 == 0 {
            "main-services"
        } else {
            "feature-services"
        };
        next.spec.bindings[0].group = router_config::GroupId::from(group);
        assert_eq!(
            engine.apply(next).unwrap().status,
            ActivationStatus::Activated
        );

        let mut stale = config();
        stale.spec.snapshot.version = version - 1;
        if engine.apply(stale).unwrap().status == ActivationStatus::Activated {
            stale_acceptances.fetch_add(1, Ordering::Relaxed);
        }
        version += 1;
    }

    stop.store(true, Ordering::Relaxed);
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(partial_groups.load(Ordering::Relaxed), 0);
    assert_eq!(stale_acceptances.load(Ordering::Relaxed), 0);
    assert_eq!(version_regressions.load(Ordering::Relaxed), 0);
    assert!(engine.snapshot().version() >= 2);
    // Readers must have actually observed reloads, or the storm proved nothing.
    assert!(highest_seen.load(Ordering::SeqCst) >= 2);
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
