use std::{
    env, fmt, fs, io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream},
    os::unix::fs::{FileTypeExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use router_config::{GatewayExposureSummary, RouterConfig};
use serde::{Deserialize, Serialize};
use switchyard_planner::Plan;

const STATE_API_VERSION: &str = "switchyard.dev/host-process/v1alpha1";

#[derive(Debug)]
pub enum HostRuntimeError {
    Io(io::Error),
    InvalidPlan(String),
    StaleState(String),
    Startup(String),
}

impl fmt::Display for HostRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::InvalidPlan(message) => write!(formatter, "invalid host gateway plan: {message}"),
            Self::StaleState(message) => write!(formatter, "stale host gateway state: {message}"),
            Self::Startup(message) => write!(formatter, "host gateway failed to start: {message}"),
        }
    }
}

impl std::error::Error for HostRuntimeError {}

impl From<io::Error> for HostRuntimeError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostGatewayStatus {
    NotConfigured,
    Stopped {
        exposure: GatewayExposureSummary,
    },
    Running {
        pid: u32,
        exposure: GatewayExposureSummary,
    },
    Drifted {
        detail: String,
    },
}

impl fmt::Display for HostGatewayStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConfigured => formatter.write_str("not configured"),
            Self::Stopped { exposure } => write!(formatter, "stopped; exposure: {exposure}"),
            Self::Running { pid, exposure } => {
                write!(formatter, "running (pid {pid}); exposure: {exposure}")
            }
            Self::Drifted { detail } => write!(formatter, "drifted ({detail})"),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProcessState {
    api_version: String,
    deployment: String,
    definition_hash: String,
    pid: u32,
    start_ticks: u64,
    executable: PathBuf,
    config: PathBuf,
    admin_socket: PathBuf,
    log: PathBuf,
}

struct RuntimePaths {
    config: PathBuf,
    state: PathBuf,
    socket: PathBuf,
    log: PathBuf,
    profiles: PathBuf,
}

pub struct HostRuntime<'a> {
    workspace_root: &'a Path,
    plan: &'a Plan,
}

impl<'a> HostRuntime<'a> {
    pub fn new(workspace_root: &'a Path, plan: &'a Plan) -> Self {
        Self {
            workspace_root,
            plan,
        }
    }

    /// Reports whether this invocation would need to launch a new host process.
    /// This performs ownership checks but never writes files or signals a process.
    pub fn requires_token_for_start(&self) -> Result<bool, HostRuntimeError> {
        if self.plan.host_router_config.is_none() {
            return Ok(false);
        }
        let status = self.status()?;
        if matches!(status, HostGatewayStatus::Drifted { .. }) {
            let paths = self.runtime_paths(false)?;
            if let Some(state) = self.read_state(&paths)? {
                self.validate_state_ownership(&state, &paths)?;
                if matches!(inspect_process(&state)?, ProcessIdentity::Owned) {
                    return Ok(true);
                }
            }
        }
        if matches!(status, HostGatewayStatus::Running { .. })
            && !self.running_config_matches_published_upstreams()?
        {
            return Ok(true);
        }
        Ok(host_token_required(true, &status))
    }

    pub fn start(&self) -> Result<HostGatewayStatus, HostRuntimeError> {
        let status = self.status()?;
        let Some(encoded_config) = &self.plan.host_router_config else {
            if !matches!(status, HostGatewayStatus::NotConfigured) {
                self.cleanup()?;
            }
            return Ok(HostGatewayStatus::NotConfigured);
        };
        let planned_exposure = self.planned_exposure()?;
        if planned_exposure.mode == router_config::GatewayExposureMode::Lan {
            eprintln!(
                "WARNING: LAN exposure is enabled; host-gateway listeners are reachable at: {}",
                planned_exposure
                    .exposed_addresses
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        match status {
            HostGatewayStatus::Running { pid, exposure } => {
                if self.running_config_matches_published_upstreams()? {
                    return Ok(HostGatewayStatus::Running { pid, exposure });
                }
                self.stop()?;
                eprintln!(
                    "switchyard: refreshing host gateway after published Docker ports changed"
                );
            }
            HostGatewayStatus::Drifted { detail } => {
                self.recover_for_restart()?;
                eprintln!("switchyard: recovered {detail}");
            }
            HostGatewayStatus::NotConfigured | HostGatewayStatus::Stopped { .. } => {}
        }
        if env::var_os("SWITCHYARD_ROUTER_TOKEN").is_none() {
            return Err(HostRuntimeError::Startup(
                "SWITCHYARD_ROUTER_TOKEN must be set".into(),
            ));
        }

        let mut config: RouterConfig = serde_json::from_str(encoded_config)
            .map_err(|error| HostRuntimeError::InvalidPlan(error.to_string()))?;
        self.resolve_published_upstreams(&mut config)?;
        let addresses = config
            .spec
            .listeners
            .iter()
            .map(|listener| {
                let host = match listener.bind.host {
                    IpAddr::V4(address) if address.is_unspecified() => {
                        IpAddr::V4(Ipv4Addr::LOCALHOST)
                    }
                    IpAddr::V6(address) if address.is_unspecified() => {
                        IpAddr::V6(Ipv6Addr::LOCALHOST)
                    }
                    address => address,
                };
                SocketAddr::new(host, listener.bind.port)
            })
            .collect::<Vec<_>>();
        let paths = self.runtime_paths(true)?;
        require_missing_or_regular(&paths.config, "runtime configuration")?;
        require_missing_or_regular(&paths.log, "host gateway log")?;
        require_missing(&paths.state, "host gateway state")?;
        require_missing(&paths.socket, "host gateway socket")?;
        write_runtime_config(&paths.config, &config)?;
        let config_path = fs::canonicalize(&paths.config)?;
        let executable = find_router_binary()?;
        let log_path = paths.log.clone();
        let log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&log_path)?;
        fs::set_permissions(&log_path, fs::Permissions::from_mode(0o600))?;
        let admin_socket = paths.socket.clone();
        let mut child = Command::new("setsid")
            .arg(&executable)
            .arg("host")
            .arg(&config_path)
            .arg(&admin_socket)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log.try_clone()?))
            .stderr(Stdio::from(log))
            .spawn()?;
        let pid = child.id();
        let start_ticks = match wait_for_start_ticks(pid, Duration::from_secs(1)) {
            Ok(start_ticks) => start_ticks,
            Err(error) => {
                stop_child(&mut child);
                let _ = cleanup_startup_artifacts(&paths, false);
                return Err(error);
            }
        };
        if let Err(error) = wait_for_executable(pid, &executable, Duration::from_secs(1)) {
            stop_child(&mut child);
            let _ = cleanup_startup_artifacts(&paths, false);
            return Err(error);
        }
        let state = ProcessState {
            api_version: STATE_API_VERSION.into(),
            deployment: self.plan.deployment.clone(),
            definition_hash: self.plan.definition_hash.clone(),
            pid,
            start_ticks,
            executable,
            config: config_path,
            admin_socket: admin_socket.clone(),
            log: log_path,
        };
        if let Err(error) = write_state(&paths.state, &state) {
            stop_child(&mut child);
            let _ = cleanup_startup_artifacts(&paths, false);
            return Err(error);
        }

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = child.try_wait()? {
                let _ = cleanup_startup_artifacts(&paths, true);
                return Err(HostRuntimeError::Startup(format!(
                    "process exited with {status}; see {}",
                    state.log.display()
                )));
            }
            let listeners_ready = addresses.iter().all(|address| {
                TcpStream::connect_timeout(address, Duration::from_millis(50)).is_ok()
            });
            if listeners_ready && admin_socket.exists() {
                return Ok(HostGatewayStatus::Running {
                    pid,
                    exposure: planned_exposure,
                });
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                let _ = cleanup_startup_artifacts(&paths, true);
                return Err(HostRuntimeError::Startup(format!(
                    "listeners were not ready within 10 seconds; see {}",
                    state.log.display()
                )));
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    pub fn stop(&self) -> Result<(), HostRuntimeError> {
        let paths = self.runtime_paths(false)?;
        let Some(state) = self.read_state(&paths)? else {
            return Ok(());
        };
        self.validate_state_ownership(&state, &paths)?;
        match inspect_process(&state)? {
            ProcessIdentity::Missing => self.remove_state_files(&paths)?,
            ProcessIdentity::Different(detail) => {
                self.remove_state_files(&paths)?;
                eprintln!("switchyard: recovered stale host gateway state: {detail}");
            }
            ProcessIdentity::Owned => {
                signal_owned(&state, "-TERM")?;
                if !wait_until_stopped(&state, Duration::from_secs(5))? {
                    signal_owned(&state, "-KILL")?;
                    if !wait_until_stopped(&state, Duration::from_secs(2))? {
                        return Err(HostRuntimeError::Startup(format!(
                            "owned host gateway pid {} did not stop",
                            state.pid
                        )));
                    }
                }
                self.remove_state_files(&paths)?;
            }
        }
        Ok(())
    }

    pub fn cleanup(&self) -> Result<(), HostRuntimeError> {
        let paths = self.runtime_paths(false)?;
        self.stop()?;
        if require_missing_or_regular(&paths.config, "runtime configuration")? {
            let executable = find_router_binary()?;
            let output = Command::new(executable)
                .arg("certificates")
                .arg("cleanup")
                .arg(&paths.config)
                .output()?;
            if !output.status.success() {
                return Err(HostRuntimeError::Startup(format!(
                    "owned credential cleanup failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                )));
            }
        }
        for path in [&paths.log, &paths.socket, &paths.state] {
            remove_regular_or_socket(path)?;
        }
        match fs::symlink_metadata(&paths.profiles) {
            Ok(metadata) if metadata.file_type().is_dir() => {
                if fs::read_dir(&paths.profiles)?.next().is_none() {
                    fs::remove_dir(&paths.profiles)?;
                }
            }
            Ok(_) => {
                return Err(HostRuntimeError::StaleState(
                    "managed-profile runtime path is not a real directory".into(),
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        remove_regular_or_socket(&paths.config)?;
        Ok(())
    }

    pub fn status(&self) -> Result<HostGatewayStatus, HostRuntimeError> {
        let paths = self.runtime_paths(false)?;
        let Some(state) = self.read_state(&paths)? else {
            if self.plan.host_router_config.is_none() {
                if runtime_artifacts_exist(&paths)? {
                    return Ok(HostGatewayStatus::Drifted {
                        detail: "owned host gateway artifacts remain after hostRouter was removed"
                            .into(),
                    });
                }
                return Ok(HostGatewayStatus::NotConfigured);
            }
            if fs::symlink_metadata(&paths.socket).is_ok() {
                return Ok(HostGatewayStatus::Drifted {
                    detail:
                        "an orphaned host gateway socket remains; the next up/down will recover it"
                            .into(),
                });
            }
            return Ok(HostGatewayStatus::Stopped {
                exposure: self.planned_exposure()?,
            });
        };
        if state.deployment != self.plan.deployment || state.api_version != STATE_API_VERSION {
            return Ok(HostGatewayStatus::Drifted {
                detail: "state ownership does not match this deployment".into(),
            });
        }
        if state.definition_hash != self.plan.definition_hash {
            return Ok(HostGatewayStatus::Drifted {
                detail: "running definition differs from the planned definition".into(),
            });
        }
        self.validate_state_ownership(&state, &paths)?;
        if self.plan.host_router_config.is_none() {
            return Ok(HostGatewayStatus::Drifted {
                detail: "an owned host gateway remains after hostRouter was removed".into(),
            });
        }
        match inspect_process(&state)? {
            ProcessIdentity::Owned => Ok(HostGatewayStatus::Running {
                pid: state.pid,
                exposure: self.planned_exposure()?,
            }),
            ProcessIdentity::Missing => Ok(HostGatewayStatus::Drifted {
                detail: "process exited but owned state remains; the next up/down will recover it"
                    .into(),
            }),
            ProcessIdentity::Different(detail) => Ok(HostGatewayStatus::Drifted { detail }),
        }
    }

    fn planned_exposure(&self) -> Result<GatewayExposureSummary, HostRuntimeError> {
        let encoded =
            self.plan.host_router_config.as_ref().ok_or_else(|| {
                HostRuntimeError::InvalidPlan("hostRouter is not configured".into())
            })?;
        let config: RouterConfig = serde_json::from_str(encoded)
            .map_err(|error| HostRuntimeError::InvalidPlan(error.to_string()))?;
        let interfaces = if config.spec.needs_interface_enumeration() {
            local_ip_address::list_afinet_netifas()
                .map_err(|error| HostRuntimeError::InvalidPlan(error.to_string()))?
                .into_iter()
                .map(|(_, address)| address)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        Ok(config.spec.exposure_summary(&interfaces))
    }

    fn recover_for_restart(&self) -> Result<(), HostRuntimeError> {
        let paths = self.runtime_paths(false)?;
        let Some(state) = self.read_state(&paths)? else {
            remove_regular_or_socket(&paths.socket)?;
            return Ok(());
        };
        self.validate_state_ownership(&state, &paths)?;
        match inspect_process(&state)? {
            ProcessIdentity::Owned => self.stop(),
            ProcessIdentity::Missing | ProcessIdentity::Different(_) => {
                self.remove_state_files(&paths)
            }
        }
    }

    fn read_state(&self, paths: &RuntimePaths) -> Result<Option<ProcessState>, HostRuntimeError> {
        match fs::symlink_metadata(&paths.state) {
            Ok(metadata)
                if !metadata.file_type().is_file()
                    || metadata.file_type().is_symlink()
                    || metadata.permissions().mode() & 0o077 != 0 =>
            {
                return Err(HostRuntimeError::StaleState(
                    "process state must be a regular owner-only file".into(),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        }
        match fs::read(&paths.state) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|error| HostRuntimeError::StaleState(error.to_string())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn remove_state_files(&self, paths: &RuntimePaths) -> Result<(), HostRuntimeError> {
        remove_regular_or_socket(&paths.socket)?;
        remove_regular_or_socket(&paths.state)
    }

    fn validate_state_ownership(
        &self,
        state: &ProcessState,
        paths: &RuntimePaths,
    ) -> Result<(), HostRuntimeError> {
        if state.api_version != STATE_API_VERSION || state.deployment != self.plan.deployment {
            return Err(HostRuntimeError::StaleState(
                "state ownership does not match this deployment; refusing to signal any process"
                    .into(),
            ));
        }
        let expected_config = match fs::canonicalize(&paths.config) {
            Ok(path) => path,
            Err(error) if error.kind() == io::ErrorKind::NotFound => paths.config.clone(),
            Err(error) => return Err(error.into()),
        };
        if state.config != expected_config
            || state.admin_socket != paths.socket
            || state.log != paths.log
        {
            return Err(HostRuntimeError::StaleState(
                "state paths do not match this generated deployment; refusing mutation".into(),
            ));
        }
        Ok(())
    }

    fn runtime_paths(&self, create: bool) -> Result<RuntimePaths, HostRuntimeError> {
        checked_runtime_paths(self.workspace_root, &self.plan.deployment, create)
    }

    fn resolve_published_upstreams(
        &self,
        config: &mut RouterConfig,
    ) -> Result<(), HostRuntimeError> {
        for (provider, upstream) in &self.plan.host_upstreams {
            let address =
                self.published_address(&upstream.compose_service, upstream.container_port)?;
            let matches = config
                .spec
                .providers
                .iter_mut()
                .filter(|candidate| candidate.id.as_str() == provider)
                .collect::<Vec<_>>();
            if matches.len() != 1 {
                return Err(HostRuntimeError::InvalidPlan(format!(
                    "published provider `{provider}` matched {} host providers",
                    matches.len()
                )));
            }
            let target = matches.into_iter().next().expect("length checked");
            target.endpoint.host = address.ip().to_string();
            target.endpoint.port = address.port();
        }
        for provider in &config.spec.providers {
            let loopback = provider.endpoint.host.eq_ignore_ascii_case("localhost")
                || provider
                    .endpoint
                    .host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|address| address.is_loopback());
            if !loopback || provider.endpoint.port == 0 {
                return Err(HostRuntimeError::InvalidPlan(format!(
                    "provider `{}` did not resolve to one nonzero loopback address",
                    provider.id
                )));
            }
        }
        Ok(())
    }

    fn running_config_matches_published_upstreams(&self) -> Result<bool, HostRuntimeError> {
        let Some(encoded) = &self.plan.host_router_config else {
            return Ok(false);
        };
        let paths = self.runtime_paths(false)?;
        if !require_missing_or_regular(&paths.config, "runtime configuration")? {
            return Ok(false);
        }
        let current: RouterConfig = serde_json::from_slice(&fs::read(&paths.config)?)
            .map_err(|error| HostRuntimeError::InvalidPlan(error.to_string()))?;
        let mut desired: RouterConfig = serde_json::from_str(encoded)
            .map_err(|error| HostRuntimeError::InvalidPlan(error.to_string()))?;
        self.resolve_published_upstreams(&mut desired)?;
        Ok(current == desired)
    }

    fn published_address(
        &self,
        service: &str,
        container_port: u16,
    ) -> Result<SocketAddr, HostRuntimeError> {
        let container_port = container_port.to_string();
        let output = Command::new("docker")
            .args([
                "compose",
                "--project-name",
                &self.plan.compose_project,
                "--project-directory",
            ])
            .arg(self.workspace_root)
            .arg("--file")
            .arg(
                self.workspace_root
                    .join(&self.plan.artifact_dir)
                    .join("compose.yaml"),
            )
            .args(["port", service, &container_port])
            .output()?;
        if !output.status.success() {
            return Err(HostRuntimeError::Startup(format!(
                "could not resolve published port for `{service}:{container_port}`: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        parse_published_address(&output.stdout, service, &container_port)
    }
}

fn host_token_required(configured: bool, status: &HostGatewayStatus) -> bool {
    configured
        && !matches!(
            status,
            HostGatewayStatus::Running { .. } | HostGatewayStatus::NotConfigured
        )
}

fn checked_runtime_paths(
    workspace_root: &Path,
    deployment: &str,
    create: bool,
) -> Result<RuntimePaths, HostRuntimeError> {
    let workspace = fs::canonicalize(workspace_root)?;
    if !fs::metadata(&workspace)?.is_dir() {
        return Err(HostRuntimeError::StaleState(
            "workspace root is not a directory".into(),
        ));
    }
    let mut current = workspace.clone();
    for component in [".switchyard", "run", deployment] {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_dir() => {}
            Ok(_) => {
                return Err(HostRuntimeError::StaleState(format!(
                    "runtime ancestor {} must be a real directory",
                    current.display()
                )));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound && create => {
                fs::create_dir(&current)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => break,
            Err(error) => return Err(error.into()),
        }
    }
    let run_dir = workspace.join(".switchyard/run").join(deployment);
    if !run_dir.starts_with(&workspace) {
        return Err(HostRuntimeError::StaleState(
            "runtime directory escapes the canonical workspace".into(),
        ));
    }
    if create {
        fs::set_permissions(&run_dir, fs::Permissions::from_mode(0o700))?;
    }
    let paths = RuntimePaths {
        config: run_dir.join("host-router.json"),
        state: run_dir.join("host-gateway.json"),
        socket: run_dir.join("host.socket"),
        log: run_dir.join("host-gateway.log"),
        profiles: run_dir.join("managed-profiles"),
    };
    for path in [
        &paths.config,
        &paths.state,
        &paths.socket,
        &paths.log,
        &paths.profiles,
    ] {
        if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(HostRuntimeError::StaleState(format!(
                "runtime path {} must not be a symbolic link",
                path.display()
            )));
        }
    }
    Ok(paths)
}

fn require_missing(path: &Path, description: &str) -> Result<(), HostRuntimeError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(HostRuntimeError::StaleState(format!(
            "{description} already exists at {}",
            path.display()
        ))),
        Err(error) => Err(error.into()),
    }
}

fn require_missing_or_regular(path: &Path, description: &str) -> Result<bool, HostRuntimeError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(true),
        Ok(_) => Err(HostRuntimeError::StaleState(format!(
            "{description} must be a regular non-symlink file at {}",
            path.display()
        ))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn runtime_artifacts_exist(paths: &RuntimePaths) -> Result<bool, HostRuntimeError> {
    for path in [
        &paths.config,
        &paths.state,
        &paths.socket,
        &paths.log,
        &paths.profiles,
    ] {
        match fs::symlink_metadata(path) {
            Ok(_) => return Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(false)
}

enum ProcessIdentity {
    Missing,
    Owned,
    Different(String),
}

fn inspect_process(state: &ProcessState) -> Result<ProcessIdentity, HostRuntimeError> {
    let stat = match fs::read_to_string(format!("/proc/{}/stat", state.pid)) {
        Ok(stat) => stat,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(ProcessIdentity::Missing);
        }
        Err(error) => return Err(error.into()),
    };
    let current_ticks = parse_start_ticks(&stat).ok_or_else(|| {
        HostRuntimeError::StaleState(format!("could not parse /proc/{}/stat", state.pid))
    })?;
    if current_ticks != state.start_ticks {
        return Ok(ProcessIdentity::Different(format!(
            "pid {} was reused by another process",
            state.pid
        )));
    }
    let expected_executable = find_router_binary()?;
    let executable = fs::canonicalize(format!("/proc/{}/exe", state.pid))?;
    if state.executable != expected_executable || executable != expected_executable {
        return Ok(ProcessIdentity::Different(format!(
            "pid {} executable does not match the currently configured router binary",
            state.pid
        )));
    }
    let command_line = fs::read(format!("/proc/{}/cmdline", state.pid))?;
    let config = state.config.as_os_str().as_encoded_bytes();
    let has_host_mode = command_line
        .split(|byte| *byte == 0)
        .any(|arg| arg == b"host");
    let has_config = command_line
        .split(|byte| *byte == 0)
        .any(|argument| argument == config);
    if !has_host_mode || !has_config {
        return Ok(ProcessIdentity::Different(format!(
            "pid {} command line does not match the generated host gateway",
            state.pid
        )));
    }
    Ok(ProcessIdentity::Owned)
}

fn parse_start_ticks(stat: &str) -> Option<u64> {
    let end = stat.rfind(')')?;
    stat.get(end + 1..)?
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()
}

fn parse_published_address(
    output: &[u8],
    service: &str,
    container_port: &str,
) -> Result<SocketAddr, HostRuntimeError> {
    let addresses = String::from_utf8_lossy(output)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::parse::<SocketAddr>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            HostRuntimeError::Startup(format!(
                "Docker returned an invalid published address for `{service}:{container_port}`: {error}"
            ))
        })?;
    if addresses.len() != 1 {
        return Err(HostRuntimeError::Startup(format!(
            "expected exactly one published address for `{service}:{container_port}`, got {}",
            addresses.len()
        )));
    }
    let address = addresses[0];
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err(HostRuntimeError::Startup(format!(
            "published address for `{service}:{container_port}` is not nonzero loopback: {address}"
        )));
    }
    Ok(address)
}

fn wait_for_start_ticks(pid: u32, timeout: Duration) -> Result<u64, HostRuntimeError> {
    let deadline = Instant::now() + timeout;
    loop {
        match fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => {
                return parse_start_ticks(&stat).ok_or_else(|| {
                    HostRuntimeError::Startup("could not read child process identity".into())
                });
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn wait_for_executable(
    pid: u32,
    expected: &Path,
    timeout: Duration,
) -> Result<(), HostRuntimeError> {
    let deadline = Instant::now() + timeout;
    loop {
        match fs::canonicalize(format!("/proc/{pid}/exe")) {
            Ok(executable) if executable == expected => return Ok(()),
            Ok(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(executable) => {
                return Err(HostRuntimeError::Startup(format!(
                    "launcher did not execute {} (running {})",
                    expected.display(),
                    executable.display()
                )));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(HostRuntimeError::Startup(
                    "host gateway launcher exited before startup".into(),
                ));
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn signal_owned(state: &ProcessState, signal: &str) -> Result<(), HostRuntimeError> {
    if !matches!(inspect_process(state)?, ProcessIdentity::Owned) {
        return Err(HostRuntimeError::StaleState(
            "process identity changed before it could be signaled".into(),
        ));
    }
    let status = Command::new("kill")
        .arg(signal)
        .arg("--")
        .arg(state.pid.to_string())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(HostRuntimeError::Startup(format!(
            "could not signal owned pid {}",
            state.pid
        )))
    }
}

fn stop_child(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn wait_until_stopped(state: &ProcessState, timeout: Duration) -> Result<bool, HostRuntimeError> {
    let deadline = Instant::now() + timeout;
    loop {
        if !matches!(inspect_process(state)?, ProcessIdentity::Owned) {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn find_router_binary() -> Result<PathBuf, HostRuntimeError> {
    if let Some(configured) = env::var_os("SWITCHYARD_ROUTER_BIN") {
        return fs::canonicalize(configured).map_err(Into::into);
    }
    let sibling = env::current_exe()?
        .parent()
        .map(|directory| directory.join("switchyard-router"));
    if let Some(sibling) = sibling {
        if sibling.is_file() {
            return fs::canonicalize(sibling).map_err(Into::into);
        }
    }
    for directory in env::split_paths(&env::var_os("PATH").unwrap_or_default()) {
        let candidate = directory.join("switchyard-router");
        if candidate.is_file() {
            return fs::canonicalize(candidate).map_err(Into::into);
        }
    }
    Err(HostRuntimeError::Startup(
        "switchyard-router was not found beside the CLI or on PATH; set SWITCHYARD_ROUTER_BIN"
            .into(),
    ))
}

fn write_state(path: &Path, state: &ProcessState) -> Result<(), HostRuntimeError> {
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let encoded = serde_json::to_vec_pretty(state)
        .map_err(|error| HostRuntimeError::InvalidPlan(error.to_string()))?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    use io::Write;
    file.write_all(&encoded)?;
    file.sync_all()?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn write_runtime_config(path: &Path, config: &RouterConfig) -> Result<(), HostRuntimeError> {
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let encoded = serde_json::to_vec_pretty(config)
        .map_err(|error| HostRuntimeError::InvalidPlan(error.to_string()))?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    use io::Write;
    file.write_all(&encoded)?;
    file.sync_all()?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn remove_regular_or_socket(path: &Path) -> Result<(), HostRuntimeError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() || metadata.file_type().is_socket() => {
            fs::remove_file(path)?;
            Ok(())
        }
        Ok(_) => Err(HostRuntimeError::StaleState(format!(
            "refusing to remove non-owned path {}",
            path.display()
        ))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn cleanup_startup_artifacts(
    paths: &RuntimePaths,
    state_was_written: bool,
) -> Result<(), HostRuntimeError> {
    remove_regular_or_socket(&paths.socket)?;
    if state_was_written {
        remove_regular_or_socket(&paths.state)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn parses_linux_start_ticks_even_when_process_name_has_spaces() {
        let stat =
            "123 (switchyard router) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 424242 20";
        assert_eq!(parse_start_ticks(stat), Some(424242));
    }

    #[test]
    fn accepts_exactly_one_nonzero_loopback_publication() {
        assert_eq!(
            parse_published_address(b"127.0.0.1:32768\n", "ui", "3000").unwrap(),
            "127.0.0.1:32768".parse().unwrap()
        );
        assert!(parse_published_address(b"0.0.0.0:32768\n", "ui", "3000").is_err());
        assert!(
            parse_published_address(b"127.0.0.1:32768\n127.0.0.1:32769\n", "ui", "3000").is_err()
        );
        assert!(
            parse_published_address(b"127.0.0.1:32768\n127.0.0.1:32768\n", "ui", "3000").is_err()
        );
    }

    #[test]
    fn runtime_paths_reject_symlinked_ancestors_and_leaves() {
        let root =
            std::env::temp_dir().join(format!("switchyard-host-paths-{}", std::process::id()));
        let outside = root.with_extension("outside");
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&outside);
        fs::create_dir_all(root.join(".switchyard")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        symlink(&outside, root.join(".switchyard/run")).unwrap();
        assert!(checked_runtime_paths(&root, "demo", false).is_err());

        fs::remove_file(root.join(".switchyard/run")).unwrap();
        let paths = checked_runtime_paths(&root, "demo", true).unwrap();
        symlink(outside.join("target"), &paths.log).unwrap();
        assert!(checked_runtime_paths(&root, "demo", false).is_err());
        fs::remove_file(&paths.log).unwrap();
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn failed_startup_cleanup_allows_a_clean_retry() {
        use std::os::unix::net::UnixListener;

        let root =
            std::env::temp_dir().join(format!("switchyard-host-retry-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        let paths = checked_runtime_paths(&root, "demo", true).unwrap();
        let socket = UnixListener::bind(&paths.socket).unwrap();
        fs::write(&paths.state, b"owned startup state").unwrap();
        drop(socket);

        cleanup_startup_artifacts(&paths, true).unwrap();
        require_missing(&paths.socket, "host gateway socket").unwrap();
        require_missing(&paths.state, "host gateway state").unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn token_is_required_only_when_a_configured_host_process_needs_starting() {
        let exposure = GatewayExposureSummary {
            mode: router_config::GatewayExposureMode::Loopback,
            exposed_addresses: Vec::new(),
        };
        assert!(host_token_required(
            true,
            &HostGatewayStatus::Stopped {
                exposure: exposure.clone()
            }
        ));
        assert!(host_token_required(
            true,
            &HostGatewayStatus::Drifted {
                detail: "dead process".into()
            }
        ));
        assert!(!host_token_required(
            true,
            &HostGatewayStatus::Running { pid: 42, exposure }
        ));
        assert!(!host_token_required(
            false,
            &HostGatewayStatus::Drifted {
                detail: "removed hostRouter".into()
            }
        ));
    }

    #[test]
    fn stopped_host_preflight_requires_a_token_but_removed_host_does_not() {
        let root =
            std::env::temp_dir().join(format!("switchyard-host-token-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        let mut plan = Plan {
            deployment: "demo".into(),
            definition_hash: "definition".into(),
            resource_hash: "resource".into(),
            compose_project: "sy--demo".into(),
            artifact_dir: ".switchyard/generated/demo".into(),
            compose_yaml: String::new(),
            resolved_deployment_yaml: String::new(),
            manifest_json: String::new(),
            route_configs: Default::default(),
            sidecars: Default::default(),
            managed_profiles: Default::default(),
            host_router_config: Some(
                include_str!("../../router-config/tests/fixtures/valid/v1alpha1-minimal.json")
                    .into(),
            ),
            host_upstreams: Default::default(),
            source_identities: Default::default(),
            origins: Default::default(),
            injected_files: Default::default(),
            runtime_secrets: Default::default(),
            has_overrides: false,
        };
        assert!(
            HostRuntime::new(&root, &plan)
                .requires_token_for_start()
                .unwrap()
        );
        plan.host_router_config = None;
        assert!(
            !HostRuntime::new(&root, &plan)
                .requires_token_for_start()
                .unwrap()
        );
        fs::remove_dir_all(root).unwrap();
    }
}
