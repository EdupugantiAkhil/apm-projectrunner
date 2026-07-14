use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use serde::Deserialize;
use serde_json::Value;

pub const MANAGED_LABEL: &str = "dev.switchyard.managed";
pub const DEPLOYMENT_LABEL: &str = "dev.switchyard.deployment";
pub const RESOURCE_HASH_LABEL: &str = "dev.switchyard.resource-hash";

#[derive(Debug)]
pub enum RuntimeError {
    Io(io::Error),
    Docker { command: String, detail: String },
    InvalidDockerResponse(String),
    UnsafeCleanup(String),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::Docker { command, detail } => {
                write!(formatter, "`{command}` failed: {detail}")
            }
            Self::InvalidDockerResponse(message) => {
                write!(
                    formatter,
                    "Docker returned invalid inspection data: {message}"
                )
            }
            Self::UnsafeCleanup(message) => write!(formatter, "cleanup refused: {message}"),
        }
    }
}

impl std::error::Error for RuntimeError {}

impl From<io::Error> for RuntimeError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
}

pub trait CommandExecutor {
    fn capture(&self, program: &str, arguments: &[String]) -> Result<CommandOutput, RuntimeError>;
    fn stream(&self, program: &str, arguments: &[String]) -> Result<(), RuntimeError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemExecutor;

impl CommandExecutor for SystemExecutor {
    fn capture(&self, program: &str, arguments: &[String]) -> Result<CommandOutput, RuntimeError> {
        let output = prepared_command(program, arguments).output()?;
        if !output.status.success() {
            return Err(command_error(program, arguments, &output.stderr));
        }
        Ok(CommandOutput {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    fn stream(&self, program: &str, arguments: &[String]) -> Result<(), RuntimeError> {
        let status = prepared_command(program, arguments)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()?;
        if !status.success() {
            return Err(RuntimeError::Docker {
                command: render_command(program, arguments),
                detail: status.to_string(),
            });
        }
        Ok(())
    }
}

fn prepared_command(program: &str, arguments: &[String]) -> Command {
    let mut command = Command::new(program);
    command.args(arguments);
    if program == "docker" {
        if std::env::var_os("SWITCHYARD_ROUTER_TOKEN").is_none() {
            // Compose expands required variables even for read-only and stop commands.
            // `up` separately rejects this placeholder before starting sidecars.
            command.env("SWITCHYARD_ROUTER_TOKEN", "unused-for-compose-parsing");
        }
        if std::env::var_os("SWITCHYARD_UID").is_none() {
            if let Some(uid) = host_id("-u") {
                command.env("SWITCHYARD_UID", uid);
            }
        }
        if std::env::var_os("SWITCHYARD_GID").is_none() {
            if let Some(gid) = host_id("-g") {
                command.env("SWITCHYARD_GID", gid);
            }
        }
    }
    command
}

fn host_id(flag: &str) -> Option<String> {
    let output = Command::new("id").arg(flag).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn command_error(program: &str, arguments: &[String], stderr: &[u8]) -> RuntimeError {
    RuntimeError::Docker {
        command: render_command(program, arguments),
        detail: String::from_utf8_lossy(stderr).trim().to_owned(),
    }
}

fn render_command(program: &str, arguments: &[String]) -> String {
    std::iter::once(program)
        .chain(arguments.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Clone, Debug)]
pub struct RuntimePlan {
    pub deployment: String,
    pub compose_project: String,
    pub project_directory: PathBuf,
    pub artifact_dir: PathBuf,
    pub requires_router_token: bool,
}

impl RuntimePlan {
    pub fn compose_path(&self) -> PathBuf {
        self.artifact_dir.join("compose.yaml")
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.artifact_dir.join("manifest.json")
    }
}

pub struct DockerRuntime<E = SystemExecutor> {
    executor: E,
}

impl Default for DockerRuntime<SystemExecutor> {
    fn default() -> Self {
        Self {
            executor: SystemExecutor,
        }
    }
}

impl<E: CommandExecutor> DockerRuntime<E> {
    #[cfg(test)]
    pub fn new(executor: E) -> Self {
        Self { executor }
    }

    pub fn up(&self, plan: &RuntimePlan) -> Result<(), RuntimeError> {
        if plan.requires_router_token && std::env::var_os("SWITCHYARD_ROUTER_TOKEN").is_none() {
            return Err(RuntimeError::Docker {
                command: "docker compose up".into(),
                detail: "SWITCHYARD_ROUTER_TOKEN must be set".into(),
            });
        }
        self.executor.stream(
            "docker",
            &compose_arguments(plan, &["--progress", "plain", "build"]),
        )?;
        self.executor.stream(
            "docker",
            &compose_arguments(plan, &["up", "--detach", "--wait", "--remove-orphans"]),
        )
    }

    pub fn logs(&self, plan: &RuntimePlan, services: &[String]) -> Result<(), RuntimeError> {
        let mut arguments = compose_arguments(plan, &["logs", "--follow"]);
        arguments.extend(services.iter().cloned());
        self.executor.stream("docker", &arguments)
    }

    pub fn down(&self, plan: &RuntimePlan) -> Result<(), RuntimeError> {
        verify_ownership(
            &plan.deployment,
            &self.discover_compose_project(&plan.deployment, &plan.compose_project)?,
        )?;
        self.executor.stream(
            "docker",
            &compose_arguments(plan, &["down", "--remove-orphans"]),
        )
    }

    pub fn cleanup(&self, plan: &RuntimePlan, confirmed: bool) -> Result<(), RuntimeError> {
        if !confirmed {
            return Err(RuntimeError::UnsafeCleanup(format!(
                "pass --yes to delete persistent volumes owned by `{}`",
                plan.deployment
            )));
        }
        let resources = self.discover_compose_project(&plan.deployment, &plan.compose_project)?;
        verify_ownership(&plan.deployment, &resources)?;
        self.executor.stream(
            "docker",
            &compose_arguments(plan, &["down", "--volumes", "--remove-orphans"]),
        )
    }

    pub fn discover(&self, deployment: &str) -> Result<Vec<OwnedResource>, RuntimeError> {
        self.discover_filtered(&format!("{DEPLOYMENT_LABEL}={deployment}"))
    }

    fn discover_compose_project(
        &self,
        deployment: &str,
        compose_project: &str,
    ) -> Result<Vec<OwnedResource>, RuntimeError> {
        let mut resources = self.discover(deployment)?;
        resources.extend(
            self.discover_filtered(&format!("com.docker.compose.project={}", compose_project))?,
        );
        resources.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.id.cmp(&right.id))
        });
        resources.dedup_by(|left, right| left.kind == right.kind && left.id == right.id);
        Ok(resources)
    }

    fn discover_filtered(&self, label_filter: &str) -> Result<Vec<OwnedResource>, RuntimeError> {
        let mut resources = Vec::new();
        for kind in ResourceKind::ALL {
            let mut arguments = kind.list_arguments();
            arguments.extend([
                "--filter".to_owned(),
                format!("label={label_filter}"),
                "--quiet".to_owned(),
            ]);
            let ids = self.executor.capture("docker", &arguments)?.stdout;
            for id in ids.lines().map(str::trim).filter(|id| !id.is_empty()) {
                let inspected = self.executor.capture(
                    "docker",
                    &[
                        kind.inspect_noun().to_owned(),
                        "inspect".to_owned(),
                        id.to_owned(),
                    ],
                )?;
                resources.push(parse_inspection(*kind, id, &inspected.stdout)?);
            }
        }
        resources.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.name.cmp(&right.name))
        });
        Ok(resources)
    }

    pub fn status(&self, plan: &RuntimePlan) -> Result<DeploymentStatus, RuntimeError> {
        let resources = self.discover_compose_project(&plan.deployment, &plan.compose_project)?;
        let expected_hash = read_resource_hash(&plan.manifest_path())?;
        Ok(DeploymentStatus::from_observation(
            plan.deployment.clone(),
            expected_hash,
            resources,
        ))
    }
}

fn compose_arguments(plan: &RuntimePlan, command: &[&str]) -> Vec<String> {
    let mut arguments = vec![
        "compose".to_owned(),
        "--project-name".to_owned(),
        plan.compose_project.clone(),
        "--project-directory".to_owned(),
        plan.project_directory.display().to_string(),
        "--file".to_owned(),
        plan.compose_path().display().to_string(),
    ];
    arguments.extend(command.iter().map(|argument| (*argument).to_owned()));
    arguments
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ResourceKind {
    Container,
    Network,
    Volume,
}

impl ResourceKind {
    const ALL: &'static [Self] = &[Self::Container, Self::Network, Self::Volume];

    fn list_arguments(self) -> Vec<String> {
        match self {
            Self::Container => vec!["container".into(), "list".into(), "--all".into()],
            Self::Network => vec!["network".into(), "list".into()],
            Self::Volume => vec!["volume".into(), "list".into()],
        }
    }

    fn inspect_noun(self) -> &'static str {
        match self {
            Self::Container => "container",
            Self::Network => "network",
            Self::Volume => "volume",
        }
    }
}

impl fmt::Display for ResourceKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.inspect_noun())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnedResource {
    pub kind: ResourceKind,
    pub id: String,
    pub name: String,
    pub labels: BTreeMap<String, String>,
    pub state: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Inspection {
    id: Option<String>,
    name: Option<String>,
    #[serde(default)]
    config: InspectionConfig,
    #[serde(default)]
    labels: BTreeMap<String, String>,
    state: Option<InspectionState>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct InspectionConfig {
    #[serde(default)]
    labels: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct InspectionState {
    status: Option<String>,
    health: Option<InspectionHealth>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct InspectionHealth {
    status: Option<String>,
}

fn parse_inspection(
    kind: ResourceKind,
    fallback_id: &str,
    response: &str,
) -> Result<OwnedResource, RuntimeError> {
    let mut inspections: Vec<Inspection> = serde_json::from_str(response)
        .map_err(|error| RuntimeError::InvalidDockerResponse(error.to_string()))?;
    if inspections.len() != 1 {
        return Err(RuntimeError::InvalidDockerResponse(format!(
            "expected one {kind}, got {}",
            inspections.len()
        )));
    }
    let inspection = inspections.pop().expect("length checked");
    let labels = if inspection.config.labels.is_empty() {
        inspection.labels
    } else {
        inspection.config.labels
    };
    let state = inspection.state.and_then(|state| {
        state
            .health
            .and_then(|health| health.status)
            .or(state.status)
    });
    Ok(OwnedResource {
        kind,
        id: inspection.id.unwrap_or_else(|| fallback_id.to_owned()),
        name: inspection
            .name
            .unwrap_or_else(|| fallback_id.to_owned())
            .trim_start_matches('/')
            .to_owned(),
        labels,
        state,
    })
}

pub fn verify_ownership(deployment: &str, resources: &[OwnedResource]) -> Result<(), RuntimeError> {
    for resource in resources {
        if resource.labels.get(MANAGED_LABEL).map(String::as_str) != Some("true")
            || resource.labels.get(DEPLOYMENT_LABEL).map(String::as_str) != Some(deployment)
        {
            return Err(RuntimeError::UnsafeCleanup(format!(
                "{} `{}` does not carry matching Switchyard ownership labels",
                resource.kind, resource.name
            )));
        }
    }
    Ok(())
}

fn read_resource_hash(path: &Path) -> Result<Option<String>, RuntimeError> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let manifest: Value = serde_json::from_str(&contents)
        .map_err(|error| RuntimeError::InvalidDockerResponse(error.to_string()))?;
    Ok(find_string(&manifest, &["resourceHash", "resource_hash"]))
}

fn find_string(value: &Value, keys: &[&str]) -> Option<String> {
    let object = value.as_object()?;
    for key in keys {
        if let Some(value) = object.get(*key).and_then(Value::as_str) {
            return Some(value.to_owned());
        }
    }
    for nested in object.values() {
        if let Some(value) = find_string(nested, keys) {
            return Some(value);
        }
    }
    None
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DriftState {
    NotRunning,
    InSync,
    Drifted,
    Unknown,
}

#[derive(Clone, Debug)]
pub struct DeploymentStatus {
    pub deployment: String,
    pub drift: DriftState,
    pub detail: String,
    pub resources: Vec<OwnedResource>,
}

impl DeploymentStatus {
    fn from_observation(
        deployment: String,
        expected_hash: Option<String>,
        resources: Vec<OwnedResource>,
    ) -> Self {
        if resources.is_empty() {
            return Self {
                deployment,
                drift: DriftState::NotRunning,
                detail: "no labeled Docker resources found".into(),
                resources,
            };
        }
        if let Err(error) = verify_ownership(&deployment, &resources) {
            return Self {
                deployment,
                drift: DriftState::Drifted,
                detail: error.to_string(),
                resources,
            };
        }
        let topology_resources = resources
            .iter()
            .filter(|resource| resource.kind != ResourceKind::Volume)
            .collect::<Vec<_>>();
        let hash_resources: Vec<_> = if topology_resources.is_empty() {
            resources.iter().collect()
        } else {
            topology_resources
        };
        let observed_hashes = hash_resources
            .into_iter()
            .filter_map(|resource| resource.labels.get(RESOURCE_HASH_LABEL).cloned())
            .collect::<BTreeSet<_>>();
        let (drift, detail) = match (expected_hash, observed_hashes.len()) {
            (None, _) => (
                DriftState::Unknown,
                "generated manifest is missing; observed resources were not changed".into(),
            ),
            (Some(_), 0) => (
                DriftState::Unknown,
                "resources lack a definition hash; observed resources were not changed".into(),
            ),
            (Some(expected), 1) if observed_hashes.contains(&expected) => (
                DriftState::InSync,
                "manifest and runtime hashes match".into(),
            ),
            (Some(expected), _) => (
                DriftState::Drifted,
                format!(
                    "manifest hash {expected} differs from runtime hash(es): {}",
                    observed_hashes.into_iter().collect::<Vec<_>>().join(", ")
                ),
            ),
        };
        Self {
            deployment,
            drift,
            detail,
            resources,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeExecutor {
        calls: Mutex<Vec<(String, Vec<String>)>>,
    }

    impl CommandExecutor for FakeExecutor {
        fn capture(
            &self,
            program: &str,
            arguments: &[String],
        ) -> Result<CommandOutput, RuntimeError> {
            self.calls
                .lock()
                .unwrap()
                .push((program.into(), arguments.to_vec()));
            Ok(CommandOutput {
                stdout: String::new(),
                stderr: String::new(),
            })
        }

        fn stream(&self, program: &str, arguments: &[String]) -> Result<(), RuntimeError> {
            self.calls
                .lock()
                .unwrap()
                .push((program.into(), arguments.to_vec()));
            Ok(())
        }
    }

    fn plan() -> RuntimePlan {
        RuntimePlan {
            deployment: "demo".into(),
            compose_project: "sy--demo".into(),
            project_directory: PathBuf::from("/tmp"),
            artifact_dir: PathBuf::from("/tmp/generated/demo"),
            requires_router_token: false,
        }
    }

    #[test]
    fn up_builds_then_waits_for_health() {
        let runtime = DockerRuntime::new(FakeExecutor::default());
        runtime.up(&plan()).unwrap();
        let calls = runtime.executor.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert!(
            calls[0]
                .1
                .ends_with(&["--progress".into(), "plain".into(), "build".into()])
        );
        assert!(calls[1].1.ends_with(&[
            "up".into(),
            "--detach".into(),
            "--wait".into(),
            "--remove-orphans".into()
        ]));
    }

    #[test]
    fn down_does_not_delete_volumes() {
        let runtime = DockerRuntime::new(FakeExecutor::default());
        runtime.down(&plan()).unwrap();
        let calls = runtime.executor.calls.lock().unwrap();
        let down = calls.last().unwrap();
        assert!(down.1.iter().any(|argument| argument == "down"));
        assert!(!down.1.iter().any(|argument| argument == "--volumes"));
    }

    #[test]
    fn destructive_cleanup_requires_confirmation() {
        let runtime = DockerRuntime::new(FakeExecutor::default());
        let error = runtime.cleanup(&plan(), false).unwrap_err();
        assert!(error.to_string().contains("--yes"));
        assert!(runtime.executor.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn ownership_rejects_a_resource_without_managed_label() {
        let resource = OwnedResource {
            kind: ResourceKind::Volume,
            id: "id".into(),
            name: "data".into(),
            labels: BTreeMap::from([(DEPLOYMENT_LABEL.into(), "demo".into())]),
            state: None,
        };
        assert!(verify_ownership("demo", &[resource]).is_err());
    }

    #[test]
    fn parses_container_health_and_labels() {
        let response = r#"[{"Id":"abc","Name":"/demo-api","Config":{"Labels":{"dev.switchyard.managed":"true"}},"State":{"Status":"running","Health":{"Status":"healthy"}}}]"#;
        let resource = parse_inspection(ResourceKind::Container, "fallback", response).unwrap();
        assert_eq!(resource.name, "demo-api");
        assert_eq!(resource.state.as_deref(), Some("healthy"));
        assert_eq!(resource.labels[MANAGED_LABEL], "true");
    }

    #[test]
    fn preserved_volume_hash_does_not_make_running_topology_drift() {
        let labels = |hash: &str| {
            BTreeMap::from([
                (MANAGED_LABEL.into(), "true".into()),
                (DEPLOYMENT_LABEL.into(), "demo".into()),
                (RESOURCE_HASH_LABEL.into(), hash.into()),
            ])
        };
        let resource = |kind, hash: &str| OwnedResource {
            kind,
            id: format!("{kind}-{hash}"),
            name: format!("{kind}-{hash}"),
            labels: labels(hash),
            state: None,
        };
        let status = DeploymentStatus::from_observation(
            "demo".into(),
            Some("current".into()),
            vec![
                resource(ResourceKind::Container, "current"),
                resource(ResourceKind::Volume, "previous"),
            ],
        );
        assert_eq!(status.drift, DriftState::InSync);
    }
}
