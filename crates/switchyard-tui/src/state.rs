use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use switchyard_ops::profiles::{ProfileOrigin, ProfileRow};
use switchyard_ops::{
    ConnectionMatrix, RunScript, connection_matrix, list_deployments, list_devices, list_profiles,
    list_sources, load_profile_block, run_scripts,
};
use switchyard_sources::SourceManager;
use switchyard_state::{RegisteredSourceKind, StateStore};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SourceProjection {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) repository_path: Option<PathBuf>,
    pub(crate) requested_ref: Option<String>,
    pub(crate) remote: Option<String>,
    pub(crate) ownership: RegisteredSourceKind,
    pub(crate) inspection: switchyard_sources::SourceInspection,
    pub(crate) available: bool,
    pub(crate) inspected_at: i64,
}

#[cfg(test)]
impl Default for SourceProjection {
    fn default() -> Self {
        let path = PathBuf::from("/switchyard-test-source-does-not-exist");
        Self {
            name: String::new(),
            path: path.clone(),
            repository_path: None,
            requested_ref: None,
            remote: None,
            ownership: RegisteredSourceKind::Unmanaged,
            inspection: SourceManager::new("/switchyard-test-workspace").inspect(&path, None),
            available: false,
            inspected_at: 0,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DeviceProjection {
    pub(crate) name: String,
    pub(crate) device: switchyard_state::RegisteredDevice,
}

#[derive(Clone, Debug)]
pub(crate) struct ProfileProjection {
    pub(crate) name: String,
    pub(crate) row: ProfileRow,
    pub(crate) definition: PathBuf,
    pub(crate) json: serde_json::Value,
    pub(crate) detail: String,
}

#[cfg(test)]
impl Default for ProfileProjection {
    fn default() -> Self {
        Self {
            name: String::new(),
            row: ProfileRow {
                name: String::new(),
                origin: ProfileOrigin::Project,
                trust: switchyard_ops::ProfileTrust::Trusted,
                shadowed: false,
                services: Vec::new(),
            },
            definition: PathBuf::new(),
            json: serde_json::Value::Null,
            detail: String::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct InstanceProjection {
    pub(crate) name: String,
    pub(crate) profile: String,
    pub(crate) source: String,
    pub(crate) device: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ServiceProjection {
    pub(crate) instance: String,
    pub(crate) service: String,
    pub(crate) device: String,
    pub(crate) status: String,
    pub(crate) health: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct DeploymentProjection {
    pub(crate) name: String,
    pub(crate) bundle: PathBuf,
    pub(crate) state: String,
    pub(crate) last_operation: Option<String>,
    pub(crate) applied: bool,
    pub(crate) instances: Vec<InstanceProjection>,
    pub(crate) source_choices: Vec<switchyard_ops::SourceChoice>,
    pub(crate) connections: ConnectionMatrix,
    pub(crate) route_statuses: Vec<switchyard_ops::RouteStatus>,
    pub(crate) services: Vec<ServiceProjection>,
    pub(crate) binding_count: usize,
    pub(crate) consumer_slot_count: usize,
    pub(crate) validation_problems: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OperationOutcome {
    Running,
    Finished(i32),
    Failed(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OperationLogEntry {
    pub(crate) label: String,
    pub(crate) deployment: Option<String>,
    pub(crate) destructive: bool,
    pub(crate) lines: Vec<String>,
    pub(crate) outcome: OperationOutcome,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct OperationLog {
    entries: Vec<OperationLogEntry>,
}

impl OperationLog {
    pub(crate) fn start(
        &mut self,
        label: impl Into<String>,
        deployment: Option<String>,
        destructive: bool,
    ) {
        self.entries.push(OperationLogEntry {
            label: label.into(),
            deployment,
            destructive,
            lines: Vec::new(),
            outcome: OperationOutcome::Running,
        });
    }

    pub(crate) fn append(&mut self, line: impl Into<String>) {
        if let Some(entry) = self.entries.last_mut() {
            entry.lines.push(line.into());
        }
    }

    pub(crate) fn finish(&mut self, outcome: OperationOutcome) {
        if let Some(entry) = self.entries.last_mut() {
            entry.outcome = outcome;
        }
    }

    pub(crate) fn entries(&self) -> &[OperationLogEntry] {
        &self.entries
    }

    pub(crate) fn last_is_running(&self) -> bool {
        self.entries
            .last()
            .is_some_and(|entry| entry.outcome == OperationOutcome::Running)
    }

    pub(crate) fn render(&self) -> String {
        let mut output = Vec::new();
        for entry in &self.entries {
            let destructive = if entry.destructive {
                " [DESTRUCTIVE]"
            } else {
                ""
            };
            let deployment = entry
                .deployment
                .as_deref()
                .map_or_else(String::new, |name| format!(" — {name}"));
            output.push(format!("{}{}{}", entry.label, destructive, deployment));
            output.extend(entry.lines.iter().map(|line| format!("  {line}")));
            output.push(format!("  => {:?}", entry.outcome));
        }
        output.join("\n")
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ProjectState {
    pub(crate) project_dir: PathBuf,
    pub(crate) sources: Vec<SourceProjection>,
    pub(crate) devices: Vec<DeviceProjection>,
    pub(crate) deployments: Vec<DeploymentProjection>,
    pub(crate) profiles: Vec<ProfileProjection>,
    pub(crate) run_scripts: Vec<RunScript>,
    pub(crate) connections: Vec<ConnectionMatrix>,
    pub(crate) sources_error: Option<String>,
    pub(crate) devices_error: Option<String>,
    pub(crate) deployments_error: Option<String>,
    pub(crate) profiles_error: Option<String>,
    pub(crate) run_scripts_error: Option<String>,
    pub(crate) connections_error: Option<String>,
    pub(crate) profile_source_errors: Vec<String>,
    pub(crate) operation_log: OperationLog,
}

impl ProjectState {
    pub(crate) fn load(project_dir: &Path) -> Self {
        let mut state = Self {
            project_dir: project_dir.to_path_buf(),
            ..Self::default()
        };
        state.refresh();
        state
    }

    pub(crate) fn refresh(&mut self) {
        let inspected_at = unix_millis();
        let sources_result = list_sources(&self.project_dir);
        self.sources_error = sources_result.as_ref().err().map(ToString::to_string);
        let sources = sources_result.unwrap_or_default();
        self.sources = sources
            .iter()
            .map(|source| SourceProjection {
                name: source.source.name.clone(),
                path: source.source.path.clone(),
                repository_path: source.source.repository_path.clone(),
                requested_ref: source.source.requested_ref.clone(),
                remote: sanitized_git_remote(&source.source.path),
                ownership: source.source.kind,
                inspection: source.inspection.clone(),
                available: source.source.path.exists(),
                inspected_at,
            })
            .collect();

        match list_devices(&self.project_dir) {
            Ok(devices) => {
                self.devices = devices
                    .into_iter()
                    .map(|device| DeviceProjection {
                        name: device.name.clone(),
                        device,
                    })
                    .collect();
                self.devices_error = None;
            }
            Err(error) => {
                self.devices.clear();
                self.devices_error = Some(error);
            }
        }

        let deployments_result = list_deployments(&self.project_dir, &sources);
        self.deployments_error = deployments_result.as_ref().err().cloned();
        let deployments = deployments_result.unwrap_or_default();

        self.refresh_profiles(&deployments);
        self.refresh_connections(&deployments);
        self.deployments = deployments
            .into_iter()
            .map(|deployment| DeploymentProjection {
                name: deployment.name,
                bundle: deployment.bundle,
                state: deployment.state,
                last_operation: deployment.last_operation,
                applied: deployment.applied,
                instances: deployment
                    .instances
                    .into_iter()
                    .map(|instance| InstanceProjection {
                        name: instance.name,
                        profile: instance.block,
                        source: instance.source,
                        device: instance.device,
                    })
                    .collect(),
                source_choices: deployment.source_choices,
                connections: deployment.connections,
                route_statuses: deployment.route_statuses,
                services: deployment
                    .services
                    .into_iter()
                    .map(|service| ServiceProjection {
                        instance: service.instance,
                        service: service.service,
                        device: service.device,
                        status: service.status,
                        health: service.health,
                    })
                    .collect(),
                binding_count: deployment.bindings.len(),
                consumer_slot_count: deployment.consumer_slot_count,
                validation_problems: deployment.validation_problems,
            })
            .collect();

        let (scripts, error) = run_scripts::load(&self.project_dir);
        self.run_scripts = scripts;
        self.run_scripts_error = error;
    }

    pub(crate) fn register_local_source(&self, name: &str, path: &Path) -> Result<(), String> {
        let store = open_store(&self.project_dir)?;
        let manager = SourceManager::new(&self.project_dir);
        let name = unique_source_name(&store, name)?;
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.project_dir.join(path)
        };
        manager
            .register_unmanaged(&store, &name, &path)
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub(crate) fn create_worktree(
        &self,
        source: &str,
        base_ref: &str,
        branch: &str,
        name: &str,
        path: Option<&Path>,
    ) -> Result<(), String> {
        let store = open_store(&self.project_dir)?;
        let manager = SourceManager::new(&self.project_dir);
        let name = unique_source_name(&store, name)?;
        let requested_path = path.map(|path| {
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                self.project_dir.join(path)
            }
        });
        manager
            .create_worktree_branch(
                &store,
                source,
                base_ref,
                branch,
                &name,
                requested_path.as_deref(),
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub(crate) fn remove_source(&self, name: &str) -> Result<(), String> {
        let store = open_store(&self.project_dir)?;
        let manager = SourceManager::new(&self.project_dir);
        let source = store
            .source(name)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("source `{name}` is not registered"))?;
        if source.kind == RegisteredSourceKind::Managed {
            manager
                .remove(&store, name, false)
                .map_err(|error| error.to_string())?;
        }
        manager
            .deregister(&store, name)
            .map_err(|error| error.to_string())
    }

    fn refresh_profiles(&mut self, deployments: &[switchyard_ops::DeploymentEntry]) {
        self.profiles.clear();
        self.profile_source_errors.clear();
        let mut errors = Vec::new();
        for deployment in deployments {
            match list_profiles(&self.project_dir, &deployment.bundle) {
                Ok(listing) => {
                    for row in listing.rows {
                        if self.profiles.iter().any(|profile| {
                            profile.name == row.name && profile.row.origin == row.origin
                        }) {
                            continue;
                        }
                        match load_profile_block(
                            &self.project_dir,
                            &deployment.bundle,
                            &row.name,
                            &row.origin,
                        ) {
                            Ok(block) => {
                                let json = serde_json::to_value(&block).unwrap_or_default();
                                let detail = profile_detail(&row, &json);
                                self.profiles.push(ProfileProjection {
                                    name: row.name.clone(),
                                    row,
                                    definition: deployment.bundle.clone(),
                                    json,
                                    detail,
                                });
                            }
                            Err(error) => errors.push(format!("{}: {error}", row.name)),
                        }
                    }
                    self.profile_source_errors.extend(
                        listing
                            .source_errors
                            .into_iter()
                            .map(|error| format!("{}: {}", error.source, error.message)),
                    );
                }
                Err(error) => errors.push(format!("{}: {error}", deployment.name)),
            }
        }
        self.profiles_error = (!errors.is_empty()).then(|| errors.join("; "));
    }

    fn refresh_connections(&mut self, deployments: &[switchyard_ops::DeploymentEntry]) {
        self.connections.clear();
        let mut errors = Vec::new();
        for deployment in deployments {
            match connection_matrix(&self.project_dir, &deployment.bundle, &deployment.services) {
                Ok(matrix) => self.connections.push(matrix),
                Err(error) => errors.push(format!("{}: {error}", deployment.name)),
            }
        }
        self.connections_error = (!errors.is_empty()).then(|| errors.join("; "));
    }
}

fn profile_detail(row: &ProfileRow, block: &serde_json::Value) -> String {
    let origin = match &row.origin {
        ProfileOrigin::Project => "Project".into(),
        ProfileOrigin::ImportedFromSource { source, commit } => {
            format!("Imported from {source}@{}", short(commit.as_deref()))
        }
        ProfileOrigin::DiscoveredInSource { source, commit } => {
            format!("Source: {source}@{}", short(commit.as_deref()))
        }
    };
    let trust = format!("{:?}", row.trust);
    let parameters = block
        .get("parameters")
        .and_then(serde_json::Value::as_object)
        .map_or_else(
            || "none".into(),
            |items| {
                items
                    .iter()
                    .map(|(name, value)| {
                        format!(
                            "{name} ({})",
                            if value
                                .get("required")
                                .and_then(serde_json::Value::as_bool)
                                .unwrap_or(false)
                            {
                                "required"
                            } else {
                                "optional"
                            }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            },
        );
    let mut lines = vec![
        format!("Origin: {origin}"),
        format!("Trust: {trust}"),
        format!("Parameters: {parameters}"),
        String::new(),
    ];
    if let Some(services) = block.get("services").and_then(serde_json::Value::as_object) {
        for (name, service) in services {
            let execution = service.get("execution").unwrap_or(&serde_json::Value::Null);
            lines.push(format!("Service: {name}"));
            lines.push(format!(
                "  Adapter: {}",
                execution
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown")
            ));
            lines.push(format!(
                "  Command: {}",
                display_value(execution.get("command"))
            ));
            lines.push(format!(
                "  Workdir: {}",
                display_value(execution.get("workingDirectory"))
            ));
            lines.push(format!(
                "  Mounts: {}",
                display_value(service.get("volumes"))
            ));
            lines.push(format!(
                "  Capabilities: {}",
                display_value(service.get("provides"))
            ));
            lines.push(format!(
                "  Consumed slots: {}",
                display_value(service.get("consumes"))
            ));
            lines.push(format!("  Probes: {}", display_value(service.get("probe"))));
            lines.push(format!(
                "  Lifecycle: {}",
                display_value(execution.get("lifecycle"))
            ));
            lines.push(String::new());
        }
    }
    lines.join("\n")
}

fn display_value(value: Option<&serde_json::Value>) -> String {
    match value {
        None | Some(serde_json::Value::Null) => "none".into(),
        Some(serde_json::Value::Array(items)) if items.is_empty() => "none".into(),
        Some(serde_json::Value::Object(items)) if items.is_empty() => "none".into(),
        Some(value) => serde_json::to_string(value).unwrap_or_else(|_| "unavailable".into()),
    }
}

fn short(value: Option<&str>) -> String {
    value
        .map(|text| text.chars().take(10).collect())
        .unwrap_or_else(|| "unknown".into())
}

fn open_store(root: &Path) -> Result<StateStore, String> {
    StateStore::open(root.join(".switchyard/state.sqlite3"))
        .map(|value| value.0)
        .map_err(|error| error.to_string())
}

pub(crate) fn unique_source_name(store: &StateStore, base: &str) -> Result<String, String> {
    if store
        .source(base)
        .map_err(|error| error.to_string())?
        .is_none()
    {
        return Ok(base.to_owned());
    }
    for suffix in 2..=10_000 {
        let candidate = format!("{base}-{suffix}");
        if store
            .source(&candidate)
            .map_err(|error| error.to_string())?
            .is_none()
        {
            return Ok(candidate);
        }
    }
    Err(format!(
        "could not find an available source name based on `{base}`"
    ))
}

fn unix_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

fn sanitized_git_remote(path: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["config", "--get", "remote.origin.url"])
        .current_dir(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let remote = String::from_utf8(output.stdout).ok()?;
    let remote = remote.trim();
    if remote.is_empty() {
        return None;
    }
    Some(strip_url_userinfo(remote))
}

fn strip_url_userinfo(remote: &str) -> String {
    if let Some((scheme, rest)) = remote.split_once("://")
        && let Some((authority, suffix)) = rest.split_once('/')
        && let Some((_, host)) = authority.rsplit_once('@')
    {
        return format!("{scheme}://{host}/{suffix}");
    }
    remote.to_owned()
}

#[cfg(test)]
mod tests {
    use super::{OperationLog, OperationOutcome, strip_url_userinfo};

    #[test]
    fn remote_projection_never_keeps_http_credentials() {
        assert_eq!(
            strip_url_userinfo("https://user:secret@example.test/team/repo.git"),
            "https://example.test/team/repo.git"
        );
        assert_eq!(
            strip_url_userinfo("git@example.test:team/repo.git"),
            "git@example.test:team/repo.git"
        );
    }

    #[test]
    fn operation_log_retains_order_output_labels_and_result() {
        let mut log = OperationLog::default();
        log.start("cleanup", Some("demo".into()), true);
        log.append("removing owned volume demo-data");
        log.finish(OperationOutcome::Finished(0));
        assert_eq!(log.entries().len(), 1);
        assert_eq!(log.entries()[0].lines, ["removing owned volume demo-data"]);
        let rendered = log.render();
        assert!(rendered.contains("cleanup [DESTRUCTIVE] — demo"));
        assert!(rendered.contains("Finished(0)"));
    }
}
