use std::{collections::BTreeMap, path::Path};

use switchyard_planner::{Bundle, Diagnostic};
use switchyard_state::{RouterBindingState, StateStore, StoredRouteSnapshot};

use crate::projections::{ServiceRow, planning_devices_for_bundle};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderDetail {
    pub instance: String,
    pub service: String,
    pub health: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectionRow {
    pub consumer: String,
    pub slot: String,
    pub current_group: Option<String>,
    pub compatible_groups: Vec<String>,
    pub providers: Vec<ProviderDetail>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConnectionMatrix {
    pub rows: Vec<ConnectionRow>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteChange {
    pub service: String,
    pub old_provider: Option<String>,
    pub new_provider: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SwitchPreview {
    pub consumer: String,
    pub old_group: Option<String>,
    pub new_group: String,
    pub old_providers: Vec<ProviderDetail>,
    pub new_providers: Vec<ProviderDetail>,
    pub affected_services: Vec<RouteChange>,
    pub diagnostics: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteHistoryEntry {
    pub version: i64,
    pub status: String,
    pub recorded_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteStatus {
    pub router: String,
    pub binding_id: String,
    pub desired_version: Option<i64>,
    pub observed_version: Option<i64>,
    pub previous_version: Option<i64>,
    pub apply_status: String,
    pub transition_state: String,
    pub last_error_code: Option<String>,
    pub history: Vec<RouteHistoryEntry>,
}

pub fn connection_matrix(
    project_dir: &Path,
    definition: &Path,
    services: &[ServiceRow],
) -> Result<ConnectionMatrix, String> {
    let bundle = effective_bundle(project_dir, definition)?;
    let devices = planning_devices_for_bundle(project_dir, &bundle)?;
    let mut compatible = BTreeMap::<String, Vec<String>>::new();
    let mut rows = Vec::new();
    for instance in &bundle.spec.instances {
        let Some(block) = bundle.spec.blocks.get(&instance.block) else {
            continue;
        };
        let slots = block
            .services
            .values()
            .flat_map(|service| service.consumes.keys().cloned())
            .collect::<std::collections::BTreeSet<_>>();
        if slots.is_empty() {
            continue;
        }
        let groups = compatible.entry(instance.name.clone()).or_insert_with(|| {
            bundle
                .spec
                .groups
                .keys()
                .filter(|group| {
                    switchyard_planner::plan_with_binding_and_devices(
                        &bundle,
                        &instance.name,
                        group,
                        &devices,
                    )
                    .is_ok()
                })
                .cloned()
                .collect()
        });
        let current_group = bundle.spec.bindings.get(&instance.name).cloned();
        let providers = current_group
            .as_deref()
            .map_or_else(Vec::new, |group| provider_details(&bundle, group, services));
        for slot in slots {
            rows.push(ConnectionRow {
                consumer: instance.name.clone(),
                slot,
                current_group: current_group.clone(),
                compatible_groups: groups.clone(),
                providers: providers.clone(),
            });
        }
    }
    Ok(ConnectionMatrix { rows })
}

pub fn switch_preview(
    project_dir: &Path,
    definition: &Path,
    consumer: &str,
    new_group: &str,
) -> Result<SwitchPreview, String> {
    let bundle = effective_bundle(project_dir, definition)?;
    let devices = planning_devices_for_bundle(project_dir, &bundle)?;
    let old_group = bundle.spec.bindings.get(consumer).cloned();
    let old_map = old_group
        .as_deref()
        .map(|group| resolved_group(&bundle, group))
        .transpose()?
        .unwrap_or_default();
    let new_map = resolved_group(&bundle, new_group).unwrap_or_default();
    let diagnostics =
        switchyard_planner::plan_with_binding_and_devices(&bundle, consumer, new_group, &devices)
            .err()
            .unwrap_or_default()
            .into_iter()
            .map(diagnostic_text)
            .collect::<Vec<_>>();
    let services = old_map
        .keys()
        .chain(new_map.keys())
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let affected_services = services
        .iter()
        .filter_map(|service| {
            let old_provider = old_map.get(service).cloned();
            let new_provider = new_map.get(service).cloned();
            (old_provider != new_provider).then(|| RouteChange {
                service: service.clone(),
                old_provider,
                new_provider,
            })
        })
        .collect();
    Ok(SwitchPreview {
        consumer: consumer.into(),
        old_group,
        new_group: new_group.into(),
        old_providers: details_from_map(&old_map, &[]),
        new_providers: details_from_map(&new_map, &[]),
        affected_services,
        diagnostics,
    })
}

pub fn route_status(project_dir: &Path, deployment: &str) -> Result<Vec<RouteStatus>, String> {
    let store = StateStore::open(project_dir.join(".switchyard/state.sqlite3"))
        .map_err(|error| error.to_string())?
        .0;
    let bindings = store
        .router_bindings(deployment)
        .map_err(|error| error.to_string())?;
    let history = store
        .route_history(deployment)
        .map_err(|error| error.to_string())?;
    Ok(project_route_status(&bindings, &history))
}

pub fn project_route_status(
    bindings: &[RouterBindingState],
    history: &[StoredRouteSnapshot],
) -> Vec<RouteStatus> {
    bindings
        .iter()
        .map(|binding| {
            let mut entries = history
                .iter()
                .filter(|entry| {
                    entry.binding.as_deref() == Some(&binding.binding)
                        && entry.router.as_deref() == Some(&binding.router)
                })
                .rev()
                .take(5)
                .map(|entry| RouteHistoryEntry {
                    version: entry.version,
                    status: entry.activation_status.clone(),
                    recorded_at: entry.recorded_at,
                })
                .collect::<Vec<_>>();
            entries.reverse();
            RouteStatus {
                router: binding.router.clone(),
                binding_id: binding.binding.clone(),
                desired_version: binding.desired_version,
                observed_version: binding.observed_version,
                previous_version: binding.previous_version,
                apply_status: binding.status.clone(),
                transition_state: transition_state(&binding.transition_json),
                last_error_code: binding.last_error_code.clone(),
                history: entries,
            }
        })
        .collect()
}

fn effective_bundle(project_dir: &Path, definition: &Path) -> Result<Bundle, String> {
    let mut authored =
        switchyard_planner::load_bundle(definition).map_err(|error| error.to_string())?;
    let resolved = project_dir
        .join(".switchyard/generated")
        .join(&authored.metadata.name)
        .join("resolved-deployment.yaml");
    if let Ok(applied) = switchyard_planner::load_bundle(&resolved) {
        if applied.metadata.name == authored.metadata.name {
            authored.spec.bindings = applied.spec.bindings;
            authored.spec.routes = applied.spec.routes;
            authored.spec.ui_routes = applied.spec.ui_routes;
        }
    }
    Ok(authored)
}

fn resolved_group(bundle: &Bundle, group: &str) -> Result<BTreeMap<String, String>, String> {
    resolved_group_inner(bundle, group, &mut std::collections::BTreeSet::new())
}

fn resolved_group_inner(
    bundle: &Bundle,
    group: &str,
    visiting: &mut std::collections::BTreeSet<String>,
) -> Result<BTreeMap<String, String>, String> {
    if !visiting.insert(group.into()) {
        return Err(format!(
            "provider group inheritance contains a cycle at `{group}`"
        ));
    }
    let Some(value) = bundle.spec.groups.get(group) else {
        return Err(format!("provider group `{group}` does not exist"));
    };
    let mut providers = if let Some(parent) = value.extends.as_deref() {
        resolved_group_inner(bundle, parent, visiting)?
    } else {
        BTreeMap::new()
    };
    providers.extend(value.providers.clone());
    visiting.remove(group);
    Ok(providers)
}

fn provider_details(bundle: &Bundle, group: &str, services: &[ServiceRow]) -> Vec<ProviderDetail> {
    resolved_group(bundle, group)
        .map(|providers| details_from_map(&providers, services))
        .unwrap_or_default()
}

fn details_from_map(
    providers: &BTreeMap<String, String>,
    services: &[ServiceRow],
) -> Vec<ProviderDetail> {
    providers
        .values()
        .map(|provider| {
            let (instance, service) = provider.split_once('/').unwrap_or((provider, "service"));
            let health = services
                .iter()
                .find(|row| row.instance == instance && row.service == service)
                .map_or("unknown", |row| row.health.as_str());
            ProviderDetail {
                instance: instance.into(),
                service: service.into(),
                health: health.into(),
            }
        })
        .collect()
}

fn diagnostic_text(diagnostic: Diagnostic) -> String {
    format!("{}: {}", diagnostic.path, diagnostic.message)
}

fn transition_state(json: &str) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return json.into();
    };
    ["state", "status", "strategy"]
        .into_iter()
        .find_map(|key| value.get(key).and_then(serde_json::Value::as_str))
        .map(str::to_owned)
        .unwrap_or_else(|| {
            if value == serde_json::json!({}) {
                "none".into()
            } else {
                json.into()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../switchyard-planner/tests/compat/routing-matrix-deployment.yaml")
    }

    #[test]
    fn matrix_and_preview_only_expose_complete_compatible_groups() {
        let definition = fixture();
        let root = definition.parent().unwrap();
        let matrix = connection_matrix(root, &definition, &[]).unwrap();
        let backend = matrix
            .rows
            .iter()
            .find(|row| row.consumer == "backend-1")
            .unwrap();
        assert_eq!(backend.current_group.as_deref(), Some("feature-services"));
        assert_eq!(
            backend.compatible_groups,
            ["feature-services", "main-services"]
        );

        let preview = switch_preview(root, &definition, "backend-1", "main-services").unwrap();
        assert!(preview.diagnostics.is_empty());
        assert_eq!(preview.old_providers.len(), 5);
        assert_eq!(preview.new_providers.len(), 5);
        assert_eq!(preview.affected_services.len(), 4);
    }

    #[test]
    fn preview_returns_planner_diagnostics_for_incompatible_switch() {
        let definition = fixture();
        let root = definition.parent().unwrap();
        let preview = switch_preview(root, &definition, "backend-1", "missing-group").unwrap();
        assert!(!preview.diagnostics.is_empty());
        assert!(
            preview
                .diagnostics
                .iter()
                .any(|item| item.contains("missing-group"))
        );
    }

    #[test]
    fn route_status_preserves_versions_failures_transition_and_recent_history() {
        let binding = RouterBindingState {
            deployment: "demo".into(),
            router: "sidecar".into(),
            binding: "consumer".into(),
            desired_version: Some(3),
            desired_checksum: None,
            current_version: Some(2),
            current_checksum: None,
            previous_version: Some(1),
            previous_checksum: None,
            observed_version: Some(2),
            observed_checksum: None,
            status: "failed".into(),
            transition_json: r#"{"state":"rolling_back"}"#.into(),
            last_error_code: Some("timeout".into()),
            updated_at: 10,
        };
        let history = StoredRouteSnapshot {
            sequence: 1,
            deployment: "demo".into(),
            router: Some("sidecar".into()),
            binding: Some("consumer".into()),
            operation_id: None,
            version: 2,
            checksum: "sum".into(),
            activation_status: "rolled_back".into(),
            recorded_at: 9,
            context_json: "{}".into(),
        };
        let projected = project_route_status(&[binding], &[history]);
        assert_eq!(projected[0].transition_state, "rolling_back");
        assert_eq!(projected[0].last_error_code.as_deref(), Some("timeout"));
        assert_eq!(projected[0].history[0].status, "rolled_back");
    }

    #[test]
    fn route_status_projects_active_applying_and_failed_router_states() {
        let state =
            |binding: &str, status: &str, observed, error: Option<&str>| RouterBindingState {
                deployment: "demo".into(),
                router: format!("{binding}-router"),
                binding: binding.into(),
                desired_version: Some(4),
                desired_checksum: None,
                current_version: observed,
                current_checksum: None,
                previous_version: Some(3),
                previous_checksum: None,
                observed_version: observed,
                observed_checksum: None,
                status: status.into(),
                transition_json: r#"{"strategy":"drain"}"#.into(),
                last_error_code: error.map(str::to_owned),
                updated_at: 10,
            };
        let projected = project_route_status(
            &[
                state("active", "active", Some(4), None),
                state("applying", "pending", Some(3), None),
                state("failed", "failed", Some(3), Some("router_timeout")),
            ],
            &[],
        );
        assert_eq!(
            projected
                .iter()
                .map(|item| item.apply_status.as_str())
                .collect::<Vec<_>>(),
            ["active", "pending", "failed"]
        );
        assert_eq!(projected[0].observed_version, Some(4));
        assert_eq!(projected[1].transition_state, "drain");
        assert_eq!(
            projected[2].last_error_code.as_deref(),
            Some("router_timeout")
        );
    }
}
