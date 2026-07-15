use std::{
    collections::BTreeSet,
    env, fmt, fs, io,
    net::IpAddr,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use router_config::{GatewayExposureMode, ListenerDestination, RouterConfig};
use serde::{Deserialize, Serialize};
use switchyard_planner::Plan;

const STATE_API_VERSION: &str = "switchyard.dev/mdns-publication/v1alpha1";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckOutcome {
    Pass,
    Warn,
    Fail,
}

impl fmt::Display for CheckOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Pass => "pass",
            Self::Warn => "warn",
            Self::Fail => "fail",
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LanCheck {
    pub name: String,
    pub outcome: CheckOutcome,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicationOutcome {
    Published,
    Failed,
}

impl fmt::Display for PublicationOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Published => "published",
            Self::Failed => "failed",
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicationStatus {
    pub name: String,
    pub address: IpAddr,
    pub outcome: PublicationOutcome,
    pub detail: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MdnsStatus {
    pub publications: Vec<PublicationStatus>,
    pub checks: Vec<LanCheck>,
}

impl MdnsStatus {
    pub fn is_empty(&self) -> bool {
        self.publications.is_empty() && self.checks.is_empty()
    }
}

#[derive(Debug)]
pub enum LanError {
    Io(io::Error),
    InvalidPlan(String),
    StaleState(String),
    Preflight(String),
    Startup(String),
}

impl fmt::Display for LanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::InvalidPlan(detail) => {
                write!(formatter, "invalid LAN publication plan: {detail}")
            }
            Self::StaleState(detail) => write!(formatter, "stale mDNS publication state: {detail}"),
            Self::Preflight(detail) => {
                write!(formatter, "LAN publication preflight failed: {detail}")
            }
            Self::Startup(detail) => write!(formatter, "mDNS publication failed: {detail}"),
        }
    }
}

impl std::error::Error for LanError {}

impl From<io::Error> for LanError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PublisherState {
    pid: u32,
    start_ticks: u64,
    executable: PathBuf,
    name: String,
    address: IpAddr,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PublicationState {
    api_version: String,
    deployment: String,
    definition_hash: String,
    publishers: Vec<PublisherState>,
    checks: Vec<LanCheck>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct InterfaceAddress {
    name: String,
    address: IpAddr,
    host_route: bool,
}

struct DesiredPublication {
    requested: bool,
    targets: Vec<(String, IpAddr)>,
    /// Every non-loopback exposed address, including VPN and container-bridge
    /// addresses that are excluded from `targets`, so preflight can warn on them.
    exposed: Vec<InterfaceAddress>,
}

trait CommandRunner {
    fn find(&self, name: &str) -> Option<PathBuf>;
    fn output(&self, program: &Path, arguments: &[&str], timeout: Duration) -> io::Result<Output>;
    fn interfaces(&self) -> Result<Vec<InterfaceAddress>, String>;
    fn spawn_publisher(
        &self,
        executable: &Path,
        name: &str,
        address: IpAddr,
        log_path: &Path,
    ) -> Result<PublisherState, LanError>;
}

struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn find(&self, name: &str) -> Option<PathBuf> {
        find_binary(name)
    }

    fn output(&self, program: &Path, arguments: &[&str], timeout: Duration) -> io::Result<Output> {
        output_with_timeout(program, arguments, timeout)
    }

    fn interfaces(&self) -> Result<Vec<InterfaceAddress>, String> {
        let addresses =
            local_ip_address::list_afinet_netifas().map_err(|error| error.to_string())?;
        let host_routes = self
            .find("ip")
            .and_then(|binary| {
                self.output(&binary, &["-o", "address", "show"], COMMAND_TIMEOUT)
                    .ok()
            })
            .filter(|output| output.status.success())
            .map(|output| parse_host_routes(&output.stdout))
            .unwrap_or_default();
        Ok(addresses
            .into_iter()
            .map(|(name, address)| InterfaceAddress {
                host_route: host_routes.contains(&(name.clone(), address)),
                name,
                address,
            })
            .collect())
    }

    fn spawn_publisher(
        &self,
        executable: &Path,
        name: &str,
        address: IpAddr,
        log_path: &Path,
    ) -> Result<PublisherState, LanError> {
        spawn_system_publisher(executable, name, address, log_path)
    }
}

pub struct LanRuntime<'a> {
    workspace_root: &'a Path,
    plan: &'a Plan,
}

impl<'a> LanRuntime<'a> {
    pub fn new(workspace_root: &'a Path, plan: &'a Plan) -> Self {
        Self {
            workspace_root,
            plan,
        }
    }

    pub fn start(&self) -> Result<MdnsStatus, LanError> {
        self.start_with(&SystemRunner)
    }

    fn start_with<R: CommandRunner>(&self, runner: &R) -> Result<MdnsStatus, LanError> {
        let desired = desired_publications(self.plan, runner)?;
        if !desired.requested {
            self.stop()?;
            return Ok(MdnsStatus {
                publications: Vec::new(),
                checks: Vec::new(),
            });
        }

        if let Some(state) = self.read_state()? {
            if state.definition_hash == self.plan.definition_hash
                && state.publishers.iter().all(|publisher| {
                    matches!(inspect_publisher(publisher), Ok(ProcessIdentity::Owned))
                })
                && state
                    .publishers
                    .iter()
                    .map(|publisher| (&publisher.name, publisher.address))
                    .collect::<BTreeSet<_>>()
                    == desired
                        .targets
                        .iter()
                        .map(|(name, address)| (name, *address))
                        .collect::<BTreeSet<_>>()
            {
                return Ok(status_from_state(&state));
            }
            self.stop()?;
        }

        let (publisher, mut checks) = preflight(runner, &desired.exposed);
        if checks
            .iter()
            .any(|check| check.outcome == CheckOutcome::Fail)
        {
            print_checks(&checks);
            return Err(LanError::Preflight(
                checks
                    .iter()
                    .filter(|check| check.outcome == CheckOutcome::Fail)
                    .map(|check| check.detail.as_str())
                    .collect::<Vec<_>>()
                    .join("; "),
            ));
        }
        let publisher = publisher.ok_or_else(|| {
            LanError::Preflight(
                "avahi-publish-address was not found; install the `avahi-utils` package".into(),
            )
        })?;
        let state_path = self.state_path(true)?;
        require_missing(&state_path)?;
        let log_path = state_path.with_file_name("mdns-publication.log");
        let mut state = PublicationState {
            api_version: STATE_API_VERSION.into(),
            deployment: self.plan.deployment.clone(),
            definition_hash: self.plan.definition_hash.clone(),
            publishers: Vec::new(),
            checks: Vec::new(),
        };
        for (name, address) in &desired.targets {
            match runner.spawn_publisher(&publisher, name, *address, &log_path) {
                Ok(publisher_state) => {
                    state.publishers.push(publisher_state);
                    if let Err(error) = write_state(&state_path, &state) {
                        let _ = stop_publishers(&state);
                        let _ = remove_regular(&state_path);
                        return Err(error);
                    }
                }
                Err(error) => {
                    let _ = stop_publishers(&state);
                    let _ = remove_regular(&state_path);
                    return Err(error);
                }
            }
        }

        for (name, _) in &desired.targets {
            checks.push(resolution_check(runner, name));
        }
        state.checks = checks;
        write_state(&state_path, &state)?;
        Ok(status_from_state(&state))
    }

    pub fn stop(&self) -> Result<(), LanError> {
        let Some(state) = self.read_state()? else {
            return Ok(());
        };
        self.validate_state(&state)?;
        stop_publishers(&state)?;
        remove_regular(&self.state_path(false)?)
    }

    pub fn status(&self) -> Result<MdnsStatus, LanError> {
        let desired = desired_publications(self.plan, &SystemRunner)?;
        let Some(state) = self.read_state()? else {
            if !desired.requested {
                return Ok(MdnsStatus {
                    publications: Vec::new(),
                    checks: Vec::new(),
                });
            }
            let (_, checks) = preflight(&SystemRunner, &desired.exposed);
            return Ok(MdnsStatus {
                publications: desired
                    .targets
                    .into_iter()
                    .map(|(name, address)| PublicationStatus {
                        name,
                        address,
                        outcome: PublicationOutcome::Failed,
                        detail: "not published; run `switchyard up` to publish this name".into(),
                    })
                    .collect(),
                checks,
            });
        };
        if self.validate_state(&state).is_err()
            || state.definition_hash != self.plan.definition_hash
        {
            return Ok(MdnsStatus {
                publications: state
                    .publishers
                    .iter()
                    .map(|publisher| PublicationStatus {
                        name: publisher.name.clone(),
                        address: publisher.address,
                        outcome: PublicationOutcome::Failed,
                        detail:
                            "owned publication state has drifted; the next up/down will recover it"
                                .into(),
                    })
                    .collect(),
                checks: state.checks,
            });
        }
        let mut status = status_from_state(&state);
        for (entry, publisher) in status.publications.iter_mut().zip(&state.publishers) {
            if !matches!(inspect_publisher(publisher)?, ProcessIdentity::Owned) {
                entry.outcome = PublicationOutcome::Failed;
                entry.detail = "publisher exited or its process identity changed".into();
            }
        }
        Ok(status)
    }

    fn state_path(&self, create: bool) -> Result<PathBuf, LanError> {
        let root = fs::canonicalize(self.workspace_root)?;
        let mut current = root.clone();
        for component in [".switchyard", "run", self.plan.deployment.as_str()] {
            current.push(component);
            match fs::symlink_metadata(&current) {
                Ok(metadata)
                    if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {}
                Ok(_) => {
                    return Err(LanError::StaleState(format!(
                        "{} must be a real directory",
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
        let run_dir = root.join(".switchyard/run").join(&self.plan.deployment);
        if create {
            fs::set_permissions(&run_dir, fs::Permissions::from_mode(0o700))?;
        }
        let path = run_dir.join("mdns-publication.json");
        if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(LanError::StaleState(
                "publication state must not be a symbolic link".into(),
            ));
        }
        Ok(path)
    }

    fn read_state(&self) -> Result<Option<PublicationState>, LanError> {
        let path = self.state_path(false)?;
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if !metadata.file_type().is_file() || metadata.permissions().mode() & 0o077 != 0 {
            return Err(LanError::StaleState(
                "publication state must be a regular owner-only file".into(),
            ));
        }
        serde_json::from_slice(&fs::read(path)?)
            .map(Some)
            .map_err(|error| LanError::StaleState(error.to_string()))
    }

    fn validate_state(&self, state: &PublicationState) -> Result<(), LanError> {
        if state.api_version != STATE_API_VERSION || state.deployment != self.plan.deployment {
            return Err(LanError::StaleState(
                "state ownership does not match this deployment; refusing to signal any process"
                    .into(),
            ));
        }
        if state.publishers.iter().any(|publisher| {
            !publisher.executable.is_absolute()
                || !publisher.name.to_ascii_lowercase().ends_with(".local")
                || publisher.address.is_loopback()
                || publisher.address.is_unspecified()
        }) {
            return Err(LanError::StaleState(
                "publisher ownership fields are invalid; refusing to signal any process".into(),
            ));
        }
        Ok(())
    }
}

fn desired_publications<R: CommandRunner>(
    plan: &Plan,
    runner: &R,
) -> Result<DesiredPublication, LanError> {
    let none = || DesiredPublication {
        requested: false,
        targets: Vec::new(),
        exposed: Vec::new(),
    };
    let Some(encoded) = &plan.host_router_config else {
        return Ok(none());
    };
    let config: RouterConfig =
        serde_json::from_str(encoded).map_err(|error| LanError::InvalidPlan(error.to_string()))?;
    if !config.spec.lan_exposure_acknowledged() {
        return Ok(none());
    }
    let interfaces = runner.interfaces().map_err(LanError::InvalidPlan)?;
    let summary = config.spec.exposure_summary(
        &interfaces
            .iter()
            .map(|interface| interface.address)
            .collect::<Vec<_>>(),
    );
    if summary.mode != GatewayExposureMode::Lan {
        return Ok(none());
    }
    let names = publication_names(&config);
    let requested = !names.is_empty();
    let exposed_addresses = summary
        .exposed_addresses
        .iter()
        .map(|address| address.ip())
        .filter(|address| !address.is_loopback() && !address.is_unspecified())
        .collect::<BTreeSet<_>>();
    let exposed = interfaces
        .into_iter()
        .filter(|interface| exposed_addresses.contains(&interface.address))
        .collect::<Vec<_>>();
    // Advertise only genuinely LAN-reachable addresses. VPN links do not carry
    // multicast DNS and container bridges are host-internal; publishing either
    // would hand other LAN devices unreachable records.
    let addresses = exposed
        .iter()
        .filter(|interface| {
            !is_vpn_interface(&interface.name, interface.host_route)
                && !is_container_bridge(&interface.name)
        })
        .map(|interface| interface.address)
        .collect::<BTreeSet<_>>();
    Ok(DesiredPublication {
        requested,
        targets: publication_targets(names, addresses),
        exposed,
    })
}

fn is_container_bridge(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with("docker")
        || name.starts_with("br-")
        || name.starts_with("veth")
        || name.starts_with("virbr")
}

fn publication_names(config: &RouterConfig) -> BTreeSet<String> {
    config
        .spec
        .listeners
        .iter()
        .flat_map(|listener| &listener.destinations)
        .filter_map(|destination| match destination {
            ListenerDestination::CustomDomain { domain, .. }
                if domain.to_ascii_lowercase().ends_with(".local") =>
            {
                Some(domain.clone())
            }
            _ => None,
        })
        .collect()
}

fn publication_targets(
    names: BTreeSet<String>,
    addresses: BTreeSet<IpAddr>,
) -> Vec<(String, IpAddr)> {
    names
        .into_iter()
        .flat_map(|name| {
            addresses
                .iter()
                .filter(|address| !address.is_loopback() && !address.is_unspecified())
                .map(move |address| (name.clone(), *address))
        })
        .collect()
}

fn preflight<R: CommandRunner>(
    runner: &R,
    exposed: &[InterfaceAddress],
) -> (Option<PathBuf>, Vec<LanCheck>) {
    let publisher = runner.find("avahi-publish-address");
    let mut checks = vec![match &publisher {
        Some(path) => check(
            "avahi-publish-address",
            CheckOutcome::Pass,
            format!("found {}", path.display()),
        ),
        None => check(
            "avahi-publish-address",
            CheckOutcome::Fail,
            "not found; install the `avahi-utils` package",
        ),
    }];
    let daemon_check = match runner.find("avahi-browse") {
        Some(binary) => match runner.output(&binary, &["--all", "--terminate"], COMMAND_TIMEOUT) {
            Ok(output) if output.status.success() => check(
                "avahi-daemon",
                CheckOutcome::Pass,
                "Avahi responded to a local browse probe",
            ),
            Ok(output) => check(
                "avahi-daemon",
                CheckOutcome::Fail,
                format!("Avahi browse probe failed: {}", output_detail(&output)),
            ),
            Err(error) => check(
                "avahi-daemon",
                CheckOutcome::Fail,
                format!("Avahi browse probe failed: {error}"),
            ),
        },
        None => check(
            "avahi-daemon",
            CheckOutcome::Fail,
            "`avahi-browse` was not found; install `avahi-utils` and ensure avahi-daemon is running",
        ),
    };
    checks.push(daemon_check);
    let usable = exposed
        .iter()
        .filter(|interface| {
            !interface.address.is_loopback()
                && !is_vpn_interface(&interface.name, interface.host_route)
                && !is_container_bridge(&interface.name)
        })
        .count();
    let vpn = exposed
        .iter()
        .filter(|interface| is_vpn_interface(&interface.name, interface.host_route))
        .map(|interface| interface.name.as_str())
        .collect::<BTreeSet<_>>();
    checks.push(if usable > 0 {
        check(
            "lan-interface",
            CheckOutcome::Pass,
            format!("{usable} non-loopback, non-VPN exposed interface address(es) found"),
        )
    } else {
        check(
            "lan-interface",
            CheckOutcome::Fail,
            "no non-loopback, non-VPN interface matches the exposed addresses",
        )
    });
    if !vpn.is_empty() {
        checks.push(check(
            "vpn-interface",
            CheckOutcome::Warn,
            format!(
                "VPN-style interface(s) {} are exposed; mDNS normally does not traverse VPN links",
                vpn.into_iter().collect::<Vec<_>>().join(", ")
            ),
        ));
    }
    checks.push(firewall_check(runner));
    checks.push(check(
        "network-boundaries",
        CheckOutcome::Warn,
        "mDNS does not normally cross subnets, VLANs, VPNs, or guest Wi-Fi/client isolation",
    ));
    (publisher, checks)
}

fn firewall_check<R: CommandRunner>(runner: &R) -> LanCheck {
    if let Some(binary) = runner.find("firewall-cmd") {
        if runner
            .output(&binary, &["--state"], COMMAND_TIMEOUT)
            .is_ok_and(|output| output.status.success())
        {
            return match runner.output(&binary, &["--query-port=5353/udp"], COMMAND_TIMEOUT) {
                Ok(output) if output.status.success() => check(
                    "firewall-udp-5353",
                    CheckOutcome::Pass,
                    "firewalld permits 5353/udp in the active zone",
                ),
                Ok(_) => check(
                    "firewall-udp-5353",
                    CheckOutcome::Warn,
                    "firewalld is active and the active zone does not permit 5353/udp",
                ),
                Err(error) => check(
                    "firewall-udp-5353",
                    CheckOutcome::Warn,
                    format!(
                        "firewalld is active, but its 5353/udp policy could not be read: {error}"
                    ),
                ),
            };
        }
    }
    if let Some(binary) = runner.find("ufw") {
        if let Ok(output) = runner.output(&binary, &["status"], COMMAND_TIMEOUT) {
            let text = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
            if text.contains("status: active") {
                return if text
                    .lines()
                    .any(|line| line.contains("5353/udp") && line.contains("allow"))
                {
                    check(
                        "firewall-udp-5353",
                        CheckOutcome::Pass,
                        "ufw has an explicit allow rule for 5353/udp",
                    )
                } else {
                    check(
                        "firewall-udp-5353",
                        CheckOutcome::Warn,
                        "ufw is active and no explicit 5353/udp allow rule was detected",
                    )
                };
            }
        }
    }
    if runner.find("nft").is_some() {
        check(
            "firewall-udp-5353",
            CheckOutcome::Warn,
            "nftables is installed; effective multicast policy could not be determined safely",
        )
    } else {
        check(
            "firewall-udp-5353",
            CheckOutcome::Warn,
            "firewall policy could not be determined; ensure inbound and outbound mDNS UDP 5353 are permitted",
        )
    }
}

fn resolution_check<R: CommandRunner>(runner: &R, name: &str) -> LanCheck {
    let Some(getent) = runner.find("getent") else {
        return check(
            "name-resolution",
            CheckOutcome::Warn,
            format!(
                "could not resolve {name}: `getent` is unavailable; verify nss-mdns and UDP 5353"
            ),
        );
    };
    match runner.output(&getent, &["hosts", name], COMMAND_TIMEOUT) {
        Ok(output) if output.status.success() && !output.stdout.is_empty() => check(
            "name-resolution",
            CheckOutcome::Pass,
            format!("{name} resolves locally"),
        ),
        Ok(_) => check(
            "name-resolution",
            CheckOutcome::Warn,
            format!(
                "{name} did not resolve locally; likely causes include UDP 5353 firewall rules or missing nss-mdns configuration"
            ),
        ),
        Err(error) => check(
            "name-resolution",
            CheckOutcome::Warn,
            format!("could not check {name}: {error}; verify UDP 5353 and nss-mdns"),
        ),
    }
}

fn is_vpn_interface(name: &str, host_route: bool) -> bool {
    let name = name.to_ascii_lowercase();
    host_route || name.starts_with("tailscale") || name.starts_with("tun") || name.starts_with("wg")
}

fn parse_host_routes(output: &[u8]) -> BTreeSet<(String, IpAddr)> {
    String::from_utf8_lossy(output)
        .lines()
        .filter_map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            let name = fields.get(1)?.trim_end_matches(':').to_owned();
            let family = *fields.get(2)?;
            let address = *fields.get(3)?;
            let (address, prefix) = address.split_once('/')?;
            let address = address.parse::<IpAddr>().ok()?;
            let host_prefix = match (family, address) {
                ("inet", IpAddr::V4(_)) => "32",
                ("inet6", IpAddr::V6(_)) => "128",
                _ => return None,
            };
            (prefix == host_prefix).then_some((name, address))
        })
        .collect()
}

fn check(name: &str, outcome: CheckOutcome, detail: impl Into<String>) -> LanCheck {
    LanCheck {
        name: name.into(),
        outcome,
        detail: detail.into(),
    }
}

fn print_checks(checks: &[LanCheck]) {
    for check in checks {
        println!(
            "LAN preflight [{}] {}: {}",
            check.outcome, check.name, check.detail
        );
    }
}

fn status_from_state(state: &PublicationState) -> MdnsStatus {
    MdnsStatus {
        publications: state
            .publishers
            .iter()
            .map(|publisher| PublicationStatus {
                name: publisher.name.clone(),
                address: publisher.address,
                outcome: PublicationOutcome::Published,
                detail: format!("published by pid {}", publisher.pid),
            })
            .collect(),
        checks: state.checks.clone(),
    }
}

fn spawn_system_publisher(
    executable: &Path,
    name: &str,
    address: IpAddr,
    log_path: &Path,
) -> Result<PublisherState, LanError> {
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(log_path)?;
    fs::set_permissions(log_path, fs::Permissions::from_mode(0o600))?;
    // `avahi-publish-address` is a symlink to `avahi-publish`, which dispatches on
    // argv[0]; the canonicalized executable path loses that, so pass `-a` explicitly.
    // `-R` skips the reverse PTR record, which would collide with avahi-daemon's own
    // reverse record for the host's primary address.
    let mut child = Command::new(executable)
        .arg("-a")
        .arg("-R")
        .arg(name)
        .arg(address.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .spawn()
        .map_err(|error| {
            LanError::Startup(format!(
                "could not start `{}` for {name}: {error}; install `avahi-utils`",
                executable.display()
            ))
        })?;
    let pid = child.id();
    let start_ticks = match wait_for_start_ticks(pid, Duration::from_secs(1)) {
        Ok(start_ticks) => start_ticks,
        Err(error) => {
            stop_child(&mut child);
            return Err(error);
        }
    };
    thread::sleep(Duration::from_millis(150));
    match child.try_wait() {
        Ok(Some(status)) => {
            let reason = fs::read_to_string(log_path)
                .ok()
                .and_then(|log| log.lines().last().map(str::to_owned))
                .filter(|line| !line.is_empty())
                .map(|line| format!(" ({line})"))
                .unwrap_or_default();
            return Err(LanError::Startup(format!(
                "`avahi-publish-address {name} {address}` exited immediately with {status}{reason}; ensure avahi-daemon is running"
            )));
        }
        Err(error) => {
            stop_child(&mut child);
            return Err(error.into());
        }
        Ok(None) => {}
    }
    Ok(PublisherState {
        pid,
        start_ticks,
        executable: executable.to_owned(),
        name: name.into(),
        address,
    })
}

enum ProcessIdentity {
    Missing,
    Owned,
    Different,
}

fn inspect_publisher(state: &PublisherState) -> Result<ProcessIdentity, LanError> {
    let stat = match fs::read_to_string(format!("/proc/{}/stat", state.pid)) {
        Ok(stat) => stat,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(ProcessIdentity::Missing);
        }
        Err(error) => return Err(error.into()),
    };
    if parse_start_ticks(&stat) != Some(state.start_ticks) {
        return Ok(ProcessIdentity::Different);
    }
    let executable = match fs::canonicalize(format!("/proc/{}/exe", state.pid)) {
        Ok(executable) => executable,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(ProcessIdentity::Missing);
        }
        Err(error) => return Err(error.into()),
    };
    if executable != state.executable {
        return Ok(ProcessIdentity::Different);
    }
    let command_line = fs::read(format!("/proc/{}/cmdline", state.pid))?;
    let arguments = command_line.split(|byte| *byte == 0).collect::<Vec<_>>();
    if !arguments
        .iter()
        .any(|argument| *argument == state.name.as_bytes())
        || !arguments
            .iter()
            .any(|argument| *argument == state.address.to_string().as_bytes())
    {
        return Ok(ProcessIdentity::Different);
    }
    Ok(ProcessIdentity::Owned)
}

fn stop_publishers(state: &PublicationState) -> Result<(), LanError> {
    for publisher in &state.publishers {
        match inspect_publisher(publisher)? {
            ProcessIdentity::Owned => {
                signal(publisher, "-TERM")?;
                if !wait_stopped(publisher, Duration::from_secs(3))? {
                    signal(publisher, "-KILL")?;
                    if !wait_stopped(publisher, Duration::from_secs(1))? {
                        return Err(LanError::Startup(format!(
                            "owned mDNS publisher pid {} did not stop",
                            publisher.pid
                        )));
                    }
                }
            }
            ProcessIdentity::Missing | ProcessIdentity::Different => {}
        }
    }
    Ok(())
}

fn signal(state: &PublisherState, signal: &str) -> Result<(), LanError> {
    if !matches!(inspect_publisher(state)?, ProcessIdentity::Owned) {
        return Err(LanError::StaleState(
            "publisher identity changed before it could be signaled".into(),
        ));
    }
    if Command::new("kill")
        .arg(signal)
        .arg("--")
        .arg(state.pid.to_string())
        .status()?
        .success()
    {
        Ok(())
    } else {
        Err(LanError::Startup(format!(
            "could not signal owned mDNS publisher pid {}",
            state.pid
        )))
    }
}

fn wait_stopped(state: &PublisherState, timeout: Duration) -> Result<bool, LanError> {
    let deadline = Instant::now() + timeout;
    loop {
        if !matches!(inspect_publisher(state)?, ProcessIdentity::Owned) {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn wait_for_start_ticks(pid: u32, timeout: Duration) -> Result<u64, LanError> {
    let deadline = Instant::now() + timeout;
    loop {
        match fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => {
                return parse_start_ticks(&stat).ok_or_else(|| {
                    LanError::Startup("could not read mDNS child process identity".into())
                });
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10))
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn parse_start_ticks(stat: &str) -> Option<u64> {
    let end = stat.rfind(')')?;
    stat.get(end + 1..)?
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()
}

fn stop_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn find_binary(name: &str) -> Option<PathBuf> {
    env::split_paths(&env::var_os("PATH").unwrap_or_default())
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
        .and_then(|path| fs::canonicalize(path).ok())
}

fn output_with_timeout(
    program: &Path,
    arguments: &[&str],
    timeout: Duration,
) -> io::Result<Output> {
    let mut child = Command::new(program)
        .args(arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let deadline = Instant::now() + timeout;
    loop {
        if child.try_wait()?.is_some() {
            return child.wait_with_output();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            return child.wait_with_output();
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn output_detail(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stderr.trim().is_empty() {
        stdout.trim().to_owned()
    } else {
        stderr.trim().to_owned()
    }
}

fn require_missing(path: &Path) -> Result<(), LanError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(LanError::StaleState(format!(
            "publication state already exists at {}",
            path.display()
        ))),
        Err(error) => Err(error.into()),
    }
}

fn write_state(path: &Path, state: &PublicationState) -> Result<(), LanError> {
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let encoded = serde_json::to_vec_pretty(state)
        .map_err(|error| LanError::InvalidPlan(error.to_string()))?;
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

fn remove_regular(path: &Path) -> Result<(), LanError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {
            fs::remove_file(path)?;
            Ok(())
        }
        Ok(_) => Err(LanError::StaleState(format!(
            "refusing to remove non-owned path {}",
            path.display()
        ))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeMap, os::unix::process::ExitStatusExt};

    struct FakeRunner {
        binaries: BTreeSet<String>,
        outputs: BTreeMap<String, Output>,
        interfaces: Vec<InterfaceAddress>,
    }
    impl CommandRunner for FakeRunner {
        fn find(&self, name: &str) -> Option<PathBuf> {
            self.binaries
                .contains(name)
                .then(|| PathBuf::from(format!("/usr/bin/{name}")))
        }
        fn output(
            &self,
            program: &Path,
            arguments: &[&str],
            _timeout: Duration,
        ) -> io::Result<Output> {
            let key = format!(
                "{} {}",
                program.file_name().unwrap().to_string_lossy(),
                arguments.join(" ")
            );
            self.outputs
                .get(&key)
                .cloned()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, key))
        }
        fn interfaces(&self) -> Result<Vec<InterfaceAddress>, String> {
            Ok(self.interfaces.clone())
        }
        fn spawn_publisher(
            &self,
            _executable: &Path,
            _name: &str,
            _address: IpAddr,
            _log_path: &Path,
        ) -> Result<PublisherState, LanError> {
            panic!("unit preflight must not launch a publisher")
        }
    }
    fn output(code: i32, stdout: &str) -> Output {
        Output {
            status: std::process::ExitStatus::from_raw(code << 8),
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        }
    }

    #[test]
    fn classifies_vpn_interfaces_and_host_routes() {
        assert!(is_vpn_interface("tailscale0", false));
        assert!(is_vpn_interface("tun12", false));
        assert!(is_vpn_interface("wg0", false));
        assert!(is_vpn_interface("eth0", true));
        assert!(!is_vpn_interface("enp3s0", false));
    }

    #[test]
    fn classifies_container_bridges() {
        assert!(is_container_bridge("docker0"));
        assert!(is_container_bridge("br-1234abcd"));
        assert!(is_container_bridge("veth99"));
        assert!(is_container_bridge("virbr0"));
        assert!(!is_container_bridge("enp3s0"));
        assert!(!is_container_bridge("wlan0"));
    }

    #[test]
    fn preflight_shapes_pass_warning_and_limitation_results_without_avahi() {
        let runner = FakeRunner {
            binaries: ["avahi-publish-address", "avahi-browse", "ufw"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            outputs: [
                ("avahi-browse --all --terminate".into(), output(0, "")),
                (
                    "ufw status".into(),
                    output(0, "Status: active\n5353/udp ALLOW Anywhere\n"),
                ),
            ]
            .into_iter()
            .collect(),
            interfaces: vec![
                InterfaceAddress {
                    name: "enp3s0".into(),
                    address: "192.168.1.5".parse().unwrap(),
                    host_route: false,
                },
                InterfaceAddress {
                    name: "tailscale0".into(),
                    address: "100.64.0.1".parse().unwrap(),
                    host_route: true,
                },
            ],
        };
        let exposed = runner.interfaces.clone();
        let (_, checks) = preflight(&runner, &exposed);
        assert!(
            checks
                .iter()
                .any(|check| check.name == "lan-interface" && check.outcome == CheckOutcome::Pass)
        );
        assert!(
            checks
                .iter()
                .any(|check| check.name == "vpn-interface" && check.outcome == CheckOutcome::Warn)
        );
        assert!(
            checks
                .iter()
                .any(|check| check.name == "network-boundaries"
                    && check.outcome == CheckOutcome::Warn)
        );
    }

    #[test]
    fn publication_state_round_trips_as_owner_only_json() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("state.json");
        let state = PublicationState {
            api_version: STATE_API_VERSION.into(),
            deployment: "demo".into(),
            definition_hash: "hash".into(),
            publishers: vec![PublisherState {
                pid: 42,
                start_ticks: 9,
                executable: "/usr/bin/avahi-publish-address".into(),
                name: "demo.local".into(),
                address: "192.168.1.5".parse().unwrap(),
            }],
            checks: vec![check("network-boundaries", CheckOutcome::Warn, "limited")],
        };
        write_state(&path, &state).unwrap();
        let decoded: PublicationState = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(decoded, state);
        assert_eq!(fs::metadata(path).unwrap().permissions().mode() & 0o077, 0);
    }

    #[test]
    fn publishes_only_local_names_to_non_loopback_addresses() {
        let config: RouterConfig = serde_json::from_value(serde_json::json!({
            "apiVersion": "switchyard.dev/router/v1alpha1",
            "kind": "RouterConfiguration",
            "metadata": {"deployment": "demo"},
            "spec": {
                "snapshot": {"id": "initial", "version": 1, "transitions": {
                    "http": {"strategy": "drain", "timeoutMs": 5000},
                    "https": {"strategy": "drain", "timeoutMs": 5000},
                    "websocket": {"strategy": "pin"},
                    "grpc": {"strategy": "drain", "timeoutMs": 5000},
                    "tcp": {"strategy": "close"}
                }},
                "listeners": [{
                    "bind": {"host": "0.0.0.0", "port": 8080},
                    "protocol": "http",
                    "destinations": [
                        {"kind": "custom_domain", "slot": "web", "domain": "demo.local"},
                        {"kind": "custom_domain", "slot": "web", "domain": "UPPER.LOCAL"},
                        {"kind": "custom_domain", "slot": "web", "domain": "demo.example"}
                    ]
                }]
            }
        }))
        .unwrap();
        let targets = publication_targets(
            publication_names(&config),
            ["127.0.0.1".parse().unwrap(), "192.168.1.5".parse().unwrap()]
                .into_iter()
                .collect(),
        );
        assert_eq!(
            targets,
            vec![
                ("UPPER.LOCAL".into(), "192.168.1.5".parse().unwrap()),
                ("demo.local".into(), "192.168.1.5".parse().unwrap())
            ]
        );
    }

    #[test]
    fn parses_ipv4_and_ipv6_host_routes() {
        let routes = parse_host_routes(b"7: tailscale0    inet 100.64.0.1/32 scope global tailscale0\n8: wg0    inet6 fd00::1/128 scope global\n2: eth0    inet 192.168.1.5/24 scope global eth0\n");
        assert!(routes.contains(&("tailscale0".into(), "100.64.0.1".parse().unwrap())));
        assert!(routes.contains(&("wg0".into(), "fd00::1".parse().unwrap())));
        assert_eq!(routes.len(), 2);
    }
}
