use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use switchyard_ops::{
    ConnectionMatrix, RunScript, connection_matrix, list_deployments, list_devices, list_profiles,
    list_sources, run_scripts,
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct DeviceProjection {
    pub(crate) name: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ProfileProjection {
    pub(crate) name: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct InstanceProjection {
    pub(crate) name: String,
    pub(crate) source: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ServiceProjection {
    pub(crate) status: String,
    pub(crate) health: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct DeploymentProjection {
    pub(crate) name: String,
    pub(crate) applied: bool,
    pub(crate) instances: Vec<InstanceProjection>,
    pub(crate) services: Vec<ServiceProjection>,
    pub(crate) binding_count: usize,
    pub(crate) consumer_slot_count: usize,
    pub(crate) validation_problems: Vec<String>,
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
                    .map(|device| DeviceProjection { name: device.name })
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
                applied: deployment.applied,
                instances: deployment
                    .instances
                    .into_iter()
                    .map(|instance| InstanceProjection {
                        name: instance.name,
                        source: instance.source,
                    })
                    .collect(),
                services: deployment
                    .services
                    .into_iter()
                    .map(|service| ServiceProjection {
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
                        if !self.profiles.iter().any(|profile| profile.name == row.name) {
                            self.profiles.push(ProfileProjection { name: row.name });
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
    use super::strip_url_userinfo;

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
}
