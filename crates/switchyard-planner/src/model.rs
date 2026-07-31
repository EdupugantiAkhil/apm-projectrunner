use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};

pub const API_VERSION: &str = "switchyard.dev/v1alpha2";
pub const KIND: &str = "Deployment";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Bundle {
    pub api_version: String,
    pub kind: String,
    pub metadata: Metadata,
    pub spec: DeploymentSpec,
    #[serde(skip)]
    pub(crate) definition_dir: PathBuf,
    #[serde(skip)]
    pub(crate) workspace_root: PathBuf,
}

impl Bundle {
    pub fn definition_dir(&self) -> &std::path::Path {
        &self.definition_dir
    }

    pub fn workspace_root(&self) -> &std::path::Path {
        &self.workspace_root
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Metadata {
    pub name: String,
    /// Labels used by deployment-level overlay selectors.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub labels: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploymentSpec {
    /// Ordered deployment-relative overlay documents.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overlays: Vec<PathBuf>,
    /// Secret-safe injected-file metadata emitted in resolved deployment artifacts.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resolved_overlay_files: BTreeMap<String, Vec<ResolvedOverlayFile>>,
    #[serde(default)]
    pub repositories: BTreeMap<String, Repository>,
    #[serde(default)]
    pub sources: BTreeMap<String, Source>,
    #[serde(default)]
    pub blocks: BTreeMap<String, Block>,
    #[serde(default)]
    pub instances: Vec<Instance>,
    #[serde(default)]
    pub groups: BTreeMap<String, ServiceGroup>,
    #[serde(default)]
    pub managed_profiles: BTreeMap<String, ManagedProfile>,
    #[serde(default)]
    pub host_router: Option<router_config::RouterConfig>,
    #[serde(default)]
    pub host_upstreams: BTreeMap<String, PublishedUpstream>,
    #[serde(default = "default_router_image")]
    pub router_image: String,
}

/// Secret-safe identity of a file resolved from an overlay.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedOverlayFile {
    pub target: PathBuf,
    pub content_hash: String,
    pub mode: String,
    pub origin: String,
}

fn default_router_image() -> String {
    "switchyard-router:local".into()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Repository {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clone: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Source {
    pub repository: String,
    pub r#ref: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Block {
    #[serde(default)]
    pub parameters: BTreeMap<String, Parameter>,
    pub services: BTreeMap<String, Service>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Parameter {
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Service {
    pub execution: Execution,
    #[serde(default)]
    pub publish: Vec<u16>,
    #[serde(default)]
    pub volumes: Vec<VolumeMount>,
    #[serde(default)]
    pub depends_on: BTreeMap<String, DependencyCondition>,
    #[serde(default)]
    pub probe: Option<Probe>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Execution {
    Container {
        #[serde(default)]
        image: Option<String>,
        #[serde(default)]
        build: Option<Build>,
        #[serde(default)]
        command: Vec<String>,
        #[serde(default)]
        working_directory: Option<PathBuf>,
        #[serde(default)]
        environment: BTreeMap<String, String>,
    },
    Script {
        image: String,
        command: Vec<String>,
        #[serde(default)]
        working_directory: Option<PathBuf>,
        #[serde(default = "default_source_mount")]
        source_mount: PathBuf,
        #[serde(default)]
        writable: bool,
        #[serde(default)]
        environment: BTreeMap<String, String>,
        #[serde(default)]
        lifecycle: ScriptLifecycle,
    },
    ProcessCompose {
        image: String,
        file: PathBuf,
        #[serde(default)]
        working_directory: Option<PathBuf>,
        #[serde(default = "default_source_mount")]
        source_mount: PathBuf,
        #[serde(default)]
        writable: bool,
        #[serde(default)]
        environment: BTreeMap<String, String>,
    },
}

fn default_source_mount() -> PathBuf {
    PathBuf::from("/workspace")
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ScriptLifecycle {
    #[default]
    Service,
    Task,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Build {
    pub context: PathBuf,
    #[serde(default)]
    pub dockerfile: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VolumeMount {
    pub name: String,
    pub target: PathBuf,
    #[serde(default)]
    pub read_only: bool,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyCondition {
    Started,
    #[default]
    Healthy,
    CompletedSuccessfully,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Probe {
    Http {
        path: String,
        port: u16,
        #[serde(default)]
        https: bool,
    },
    Tcp {
        port: u16,
    },
    Command {
        command: Vec<String>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Instance {
    pub name: String,
    pub block: String,
    pub source: String,
    /// Execution placement. `local` is the only supported device in this release.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(default)]
    pub parameters: BTreeMap<String, String>,
    /// Labels used by instance overlay selectors.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub labels: BTreeMap<String, String>,
    /// Instance-wide environment values, applied after deployment overlays.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub environment: BTreeMap<String, String>,
    /// Environment keys removed after inherited service defaults are applied.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub environment_unset: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceGroup {
    #[serde(default)]
    pub instances: Vec<String>,
    /// Members excluded only from this group's routing while their authored
    /// priority position and running instance are preserved.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
}

/// Declares a UI which may be opened in an isolated managed browser profile.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedProfile {
    /// Explicit route identity supplied by this profile's dedicated proxy listener.
    pub route: String,
    /// Initial page opened by the managed browser.
    pub start_url: String,
}

/// Resolves one host-router provider from a dynamically published Compose port.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublishedUpstream {
    pub instance: String,
    pub service: String,
    pub port: u16,
}
