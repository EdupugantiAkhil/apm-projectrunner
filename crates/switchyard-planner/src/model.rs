use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};

pub const API_VERSION: &str = "switchyard.dev/v1alpha1";
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Metadata {
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploymentSpec {
    #[serde(default)]
    pub sources: BTreeMap<String, Source>,
    #[serde(default)]
    pub blocks: BTreeMap<String, Block>,
    #[serde(default)]
    pub instances: Vec<Instance>,
    #[serde(default)]
    pub groups: BTreeMap<String, ServiceGroup>,
    #[serde(default)]
    pub bindings: BTreeMap<String, String>,
    #[serde(default)]
    pub routes: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(default = "default_router_image")]
    pub router_image: String,
}

fn default_router_image() -> String {
    "switchyard-router:local".into()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Source {
    #[serde(default)]
    pub r#type: SourceType,
    pub path: PathBuf,
    #[serde(default)]
    pub repository: Option<PathBuf>,
    #[serde(default)]
    pub r#ref: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceType {
    #[default]
    Path,
    Worktree,
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
    pub provides: BTreeMap<String, Capability>,
    #[serde(default)]
    pub consumes: BTreeMap<String, RouteSlot>,
    #[serde(default)]
    pub publish: Vec<u16>,
    #[serde(default)]
    pub volumes: Vec<VolumeMount>,
    #[serde(default)]
    pub depends_on: BTreeMap<String, DependencyCondition>,
    #[serde(default)]
    pub hooks: LifecycleHooks,
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
pub struct Capability {
    #[serde(default)]
    pub protocol: Protocol,
    pub port: u16,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteSlot {
    #[serde(default)]
    pub protocol: Protocol,
    pub address: ListenAddress,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListenAddress {
    #[serde(default = "default_loopback")]
    pub host: String,
    pub port: u16,
}

fn default_loopback() -> String {
    "127.0.0.1".into()
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    #[default]
    Http,
    Https,
    Websocket,
    Grpc,
    Tcp,
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

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LifecycleHooks {
    #[serde(default)]
    pub prepare: Vec<Vec<String>>,
    #[serde(default)]
    pub post_ready: Vec<Vec<String>>,
    #[serde(default)]
    pub stop: Vec<Vec<String>>,
    #[serde(default)]
    pub cleanup: Vec<Vec<String>>,
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
    #[serde(default)]
    pub parameters: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceGroup {
    #[serde(default)]
    pub extends: Option<String>,
    #[serde(default)]
    pub providers: BTreeMap<String, String>,
}
