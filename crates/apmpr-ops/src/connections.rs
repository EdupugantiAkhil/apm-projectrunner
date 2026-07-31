use std::path::Path;

use apmpr_planner::{Bundle, Diagnostic};
use apmpr_state::{RouterBindingState, StateStore, StoredRouteSnapshot};

use crate::projections::{ServiceRow, planning_devices_for_bundle};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemberDetail {
    pub instance: String,
    pub service: String,
    pub health: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectionRow {
    pub instance: String,
    pub current_group: Option<String>,
    pub groups: Vec<String>,
    pub members: Vec<MemberDetail>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConnectionMatrix {
    pub rows: Vec<ConnectionRow>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MembershipChange {
    pub member: String,
    pub old_member: Option<String>,
    pub new_member: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SwitchPreview {
    pub instance: String,
    pub old_group: Option<String>,
    pub new_group: String,
    pub old_members: Vec<MemberDetail>,
    pub new_members: Vec<MemberDetail>,
    pub membership_changes: Vec<MembershipChange>,
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
    pub instance_id: String,
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
    let mut rows = Vec::new();
    for instance in &bundle.spec.instances {
        let current_group = bundle.spec.groups.iter().find_map(|(name, group)| {
            group
                .instances
                .iter()
                .any(|member| member.split('/').next() == Some(instance.name.as_str()))
                .then(|| name.clone())
        });
        let members = current_group
            .as_deref()
            .map(|group| provider_details(&bundle, group, services))
            .transpose()?
            .unwrap_or_default();
        rows.push(ConnectionRow {
            instance: instance.name.clone(),
            current_group,
            groups: bundle.spec.groups.keys().cloned().collect(),
            members,
        });
    }
    Ok(ConnectionMatrix { rows })
}

pub fn switch_preview(
    project_dir: &Path,
    definition: &Path,
    instance: &str,
    new_group: &str,
) -> Result<SwitchPreview, String> {
    let bundle = effective_bundle(project_dir, definition)?;
    let devices = planning_devices_for_bundle(project_dir, &bundle)?;
    let old_group = bundle.spec.groups.iter().find_map(|(name, group)| {
        group
            .instances
            .iter()
            .any(|member| member.split('/').next() == Some(instance))
            .then(|| name.clone())
    });
    let old_members = old_group
        .as_deref()
        .map(|group| group_members(&bundle, group))
        .transpose()?
        .unwrap_or_default();
    let mut moved = bundle.clone();
    let mut moved_member = None;
    for group in moved.spec.groups.values_mut() {
        group.instances.retain(|member| {
            if member.split('/').next() == Some(instance) {
                moved_member.get_or_insert_with(|| member.clone());
                false
            } else {
                true
            }
        });
        group.disabled.retain(|member| member != instance);
    }
    if let Some(group) = moved.spec.groups.get_mut(new_group) {
        group
            .instances
            .push(moved_member.unwrap_or_else(|| instance.to_owned()));
    }
    let new_members = group_members(&moved, new_group).unwrap_or_default();
    let diagnostics =
        apmpr_planner::plan_with_membership_and_devices(&bundle, instance, new_group, &devices)
            .err()
            .unwrap_or_default()
            .into_iter()
            .map(diagnostic_text)
            .collect::<Vec<_>>();
    let members = old_members
        .iter()
        .chain(&new_members)
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let membership_changes = members
        .iter()
        .filter_map(|member| {
            let old_member = old_members.contains(member).then(|| member.clone());
            let new_member = new_members.contains(member).then(|| member.clone());
            (old_member != new_member).then(|| MembershipChange {
                member: member.clone(),
                old_member,
                new_member,
            })
        })
        .collect();
    Ok(SwitchPreview {
        instance: instance.into(),
        old_group,
        new_group: new_group.into(),
        old_members: details_from_members(&bundle, &old_members, &[])?,
        new_members: details_from_members(&bundle, &new_members, &[])?,
        membership_changes,
        diagnostics,
    })
}

pub fn route_status(project_dir: &Path, deployment: &str) -> Result<Vec<RouteStatus>, String> {
    let store = StateStore::open(project_dir.join(".apmpr/state.sqlite3"))
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
                instance_id: binding.binding.clone(),
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
    let mut authored = apmpr_planner::load_bundle(definition).map_err(|error| error.to_string())?;
    let resolved = project_dir
        .join(".apmpr/generated")
        .join(&authored.metadata.name)
        .join("resolved-deployment.yaml");
    if let Ok(applied) = apmpr_planner::load_bundle(&resolved) {
        if applied.metadata.name == authored.metadata.name {
            authored.spec.groups = applied.spec.groups;
        }
    }
    Ok(authored)
}

fn group_members(bundle: &Bundle, group: &str) -> Result<Vec<String>, String> {
    bundle
        .spec
        .groups
        .get(group)
        .map(|definition| {
            definition
                .instances
                .iter()
                .filter(|member| {
                    !definition
                        .disabled
                        .iter()
                        .any(|disabled| member.split('/').next() == Some(disabled))
                })
                .cloned()
                .collect()
        })
        .ok_or_else(|| format!("group `{group}` does not exist"))
}

fn provider_details(
    bundle: &Bundle,
    group: &str,
    services: &[ServiceRow],
) -> Result<Vec<MemberDetail>, String> {
    let members = group_members(bundle, group)?;
    details_from_members(bundle, &members, services)
}

fn details_from_members(
    bundle: &Bundle,
    members: &[String],
    services: &[ServiceRow],
) -> Result<Vec<MemberDetail>, String> {
    Ok(members
        .iter()
        .flat_map(|member| {
            let (instance, explicit_service) = member
                .split_once('/')
                .map_or((member.as_str(), None), |(instance, service)| {
                    (instance, Some(service))
                });
            let service_names = explicit_service.map_or_else(
                || {
                    bundle
                        .spec
                        .instances
                        .iter()
                        .find(|candidate| candidate.name == instance)
                        .map(|candidate| {
                            if candidate.is_external() {
                                vec![format!(
                                    "external ports {}",
                                    candidate
                                        .expanded_external_ports()
                                        .iter()
                                        .map(u16::to_string)
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                )]
                            } else {
                                bundle
                                    .spec
                                    .blocks
                                    .get(&candidate.block)
                                    .map(|block| block.services.keys().cloned().collect())
                                    .unwrap_or_default()
                            }
                        })
                        .unwrap_or_default()
                },
                |service| vec![service.to_owned()],
            );
            service_names.into_iter().map(move |service| {
                let health = services
                    .iter()
                    .find(|row| row.instance == instance && row.service == service)
                    .map_or_else(
                        || {
                            if service.starts_with("external ports ") {
                                "external"
                            } else {
                                "unknown"
                            }
                        },
                        |row| row.health.as_str(),
                    );
                MemberDetail {
                    instance: instance.into(),
                    service,
                    health: health.into(),
                }
            })
        })
        .collect())
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
            .join("../apmpr-planner/tests/compat/routing-matrix-deployment.yaml")
    }

    #[test]
    fn matrix_and_preview_expose_complete_groups_and_membership_changes() {
        let definition = fixture();
        let root = definition.parent().unwrap();
        let matrix = connection_matrix(root, &definition, &[]).unwrap();
        let backend = matrix
            .rows
            .iter()
            .find(|row| row.instance == "backend-1")
            .unwrap();
        assert_eq!(backend.current_group.as_deref(), Some("feature-services"));
        assert_eq!(backend.groups, ["feature-services", "main-services"]);

        let preview = switch_preview(root, &definition, "backend-1", "main-services").unwrap();
        assert!(preview.diagnostics.is_empty());
        assert_eq!(preview.old_members.len(), 6);
        assert_eq!(preview.new_members.len(), 7);
        assert_eq!(preview.membership_changes.len(), 11);
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
