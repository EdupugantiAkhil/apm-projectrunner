use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use switchyard_sources::RegisteredSourceInspection;
use switchyard_state::{OwnedResourceObservation, RegisteredDevice, StateStore, StoredDeployment};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceRow {
    pub instance: String,
    pub service: String,
    pub status: String,
    pub health: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeploymentEntry {
    pub name: String,
    pub bundle: PathBuf,
    pub state: String,
    pub services: Vec<ServiceRow>,
    pub instances: Vec<InstanceRow>,
    pub blocks: Vec<String>,
    pub source_choices: Vec<SourceChoice>,
    pub bindings: Vec<BindingRow>,
    pub last_operation: Option<String>,
    pub applied: bool,
    pub consumer_slot_count: usize,
    pub validation_problems: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstanceRow {
    pub name: String,
    pub block: String,
    pub source: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceChoice {
    pub name: String,
    pub path: PathBuf,
    pub declared: bool,
    pub worktree: bool,
    pub repository: Option<PathBuf>,
    pub requested_ref: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DefinitionTopology {
    instances: Vec<InstanceRow>,
    blocks: Vec<String>,
    source_choices: Vec<SourceChoice>,
    bindings: Vec<BindingRow>,
}

#[derive(Default)]
struct DefinitionStatus {
    consumer_slot_count: usize,
    validation_problems: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BindingRow {
    pub consumer: String,
    pub group: String,
    pub compatible_groups: Vec<String>,
}

#[derive(Deserialize)]
pub struct DefinitionHeader {
    kind: String,
    metadata: DefinitionMetadata,
}

#[derive(Deserialize)]
pub struct DefinitionMetadata {
    name: String,
}

#[derive(Deserialize)]
pub struct ServiceManifest {
    #[serde(default)]
    services: Vec<ManifestService>,
}

#[derive(Deserialize)]
pub struct ManifestService {
    instance: String,
    component: String,
    service: String,
}

fn open_store(root: &Path) -> Result<StateStore, String> {
    StateStore::open(root.join(".switchyard/state.sqlite3"))
        .map(|value| value.0)
        .map_err(|error| error.to_string())
}

pub fn list_sources(root: &Path) -> Result<Vec<RegisteredSourceInspection>, Box<dyn Error>> {
    let store = StateStore::open(root.join(".switchyard/state.sqlite3"))?.0;
    Ok(switchyard_sources::SourceManager::new(root).list(&store)?)
}

pub fn list_devices(root: &Path) -> Result<Vec<RegisteredDevice>, String> {
    open_store(root)?
        .devices()
        .map_err(|error| error.to_string())
}

fn definition_paths(root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let primary = root.join("deployment.yaml");
    if primary.is_file() {
        paths.push(primary);
    }
    for directory in [root.to_path_buf(), root.join("deployments")] {
        if let Ok(entries) = fs::read_dir(directory) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file()
                    && matches!(
                        path.extension().and_then(|value| value.to_str()),
                        Some("yaml" | "yml")
                    )
                    && !paths.contains(&path)
                {
                    paths.push(path);
                }
            }
        }
    }
    paths.sort();
    paths
}

pub fn list_deployments(
    root: &Path,
    registered_sources: &[RegisteredSourceInspection],
) -> Result<Vec<DeploymentEntry>, String> {
    // A standalone TUI has no daemon startup cycle to reconcile Docker observations.
    // Keep the authored view available when Docker itself is unavailable.
    let _ = switchyard_daemon::reconcile_project(root);
    let store = open_store(root)?;
    let stored = store.deployments().map_err(|error| error.to_string())?;
    let mut definitions = BTreeMap::new();
    for path in definition_paths(root) {
        if let Ok(contents) = fs::read_to_string(&path) {
            if let Ok(header) = serde_yaml::from_str::<DefinitionHeader>(&contents) {
                if header.kind == "Deployment" {
                    definitions.entry(header.metadata.name).or_insert(path);
                }
            }
        }
    }
    let mut names = stored
        .iter()
        .map(|entry| entry.deployment.clone())
        .collect::<BTreeSet<_>>();
    names.extend(definitions.keys().cloned());
    let by_name = stored
        .into_iter()
        .map(|entry| (entry.deployment.clone(), entry))
        .collect::<BTreeMap<_, _>>();
    names
        .into_iter()
        .map(|name| {
            let record = by_name.get(&name);
            let bundle = definitions
                .get(&name)
                .cloned()
                .unwrap_or_else(|| root.join("deployments").join(format!("{name}.yaml")));
            let resources = store
                .active_resources(&name)
                .map_err(|error| error.to_string())?;
            let manifest = load_manifest_services(root, &name);
            let (instances, blocks, source_choices) =
                load_definition_choices(&bundle, registered_sources);
            let topology = DefinitionTopology {
                instances,
                blocks,
                source_choices,
                bindings: load_bindings(root, &name, &bundle),
            };
            let definition_status = definition_status(&bundle);
            Ok(deployment_entry(
                name,
                bundle,
                record,
                &resources,
                &manifest,
                topology,
                definition_status,
            ))
        })
        .collect()
}

fn load_manifest_services(root: &Path, deployment: &str) -> Vec<ManifestService> {
    let path = root
        .join(".switchyard/generated")
        .join(deployment)
        .join("manifest.json");
    fs::read_to_string(path)
        .ok()
        .and_then(|contents| serde_yaml::from_str::<ServiceManifest>(&contents).ok())
        .map_or_else(Vec::new, |manifest| manifest.services)
}

fn deployment_entry(
    name: String,
    bundle: PathBuf,
    stored: Option<&StoredDeployment>,
    resources: &[OwnedResourceObservation],
    manifest: &[ManifestService],
    topology: DefinitionTopology,
    definition_status: DefinitionStatus,
) -> DeploymentEntry {
    let operation = stored.and_then(|entry| entry.last_operation.as_ref());
    let active_operation =
        operation.filter(|operation| matches!(operation.status.as_str(), "pending" | "running"));
    let state = if let Some(operation) = active_operation {
        format!("{} ({})", operation.status, operation.kind)
    } else if resources.is_empty()
        && (stored
            .and_then(|entry| entry.definition_hash.as_ref())
            .is_some()
            || !manifest.is_empty())
    {
        "stopped".into()
    } else if resources.is_empty() {
        "not applied".into()
    } else if resources.iter().any(|resource| {
        resource.state.as_deref().is_some_and(|state| {
            ["unhealthy", "exited", "dead", "failed"]
                .iter()
                .any(|bad| state.to_ascii_lowercase().contains(bad))
        })
    }) {
        "degraded".into()
    } else {
        "running".into()
    };
    let services = resource_rows(&name, resources, manifest);
    let last_operation =
        operation.map(|operation| format!("{} {}", operation.kind, operation.status));
    DeploymentEntry {
        name,
        bundle,
        state,
        services,
        instances: topology.instances,
        blocks: topology.blocks,
        source_choices: topology.source_choices,
        bindings: topology.bindings,
        last_operation,
        applied: stored
            .and_then(|entry| entry.definition_hash.as_ref())
            .is_some()
            || !resources.is_empty(),
        consumer_slot_count: definition_status.consumer_slot_count,
        validation_problems: definition_status.validation_problems,
    }
}

fn definition_status(definition: &Path) -> DefinitionStatus {
    let Ok(bundle) = switchyard_planner::load_bundle(definition) else {
        return DefinitionStatus {
            consumer_slot_count: 0,
            validation_problems: vec!["The deployment definition could not be loaded.".into()],
        };
    };
    let consumer_slot_count = bundle
        .spec
        .blocks
        .values()
        .flat_map(|block| block.services.values())
        .map(|service| service.consumes.len())
        .sum();
    let validation_problems = switchyard_planner::plan(&bundle)
        .err()
        .unwrap_or_default()
        .into_iter()
        .map(|diagnostic| diagnostic.message)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    DefinitionStatus {
        consumer_slot_count,
        validation_problems,
    }
}

pub fn load_definition_choices(
    definition: &Path,
    registered_sources: &[RegisteredSourceInspection],
) -> (Vec<InstanceRow>, Vec<String>, Vec<SourceChoice>) {
    let Ok(bundle) = switchyard_planner::load_bundle(definition) else {
        return (Vec::new(), Vec::new(), Vec::new());
    };
    let instances = bundle
        .spec
        .instances
        .iter()
        .map(|instance| InstanceRow {
            name: instance.name.clone(),
            block: instance.block.clone(),
            source: instance.source.clone(),
        })
        .collect();
    let blocks = bundle.spec.blocks.keys().cloned().collect();
    let mut source_choices = bundle
        .spec
        .sources
        .iter()
        .map(|(name, source)| SourceChoice {
            name: name.clone(),
            path: source.path.clone(),
            declared: true,
            worktree: matches!(source.r#type, switchyard_planner::SourceType::Worktree),
            repository: source.repository.clone(),
            requested_ref: source.r#ref.clone(),
        })
        .collect::<Vec<_>>();
    for source in registered_sources {
        if !source_choices
            .iter()
            .any(|choice| choice.name == source.source.name)
        {
            source_choices.push(SourceChoice {
                name: source.source.name.clone(),
                path: source.source.path.clone(),
                declared: false,
                worktree: source.inspection.linked_worktree == Some(true),
                repository: source.source.repository_path.clone(),
                requested_ref: source.source.requested_ref.clone(),
            });
        }
    }
    source_choices.sort_by(|left, right| left.name.cmp(&right.name));
    (instances, blocks, source_choices)
}

fn load_bindings(root: &Path, deployment: &str, definition: &Path) -> Vec<BindingRow> {
    let Ok(mut authored) = switchyard_planner::load_bundle(definition) else {
        return Vec::new();
    };
    let resolved_path = root
        .join(".switchyard/generated")
        .join(deployment)
        .join("resolved-deployment.yaml");
    if let Ok(resolved) = switchyard_planner::load_bundle(&resolved_path) {
        if resolved.metadata.name == authored.metadata.name {
            authored.spec.bindings = resolved.spec.bindings;
            authored.spec.routes = resolved.spec.routes;
            authored.spec.ui_routes = resolved.spec.ui_routes;
        }
    }
    authored
        .spec
        .bindings
        .iter()
        .map(|(consumer, group)| {
            let mut compatible_groups = authored
                .spec
                .groups
                .keys()
                .filter(|candidate| {
                    switchyard_planner::plan_with_binding(&authored, consumer, candidate).is_ok()
                })
                .cloned()
                .collect::<Vec<_>>();
            if !compatible_groups.contains(group) {
                compatible_groups.push(group.clone());
                compatible_groups.sort();
            }
            BindingRow {
                consumer: consumer.clone(),
                group: group.clone(),
                compatible_groups,
            }
        })
        .collect()
}

fn resource_rows(
    deployment: &str,
    resources: &[OwnedResourceObservation],
    manifest: &[ManifestService],
) -> Vec<ServiceRow> {
    let containers = resources
        .iter()
        .filter(|resource| matches!(resource.kind, switchyard_state::ResourceKind::Container))
        .collect::<Vec<_>>();
    let mut matched = BTreeSet::new();
    let mut rows = manifest
        .iter()
        .map(|service| {
            let resource = containers
                .iter()
                .enumerate()
                .find(|(_, resource)| resource.name.contains(&service.service));
            if let Some((index, _)) = resource {
                matched.insert(index);
            }
            let status = resource
                .and_then(|(_, resource)| resource.state.clone())
                .unwrap_or_else(|| "stopped".into());
            ServiceRow {
                instance: service.instance.clone(),
                service: service.component.clone(),
                health: health_label(&status).into(),
                status,
            }
        })
        .collect::<Vec<_>>();
    rows.extend(
        containers
            .into_iter()
            .enumerate()
            .filter(|(index, _)| !matched.contains(index))
            .map(|(_, resource)| {
                let logical = resource
                    .name
                    .trim_start_matches('/')
                    .strip_prefix(&format!("sy-{deployment}-"))
                    .unwrap_or(&resource.name);
                let (instance, service) = logical.split_once('-').unwrap_or((logical, "container"));
                let status = resource.state.clone().unwrap_or_else(|| "observed".into());
                ServiceRow {
                    instance: instance.into(),
                    service: service.into(),
                    health: health_label(&status).into(),
                    status,
                }
            }),
    );
    rows
}

fn health_label(status: &str) -> &'static str {
    if status.to_ascii_lowercase().contains("unhealthy") {
        "unhealthy"
    } else if status.to_ascii_lowercase().contains("healthy") {
        "healthy"
    } else {
        "-"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applied_deployment_without_observations_is_stopped() {
        let stored = StoredDeployment {
            deployment: "demo".into(),
            definition_hash: Some("definition".into()),
            snapshot_json: None,
            applied_at: Some(1),
            last_operation: None,
        };
        let entry = deployment_entry(
            "demo".into(),
            "deployment.yaml".into(),
            Some(&stored),
            &[],
            &[],
            DefinitionTopology::default(),
            DefinitionStatus::default(),
        );
        assert_eq!(entry.state, "stopped");
    }
}
