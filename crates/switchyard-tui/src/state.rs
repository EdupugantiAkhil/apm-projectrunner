use std::path::{Path, PathBuf};

use switchyard_ops::{
    ConnectionMatrix, RunScript, connection_matrix, list_deployments, list_devices, list_profiles,
    list_sources, run_scripts,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct SourceProjection {
    pub(crate) name: String,
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
        let sources_result = list_sources(&self.project_dir);
        self.sources_error = sources_result.as_ref().err().map(ToString::to_string);
        let sources = sources_result.unwrap_or_default();
        self.sources = sources
            .iter()
            .map(|source| SourceProjection {
                name: source.source.name.clone(),
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
