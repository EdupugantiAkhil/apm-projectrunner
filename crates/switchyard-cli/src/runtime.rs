use std::io::Read;
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
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
    Docker {
        command: String,
        detail: String,
    },
    InvalidDockerResponse(String),
    UnsafeCleanup(String),
    Device {
        device: String,
        command: String,
        detail: String,
    },
    Teardown(Vec<RuntimeError>),
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
            Self::Device {
                device,
                command,
                detail,
            } => write!(formatter, "device `{device}`: `{command}` failed: {detail}"),
            Self::Teardown(errors) => {
                writeln!(
                    formatter,
                    "teardown failed for {} project(s):",
                    errors.len()
                )?;
                for error in errors {
                    writeln!(formatter, "- {error}")?;
                }
                Ok(())
            }
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
    fn capture_with_environment(
        &self,
        program: &str,
        arguments: &[String],
        _environment: &BTreeMap<String, OsString>,
    ) -> Result<CommandOutput, RuntimeError> {
        self.capture(program, arguments)
    }
    fn stream_with_environment(
        &self,
        program: &str,
        arguments: &[String],
        _environment: &BTreeMap<String, OsString>,
    ) -> Result<(), RuntimeError> {
        self.stream(program, arguments)
    }
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

    fn capture_with_environment(
        &self,
        program: &str,
        arguments: &[String],
        environment: &BTreeMap<String, OsString>,
    ) -> Result<CommandOutput, RuntimeError> {
        let output = prepared_command(program, arguments)
            .envs(environment)
            .output()?;
        if !output.status.success() {
            return Err(command_error(program, arguments, &output.stderr));
        }
        Ok(CommandOutput {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    fn stream_with_environment(
        &self,
        program: &str,
        arguments: &[String],
        environment: &BTreeMap<String, OsString>,
    ) -> Result<(), RuntimeError> {
        let mut child = prepared_command(program, arguments)
            .envs(environment)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| io::Error::other("could not capture command stderr"))?;
        let stderr_reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let mut buffer = [0_u8; 8192];
            loop {
                match stderr.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        eprint!("{}", String::from_utf8_lossy(&buffer[..count]));
                        bytes.extend_from_slice(&buffer[..count]);
                    }
                    Err(_) => break,
                }
            }
            bytes
        });
        let status = child.wait()?;
        let stderr = stderr_reader
            .join()
            .map_err(|_| io::Error::other("stderr reader panicked"))?;
        if !status.success() {
            let stderr = String::from_utf8_lossy(&stderr).trim().to_owned();
            return Err(RuntimeError::Docker {
                command: render_command(program, arguments),
                detail: if stderr.is_empty() {
                    status.to_string()
                } else {
                    stderr
                },
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
    pub runtime_secrets: Vec<switchyard_planner::RuntimeSecretPlan>,
    pub remote_projects: Vec<RemoteRuntimeProject>,
}

#[derive(Clone, Debug)]
pub struct RemoteRuntimeProject {
    pub name: String,
    pub user: String,
    pub host: String,
    pub port: u16,
    pub identity_file: Option<PathBuf>,
    pub compose_project: String,
    pub compose_file: PathBuf,
    pub services: Vec<String>,
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
        self.check_remote_eligibility(plan)?;
        for remote in &plan.remote_projects {
            self.up_project(plan, Some(remote))?;
        }
        self.up_project(plan, None)
    }

    /// Verifies every selected remote Docker daemon before any Compose mutation.
    pub fn check_remote_eligibility(&self, plan: &RuntimePlan) -> Result<(), RuntimeError> {
        for remote in &plan.remote_projects {
            let environment = project_environment(plan, Some(remote))?;
            let arguments = vec![
                "version".into(),
                "--format".into(),
                "{{.Server.Version}}".into(),
            ];
            self.executor
                .capture_with_environment("docker", &arguments, &environment)
                .map_err(|error| {
                    device_error(
                        remote,
                        "docker version --format '{{.Server.Version}}'",
                        error,
                    )
                })?;
        }
        Ok(())
    }

    pub fn logs(&self, plan: &RuntimePlan, services: &[String]) -> Result<(), RuntimeError> {
        for remote in &plan.remote_projects {
            let selected = services
                .iter()
                .filter(|service| remote.services.contains(service))
                .cloned()
                .collect::<Vec<_>>();
            if services.is_empty() || !selected.is_empty() {
                self.logs_project(plan, Some(remote), &selected)?;
            }
        }
        let remote_services = plan
            .remote_projects
            .iter()
            .flat_map(|remote| &remote.services)
            .collect::<BTreeSet<_>>();
        let local = services
            .iter()
            .filter(|service| !remote_services.contains(service))
            .cloned()
            .collect::<Vec<_>>();
        if services.is_empty() || !local.is_empty() {
            self.logs_project(plan, None, &local)?;
        }
        Ok(())
    }

    pub fn down(&self, plan: &RuntimePlan) -> Result<(), RuntimeError> {
        self.teardown_projects(plan, false)
    }

    pub fn cleanup(&self, plan: &RuntimePlan, confirmed: bool) -> Result<(), RuntimeError> {
        if !confirmed {
            return Err(RuntimeError::UnsafeCleanup(format!(
                "pass --yes to delete persistent volumes owned by `{}`",
                plan.deployment
            )));
        }
        self.teardown_projects(plan, true)
    }

    pub fn discover(&self, deployment: &str) -> Result<Vec<OwnedResource>, RuntimeError> {
        self.discover_filtered(
            &format!("{DEPLOYMENT_LABEL}={deployment}"),
            &BTreeMap::new(),
            "local",
        )
    }

    fn discover_compose_project(
        &self,
        deployment: &str,
        compose_project: &str,
        environment: &BTreeMap<String, OsString>,
        device: &str,
    ) -> Result<Vec<OwnedResource>, RuntimeError> {
        let mut resources = self.discover_filtered(
            &format!("{DEPLOYMENT_LABEL}={deployment}"),
            environment,
            device,
        )?;
        resources.extend(self.discover_filtered(
            &format!("com.docker.compose.project={compose_project}"),
            environment,
            device,
        )?);
        resources.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.id.cmp(&right.id))
        });
        resources.dedup_by(|left, right| left.kind == right.kind && left.id == right.id);
        Ok(resources)
    }

    fn discover_filtered(
        &self,
        label_filter: &str,
        environment: &BTreeMap<String, OsString>,
        device: &str,
    ) -> Result<Vec<OwnedResource>, RuntimeError> {
        let mut resources = Vec::new();
        for kind in ResourceKind::ALL {
            let mut arguments = kind.list_arguments();
            arguments.extend([
                "--filter".to_owned(),
                format!("label={label_filter}"),
                "--quiet".to_owned(),
            ]);
            let ids = self
                .executor
                .capture_with_environment("docker", &arguments, environment)?
                .stdout;
            for id in ids.lines().map(str::trim).filter(|id| !id.is_empty()) {
                let inspected = self.executor.capture_with_environment(
                    "docker",
                    &[
                        kind.inspect_noun().to_owned(),
                        "inspect".to_owned(),
                        id.to_owned(),
                    ],
                    environment,
                )?;
                let mut resource = parse_inspection(*kind, id, &inspected.stdout)?;
                resource.device = device.to_owned();
                resources.push(resource);
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
        let (resources, device_observations) = self.discover_plan(plan)?;
        let expected_hash = read_resource_hash(&plan.manifest_path())?;
        let mut status =
            DeploymentStatus::from_observation(plan.deployment.clone(), expected_hash, resources);
        status.device_observations = device_observations;
        if !status.device_observations.is_empty() {
            status.drift = DriftState::Unknown;
            status.detail = status
                .device_observations
                .iter()
                .map(|observation| {
                    format!(
                        "device `{}` unreachable: {}",
                        observation.device, observation.detail
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
        }
        Ok(status)
    }

    /// Discovers local and remote labeled resources, retaining unreachable devices as
    /// explicit observations instead of converting them into missing resources.
    pub fn discover_plan(
        &self,
        plan: &RuntimePlan,
    ) -> Result<(Vec<OwnedResource>, Vec<DeviceObservation>), RuntimeError> {
        let local_environment = project_environment(plan, None)?;
        let mut resources = self.discover_compose_project(
            &plan.deployment,
            &plan.compose_project,
            &local_environment,
            "local",
        )?;
        let mut device_observations = Vec::new();
        for remote in &plan.remote_projects {
            let environment = project_environment(plan, Some(remote))?;
            match self.discover_compose_project(
                &plan.deployment,
                &remote.compose_project,
                &environment,
                &remote.name,
            ) {
                Ok(mut observed) => resources.append(&mut observed),
                Err(error) => device_observations.push(DeviceObservation {
                    device: remote.name.clone(),
                    state: DeviceObservationState::Unreachable,
                    detail: error.to_string(),
                }),
            }
        }
        Ok((resources, device_observations))
    }

    fn up_project(
        &self,
        plan: &RuntimePlan,
        remote: Option<&RemoteRuntimeProject>,
    ) -> Result<(), RuntimeError> {
        let environment = project_environment(plan, remote)?;
        let (project, device) = project_identity(plan, remote);
        let resources = self
            .discover_compose_project(&plan.deployment, project, &environment, device)
            .map_err(|error| map_project_error(remote, "docker resource discovery", error))?;
        verify_ownership(&plan.deployment, &resources)?;
        for command in [
            &["--progress", "plain", "build"][..],
            &["up", "--detach", "--wait", "--remove-orphans"][..],
        ] {
            let arguments = compose_arguments(plan, remote, command);
            self.executor
                .stream_with_environment("docker", &arguments, &environment)
                .map_err(|error| {
                    map_project_error(remote, &render_command("docker", &arguments), error)
                })?;
        }
        Ok(())
    }

    fn down_project(
        &self,
        plan: &RuntimePlan,
        remote: Option<&RemoteRuntimeProject>,
        volumes: bool,
    ) -> Result<(), RuntimeError> {
        let environment = project_environment(plan, remote)
            .map_err(|error| map_project_error(remote, "prepare Docker environment", error))?;
        let (project, device) = project_identity(plan, remote);
        let resources = self
            .discover_compose_project(&plan.deployment, project, &environment, device)
            .map_err(|error| map_project_error(remote, "docker resource discovery", error))?;
        verify_ownership(&plan.deployment, &resources)
            .map_err(|error| map_project_error(remote, "verify resource ownership", error))?;
        let command = if volumes {
            &["down", "--volumes", "--remove-orphans"][..]
        } else {
            &["down", "--remove-orphans"][..]
        };
        let arguments = compose_arguments(plan, remote, command);
        self.executor
            .stream_with_environment("docker", &arguments, &environment)
            .map_err(|error| {
                map_project_error(remote, &render_command("docker", &arguments), error)
            })
    }

    fn teardown_projects(&self, plan: &RuntimePlan, volumes: bool) -> Result<(), RuntimeError> {
        let mut errors = Vec::new();
        if let Err(error) = self.down_project(plan, None, volumes) {
            errors.push(error);
        }
        for remote in plan.remote_projects.iter().rev() {
            if let Err(error) = self.down_project(plan, Some(remote), volumes) {
                errors.push(error);
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(RuntimeError::Teardown(errors))
        }
    }

    fn logs_project(
        &self,
        plan: &RuntimePlan,
        remote: Option<&RemoteRuntimeProject>,
        services: &[String],
    ) -> Result<(), RuntimeError> {
        let mut arguments = compose_arguments(plan, remote, &["logs", "--follow"]);
        arguments.extend(services.iter().cloned());
        self.executor
            .stream_with_environment("docker", &arguments, &project_environment(plan, remote)?)
            .map_err(|error| {
                map_project_error(remote, &render_command("docker", &arguments), error)
            })
    }
}

fn secret_environment(plan: &RuntimePlan) -> Result<BTreeMap<String, OsString>, RuntimeError> {
    let mut environment = BTreeMap::new();
    for secret in &plan.runtime_secrets {
        let value = match (
            &secret.reference.environment_variable,
            &secret.reference.file,
        ) {
            (Some(name), None) => std::env::var_os(name).ok_or_else(|| RuntimeError::Docker {
                command: "docker compose".into(),
                detail: format!("required overlay secret environment variable `{name}` is not set"),
            })?,
            (None, Some(path)) => {
                let mut bytes = fs::read(path)?;
                if bytes.last() == Some(&b'\n') {
                    bytes.pop();
                    if bytes.last() == Some(&b'\r') {
                        bytes.pop();
                    }
                }
                #[cfg(unix)]
                {
                    use std::os::unix::ffi::OsStringExt;
                    OsString::from_vec(bytes)
                }
                #[cfg(not(unix))]
                {
                    OsString::from(String::from_utf8(bytes).map_err(io::Error::other)?)
                }
            }
            _ => {
                return Err(RuntimeError::Docker {
                    command: "docker compose".into(),
                    detail: "invalid overlay secret reference".into(),
                });
            }
        };
        environment.insert(secret.variable.clone(), value);
    }
    Ok(environment)
}

fn project_environment(
    plan: &RuntimePlan,
    remote: Option<&RemoteRuntimeProject>,
) -> Result<BTreeMap<String, OsString>, RuntimeError> {
    let mut environment = secret_environment(plan)?;
    if let Some(remote) = remote {
        environment.insert(
            "DOCKER_HOST".into(),
            format!("ssh://{}@{}:{}", remote.user, remote.host, remote.port).into(),
        );
        let mut ssh_options = String::new();
        if let Some(identity) = &remote.identity_file {
            ssh_options.push_str(&format!("-i {} ", identity.display()));
        }
        ssh_options.push_str("-o BatchMode=yes");
        environment.insert("DOCKER_SSH_OPTS".into(), ssh_options.into());
    }
    Ok(environment)
}

fn project_identity<'a>(
    plan: &'a RuntimePlan,
    remote: Option<&'a RemoteRuntimeProject>,
) -> (&'a str, &'a str) {
    remote.map_or((&plan.compose_project, "local"), |remote| {
        (&remote.compose_project, &remote.name)
    })
}

fn map_project_error(
    remote: Option<&RemoteRuntimeProject>,
    command: &str,
    error: RuntimeError,
) -> RuntimeError {
    match remote {
        Some(remote) => device_error(remote, command, error),
        None => error,
    }
}

fn device_error(remote: &RemoteRuntimeProject, command: &str, error: RuntimeError) -> RuntimeError {
    let detail = match error {
        RuntimeError::Docker { detail, .. } | RuntimeError::Device { detail, .. } => detail,
        other => other.to_string(),
    };
    RuntimeError::Device {
        device: remote.name.clone(),
        command: command.into(),
        detail,
    }
}

fn compose_arguments(
    plan: &RuntimePlan,
    remote: Option<&RemoteRuntimeProject>,
    command: &[&str],
) -> Vec<String> {
    let (compose_project, compose_path) = remote.map_or_else(
        || (plan.compose_project.clone(), plan.compose_path()),
        |remote| {
            (
                remote.compose_project.clone(),
                plan.artifact_dir.join(&remote.compose_file),
            )
        },
    );
    let mut arguments = vec![
        "compose".to_owned(),
        "--project-name".to_owned(),
        compose_project,
        "--project-directory".to_owned(),
        plan.project_directory.display().to_string(),
        "--file".to_owned(),
        compose_path.display().to_string(),
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
    pub device: String,
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
        device: "local".into(),
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
    pub device_observations: Vec<DeviceObservation>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceObservationState {
    Unreachable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceObservation {
    pub device: String,
    pub state: DeviceObservationState,
    pub detail: String,
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
                device_observations: Vec::new(),
            };
        }
        if let Err(error) = verify_ownership(&deployment, &resources) {
            return Self {
                deployment,
                drift: DriftState::Drifted,
                detail: error.to_string(),
                resources,
                device_observations: Vec::new(),
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
            device_observations: Vec::new(),
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
            runtime_secrets: Vec::new(),
            remote_projects: Vec::new(),
        }
    }

    fn remote_plan() -> RuntimePlan {
        let mut plan = plan();
        plan.remote_projects.push(RemoteRuntimeProject {
            name: "builder".into(),
            user: "akhil".into(),
            host: "example-host".into(),
            port: 22,
            identity_file: Some("/keys/build".into()),
            compose_project: "sy--demo-builder".into(),
            compose_file: "compose.builder.yaml".into(),
            services: vec!["demo--provider--api".into()],
        });
        plan
    }

    #[derive(Clone, Debug)]
    struct EnvironmentCall {
        arguments: Vec<String>,
        environment: BTreeMap<String, OsString>,
        streamed: bool,
    }

    #[derive(Default)]
    struct EnvironmentExecutor {
        calls: Mutex<Vec<EnvironmentCall>>,
        fail_remote_version: bool,
        fail_remote_discovery: bool,
        fail_local_down: bool,
        fail_remote_down: bool,
        unowned_remote_network: bool,
    }

    impl CommandExecutor for EnvironmentExecutor {
        fn capture(
            &self,
            _program: &str,
            arguments: &[String],
        ) -> Result<CommandOutput, RuntimeError> {
            self.capture_with_environment("docker", arguments, &BTreeMap::new())
        }

        fn stream(&self, _program: &str, arguments: &[String]) -> Result<(), RuntimeError> {
            self.stream_with_environment("docker", arguments, &BTreeMap::new())
        }

        fn capture_with_environment(
            &self,
            _program: &str,
            arguments: &[String],
            environment: &BTreeMap<String, OsString>,
        ) -> Result<CommandOutput, RuntimeError> {
            self.calls.lock().unwrap().push(EnvironmentCall {
                arguments: arguments.to_vec(),
                environment: environment.clone(),
                streamed: false,
            });
            let remote = environment.contains_key("DOCKER_HOST");
            let project_discovery = arguments
                .iter()
                .any(|argument| argument.starts_with("label=com.docker.compose.project="));
            let remote_network_list = remote
                && self.unowned_remote_network
                && arguments.first().is_some_and(|arg| arg == "network")
                && arguments.get(1).is_some_and(|arg| arg == "list")
                && project_discovery;
            let remote_network_inspect = remote
                && self.unowned_remote_network
                && arguments.first().is_some_and(|arg| arg == "network")
                && arguments.get(1).is_some_and(|arg| arg == "inspect");
            if remote
                && ((self.fail_remote_version
                    && arguments.first().is_some_and(|arg| arg == "version"))
                    || (self.fail_remote_discovery
                        && arguments.get(1).is_some_and(|arg| arg == "list")))
            {
                return Err(RuntimeError::Docker {
                    command: render_command("docker", arguments),
                    detail: "ssh connection refused".into(),
                });
            }
            Ok(CommandOutput {
                stdout: if remote_network_list {
                    "remote-network-id\n".into()
                } else if remote_network_inspect {
                    r#"[{"Id":"remote-network-id","Name":"sy--demo-builder--private","Labels":{}}]"#
                        .into()
                } else if arguments.first().is_some_and(|arg| arg == "version") {
                    "27.0.0\n".into()
                } else {
                    String::new()
                },
                stderr: String::new(),
            })
        }

        fn stream_with_environment(
            &self,
            _program: &str,
            arguments: &[String],
            environment: &BTreeMap<String, OsString>,
        ) -> Result<(), RuntimeError> {
            self.calls.lock().unwrap().push(EnvironmentCall {
                arguments: arguments.to_vec(),
                environment: environment.clone(),
                streamed: true,
            });
            let remote = environment.contains_key("DOCKER_HOST");
            let down = arguments.iter().any(|argument| argument == "down");
            if down && ((!remote && self.fail_local_down) || (remote && self.fail_remote_down)) {
                return Err(RuntimeError::Docker {
                    command: render_command("docker", arguments),
                    detail: if remote {
                        "remote teardown failed".into()
                    } else {
                        "local teardown failed".into()
                    },
                });
            }
            Ok(())
        }
    }

    #[test]
    fn remote_up_uses_ssh_environment_and_precedes_local_up() {
        let runtime = DockerRuntime::new(EnvironmentExecutor::default());
        runtime.up(&remote_plan()).unwrap();
        let calls = runtime.executor.calls.lock().unwrap();
        let remote_streams = calls
            .iter()
            .filter(|call| call.streamed && call.environment.contains_key("DOCKER_HOST"))
            .collect::<Vec<_>>();
        assert_eq!(
            remote_streams[0].environment["DOCKER_HOST"],
            OsString::from("ssh://akhil@example-host:22")
        );
        assert_eq!(
            remote_streams[0].environment["DOCKER_SSH_OPTS"],
            OsString::from("-i /keys/build -o BatchMode=yes")
        );
        let remote_up = calls.iter().position(|call| {
            call.streamed
                && call.environment.contains_key("DOCKER_HOST")
                && call.arguments.iter().any(|arg| arg == "up")
        });
        let local_up = calls.iter().position(|call| {
            call.streamed
                && !call.environment.contains_key("DOCKER_HOST")
                && call.arguments.iter().any(|arg| arg == "up")
        });
        assert!(remote_up < local_up);
    }

    #[test]
    fn local_down_precedes_remote_down() {
        let runtime = DockerRuntime::new(EnvironmentExecutor::default());
        runtime.down(&remote_plan()).unwrap();
        let calls = runtime.executor.calls.lock().unwrap();
        let downs = calls
            .iter()
            .filter(|call| call.streamed && call.arguments.iter().any(|arg| arg == "down"))
            .collect::<Vec<_>>();
        assert_eq!(downs.len(), 2);
        assert!(!downs[0].environment.contains_key("DOCKER_HOST"));
        assert!(downs[1].environment.contains_key("DOCKER_HOST"));
    }

    #[test]
    fn down_runs_remote_teardown_after_local_failure() {
        let runtime = DockerRuntime::new(EnvironmentExecutor {
            fail_local_down: true,
            ..Default::default()
        });
        let error = runtime.down(&remote_plan()).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("local teardown failed"), "{message}");
        let calls = runtime.executor.calls.lock().unwrap();
        assert!(calls.iter().any(|call| {
            call.streamed
                && call.environment.contains_key("DOCKER_HOST")
                && call.arguments.iter().any(|argument| argument == "down")
        }));
    }

    #[test]
    fn cleanup_runs_local_and_all_remote_projects_and_aggregates_failures() {
        let mut plan = remote_plan();
        let mut second = plan.remote_projects[0].clone();
        second.name = "tester".into();
        second.host = "second-host".into();
        second.compose_project = "sy--demo-tester".into();
        second.compose_file = "compose.tester.yaml".into();
        plan.remote_projects.push(second);
        let runtime = DockerRuntime::new(EnvironmentExecutor {
            fail_remote_down: true,
            ..Default::default()
        });

        let error = runtime.cleanup(&plan, true).unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("teardown failed for 2 project(s)"),
            "{message}"
        );
        assert!(message.contains("device `builder`"), "{message}");
        assert!(message.contains("device `tester`"), "{message}");
        let calls = runtime.executor.calls.lock().unwrap();
        let downs = calls
            .iter()
            .filter(|call| call.streamed && call.arguments.iter().any(|arg| arg == "down"))
            .collect::<Vec<_>>();
        assert_eq!(downs.len(), 3);
        assert!(!downs[0].environment.contains_key("DOCKER_HOST"));
        assert!(downs.iter().all(|call| {
            call.arguments
                .iter()
                .any(|argument| argument == "--volumes")
        }));
    }

    #[test]
    fn remote_ownership_failure_names_device_and_resource() {
        let runtime = DockerRuntime::new(EnvironmentExecutor {
            unowned_remote_network: true,
            ..Default::default()
        });
        let error = runtime.down(&remote_plan()).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("device `builder`"), "{message}");
        assert!(
            message.contains("network `sy--demo-builder--private`"),
            "{message}"
        );
    }

    #[test]
    fn eligibility_failure_aborts_before_compose_mutation() {
        let runtime = DockerRuntime::new(EnvironmentExecutor {
            fail_remote_version: true,
            ..Default::default()
        });
        let error = runtime.up(&remote_plan()).unwrap_err();
        assert!(error.to_string().contains("device `builder`"));
        assert!(error.to_string().contains("ssh connection refused"));
        let calls = runtime.executor.calls.lock().unwrap();
        assert!(!calls.iter().any(|call| call.streamed));
    }

    #[test]
    fn unreachable_remote_status_is_explicit_and_not_missing() {
        let runtime = DockerRuntime::new(EnvironmentExecutor {
            fail_remote_discovery: true,
            ..Default::default()
        });
        let status = runtime.status(&remote_plan()).unwrap();
        assert_eq!(status.drift, DriftState::Unknown);
        assert_eq!(status.device_observations.len(), 1);
        assert_eq!(status.device_observations[0].device, "builder");
        assert_eq!(
            status.device_observations[0].state,
            DeviceObservationState::Unreachable
        );
        assert!(status.detail.contains("device `builder` unreachable"));
    }

    #[test]
    fn up_builds_then_waits_for_health() {
        let runtime = DockerRuntime::new(FakeExecutor::default());
        runtime.up(&plan()).unwrap();
        let calls = runtime.executor.calls.lock().unwrap();
        // Ownership-discovery captures precede the two compose invocations.
        assert!(calls.len() > 2);
        assert!(calls.iter().any(|(_, arguments)| {
            arguments
                .iter()
                .any(|argument| argument.starts_with("label=com.docker.compose.project="))
        }));
        let build = &calls[calls.len() - 2];
        assert!(
            build
                .1
                .ends_with(&["--progress".into(), "plain".into(), "build".into()])
        );
        assert!(calls[calls.len() - 1].1.ends_with(&[
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
            device: "local".into(),
        };
        assert!(verify_ownership("demo", &[resource]).is_err());
    }

    #[test]
    fn up_refuses_when_the_compose_project_contains_an_unowned_container() {
        struct OrphanExecutor {
            calls: Mutex<Vec<(String, Vec<String>)>>,
        }
        impl CommandExecutor for OrphanExecutor {
            fn capture(
                &self,
                program: &str,
                arguments: &[String],
            ) -> Result<CommandOutput, RuntimeError> {
                self.calls
                    .lock()
                    .unwrap()
                    .push((program.into(), arguments.to_vec()));
                let is_container_list = arguments.first().is_some_and(|noun| noun == "container")
                    && arguments.get(1).is_some_and(|verb| verb == "list");
                let matches_project = arguments
                    .iter()
                    .any(|argument| argument.starts_with("label=com.docker.compose.project="));
                let stdout = if is_container_list && matches_project {
                    "orphan1\n".into()
                } else if arguments.iter().any(|argument| argument == "inspect") {
                    // A container in the Compose project without Switchyard labels.
                    r#"[{"Id":"orphan1","Name":"/coincidental","Config":{"Labels":{}},"State":{"Status":"running"}}]"#.into()
                } else {
                    String::new()
                };
                Ok(CommandOutput {
                    stdout,
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
        let runtime = DockerRuntime::new(OrphanExecutor {
            calls: Mutex::new(Vec::new()),
        });
        let error = runtime.up(&plan()).unwrap_err();
        assert!(error.to_string().contains("ownership"), "{error}");
        // The destructive compose invocations must never have been reached.
        let calls = runtime.executor.calls.lock().unwrap();
        assert!(!calls.iter().any(|(_, arguments)| {
            arguments
                .iter()
                .any(|argument| argument == "up" || argument == "build")
        }));
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
            device: "local".into(),
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
