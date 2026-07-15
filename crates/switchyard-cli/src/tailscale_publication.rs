use std::{
    collections::BTreeSet,
    fmt, fs, io,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use router_config::{GatewayExposureMode, RouterConfig};
use serde::{Deserialize, Serialize};
use serde_json::json;
use switchyard_adapter_sdk::{
    Diagnostic, PublicationAdapter, PublicationCheck, PublicationCheckOutcome, PublicationRecord,
};
use switchyard_adapters::TailscalePublicationAdapter;
use switchyard_planner::Plan;

const STATE_API_VERSION: &str = "switchyard.dev/tailscale-publication/v1alpha1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TailscaleStatus {
    pub configured: bool,
    pub record: Option<PublicationRecord>,
    pub checks: Vec<PublicationCheck>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PublicationState {
    api_version: String,
    deployment: String,
    definition_hash: String,
    record: PublicationRecord,
}

#[derive(Debug)]
pub enum TailscaleError {
    Io(io::Error),
    InvalidPlan(String),
    StaleState(String),
    Publication(String),
}

impl fmt::Display for TailscaleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::InvalidPlan(detail) => {
                write!(formatter, "invalid Tailscale publication plan: {detail}")
            }
            Self::StaleState(detail) => {
                write!(formatter, "stale Tailscale publication state: {detail}")
            }
            Self::Publication(detail) => {
                write!(formatter, "Tailscale publication failed: {detail}")
            }
        }
    }
}

impl std::error::Error for TailscaleError {}
impl From<io::Error> for TailscaleError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub struct TailscaleRuntime<'a> {
    workspace_root: &'a Path,
    plan: &'a Plan,
}

impl<'a> TailscaleRuntime<'a> {
    pub fn new(workspace_root: &'a Path, plan: &'a Plan) -> Self {
        Self {
            workspace_root,
            plan,
        }
    }

    pub fn start(&self) -> Result<TailscaleStatus, TailscaleError> {
        let Some(configuration) = self.configuration()? else {
            self.stop()?;
            return Ok(TailscaleStatus {
                configured: false,
                record: None,
                checks: Vec::new(),
            });
        };
        let record = TailscalePublicationAdapter::default()
            .publish(&configuration)
            .map_err(publication_error)?;
        self.write_state(&PublicationState {
            api_version: STATE_API_VERSION.into(),
            deployment: self.plan.deployment.clone(),
            definition_hash: self.plan.definition_hash.clone(),
            record: record.clone(),
        })?;
        Ok(TailscaleStatus {
            configured: true,
            checks: record.checks.clone(),
            record: Some(record),
        })
    }

    pub fn status(&self) -> Result<TailscaleStatus, TailscaleError> {
        let Some(configuration) = self.configuration()? else {
            return Ok(TailscaleStatus {
                configured: false,
                record: None,
                checks: Vec::new(),
            });
        };
        let current = TailscalePublicationAdapter::default().inspect(&configuration);
        let state = self.read_state();
        match current {
            Ok(record) => {
                let mut checks = record.checks.clone();
                match state {
                    Ok(Some(state)) if state.api_version == STATE_API_VERSION
                        && state.deployment == self.plan.deployment
                        && state.definition_hash == self.plan.definition_hash
                        && state.record == record => {}
                    Ok(Some(_)) => checks.push(stale_check("persisted reachability differs from current tailnet or deployment state; run `switchyard up` to refresh it")),
                    Ok(None) => checks.push(stale_check("no persisted publication record exists; run `switchyard up` after the gateway is ready")),
                    Err(error) => checks.push(stale_check(error.to_string())),
                }
                Ok(TailscaleStatus {
                    configured: true,
                    record: Some(record),
                    checks,
                })
            }
            Err(diagnostics) => Ok(TailscaleStatus {
                configured: true,
                record: None,
                checks: diagnostic_checks(&diagnostics),
            }),
        }
    }

    pub fn stop(&self) -> Result<(), TailscaleError> {
        let Some(state) = self.read_state()? else {
            return Ok(());
        };
        self.validate_state(&state)?;
        let path = self.state_path(false)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata)
                if metadata.file_type().is_file() && !metadata.file_type().is_symlink() =>
            {
                fs::remove_file(path).map_err(Into::into)
            }
            Ok(_) => Err(TailscaleError::StaleState(
                "publication state must be a regular file".into(),
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    fn configuration(&self) -> Result<Option<serde_json::Value>, TailscaleError> {
        let Some(encoded) = &self.plan.host_router_config else {
            return Ok(None);
        };
        let config: RouterConfig = serde_json::from_str(encoded)
            .map_err(|error| TailscaleError::InvalidPlan(error.to_string()))?;
        let requested = config.spec.exposure.as_ref().is_some_and(|exposure| {
            exposure.publish_tailscale
                && exposure.mode == GatewayExposureMode::Lan
                && exposure.acknowledge_lan_exposure_risk
        });
        if !requested {
            return Ok(None);
        }
        let interfaces = local_ip_address::list_afinet_netifas()
            .map_err(|error| TailscaleError::InvalidPlan(error.to_string()))?
            .into_iter()
            .map(|(_, address)| address)
            .collect::<Vec<_>>();
        let summary = config.spec.exposure_summary(&interfaces);
        let addresses = summary
            .exposed_addresses
            .iter()
            .map(|address| address.ip().to_string())
            .collect::<BTreeSet<_>>();
        let ports = summary
            .exposed_addresses
            .iter()
            .map(|address| address.port())
            .collect::<BTreeSet<_>>();
        Ok(Some(
            json!({ "exposedAddresses": addresses, "ports": ports }),
        ))
    }

    fn state_path(&self, create: bool) -> Result<PathBuf, TailscaleError> {
        let root = fs::canonicalize(self.workspace_root)?;
        let mut current = root.clone();
        for component in [".switchyard", "run", self.plan.deployment.as_str()] {
            current.push(component);
            match fs::symlink_metadata(&current) {
                Ok(metadata)
                    if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {}
                Ok(_) => {
                    return Err(TailscaleError::StaleState(format!(
                        "{} must be a real directory",
                        current.display()
                    )));
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound && create => {
                    fs::create_dir(&current)?
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => break,
                Err(error) => return Err(error.into()),
            }
        }
        let run_dir = root.join(".switchyard/run").join(&self.plan.deployment);
        if create {
            fs::set_permissions(&run_dir, fs::Permissions::from_mode(0o700))?;
        }
        let path = run_dir.join("tailscale-publication.json");
        if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(TailscaleError::StaleState(
                "publication state must not be a symbolic link".into(),
            ));
        }
        Ok(path)
    }

    fn read_state(&self) -> Result<Option<PublicationState>, TailscaleError> {
        let path = self.state_path(false)?;
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if !metadata.file_type().is_file() || metadata.permissions().mode() & 0o077 != 0 {
            return Err(TailscaleError::StaleState(
                "publication state must be a regular owner-only file".into(),
            ));
        }
        serde_json::from_slice(&fs::read(path)?)
            .map(Some)
            .map_err(|error| TailscaleError::StaleState(error.to_string()))
    }

    fn write_state(&self, state: &PublicationState) -> Result<(), TailscaleError> {
        if let Some(existing) = self.read_state()? {
            self.validate_state(&existing)?;
        }
        let path = self.state_path(true)?;
        let temporary = path.with_extension(format!("json.tmp.{}", std::process::id()));
        let bytes = serde_json::to_vec_pretty(state)
            .map_err(|error| TailscaleError::Publication(error.to_string()))?;
        let result = (|| {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)?;
            use std::io::Write;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, &path)?;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result.map_err(Into::into)
    }

    fn validate_state(&self, state: &PublicationState) -> Result<(), TailscaleError> {
        if state.api_version != STATE_API_VERSION || state.deployment != self.plan.deployment {
            return Err(TailscaleError::StaleState(
                "state ownership does not match this deployment; refusing to replace or remove it"
                    .into(),
            ));
        }
        Ok(())
    }
}

fn publication_error(diagnostics: Vec<Diagnostic>) -> TailscaleError {
    TailscaleError::Publication(
        diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>()
            .join("; "),
    )
}

fn diagnostic_checks(diagnostics: &[Diagnostic]) -> Vec<PublicationCheck> {
    diagnostics
        .iter()
        .map(|diagnostic| PublicationCheck {
            name: diagnostic.code.clone(),
            outcome: PublicationCheckOutcome::Fail,
            detail: diagnostic.message.clone(),
        })
        .collect()
}

fn stale_check(detail: impl Into<String>) -> PublicationCheck {
    PublicationCheck {
        name: "persisted-state".into(),
        outcome: PublicationCheckOutcome::Warn,
        detail: detail.into(),
    }
}
