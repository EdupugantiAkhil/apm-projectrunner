use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

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
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub block: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    /// Hostname or IP address of an instance Switchyard routes to but never starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<String>,
    /// Port-for-port routes exposed by an external instance.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<ExternalPort>,
    /// Optional reachability check performed during `up`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe: Option<Probe>,
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

impl Instance {
    pub fn is_external(&self) -> bool {
        self.external.is_some()
    }

    pub fn expanded_external_ports(&self) -> Vec<u16> {
        self.ports.iter().flat_map(|port| port.expanded()).collect()
    }
}

/// One external port or inclusive range. Ranges serialize as quoted YAML strings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalPort {
    Single(u16),
    Range { start: u16, end: u16 },
}

impl ExternalPort {
    pub fn expanded(self) -> std::ops::RangeInclusive<u16> {
        match self {
            Self::Single(port) => port..=port,
            Self::Range { start, end } => start..=end,
        }
    }
}

impl Serialize for ExternalPort {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Single(port) => serializer.serialize_u16(*port),
            Self::Range { start, end } => serializer.serialize_str(&format!("{start}-{end}")),
        }
    }
}

impl<'de> Deserialize<'de> for ExternalPort {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> de::Visitor<'de> for Visitor {
            type Value = ExternalPort;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a nonzero port integer or an inclusive \"start-end\" range")
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                let port = u16::try_from(value)
                    .map_err(|_| E::custom("port must be between 1 and 65535"))?;
                Ok(ExternalPort::Single(port))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                let value = u64::try_from(value)
                    .map_err(|_| E::custom("port must be between 1 and 65535"))?;
                self.visit_u64(value)
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                let (start, end) = value
                    .split_once('-')
                    .ok_or_else(|| E::custom("port range must use \"start-end\""))?;
                if start.is_empty() || end.is_empty() || end.contains('-') {
                    return Err(E::custom("port range must use \"start-end\""));
                }
                let start = start
                    .parse::<u16>()
                    .map_err(|_| E::custom("range start must be between 1 and 65535"))?;
                let end = end
                    .parse::<u16>()
                    .map_err(|_| E::custom("range end must be between 1 and 65535"))?;
                Ok(ExternalPort::Range { start, end })
            }
        }

        deserializer.deserialize_any(Visitor)
    }
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

/// Declares an instance which may be opened in an isolated managed browser profile.
/// Any authored instance may have one; nothing requires it to be a user interface.
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
