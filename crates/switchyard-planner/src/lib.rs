//! Deterministic, side-effect-free deployment planning.

mod bundle;
mod model;
mod overlay;

pub use bundle::*;
pub use model::*;
pub use overlay::*;

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use switchyard_adapter_sdk::{AdapterKind, Diagnostic as AdapterDiagnostic, SourceIdentity};
use switchyard_adapters::built_in_registry;
use switchyard_sources::SourceManager;

const INSTANCE_LABEL: &str = "dev.switchyard.instance";
const SERVICE_LABEL: &str = "dev.switchyard.service";

#[derive(Debug)]
pub enum PlannerError {
    Io(io::Error),
    Yaml(serde_yaml::Error),
    MigrationRequired(String),
    OverlayIo(io::Error),
    OverlayYaml(serde_yaml::Error),
}

impl fmt::Display for PlannerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "could not read deployment: {error}"),
            Self::Yaml(error) => write!(f, "invalid deployment YAML: {error}"),
            Self::MigrationRequired(version) => write!(
                f,
                "deployment uses apiVersion {version}; run `switchyard migrate` to update it to {API_VERSION}"
            ),
            Self::OverlayIo(error) => write!(f, "could not read overlay: {error}"),
            Self::OverlayYaml(error) => write!(f, "invalid overlay YAML: {error}"),
        }
    }
}

impl std::error::Error for PlannerError {}

impl From<io::Error> for PlannerError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_yaml::Error> for PlannerError {
    fn from(value: serde_yaml::Error) -> Self {
        Self::Yaml(value)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticCode {
    UnsupportedSchema,
    InvalidName,
    InvalidPath,
    DuplicateName,
    MissingReference,
    MissingVariable,
    DependencyCycle,
    ListenerConflict,
    IncompleteGroup,
    InvalidOverlay,
    SelectorNoMatch,
    OverlayConflict,
    UnsupportedSecret,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub code: DiagnosticCode,
    pub path: String,
    pub message: String,
}

impl Diagnostic {
    fn new(code: DiagnosticCode, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            path: path.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} at {}: {}", self.code, self.path, self.message)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannerWarning {
    pub code: String,
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub deployment: String,
    pub definition_hash: String,
    pub resource_hash: String,
    pub compose_project: String,
    pub artifact_dir: PathBuf,
    pub compose_yaml: String,
    /// Number of services in the local Compose project. Zero when every
    /// instance is placed on a remote device; the runtime must then skip the
    /// local project entirely instead of running an empty `compose up`.
    #[serde(default)]
    pub local_service_count: usize,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub remote_projects: BTreeMap<String, RemoteComposePlan>,
    pub resolved_deployment_yaml: String,
    pub manifest_json: String,
    pub route_configs: BTreeMap<String, String>,
    pub sidecars: BTreeMap<String, SidecarPlan>,
    #[serde(default)]
    pub managed_profiles: BTreeMap<String, ManagedProfilePlan>,
    #[serde(default)]
    pub host_router_config: Option<String>,
    #[serde(default)]
    pub host_upstreams: BTreeMap<String, HostUpstreamPlan>,
    /// Exact live-derived source identity captured for every instance at plan time.
    #[serde(default)]
    pub source_identities: BTreeMap<String, SourceIdentity>,
    /// Secret-safe provenance for values resolved from overlays.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub origins: Vec<OriginTrace>,
    /// File payloads written only beneath the generated artifact directory.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub injected_files: Vec<InjectedFilePlan>,
    /// Non-fatal findings produced while selecting routes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<PlannerWarning>,
    /// Reachability checks for external instances, performed by `up` after managed
    /// services have become healthy.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub external_probes: Vec<ExternalProbePlan>,
    /// Apply-time secret bindings, deliberately excluded from serialization.
    #[serde(skip)]
    pub runtime_secrets: Vec<RuntimeSecretPlan>,
    /// Whether explicit overlays, a variation, or ephemeral values participated.
    #[serde(skip)]
    pub has_overrides: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalProbePlan {
    pub instance: String,
    pub host: String,
    pub probe: Probe,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarPlan {
    pub service: String,
    pub admin_socket: PathBuf,
    pub config_path: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedProfilePlan {
    pub api_version: String,
    pub deployment: String,
    pub ui: String,
    pub route: String,
    pub proxy_address: String,
    pub start_url: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostUpstreamPlan {
    pub compose_service: String,
    pub container_port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_address: Option<String>,
}

/// SSH/Docker connection fields supplied by the caller without coupling the planner to state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanningDevice {
    pub user: String,
    pub host: String,
    pub port: u16,
    pub identity_file: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteComposePlan {
    pub device: PlanningDevice,
    pub compose_project: String,
    pub compose_file: PathBuf,
    pub compose_yaml: String,
    pub services: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DocumentVersion {
    api_version: String,
}

type GroupMemberMap = BTreeMap<String, Vec<String>>;

struct ResolvedGroups {
    members: GroupMemberMap,
    declared_members: GroupMemberMap,
}

struct TransparentRoute<'a> {
    enabled: bool,
    group: Option<&'a str>,
    groups: &'a ResolvedGroups,
}

struct ValidationResult {
    groups: ResolvedGroups,
    warnings: Vec<PlannerWarning>,
}

/// Loads one self-contained deployment bundle without changing runtime state.
pub fn load_bundle(path: &Path) -> Result<Bundle, PlannerError> {
    let input = fs::read_to_string(path)?;
    load_bundle_from_str(&input, path)
}

/// Loads one deployment bundle from an in-memory draft using `path` for relative paths.
pub fn load_bundle_from_str(input: &str, path: &Path) -> Result<Bundle, PlannerError> {
    let version: DocumentVersion = serde_yaml::from_str(input)?;
    if version.api_version == "switchyard.dev/v1alpha1" {
        return Err(PlannerError::MigrationRequired(version.api_version));
    }
    let mut bundle: Bundle = serde_yaml::from_str(input)?;
    bundle.definition_dir = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .canonicalize()?;
    bundle.workspace_root = bundle
        .definition_dir
        .ancestors()
        .find(|ancestor| ancestor.join(".git").exists())
        .unwrap_or(&bundle.definition_dir)
        .to_owned();
    Ok(bundle)
}

/// Produces a deterministic Compose document and recovery artifacts without writing them.
pub fn plan(bundle: &Bundle) -> Result<Plan, Vec<Diagnostic>> {
    plan_with_devices(bundle, &BTreeMap::new())
}

pub fn plan_with_devices(
    bundle: &Bundle,
    devices: &BTreeMap<String, PlanningDevice>,
) -> Result<Plan, Vec<Diagnostic>> {
    if !bundle.spec.overlays.is_empty() {
        return plan_with_overlays_and_devices(bundle, &OverlayOptions::default(), devices);
    }
    let effective = with_derived_host_routing(bundle);
    let validation = validate(&effective, devices)?;
    generate(
        &effective,
        &validation.groups,
        validation.warnings,
        None,
        devices,
    )
    .map_err(|error| {
        vec![Diagnostic::new(
            DiagnosticCode::InvalidPath,
            "$",
            error.to_string(),
        )]
    })
}

fn with_derived_host_routing(bundle: &Bundle) -> Bundle {
    let mut effective = bundle.clone();
    let has_addresses = effective
        .spec
        .instances
        .iter()
        .any(|instance| instance.address.is_some())
        || effective
            .spec
            .groups
            .values()
            .any(|group| group.address.is_some());
    if effective.spec.host_router.is_some() || !has_addresses {
        return effective;
    }
    let browser_members = effective
        .spec
        .instances
        .iter()
        .filter(|instance| instance.address.is_some())
        .map(|instance| instance.name.as_str())
        .chain(effective.spec.groups.values().flat_map(|group| {
            group.instances.iter().filter_map(|member| {
                let instance = provider_reference(member).0;
                (!group.disabled.iter().any(|disabled| disabled == instance)).then_some(instance)
            })
        }))
        .collect::<BTreeSet<_>>();
    for instance in effective.spec.instances.iter().filter(|instance| {
        !instance.is_external() && browser_members.contains(instance.name.as_str())
    }) {
        let Some(block) = effective.spec.blocks.get(&instance.block) else {
            continue;
        };
        let candidates = block
            .services
            .iter()
            .filter_map(|(service_name, service)| match &service.probe {
                Some(Probe::Http { path, port, https }) if service.publish.contains(port) => {
                    Some((service_name, service, path, *port, *https))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        for (service_name, _, _, port, _) in candidates {
            if effective.spec.host_upstreams.values().any(|upstream| {
                upstream.instance == instance.name && upstream.service == *service_name
            }) {
                continue;
            }
            let preferred = if block
                .services
                .values()
                .filter(|service| matches!(service.probe, Some(Probe::Http { .. })))
                .count()
                == 1
            {
                instance.name.clone()
            } else {
                resource_name(&[&instance.name, service_name])
            };
            let mut provider = preferred.clone();
            let mut suffix = 2;
            while effective.spec.host_upstreams.contains_key(&provider) {
                provider = format!("{preferred}-{suffix}");
                suffix += 1;
            }
            effective.spec.host_upstreams.insert(
                provider,
                PublishedUpstream {
                    instance: instance.name.clone(),
                    service: service_name.clone(),
                    port,
                },
            );
        }
    }

    effective.spec.host_router = Some(derived_host_router(&effective));
    effective
}

fn derived_host_router(bundle: &Bundle) -> router_config::RouterConfig {
    use router_config::{
        BrowserIdentity, BrowserRoute, ComponentId, ConfigMetadata, ConnectionTransitionPolicies,
        ConnectionTransitionPolicy, HealthCheck, HealthCheckProtocol, IdentityPolicy, InstanceId,
        Listener, ListenerDestination, Protocol, Provider, Route, RouteSlotId, RouteSnapshot,
        RouteSnapshotId, RouterConfig, RouterSpec, SocketAddress, UpstreamEndpoint,
    };

    let browser_targets = bundle
        .spec
        .instances
        .iter()
        .filter(|instance| instance.address.is_some())
        .map(|instance| instance.name.as_str())
        .collect::<BTreeSet<_>>();
    let mut providers = Vec::new();
    let mut routes = Vec::new();
    let mut upstream_by_instance_port = BTreeMap::<(String, u16), String>::new();
    for (provider, upstream) in &bundle.spec.host_upstreams {
        let service = bundle
            .spec
            .instances
            .iter()
            .find(|instance| instance.name == upstream.instance)
            .and_then(|instance| bundle.spec.blocks.get(&instance.block))
            .and_then(|block| block.services.get(&upstream.service));
        let (protocol, health_check) = match service.and_then(|service| service.probe.as_ref()) {
            Some(Probe::Http { path, https, .. }) => (
                if *https {
                    Protocol::Https
                } else {
                    Protocol::Http
                },
                Some(HealthCheck {
                    protocol: if *https {
                        HealthCheckProtocol::Https
                    } else {
                        HealthCheckProtocol::Http
                    },
                    path: Some(path.clone()),
                    interval_ms: 1_000,
                    timeout_ms: 500,
                }),
            ),
            _ => (Protocol::Http, None),
        };
        providers.push(Provider {
            id: ComponentId::from(provider.as_str()),
            endpoint: UpstreamEndpoint {
                protocol,
                host: "127.0.0.1".into(),
                port: 0,
            },
            health_check,
            receive_identity_header: false,
        });
        if browser_targets.contains(upstream.instance.as_str()) {
            routes.push(Route {
                consumer: InstanceId::from("gateway"),
                slot: RouteSlotId::from(resource_name(&["host", provider]).as_str()),
                provider: ComponentId::from(provider.as_str()),
            });
        }
        upstream_by_instance_port
            .insert((upstream.instance.clone(), upstream.port), provider.clone());
    }

    let gateway_digest = Sha256::digest(bundle.metadata.name.as_bytes());
    let gateway_port = 18_000 + u16::from_be_bytes([gateway_digest[0], gateway_digest[1]]) % 2_000;
    let mut listeners = vec![Listener {
        consumer: Some(InstanceId::from("gateway")),
        bind: SocketAddress {
            host: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            port: gateway_port,
        },
        protocol: Protocol::Http,
        tls: None,
        destinations: Vec::new(),
        proxy_identity: None,
        proxy_authentication: None,
    }];
    let mut browser_routes = Vec::new();
    let instances = bundle
        .spec
        .instances
        .iter()
        .map(|instance| (instance.name.as_str(), instance))
        .collect::<BTreeMap<_, _>>();
    let mut browser_ports = BTreeSet::new();
    let candidate_ports = upstream_by_instance_port
        .keys()
        .map(|(_, port)| *port)
        .collect::<BTreeSet<_>>();
    for group in bundle.spec.groups.values() {
        let active = group
            .instances
            .iter()
            .filter(|member| {
                !group
                    .disabled
                    .iter()
                    .any(|name| name == provider_reference(member).0)
            })
            .collect::<Vec<_>>();
        for port in &candidate_ports {
            let winner = active.iter().find_map(|member| {
                let (instance_name, requested_service) = provider_reference(member);
                let candidate = bundle.spec.host_upstreams.iter().find(|(_, upstream)| {
                    upstream.instance == instance_name
                        && upstream.port == *port
                        && requested_service.is_none_or(|service| upstream.service == service)
                });
                candidate.map(|(id, _)| id)
            });
            let Some(winner) = winner else { continue };
            browser_ports.insert(*port);
            for member in &active {
                let identity = provider_reference(member).0;
                if !instances.contains_key(identity) {
                    continue;
                }
                let route = BrowserRoute {
                    identity: BrowserIdentity::ExplicitHeader {
                        value: router_config::BindingId::from(identity),
                    },
                    destination: RouteSlotId::from(format!("browser-{port}").as_str()),
                    provider: ComponentId::from(winner.as_str()),
                };
                if !browser_routes.iter().any(|existing: &BrowserRoute| {
                    existing.identity == route.identity && existing.destination == route.destination
                }) {
                    browser_routes.push(route);
                }
            }
        }
    }
    for port in browser_ports {
        listeners.push(Listener {
            consumer: None,
            bind: SocketAddress {
                host: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                port,
            },
            protocol: Protocol::Http,
            tls: None,
            destinations: vec![ListenerDestination::LegacyLocalhost {
                slot: RouteSlotId::from(format!("browser-{port}").as_str()),
                host: "localhost".into(),
            }],
            proxy_identity: None,
            proxy_authentication: None,
        });
    }

    RouterConfig {
        api_version: router_config::API_VERSION.into(),
        kind: router_config::KIND.into(),
        metadata: ConfigMetadata {
            deployment: router_config::DeploymentId::from(bundle.metadata.name.as_str()),
        },
        spec: RouterSpec {
            snapshot: RouteSnapshot {
                id: RouteSnapshotId::from(
                    resource_name(&[&bundle.metadata.name, "host", "initial"]).as_str(),
                ),
                version: 1,
                transitions: ConnectionTransitionPolicies {
                    http: ConnectionTransitionPolicy::Drain { timeout_ms: 5_000 },
                    https: ConnectionTransitionPolicy::Drain { timeout_ms: 5_000 },
                    websocket: ConnectionTransitionPolicy::Pin,
                    grpc: ConnectionTransitionPolicy::Drain { timeout_ms: 5_000 },
                    tcp: ConnectionTransitionPolicy::Close,
                },
            },
            exposure: None,
            listeners,
            providers,
            groups: Vec::new(),
            bindings: Vec::new(),
            routes,
            browser_routes,
            transparent_proxy: None,
            identity: IdentityPolicy::default(),
        },
    }
}

/// Validates a reusable block with the same contracts used by deployment planning.
pub fn validate_block(name: &str, block: &Block) -> Result<(), Vec<Diagnostic>> {
    let mut errors = Vec::new();
    validate_name(name, format!("spec.blocks.{name}"), &mut errors);
    if block.services.is_empty() {
        errors.push(Diagnostic::new(
            DiagnosticCode::MissingReference,
            format!("spec.blocks.{name}.services"),
            "a block must contain at least one service",
        ));
    }
    let adapters = built_in_registry();
    for (service_name, service) in &block.services {
        validate_name(
            service_name,
            format!("spec.blocks.{name}.services.{service_name}"),
            &mut errors,
        );
        validate_execution(name, service_name, service, &adapters, &mut errors);
        if let Some(probe) = &service.probe {
            validate_probe(name, service_name, probe, &adapters, &mut errors);
        }
        for volume in &service.volumes {
            validate_name(
                &volume.name,
                format!("spec.blocks.{name}.services.{service_name}.volumes"),
                &mut errors,
            );
            if !volume.target.is_absolute() {
                errors.push(Diagnostic::new(
                    DiagnosticCode::InvalidPath,
                    format!("spec.blocks.{name}.services.{service_name}.volumes"),
                    "volume target must be an absolute container path",
                ));
            }
        }
    }
    validate_local_dependencies(name, block, &mut errors);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Replans after moving one instance into a service group.
pub fn plan_with_membership(
    bundle: &Bundle,
    instance: &str,
    group: &str,
) -> Result<Plan, Vec<Diagnostic>> {
    plan_with_membership_and_devices(bundle, instance, group, &BTreeMap::new())
}

pub fn plan_with_membership_and_devices(
    bundle: &Bundle,
    instance: &str,
    group: &str,
    devices: &BTreeMap<String, PlanningDevice>,
) -> Result<Plan, Vec<Diagnostic>> {
    let mut errors = Vec::new();
    if !bundle
        .spec
        .instances
        .iter()
        .any(|candidate| candidate.name == instance)
    {
        errors.push(Diagnostic::new(
            DiagnosticCode::MissingReference,
            "spec.instances",
            format!("instance `{instance}` does not exist"),
        ));
    }
    if !bundle.spec.groups.contains_key(group) {
        errors.push(Diagnostic::new(
            DiagnosticCode::MissingReference,
            "spec.groups",
            format!("group `{group}` does not exist"),
        ));
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut updated = bundle.clone();
    move_instance_to_group(&mut updated, instance, group);
    plan_with_devices(&updated, devices)
}

fn move_instance_to_group(bundle: &mut Bundle, instance: &str, target_group: &str) {
    let mut moved_member = None;
    for group in bundle.spec.groups.values_mut() {
        group.instances.retain(|member| {
            if provider_reference(member).0 == instance {
                if moved_member.is_none() {
                    moved_member = Some(member.clone());
                }
                false
            } else {
                true
            }
        });
        group.disabled.retain(|member| member != instance);
    }
    if let Some(group) = bundle.spec.groups.get_mut(target_group) {
        group
            .instances
            .push(moved_member.unwrap_or_else(|| instance.to_owned()));
    }
}

/// Atomically writes disposable artifacts beneath `.switchyard/generated/<deployment>`.
pub fn write_plan(workspace_root: &Path, plan: &Plan) -> io::Result<PathBuf> {
    let artifact_dir = workspace_root.join(&plan.artifact_dir);
    fs::create_dir_all(artifact_dir.join("routes"))?;
    let overlays_dir = artifact_dir.join("overlays");
    if overlays_dir.exists() {
        fs::remove_dir_all(&overlays_dir)?;
    }
    overlay::materialize_injected_files(&artifact_dir, &plan.injected_files)?;
    write_atomic(
        &artifact_dir.join("compose.yaml"),
        plan.compose_yaml.as_bytes(),
    )?;
    for remote in plan.remote_projects.values() {
        write_atomic(
            &artifact_dir.join(&remote.compose_file),
            remote.compose_yaml.as_bytes(),
        )?;
    }
    write_atomic(
        &artifact_dir.join("resolved-deployment.yaml"),
        plan.resolved_deployment_yaml.as_bytes(),
    )?;
    write_atomic(
        &artifact_dir.join("manifest.json"),
        plan.manifest_json.as_bytes(),
    )?;
    for (consumer, config) in &plan.route_configs {
        write_atomic(
            &artifact_dir.join("routes").join(format!("{consumer}.json")),
            config.as_bytes(),
        )?;
    }
    let managed_profiles_dir = artifact_dir.join("managed-profiles");
    if managed_profiles_dir.exists() {
        fs::remove_dir_all(&managed_profiles_dir)?;
    }
    if !plan.managed_profiles.is_empty() {
        fs::create_dir_all(&managed_profiles_dir)?;
        for (ui, profile) in &plan.managed_profiles {
            let encoded = serde_json::to_vec_pretty(profile).map_err(io::Error::other)?;
            write_atomic(&managed_profiles_dir.join(format!("{ui}.json")), &encoded)?;
        }
    }
    if let Some(config) = &plan.host_router_config {
        write_atomic(&artifact_dir.join("host-router.json"), config.as_bytes())?;
    } else {
        match fs::remove_file(artifact_dir.join("host-router.json")) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(artifact_dir)
}

/// Returns deterministic Docker resource names that a generated plan would claim.
pub fn planned_docker_resource_names(plan: &Plan) -> BTreeMap<String, BTreeSet<String>> {
    let mut names = BTreeMap::<String, BTreeSet<String>>::new();
    let Ok(compose) = serde_yaml::from_str::<Value>(&plan.compose_yaml) else {
        return names;
    };
    if let Some(services) = compose.get("services").and_then(Value::as_object) {
        for service in services.values() {
            if let Some(name) = service.get("container_name").and_then(Value::as_str) {
                names
                    .entry("container".into())
                    .or_default()
                    .insert(name.to_owned());
            }
        }
    }
    for (section, kind) in [("networks", "network"), ("volumes", "volume")] {
        if let Some(values) = compose.get(section).and_then(Value::as_object) {
            for value in values.values() {
                if let Some(name) = value.get("name").and_then(Value::as_str) {
                    names
                        .entry(kind.into())
                        .or_default()
                        .insert(name.to_owned());
                }
            }
        }
    }
    names
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, path)
}

fn validate(
    bundle: &Bundle,
    devices: &BTreeMap<String, PlanningDevice>,
) -> Result<ValidationResult, Vec<Diagnostic>> {
    let mut errors = Vec::new();
    let mut warnings = Vec::<PlannerWarning>::new();
    if bundle.api_version != API_VERSION || bundle.kind != KIND {
        errors.push(Diagnostic::new(
            DiagnosticCode::UnsupportedSchema,
            "apiVersion",
            format!("expected {API_VERSION} kind {KIND}"),
        ));
    }
    validate_name(&bundle.metadata.name, "metadata.name", &mut errors);

    let adapters = built_in_registry();
    let generated_definition = bundle
        .definition_dir
        .components()
        .any(|component| component.as_os_str() == "generated")
        && bundle
            .definition_dir
            .components()
            .any(|component| component.as_os_str() == ".switchyard");

    let source_manager = SourceManager::new(&bundle.workspace_root);
    let mut repository_paths = BTreeMap::new();
    for (name, repository) in &bundle.spec.repositories {
        validate_name(name, format!("spec.repositories.{name}"), &mut errors);
        match (&repository.url, &repository.clone) {
            (Some(url), None) if !url.trim().is_empty() => {
                repository_paths.insert(name.as_str(), source_manager.clone_root().join(name));
            }
            (None, Some(path)) => {
                repository_paths.insert(name.as_str(), resolve_path(&bundle.definition_dir, path));
            }
            (Some(_), None) => errors.push(Diagnostic::new(
                DiagnosticCode::InvalidPath,
                format!("spec.repositories.{name}.url"),
                "repository URL must not be empty",
            )),
            _ => errors.push(Diagnostic::new(
                DiagnosticCode::UnsupportedSchema,
                format!("spec.repositories.{name}"),
                "exactly one of `url` or `clone` is required",
            )),
        }
    }

    let mut source_paths = BTreeMap::<PathBuf, &str>::new();
    for (name, source) in &bundle.spec.sources {
        validate_name(name, format!("spec.sources.{name}"), &mut errors);
        let (adapter_id, configuration) = source_adapter_configuration(source);
        if let Some(adapter) = adapters.lookup(AdapterKind::Source, adapter_id) {
            extend_adapter_diagnostics(
                &mut errors,
                adapter.adapter().validate_configuration(&configuration),
                DiagnosticCode::InvalidPath,
                &format!("spec.sources.{name}"),
            );
        } else {
            errors.push(Diagnostic::new(
                DiagnosticCode::UnsupportedSchema,
                format!("spec.sources.{name}"),
                format!("built-in adapter {adapter_id} is not registered"),
            ));
        }
        if source.path.is_absolute() && !generated_definition {
            errors.push(Diagnostic::new(
                DiagnosticCode::InvalidPath,
                format!("spec.sources.{name}.path"),
                "source path must be relative to the deployment file",
            ));
        }
        let path = normalize_path(&resolve_path(&bundle.definition_dir, &source.path));
        if !generated_definition && !path.starts_with(&bundle.workspace_root) {
            errors.push(Diagnostic::new(
                DiagnosticCode::InvalidPath,
                format!("spec.sources.{name}.path"),
                format!(
                    "source path `{}` escapes project directory `{}`",
                    source.path.display(),
                    bundle.workspace_root.display()
                ),
            ));
        }
        if let Some(previous) = source_paths.insert(path.clone(), name) {
            errors.push(Diagnostic::new(
                DiagnosticCode::DuplicateName,
                format!("spec.sources.{name}.path"),
                format!("source path is already used by `{previous}`"),
            ));
        }
        if source.r#ref.trim().is_empty() {
            errors.push(Diagnostic::new(
                DiagnosticCode::MissingReference,
                format!("spec.sources.{name}.ref"),
                "source ref must not be empty",
            ));
        }
        match repository_paths.get(source.repository.as_str()) {
            None => errors.push(Diagnostic::new(
                DiagnosticCode::MissingReference,
                format!("spec.sources.{name}.repository"),
                format!("unknown repository `{}`", source.repository),
            )),
            Some(repository_path) if paths_overlap(&path, &normalize_path(repository_path)) => {
                errors.push(Diagnostic::new(
                    DiagnosticCode::InvalidPath,
                    format!("spec.sources.{name}.path"),
                    "source worktree and repository clone may not be the same directory or contain one another",
                ));
            }
            Some(_) => {}
        }
    }

    for (block_name, block) in &bundle.spec.blocks {
        if let Err(block_errors) = validate_block(block_name, block) {
            errors.extend(block_errors);
        }
    }

    let mut instances = BTreeMap::new();
    for (index, instance) in bundle.spec.instances.iter().enumerate() {
        let path = format!("spec.instances[{index}]");
        validate_name(&instance.name, format!("{path}.name"), &mut errors);
        if instances.insert(instance.name.as_str(), instance).is_some() {
            errors.push(Diagnostic::new(
                DiagnosticCode::DuplicateName,
                format!("{path}.name"),
                "instance name is declared more than once",
            ));
        }
        if instance.is_external() {
            validate_external_instance(instance, &path, &adapters, &mut errors);
            continue;
        }
        if !instance.ports.is_empty() || instance.probe.is_some() {
            errors.push(Diagnostic::new(
                DiagnosticCode::UnsupportedSchema,
                &path,
                "`ports` and instance-level `probe` are valid only with `external`",
            ));
        }
        let Some(block) = bundle.spec.blocks.get(&instance.block) else {
            errors.push(Diagnostic::new(
                DiagnosticCode::MissingReference,
                format!("{path}.block"),
                format!("unknown block {}", instance.block),
            ));
            continue;
        };
        if let Some(device) = instance
            .device
            .as_deref()
            .filter(|device| *device != "local")
        {
            validate_name(device, format!("{path}.device"), &mut errors);
            if !devices.contains_key(device) {
                errors.push(Diagnostic::new(
                    DiagnosticCode::MissingReference,
                    format!("{path}.device"),
                    format!(
                        "instance `{}` references unregistered device `{device}`",
                        instance.name
                    ),
                ));
            }
            for (service_name, service) in &block.services {
                if !matches!(service.execution, Execution::Container { .. }) {
                    errors.push(Diagnostic::new(
                        DiagnosticCode::UnsupportedSchema,
                        format!(
                            "spec.blocks.{}.services.{service_name}.execution",
                            instance.block
                        ),
                        format!(
                            "remote instance `{}` only supports container execution",
                            instance.name
                        ),
                    ));
                }
                let _ = service_name;
            }
        }
        if !bundle.spec.sources.contains_key(&instance.source) {
            errors.push(Diagnostic::new(
                DiagnosticCode::MissingReference,
                format!("{path}.source"),
                format!("unknown source {}", instance.source),
            ));
        }
        for (name, parameter) in &block.parameters {
            if parameter.required
                && parameter.default.is_none()
                && !instance.parameters.contains_key(name)
            {
                errors.push(Diagnostic::new(
                    DiagnosticCode::MissingVariable,
                    format!("{path}.parameters.{name}"),
                    "required block parameter has no value",
                ));
            }
        }
        if let Some(source) = bundle.spec.sources.get(&instance.source) {
            let source_path = resolve_path(&bundle.definition_dir, &source.path);
            if !source_path.exists() {
                continue;
            }
            for (service_name, service) in &block.services {
                let relative = match &service.execution {
                    Execution::Container {
                        build: Some(build), ..
                    } => Some((&build.context, "build context")),
                    Execution::ProcessCompose { file, .. } => Some((file, "Process Compose file")),
                    _ => None,
                };
                if let Some((relative, description)) = relative {
                    let resolved = source_path.join(relative);
                    if !resolved.exists() {
                        errors.push(Diagnostic::new(
                            DiagnosticCode::InvalidPath,
                            format!(
                                "spec.blocks.{}.services.{service_name}.execution",
                                instance.block
                            ),
                            format!("{description} does not exist: {}", resolved.display()),
                        ));
                    }
                }
            }
        }
    }

    let resolved_groups = resolve_groups(bundle, &instances, &mut errors);
    validate_expanded_dependencies(bundle, &instances, &mut errors);
    validate_address_claims(bundle, &mut errors);
    let has_addresses = bundle
        .spec
        .instances
        .iter()
        .any(|instance| instance.address.is_some())
        || bundle
            .spec
            .groups
            .values()
            .any(|group| group.address.is_some());
    let mut effective_host_router = bundle.spec.host_router.clone();
    match &mut effective_host_router {
        Some(config) => apply_addresses(bundle, &instances, &resolved_groups, config, &mut errors),
        None if has_addresses => errors.push(Diagnostic::new(
            DiagnosticCode::MissingReference,
            "spec.hostRouter",
            "group and instance addresses require spec.hostRouter",
        )),
        None => {}
    }
    for (instance_name, profile) in &bundle.spec.managed_profiles {
        let path = format!("spec.managedProfiles.{instance_name}");
        validate_name(instance_name, &path, &mut errors);
        validate_name(&profile.route, format!("{path}.route"), &mut errors);
        if !instances.contains_key(instance_name.as_str()) {
            errors.push(Diagnostic::new(
                DiagnosticCode::MissingReference,
                &path,
                "managed profile key must name a declared instance",
            ));
        }
        let valid_start_url = is_local_http_url(&profile.start_url);
        if !valid_start_url {
            errors.push(Diagnostic::new(
                DiagnosticCode::InvalidPath,
                format!("{path}.startUrl"),
                "managed profiles currently require a local http:// URL (localhost, loopback, or *.localhost); use Origin or explicit-header routing for HTTPS",
            ));
        }
        let Some(host_router) = &effective_host_router else {
            errors.push(Diagnostic::new(
                DiagnosticCode::MissingReference,
                &path,
                "managed profiles require spec.hostRouter",
            ));
            continue;
        };
        if valid_start_url {
            if let Err(message) =
                managed_profile_destinations(host_router, &profile.route, &profile.start_url)
            {
                errors.push(Diagnostic::new(
                    DiagnosticCode::MissingReference,
                    format!("{path}.route"),
                    message,
                ));
            }
        }
    }
    if let Some(host_router) = &effective_host_router {
        if host_router.metadata.deployment.as_str() != bundle.metadata.name {
            errors.push(Diagnostic::new(
                DiagnosticCode::MissingReference,
                "spec.hostRouter.metadata.deployment",
                "host router deployment must match deployment metadata.name",
            ));
        }
        if let Err(router_errors) = host_router.validate() {
            errors.extend(router_errors.into_iter().map(|error| {
                Diagnostic::new(
                    DiagnosticCode::MissingReference,
                    format!("spec.hostRouter.{}", error.path),
                    error.message,
                )
            }));
        }
        validate_host_upstreams(bundle, host_router, &instances, &mut errors);
    } else if !bundle.spec.host_upstreams.is_empty() {
        errors.push(Diagnostic::new(
            DiagnosticCode::MissingReference,
            "spec.hostUpstreams",
            "published host upstreams require spec.hostRouter",
        ));
    }

    if errors.is_empty() {
        warnings
            .sort_by(|left, right| (&left.path, &left.message).cmp(&(&right.path, &right.message)));
        warnings.dedup();
        Ok(ValidationResult {
            groups: resolved_groups,
            warnings,
        })
    } else {
        Err(errors)
    }
}

fn validate_execution(
    block_name: &str,
    service_name: &str,
    service: &Service,
    adapters: &switchyard_adapter_sdk::AdapterRegistry,
    errors: &mut Vec<Diagnostic>,
) {
    let path = format!("spec.blocks.{block_name}.services.{service_name}.execution");
    let (kind, adapter_id, configuration) = execution_adapter_configuration(&service.execution);
    let Some(adapter) = adapters.lookup(kind, adapter_id) else {
        errors.push(Diagnostic::new(
            DiagnosticCode::UnsupportedSchema,
            path,
            format!("built-in adapter {adapter_id} is not registered"),
        ));
        return;
    };
    for diagnostic in adapter.adapter().validate_configuration(&configuration) {
        let code = match diagnostic.code.as_str() {
            "adapter_missing_reference" | "adapter_config_schema"
                if matches!(
                    service.execution,
                    Execution::Container { .. } | Execution::Script { .. }
                ) =>
            {
                DiagnosticCode::MissingReference
            }
            _ => DiagnosticCode::InvalidPath,
        };
        errors.push(Diagnostic::new(code, &path, diagnostic.message));
    }
}

fn source_adapter_configuration(source: &Source) -> (&'static str, Value) {
    (
        "source-git",
        json!({
            "path": source.path.to_string_lossy(),
            "repository": source.repository,
            "ref": source.r#ref,
        }),
    )
}

fn normalize_path(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

fn execution_adapter_configuration(execution: &Execution) -> (AdapterKind, &'static str, Value) {
    match execution {
        Execution::Container {
            image,
            build,
            command,
            working_directory,
            environment,
        } => (
            AdapterKind::Execution,
            "execution-container",
            json!({
                "image": image,
                "build": build.as_ref().map(|build| json!({
                    "context": build.context.to_string_lossy(),
                    "dockerfile": build.dockerfile.as_ref().map(|path| path.to_string_lossy()),
                })),
                "command": command,
                "workingDirectory": working_directory.as_ref().map(|path| path.to_string_lossy()),
                "environment": environment,
            }),
        ),
        Execution::Script {
            image,
            command,
            working_directory,
            source_mount,
            writable,
            environment,
            lifecycle,
        } => (
            AdapterKind::Execution,
            "execution-runner-script",
            json!({
                "image": image,
                "command": command,
                "workingDirectory": working_directory.as_ref().map(|path| path.to_string_lossy()),
                "sourceMount": source_mount.to_string_lossy(),
                "writable": writable,
                "environment": environment,
                "lifecycle": match lifecycle {
                    ScriptLifecycle::Service => "service",
                    ScriptLifecycle::Task => "task",
                },
            }),
        ),
        Execution::ProcessCompose {
            image,
            file,
            working_directory,
            source_mount,
            writable,
            environment,
        } => (
            AdapterKind::Supervisor,
            "supervisor-process-compose",
            json!({
                "image": image,
                "file": file.to_string_lossy(),
                "workingDirectory": working_directory.as_ref().map(|path| path.to_string_lossy()),
                "sourceMount": source_mount.to_string_lossy(),
                "writable": writable,
                "environment": environment,
                "children": [],
            }),
        ),
    }
}

fn validate_probe(
    block_name: &str,
    service_name: &str,
    probe: &Probe,
    adapters: &switchyard_adapter_sdk::AdapterRegistry,
    errors: &mut Vec<Diagnostic>,
) {
    let path = format!("spec.blocks.{block_name}.services.{service_name}.probe");
    let configuration = match probe {
        Probe::Http { path, port, https } => {
            json!({ "type": "http", "path": path, "port": port, "https": https })
        }
        Probe::Tcp { port } => json!({ "type": "tcp", "port": port }),
        Probe::Command { command } => json!({ "type": "command", "command": command }),
    };
    match adapters.lookup(AdapterKind::Probe, "probe-health") {
        Some(adapter) => extend_adapter_diagnostics(
            errors,
            adapter.adapter().validate_configuration(&configuration),
            DiagnosticCode::MissingReference,
            &path,
        ),
        None => errors.push(Diagnostic::new(
            DiagnosticCode::UnsupportedSchema,
            path,
            "built-in adapter probe-health is not registered",
        )),
    }
}

fn validate_external_instance(
    instance: &Instance,
    path: &str,
    adapters: &switchyard_adapter_sdk::AdapterRegistry,
    errors: &mut Vec<Diagnostic>,
) {
    let host = instance.external.as_deref().unwrap_or_default();
    if host.trim().is_empty()
        || !(plausible_hostname(host) || host.parse::<std::net::IpAddr>().is_ok())
    {
        errors.push(Diagnostic::new(
            DiagnosticCode::InvalidPath,
            format!("{path}.external"),
            "external must be a nonempty hostname or IP address without a port",
        ));
    }
    if !instance.block.is_empty()
        || !instance.source.is_empty()
        || instance.device.is_some()
        || instance.address.is_some()
        || !instance.parameters.is_empty()
        || !instance.environment.is_empty()
        || !instance.environment_unset.is_empty()
    {
        errors.push(Diagnostic::new(
            DiagnosticCode::UnsupportedSchema,
            path,
            "an external instance may contain only `name`, `external`, `ports`, optional `probe`, and labels",
        ));
    }
    if instance.ports.is_empty() {
        errors.push(Diagnostic::new(
            DiagnosticCode::MissingReference,
            format!("{path}.ports"),
            "an external instance must declare at least one port",
        ));
    }
    let mut expanded = BTreeSet::new();
    for (index, port) in instance.ports.iter().enumerate() {
        let port_path = format!("{path}.ports[{index}]");
        let (start, end) = match port {
            ExternalPort::Single(port) => (*port, *port),
            ExternalPort::Range { start, end } => (*start, *end),
        };
        if start == 0 || end == 0 {
            errors.push(Diagnostic::new(
                DiagnosticCode::InvalidPath,
                &port_path,
                "external ports must be between 1 and 65535",
            ));
            continue;
        }
        if start > end {
            errors.push(Diagnostic::new(
                DiagnosticCode::InvalidPath,
                &port_path,
                format!("external port range start {start} exceeds end {end}"),
            ));
            continue;
        }
        if u32::from(end) - u32::from(start) + 1 > 1024 {
            errors.push(Diagnostic::new(
                DiagnosticCode::InvalidPath,
                &port_path,
                "external port ranges may contain at most 1024 ports",
            ));
            continue;
        }
        for value in start..=end {
            if !expanded.insert(value) {
                errors.push(Diagnostic::new(
                    DiagnosticCode::DuplicateName,
                    &port_path,
                    format!("external port {value} is declared more than once"),
                ));
            }
        }
    }
    if let Some(probe) = &instance.probe {
        let probe_path = format!("{path}.probe");
        let configuration = match probe {
            Probe::Http { path, port, https } => {
                json!({ "type": "http", "path": path, "port": port, "https": https })
            }
            Probe::Tcp { port } => json!({ "type": "tcp", "port": port }),
            Probe::Command { command } => json!({ "type": "command", "command": command }),
        };
        match adapters.lookup(AdapterKind::Probe, "probe-health") {
            Some(adapter) => extend_adapter_diagnostics(
                errors,
                adapter.adapter().validate_configuration(&configuration),
                DiagnosticCode::MissingReference,
                &probe_path,
            ),
            None => errors.push(Diagnostic::new(
                DiagnosticCode::UnsupportedSchema,
                probe_path,
                "built-in adapter probe-health is not registered",
            )),
        }
        let probe_port = match probe {
            Probe::Http { port, .. } | Probe::Tcp { port } => Some(*port),
            Probe::Command { .. } => None,
        };
        if probe_port.is_some_and(|port| !expanded.contains(&port)) {
            errors.push(Diagnostic::new(
                DiagnosticCode::MissingReference,
                format!("{path}.probe.port"),
                "external probe port must be included in the instance `ports` list",
            ));
        }
    }
}

fn extend_adapter_diagnostics(
    errors: &mut Vec<Diagnostic>,
    diagnostics: Vec<AdapterDiagnostic>,
    code: DiagnosticCode,
    path: &str,
) {
    errors.extend(
        diagnostics
            .into_iter()
            .map(|diagnostic| Diagnostic::new(code.clone(), path, diagnostic.message)),
    );
}

fn validate_host_upstreams(
    bundle: &Bundle,
    host_router: &router_config::RouterConfig,
    instances: &BTreeMap<&str, &Instance>,
    errors: &mut Vec<Diagnostic>,
) {
    for provider in &host_router.spec.providers {
        let id = provider.id.as_str();
        let loopback = provider.endpoint.host.eq_ignore_ascii_case("localhost")
            || provider
                .endpoint
                .host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback());
        if !loopback {
            errors.push(Diagnostic::new(
                DiagnosticCode::InvalidPath,
                format!("spec.hostRouter.providers.{id}.endpoint.host"),
                "host-router providers must use localhost or a loopback IP address",
            ));
        }
        if provider.endpoint.port == 0 && !bundle.spec.host_upstreams.contains_key(id) {
            errors.push(Diagnostic::new(
                DiagnosticCode::MissingReference,
                format!("spec.hostRouter.providers.{id}"),
                "provider port 0 requires one spec.hostUpstreams mapping",
            ));
        }
    }
    for (provider, upstream) in &bundle.spec.host_upstreams {
        let path = format!("spec.hostUpstreams.{provider}");
        if upstream.port == 0 {
            errors.push(Diagnostic::new(
                DiagnosticCode::InvalidPath,
                format!("{path}.port"),
                "mapped container port must be nonzero",
            ));
        }
        if !host_router
            .spec
            .providers
            .iter()
            .any(|candidate| candidate.id.as_str() == provider)
        {
            errors.push(Diagnostic::new(
                DiagnosticCode::MissingReference,
                &path,
                "mapping refers to an unknown host-router provider",
            ));
        }
        let Some(instance) = instances.get(upstream.instance.as_str()) else {
            errors.push(Diagnostic::new(
                DiagnosticCode::MissingReference,
                format!("{path}.instance"),
                "mapping refers to an unknown instance",
            ));
            continue;
        };
        if instance.is_external() {
            errors.push(Diagnostic::new(
                DiagnosticCode::UnsupportedSchema,
                format!("{path}.instance"),
                "external instances are routed by group membership and may not be host upstreams",
            ));
            continue;
        }
        let Some(service) = bundle.spec.blocks[&instance.block]
            .services
            .get(&upstream.service)
        else {
            errors.push(Diagnostic::new(
                DiagnosticCode::MissingReference,
                format!("{path}.service"),
                "mapping refers to an unknown instance service",
            ));
            continue;
        };
        if !service.publish.contains(&upstream.port) {
            errors.push(Diagnostic::new(
                DiagnosticCode::MissingReference,
                format!("{path}.port"),
                "mapped container port is not declared in service.publish",
            ));
        }
    }
}

fn validate_local_dependencies(block_name: &str, block: &Block, errors: &mut Vec<Diagnostic>) {
    fn visit<'a>(
        node: &'a str,
        block: &'a Block,
        visiting: &mut BTreeSet<&'a str>,
        visited: &mut BTreeSet<&'a str>,
    ) -> bool {
        if visiting.contains(node) {
            return true;
        }
        if !visited.insert(node) {
            return false;
        }
        visiting.insert(node);
        let cyclic = block.services[node]
            .depends_on
            .keys()
            .filter(|dependency| !dependency.contains('/'))
            .filter(|dependency| block.services.contains_key(*dependency))
            .any(|dependency| visit(dependency, block, visiting, visited));
        visiting.remove(node);
        cyclic
    }

    let mut visited = BTreeSet::new();
    for name in block.services.keys() {
        let mut visiting = BTreeSet::new();
        if visit(name, block, &mut visiting, &mut visited) {
            errors.push(Diagnostic::new(
                DiagnosticCode::DependencyCycle,
                format!("spec.blocks.{block_name}.services.{name}.dependsOn"),
                "service dependency cycle detected",
            ));
            break;
        }
        for dependency in block.services[name]
            .depends_on
            .keys()
            .filter(|dependency| !dependency.contains('/'))
        {
            if !block.services.contains_key(dependency) {
                errors.push(Diagnostic::new(
                    DiagnosticCode::MissingReference,
                    format!("spec.blocks.{block_name}.services.{name}.dependsOn"),
                    format!("unknown local service {dependency}"),
                ));
            }
        }
    }
}

fn resolve_groups(
    bundle: &Bundle,
    instances: &BTreeMap<&str, &Instance>,
    errors: &mut Vec<Diagnostic>,
) -> ResolvedGroups {
    let mut members = BTreeMap::new();
    let mut declared_members = BTreeMap::new();
    let mut instance_groups = BTreeMap::<&str, (&str, usize)>::new();
    for (name, group) in &bundle.spec.groups {
        validate_name(name, format!("spec.groups.{name}"), errors);
        let mut valid_members = Vec::new();
        let mut seen = BTreeSet::new();
        for (index, member) in group.instances.iter().enumerate() {
            let path = format!("spec.groups.{name}.instances[{index}]");
            let (instance_name, requested_service) = provider_reference(member);
            let Some(instance) = instances.get(instance_name) else {
                errors.push(Diagnostic::new(
                    DiagnosticCode::MissingReference,
                    path,
                    format!("group member `{member}` does not exist"),
                ));
                continue;
            };
            if instance.is_external() {
                if requested_service.is_some() {
                    errors.push(Diagnostic::new(
                        DiagnosticCode::MissingReference,
                        path,
                        format!("external group member `{instance_name}` must not name a service"),
                    ));
                    continue;
                }
                if !seen.insert(member.as_str()) {
                    errors.push(Diagnostic::new(
                        DiagnosticCode::DuplicateName,
                        path,
                        format!("group member `{member}` is listed more than once"),
                    ));
                    continue;
                }
                if let Some((first_group, first_index)) =
                    instance_groups.insert(instance_name, (name, index))
                {
                    errors.push(Diagnostic::new(
                        DiagnosticCode::DuplicateName,
                        &path,
                        format!(
                            "instance `{instance_name}` belongs to both group `{first_group}` \
                             (spec.groups.{first_group}.instances[{first_index}]) and group `{name}`; \
                             create a separate external instance for each group"
                        ),
                    ));
                    continue;
                }
                valid_members.push(member.clone());
                continue;
            }
            let Some(block) = bundle.spec.blocks.get(&instance.block) else {
                continue;
            };
            if requested_service.is_some_and(|service| !block.services.contains_key(service)) {
                errors.push(Diagnostic::new(
                    DiagnosticCode::MissingReference,
                    path,
                    format!("`{member}` does not name a service on group member `{instance_name}`"),
                ));
                continue;
            }
            if !seen.insert(member.as_str()) {
                errors.push(Diagnostic::new(
                    DiagnosticCode::DuplicateName,
                    path,
                    format!("group member `{member}` is listed more than once"),
                ));
                continue;
            }
            if let Some((first_group, first_index)) =
                instance_groups.insert(instance_name, (name, index))
            {
                errors.push(Diagnostic::new(
                    DiagnosticCode::DuplicateName,
                    &path,
                    format!(
                        "instance `{instance_name}` belongs to both group `{first_group}` \
                         (spec.groups.{first_group}.instances[{first_index}]) and group `{name}`; \
                         create a separate instance to reuse the same source or block"
                    ),
                ));
                continue;
            }
            valid_members.push(member.clone());
        }
        let resolved_instance_names = valid_members
            .iter()
            .map(|member| provider_reference(member).0)
            .collect::<BTreeSet<_>>();
        let mut disabled = BTreeSet::new();
        for (index, instance_name) in group.disabled.iter().enumerate() {
            let path = format!("spec.groups.{name}.disabled[{index}]");
            validate_name(instance_name, &path, errors);
            if !resolved_instance_names.contains(instance_name.as_str()) {
                errors.push(Diagnostic::new(
                    DiagnosticCode::MissingReference,
                    &path,
                    format!(
                        "disabled instance `{instance_name}` is not a resolved member of group `{name}`"
                    ),
                ));
            }
            if !disabled.insert(instance_name.as_str()) {
                errors.push(Diagnostic::new(
                    DiagnosticCode::DuplicateName,
                    path,
                    format!("disabled instance `{instance_name}` is listed more than once"),
                ));
            }
        }
        let active_members = valid_members
            .iter()
            .filter(|member| !disabled.contains(provider_reference(member).0))
            .cloned()
            .collect::<Vec<_>>();
        members.insert(name.clone(), active_members);
        declared_members.insert(name.clone(), valid_members);
    }
    ResolvedGroups {
        members,
        declared_members,
    }
}

fn plausible_hostname(value: &str) -> bool {
    let value = value.strip_suffix('.').unwrap_or(value);
    !value.is_empty()
        && value.len() <= 253
        && value.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                && label
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && label
                    .as_bytes()
                    .last()
                    .is_some_and(u8::is_ascii_alphanumeric)
        })
}

fn normalized_hostname(value: &str) -> String {
    value.trim_end_matches('.').to_ascii_lowercase()
}

fn host_provider_for_instance_service<'a>(
    bundle: &'a Bundle,
    instance: &str,
    service: &str,
) -> Vec<&'a str> {
    bundle
        .spec
        .host_upstreams
        .iter()
        .filter(|(_, upstream)| upstream.instance == instance && upstream.service == service)
        .map(|(provider, _)| provider.as_str())
        .collect()
}

fn addressed_instance_provider<'a>(
    bundle: &'a Bundle,
    instance: &'a Instance,
) -> Result<(&'a str, &'a str), String> {
    if !bundle.spec.blocks.contains_key(&instance.block) {
        return Err(format!(
            "instance `{}` has no resolvable block",
            instance.name
        ));
    }
    let services = bundle
        .spec
        .host_upstreams
        .values()
        .filter(|upstream| upstream.instance == instance.name)
        .map(|upstream| upstream.service.as_str())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if services.len() != 1 {
        return Err(format!(
            "instance `{}` address needs exactly one browser-reachable service; candidates: {}",
            instance.name,
            if services.is_empty() {
                "none".into()
            } else {
                services.join(", ")
            }
        ));
    }
    let service = services[0];
    let providers = host_provider_for_instance_service(bundle, &instance.name, service);
    if providers.len() != 1 {
        return Err(format!(
            "instance `{}` service `{service}` maps to {} host-router providers; expected exactly one",
            instance.name,
            providers.len()
        ));
    }
    Ok((service, providers[0]))
}

fn addressed_member_provider<'a>(
    bundle: &'a Bundle,
    instances: &BTreeMap<&str, &'a Instance>,
    member: &str,
) -> Result<(&'a str, &'a str), String> {
    let (instance_name, requested_service) = provider_reference(member);
    let instance = instances
        .get(instance_name)
        .ok_or_else(|| format!("group member `{member}` does not name an instance"))?;
    let provider = if let Some(service) = requested_service {
        let providers = host_provider_for_instance_service(bundle, instance_name, service);
        if providers.len() != 1 {
            return Err(format!(
                "group member `{member}` maps to {} host-router providers; expected exactly one",
                providers.len()
            ));
        }
        providers[0]
    } else {
        addressed_instance_provider(bundle, instance)?.1
    };
    Ok((instance.name.as_str(), provider))
}

fn address_origin(listener: &router_config::Listener, address: &str) -> Result<String, String> {
    let (scheme, default_port) = match listener.protocol {
        router_config::Protocol::Http => ("http", 80),
        router_config::Protocol::Https => ("https", 443),
        _ => return Err("address listeners must use HTTP or HTTPS".into()),
    };
    Ok(if listener.bind.port == default_port {
        format!("{scheme}://{address}")
    } else {
        format!("{scheme}://{address}:{}", listener.bind.port)
    })
}

struct AddressRoute {
    listener: usize,
    slot: router_config::RouteSlotId,
}

struct GroupMemberTarget<'a> {
    group_name: &'a str,
    group_address: &'a str,
    member: &'a str,
    provider: &'a str,
    base: &'a AddressRoute,
    path: &'a str,
}

fn add_browser_origin_routes(
    bundle: &Bundle,
    config: &mut router_config::RouterConfig,
    origin: &str,
    identity: &str,
    path: &str,
    errors: &mut Vec<Diagnostic>,
) {
    let templates = config
        .spec
        .browser_routes
        .iter()
        .filter(|route| {
            matches!(
                &route.identity,
                router_config::BrowserIdentity::ExplicitHeader { value }
                    if value.as_str() == identity
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    for template in templates {
        let generated = router_config::BrowserRoute {
            identity: router_config::BrowserIdentity::Origin {
                origin: origin.to_owned(),
            },
            destination: template.destination.clone(),
            provider: template.provider.clone(),
        };
        let conflicting = config.spec.browser_routes.iter().find(|route| {
            route.destination == generated.destination
                && matches!(
                    &route.identity,
                    router_config::BrowserIdentity::Origin { origin: candidate }
                        if candidate == origin
                )
        });
        match conflicting {
            Some(route) if route.provider != generated.provider => errors.push(Diagnostic::new(
                DiagnosticCode::ListenerConflict,
                path,
                format!(
                    "origin `{origin}` already routes destination `{}` to `{}` instead of `{}`",
                    generated.destination, route.provider, generated.provider
                ),
            )),
            Some(_) => {}
            None => config.spec.browser_routes.push(generated),
        }
        if !bundle
            .spec
            .host_upstreams
            .contains_key(template.provider.as_str())
        {
            errors.push(Diagnostic::new(
                DiagnosticCode::MissingReference,
                path,
                format!(
                    "browser provider `{}` has no spec.hostUpstreams mapping to an instance",
                    template.provider
                ),
            ));
        }
    }
}

fn add_address_route(
    bundle: &Bundle,
    config: &mut router_config::RouterConfig,
    address: &str,
    instance: &str,
    provider: &str,
    path: &str,
    errors: &mut Vec<Diagnostic>,
) -> Option<AddressRoute> {
    let normalized = normalized_hostname(address);
    let mut existing = Vec::new();
    for (listener_index, listener) in config.spec.listeners.iter().enumerate() {
        for destination in &listener.destinations {
            if let router_config::ListenerDestination::CustomDomain { slot, domain } = destination {
                if normalized_hostname(domain) == normalized {
                    existing.push((listener_index, slot.clone()));
                }
            }
        }
    }
    let candidates = config
        .spec
        .routes
        .iter()
        .filter(|route| {
            route.provider.as_str() == provider && !route.slot.as_str().starts_with("group--")
        })
        .flat_map(|route| {
            config
                .spec
                .listeners
                .iter()
                .enumerate()
                .filter(move |(_, listener)| {
                    listener.proxy_identity.is_none()
                        && listener.consumer.as_ref() == Some(&route.consumer)
                        && matches!(
                            listener.protocol,
                            router_config::Protocol::Http | router_config::Protocol::Https
                        )
                })
                .map(move |(listener_index, _)| (listener_index, route.slot.clone()))
        })
        .collect::<BTreeSet<_>>();
    let selected = if existing.is_empty() {
        if candidates.len() != 1 {
            errors.push(Diagnostic::new(
                DiagnosticCode::MissingReference,
                path,
                format!(
                    "address `{address}` for `{instance}` maps to {} host-router listener routes; expected exactly one",
                    candidates.len()
                ),
            ));
            return None;
        }
        candidates.iter().next().cloned().expect("one candidate")
    } else {
        let first = existing[0].clone();
        if existing.iter().any(|candidate| candidate != &first) {
            errors.push(Diagnostic::new(
                DiagnosticCode::ListenerConflict,
                path,
                format!("custom domain `{address}` is declared for multiple route slots"),
            ));
            return None;
        }
        if !candidates.contains(&first) {
            errors.push(Diagnostic::new(
                DiagnosticCode::ListenerConflict,
                path,
                format!(
                    "custom domain `{address}` is authored for slot `{}`, which does not route to `{provider}`",
                    first.1
                ),
            ));
            return None;
        }
        first
    };
    if existing.is_empty() {
        config.spec.listeners[selected.0].destinations.push(
            router_config::ListenerDestination::CustomDomain {
                slot: selected.1.clone(),
                domain: address.to_owned(),
            },
        );
    }
    let origin = match address_origin(&config.spec.listeners[selected.0], address) {
        Ok(origin) => origin,
        Err(message) => {
            errors.push(Diagnostic::new(DiagnosticCode::InvalidPath, path, message));
            return None;
        }
    };
    add_browser_origin_routes(bundle, config, &origin, instance, path, errors);
    Some(AddressRoute {
        listener: selected.0,
        slot: selected.1,
    })
}

fn browser_protocols_compatible(
    listener: router_config::Protocol,
    provider: router_config::Protocol,
) -> bool {
    listener == provider
        || matches!(
            (listener, provider),
            (
                router_config::Protocol::Http | router_config::Protocol::Https,
                router_config::Protocol::Http | router_config::Protocol::Https
            )
        )
}

fn add_group_member_target(
    bundle: &Bundle,
    config: &mut router_config::RouterConfig,
    target: GroupMemberTarget<'_>,
    errors: &mut Vec<Diagnostic>,
) {
    let GroupMemberTarget {
        group_name,
        group_address,
        member,
        provider,
        base,
        path,
    } = target;
    let (instance_name, _) = provider_reference(member);
    let Some(provider_definition) = config
        .spec
        .providers
        .iter()
        .find(|candidate| candidate.id.as_str() == provider)
    else {
        errors.push(Diagnostic::new(
            DiagnosticCode::MissingReference,
            path,
            format!("group member `{member}` provider `{provider}` is not declared"),
        ));
        return;
    };
    let listener_protocol = config.spec.listeners[base.listener].protocol;
    if !browser_protocols_compatible(listener_protocol, provider_definition.endpoint.protocol) {
        return;
    }
    let Some(consumer) = config.spec.listeners[base.listener].consumer.clone() else {
        errors.push(Diagnostic::new(
            DiagnosticCode::MissingReference,
            path,
            "group addresses require a host-router listener with a consumer identity",
        ));
        return;
    };

    let subdomain = format!("{instance_name}.{group_address}");
    if !plausible_hostname(&subdomain) {
        errors.push(Diagnostic::new(
            DiagnosticCode::InvalidPath,
            path,
            format!("generated member subdomain `{subdomain}` is not a valid hostname"),
        ));
        return;
    }
    let normalized = normalized_hostname(&subdomain);
    let existing_domain = config
        .spec
        .listeners
        .iter()
        .enumerate()
        .flat_map(|(listener_index, listener)| {
            let normalized = &normalized;
            listener.destinations.iter().filter_map(move |destination| {
                if let router_config::ListenerDestination::CustomDomain { slot, domain } =
                    destination
                {
                    (normalized_hostname(domain) == normalized.as_str()).then_some((
                        listener_index,
                        slot.clone(),
                        domain.clone(),
                    ))
                } else {
                    None
                }
            })
        })
        .next();
    if let Some((listener_index, slot, domain)) = existing_domain {
        let reusable = listener_index == base.listener
            && config.spec.routes.iter().any(|route| {
                route.consumer == consumer
                    && route.slot == slot
                    && route.provider.as_str() == provider
            });
        if !reusable {
            errors.push(Diagnostic::new(
                DiagnosticCode::ListenerConflict,
                path,
                format!(
                    "generated member subdomain `{subdomain}` conflicts with custom domain `{domain}` on slot `{slot}`"
                ),
            ));
            return;
        }
    } else {
        let slot_name = resource_name(&["group", group_name, instance_name]);
        let slot = router_config::RouteSlotId::from(slot_name.as_str());
        config.spec.listeners[base.listener].destinations.push(
            router_config::ListenerDestination::CustomDomain {
                slot: slot.clone(),
                domain: subdomain.clone(),
            },
        );
        config.spec.routes.push(router_config::Route {
            consumer,
            slot,
            provider: router_config::ComponentId::from(provider),
        });
    }

    let targeted = router_config::BrowserRoute {
        identity: router_config::BrowserIdentity::ExplicitHeader {
            value: router_config::BindingId::from(instance_name),
        },
        destination: base.slot.clone(),
        provider: router_config::ComponentId::from(provider),
    };
    match config.spec.browser_routes.iter().find(|route| {
        route.destination == targeted.destination && route.identity == targeted.identity
    }) {
        Some(existing) if existing.provider != targeted.provider => errors.push(Diagnostic::new(
            DiagnosticCode::ListenerConflict,
            path,
            format!(
                "browser identity `{instance_name}` already targets `{}` instead of `{provider}`",
                existing.provider
            ),
        )),
        Some(_) => {}
        None => config.spec.browser_routes.push(targeted),
    }

    match address_origin(&config.spec.listeners[base.listener], &subdomain) {
        Ok(origin) => {
            add_browser_origin_routes(bundle, config, &origin, instance_name, path, errors);
        }
        Err(message) => errors.push(Diagnostic::new(DiagnosticCode::InvalidPath, path, message)),
    }
}

fn validate_address_claims(bundle: &Bundle, errors: &mut Vec<Diagnostic>) {
    let mut claimed = BTreeMap::<String, String>::new();
    let addresses = bundle
        .spec
        .instances
        .iter()
        .enumerate()
        .filter_map(|(index, instance)| {
            instance
                .address
                .as_deref()
                .map(|address| (format!("spec.instances[{index}].address"), address))
        })
        .chain(bundle.spec.groups.iter().filter_map(|(name, group)| {
            group
                .address
                .as_deref()
                .map(|address| (format!("spec.groups.{name}.address"), address))
        }));
    for (path, address) in addresses {
        if !plausible_hostname(address) {
            errors.push(Diagnostic::new(
                DiagnosticCode::InvalidPath,
                &path,
                "address must be a plausible hostname",
            ));
            continue;
        }
        if let Some(first) = claimed.insert(normalized_hostname(address), path.clone()) {
            errors.push(Diagnostic::new(
                DiagnosticCode::DuplicateName,
                path,
                format!("address `{address}` is already claimed at {first}"),
            ));
        }
    }
}

fn apply_addresses(
    bundle: &Bundle,
    instances: &BTreeMap<&str, &Instance>,
    groups: &ResolvedGroups,
    config: &mut router_config::RouterConfig,
    errors: &mut Vec<Diagnostic>,
) {
    for (index, instance) in bundle.spec.instances.iter().enumerate() {
        let Some(address) = instance.address.as_deref() else {
            continue;
        };
        let path = format!("spec.instances[{index}].address");
        match addressed_instance_provider(bundle, instance) {
            Ok((_, provider)) => {
                let _ = add_address_route(
                    bundle,
                    config,
                    address,
                    &instance.name,
                    provider,
                    &path,
                    errors,
                );
            }
            Err(message) => errors.push(Diagnostic::new(
                DiagnosticCode::MissingReference,
                &path,
                message,
            )),
        }
    }

    for (group_name, group) in &bundle.spec.groups {
        let Some(address) = group.address.as_deref() else {
            continue;
        };
        let path = format!("spec.groups.{group_name}.address");
        let active_members = groups
            .members
            .get(group_name)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let candidates = active_members
            .iter()
            .filter(|member| {
                let (instance_name, _) = provider_reference(member);
                instances
                    .get(instance_name)
                    .is_some_and(|instance| instance.address.is_some())
            })
            .cloned()
            .collect::<Vec<_>>();
        if candidates.len() != 1 {
            errors.push(Diagnostic::new(
                DiagnosticCode::IncompleteGroup,
                &path,
                format!(
                    "group address needs exactly one active member with its own address; candidates: {}",
                    if candidates.is_empty() {
                        "none".into()
                    } else {
                        candidates.join(", ")
                    }
                ),
            ));
            continue;
        }
        let default_member = &candidates[0];
        let Ok((default_instance, default_provider)) =
            addressed_member_provider(bundle, instances, default_member)
        else {
            errors.push(Diagnostic::new(
                DiagnosticCode::MissingReference,
                &path,
                format!(
                    "default group member `{default_member}` is not independently browser-addressable"
                ),
            ));
            continue;
        };
        let Some(base) = add_address_route(
            bundle,
            config,
            address,
            default_instance,
            default_provider,
            &path,
            errors,
        ) else {
            continue;
        };
        for member in active_members {
            if let Ok((_, provider)) = addressed_member_provider(bundle, instances, member) {
                add_group_member_target(
                    bundle,
                    config,
                    GroupMemberTarget {
                        group_name,
                        group_address: address,
                        member,
                        provider,
                        base: &base,
                        path: &path,
                    },
                    errors,
                );
            }
        }
    }
}

fn validate_expanded_dependencies(
    bundle: &Bundle,
    instances: &BTreeMap<&str, &Instance>,
    errors: &mut Vec<Diagnostic>,
) {
    let mut graph = BTreeMap::<String, Vec<String>>::new();
    for instance in instances.values() {
        if instance.is_external() {
            continue;
        }
        let block = &bundle.spec.blocks[&instance.block];
        for (service_name, service) in &block.services {
            let node = format!("{}/{service_name}", instance.name);
            let edges = graph.entry(node.clone()).or_default();
            for reference in service.depends_on.keys() {
                let target = reference.split_once('/').map_or_else(
                    || format!("{}/{reference}", instance.name),
                    |(target_instance, target_service)| {
                        format!("{target_instance}/{target_service}")
                    },
                );
                let Some((target_instance, target_service)) = target.split_once('/') else {
                    continue;
                };
                let valid = instances
                    .get(target_instance)
                    .filter(|candidate| !candidate.is_external())
                    .and_then(|candidate| bundle.spec.blocks.get(&candidate.block))
                    .is_some_and(|target_block| target_block.services.contains_key(target_service));
                if valid {
                    edges.push(target);
                } else {
                    errors.push(Diagnostic::new(
                        DiagnosticCode::MissingReference,
                        format!(
                            "spec.instances.{}.services.{service_name}.dependsOn",
                            instance.name
                        ),
                        format!("unknown service dependency {reference}"),
                    ));
                }
            }
        }
    }

    fn cyclic(
        node: &str,
        graph: &BTreeMap<String, Vec<String>>,
        active: &mut BTreeSet<String>,
        done: &mut BTreeSet<String>,
    ) -> bool {
        if active.contains(node) {
            return true;
        }
        if !done.insert(node.to_owned()) {
            return false;
        }
        active.insert(node.to_owned());
        let found = graph
            .get(node)
            .into_iter()
            .flatten()
            .any(|next| cyclic(next, graph, active, done));
        active.remove(node);
        found
    }

    let mut done = BTreeSet::new();
    for node in graph.keys() {
        if cyclic(node, &graph, &mut BTreeSet::new(), &mut done) {
            errors.push(Diagnostic::new(
                DiagnosticCode::DependencyCycle,
                format!("spec.instances.{node}.dependsOn"),
                "expanded service dependency cycle detected",
            ));
            break;
        }
    }
}

fn provider_reference(reference: &str) -> (&str, Option<&str>) {
    reference
        .split_once('/')
        .map_or((reference, None), |(instance, service)| {
            (instance, Some(service))
        })
}

fn validate_name(name: &str, path: impl Into<String>, errors: &mut Vec<Diagnostic>) {
    let valid = !name.is_empty()
        && name.len() <= 63
        && name.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit() && index > 0
                || byte == b'-' && index > 0
        })
        && !name.ends_with('-');
    if !valid {
        errors.push(Diagnostic::new(
            DiagnosticCode::InvalidName,
            path,
            "name must be a lowercase DNS label (letters, digits, and hyphens)",
        ));
    }
}

fn resolve_path(base: &Path, value: &Path) -> PathBuf {
    if value.is_absolute() {
        value.to_owned()
    } else {
        base.join(value)
    }
}

fn generate(
    bundle: &Bundle,
    groups: &ResolvedGroups,
    warnings: Vec<PlannerWarning>,
    overlay: Option<&OverlayResolution>,
    devices: &BTreeMap<String, PlanningDevice>,
) -> Result<Plan, Box<dyn std::error::Error>> {
    let deployment = &bundle.metadata.name;
    let project = resource_name(&["sy", deployment]);
    let network = resource_name(&["sy", deployment, "private"]);
    let artifact_dir = PathBuf::from(".switchyard/generated").join(deployment);
    let artifact_bind_dir = bundle.workspace_root.join(&artifact_dir);
    let definition_bytes = serde_json::to_vec(bundle)?;
    let mut definition_digest = Sha256::new();
    definition_digest.update(definition_bytes);
    if let Some(overlay) = overlay {
        definition_digest.update(serde_json::to_vec(&overlay.files)?);
    }
    let definition_hash = format!("{:x}", definition_digest.finalize());
    let mut resource_definition = bundle.clone();
    for instance in &mut resource_definition.spec.instances {
        instance.address = None;
    }
    resource_definition
        .spec
        .instances
        .retain(|instance| !instance.is_external());
    let routed_instances = resource_definition
        .spec
        .groups
        .values()
        .flat_map(|group| &group.instances)
        .map(|member| provider_reference(member).0.to_owned())
        .filter(|name| {
            resource_definition
                .spec
                .instances
                .iter()
                .any(|instance| instance.name == *name)
        })
        .collect::<BTreeSet<_>>();
    resource_definition.spec.groups.clear();
    if !routed_instances.is_empty() {
        resource_definition.spec.groups.insert(
            "__routed_instances__".into(),
            ServiceGroup {
                instances: routed_instances.into_iter().collect(),
                ..ServiceGroup::default()
            },
        );
    }
    resource_definition.spec.managed_profiles.clear();
    resource_definition.spec.host_router = None;
    resource_definition.spec.host_upstreams.clear();
    let mut resource_digest = Sha256::new();
    resource_digest.update(serde_json::to_vec(&resource_definition)?);
    let referenced_devices = bundle
        .spec
        .instances
        .iter()
        .filter_map(|instance| instance.device.as_deref())
        .filter(|device| *device != "local")
        .filter_map(|device| devices.get(device).map(|details| (device, details)))
        .collect::<BTreeMap<_, _>>();
    if !referenced_devices.is_empty() {
        resource_digest.update(serde_json::to_vec(&referenced_devices)?);
    }
    if let Some(overlay) = overlay {
        resource_digest.update(serde_json::to_vec(&overlay.files)?);
    }
    let resource_hash = format!("{:x}", resource_digest.finalize());
    let labels = ownership_labels(deployment, &resource_hash);
    let instances = bundle
        .spec
        .instances
        .iter()
        .map(|instance| (instance.name.as_str(), instance))
        .collect::<BTreeMap<_, _>>();
    let routing_groups = bundle
        .spec
        .instances
        .iter()
        .filter(|instance| !instance.is_external())
        .filter_map(|instance| {
            selected_group_for_instance(bundle, groups, &instance.name)
                .map(|group| (instance.name.clone(), group.to_owned()))
        })
        .collect::<BTreeMap<_, _>>();
    let routed_instances = bundle
        .spec
        .instances
        .iter()
        .filter(|instance| {
            !instance.is_external()
                && instance
                    .device
                    .as_deref()
                    .is_none_or(|device| device == "local")
                && (groups.declared_members.values().any(|members| {
                    members
                        .iter()
                        .any(|member| provider_reference(member).0 == instance.name.as_str())
                }) || routing_groups.contains_key(&instance.name))
        })
        .map(|instance| instance.name.as_str())
        .collect::<BTreeSet<_>>();

    let mut services = serde_json::Map::new();
    let mut volumes = serde_json::Map::new();
    let mut remote_services = BTreeMap::<String, serde_json::Map<String, Value>>::new();
    let mut remote_volumes = BTreeMap::<String, serde_json::Map<String, Value>>::new();
    let mut remote_service_names = BTreeMap::<String, Vec<String>>::new();
    let mut manifest_services = Vec::new();
    let mut route_configs = BTreeMap::new();
    let mut sidecars = BTreeMap::new();
    let managed_profiles = managed_profiles(bundle);
    let host_router_config =
        generate_host_router_config(bundle, groups, &managed_profiles, devices)?;
    let host_upstreams = host_upstreams(bundle, devices);
    let mut source_identities = BTreeMap::new();
    let source_manager = SourceManager::new(&bundle.workspace_root);
    let external_probes = bundle
        .spec
        .instances
        .iter()
        .filter_map(|instance| {
            instance.probe.clone().map(|probe| ExternalProbePlan {
                instance: instance.name.clone(),
                host: instance.external.clone().expect("validated external probe"),
                probe,
            })
        })
        .collect::<Vec<_>>();

    for instance in &bundle.spec.instances {
        if instance.is_external() {
            continue;
        }
        let mut instance_labels = labels.clone();
        instance_labels.insert(INSTANCE_LABEL.into(), instance.name.clone());
        let remote_device = instance
            .device
            .as_deref()
            .filter(|device| *device != "local");
        if let Some(device) = remote_device {
            instance_labels.insert("dev.switchyard.device".into(), device.to_owned());
        }
        let block = &bundle.spec.blocks[&instance.block];
        let source = resolve_path(
            &bundle.definition_dir,
            &bundle.spec.sources[&instance.source].path,
        );
        let source_definition = &bundle.spec.sources[&instance.source];
        let identity = source_manager
            .inspect(&source, Some(&source_definition.r#ref))
            .identity;
        source_identities.insert(instance.name.clone(), identity);
        let transparent = routing_groups.contains_key(&instance.name)
            || groups.declared_members.values().any(|members| {
                members
                    .iter()
                    .any(|member| provider_reference(member).0 == instance.name.as_str())
            });
        let routed = remote_device.is_none() && routed_instances.contains(instance.name.as_str());
        let namespace_name =
            routed.then(|| resource_name(&[deployment, &instance.name, "namespace"]));
        let sidecar_name = routed.then(|| resource_name(&[deployment, &instance.name, "router"]));
        if let Some(namespace_name) = &namespace_name {
            let mut namespace_labels = instance_labels.clone();
            namespace_labels.insert(SERVICE_LABEL.into(), "namespace".into());
            let mut namespace =
                compose_namespace_service(namespace_name, &network, &namespace_labels);
            let published = block
                .services
                .values()
                .flat_map(|service| service.publish.iter().copied())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            let object = namespace.as_object_mut().expect("namespace is an object");
            if !published.is_empty() {
                object.insert("ports".into(), compose_ports(&published));
            }
            let aliases = block
                .services
                .keys()
                .map(|service| service_name_for(deployment, &instance.name, service))
                .collect::<Vec<_>>();
            object.insert(
                "networks".into(),
                json!({ network.clone(): { "aliases": aliases } }),
            );
            services.insert(namespace_name.clone(), namespace);
        }
        for (service_name, service) in &block.services {
            let mut service_labels = instance_labels.clone();
            service_labels.insert(SERVICE_LABEL.into(), service_name.clone());
            let base_name = service_name_for(deployment, &instance.name, service_name);
            if let Some(device) = remote_device {
                let remote_network = remote_network_name(deployment, device);
                let mut app = compose_application(
                    service,
                    instance,
                    &source,
                    &remote_network,
                    &service_labels,
                    bundle,
                    block,
                );
                add_injected_mounts(&mut app, overlay, &instance.name, &artifact_bind_dir);
                apply_overlay_environment(&mut app, overlay, instance);
                let app_object = app.as_object_mut().expect("service is an object");
                app_object.insert("ports".into(), compose_remote_ports(&service.publish));
                add_compose_dependencies(app_object, bundle, instance, service, &routed_instances);
                remote_services
                    .entry(device.to_owned())
                    .or_default()
                    .insert(base_name.clone(), app);
                remote_service_names
                    .entry(device.to_owned())
                    .or_default()
                    .push(base_name.clone());
                manifest_services.push(json!({
                    "instance": instance.name,
                    "component": service_name,
                    "service": base_name,
                    "device": device,
                    "labels": service_labels,
                }));
                for mount in &service.volumes {
                    let volume_name = resource_name(&[deployment, &instance.name, &mount.name]);
                    remote_volumes.entry(device.to_owned()).or_default().insert(
                        volume_name,
                        json!({ "labels": service_labels, "name": resource_name(&["sy", deployment, &instance.name, &mount.name]) }),
                    );
                }
                continue;
            }
            let mut app = compose_application(
                service,
                instance,
                &source,
                &network,
                &service_labels,
                bundle,
                block,
            );
            add_injected_mounts(&mut app, overlay, &instance.name, &artifact_bind_dir);
            apply_overlay_environment(&mut app, overlay, instance);
            let app_object = app.as_object_mut().expect("service is an object");
            add_compose_dependencies(app_object, bundle, instance, service, &routed_instances);
            let generated_service = if routed {
                let app_name = resource_name(&[&base_name, "app"]);
                app_object.remove("networks");
                app_object.remove("ports");
                app_object.insert(
                    "network_mode".into(),
                    Value::String(format!(
                        "service:{}",
                        namespace_name.as_deref().expect("routed namespace")
                    )),
                );
                app_object
                    .entry("depends_on")
                    .or_insert_with(|| json!({}))
                    .as_object_mut()
                    .expect("depends_on is an object")
                    .insert(
                        sidecar_name.as_ref().expect("routed sidecar").clone(),
                        json!({ "condition": "service_healthy" }),
                    );
                app_name
            } else {
                base_name
            };
            services.insert(generated_service.clone(), app);
            let mut manifest_service = json!({
                "instance": instance.name,
                "component": service_name,
                "service": generated_service,
                "labels": service_labels,
            });
            if routed {
                manifest_service["namespaceService"] =
                    json!(namespace_name.as_ref().expect("routed namespace"));
                manifest_service["sidecar"] = json!(sidecar_name.as_ref().expect("routed sidecar"));
            }
            manifest_services.push(manifest_service);
            for mount in &service.volumes {
                let volume_name = resource_name(&[deployment, &instance.name, &mount.name]);
                volumes.insert(
                    volume_name,
                    json!({ "labels": service_labels, "name": resource_name(&["sy", deployment, &instance.name, &mount.name]) }),
                );
            }
        }
        if routed {
            let namespace_name = namespace_name.as_ref().expect("routed namespace");
            let sidecar_name = sidecar_name.as_ref().expect("routed sidecar");
            let mut router_labels = instance_labels.clone();
            router_labels.insert(SERVICE_LABEL.into(), "router".into());
            let transparent_group = routing_groups.get(&instance.name).map(String::as_str);
            let config = router_config(
                bundle,
                &instances,
                instance,
                TransparentRoute {
                    enabled: transparent,
                    group: transparent_group,
                    groups,
                },
                devices,
            )?;
            let config_path = artifact_dir
                .join("routes")
                .join(format!("{}.json", instance.name));
            let admin_socket = PathBuf::from("/tmp/switchyard-admin.socket");
            let sidecar = compose_sidecar(
                &bundle.spec.router_image,
                namespace_name,
                sidecar_name,
                &artifact_bind_dir
                    .join("routes")
                    .join(format!("{}.json", instance.name)),
                &router_labels,
                transparent,
            );
            services.insert(sidecar_name.clone(), sidecar);
            route_configs.insert(
                instance.name.clone(),
                serde_json::to_string_pretty(&config)?,
            );
            sidecars.insert(
                instance.name.clone(),
                SidecarPlan {
                    service: sidecar_name.clone(),
                    admin_socket,
                    config_path,
                },
            );
        }
    }

    let compose = json!({
        "name": project,
        "services": services,
        "networks": {
            network.clone(): {
                "name": network,
                "driver": "bridge",
                "labels": labels,
            }
        },
        "volumes": volumes,
    });
    let compose_yaml = serde_yaml::to_string(&compose)?;
    let mut remote_projects = BTreeMap::new();
    for (device_name, services) in remote_services {
        let remote_project = format!("{project}-{device_name}");
        let remote_network = remote_network_name(deployment, &device_name);
        let mut remote_labels = labels.clone();
        remote_labels.insert("dev.switchyard.device".into(), device_name.clone());
        let compose_file = PathBuf::from(format!("compose.{device_name}.yaml"));
        let remote_compose = json!({
            "name": remote_project,
            "services": services,
            "networks": {
                remote_network.clone(): {
                    "name": remote_network,
                    "driver": "bridge",
                    "labels": remote_labels,
                }
            },
            "volumes": remote_volumes.remove(&device_name).unwrap_or_default(),
        });
        remote_projects.insert(
            device_name.clone(),
            RemoteComposePlan {
                device: devices[&device_name].clone(),
                compose_project: remote_project,
                compose_file,
                compose_yaml: serde_yaml::to_string(&remote_compose)?,
                services: remote_service_names
                    .remove(&device_name)
                    .unwrap_or_default(),
            },
        );
    }
    let mut resolved = bundle.clone();
    for (name, repository) in &mut resolved.spec.repositories {
        if repository.url.is_some() {
            repository.clone = Some(source_manager.clone_root().join(name));
            repository.url = None;
        } else if let Some(path) = &mut repository.clone {
            *path = resolve_path(&bundle.definition_dir, path);
        }
    }
    for source in resolved.spec.sources.values_mut() {
        source.path = resolve_path(&bundle.definition_dir, &source.path);
    }
    let resolved_deployment_yaml = serde_yaml::to_string(&resolved)?;
    let mut manifest = json!({
        "apiVersion": API_VERSION,
        "deployment": deployment,
        "definitionHash": definition_hash,
        "resourceHash": resource_hash,
        "composeProject": project,
        "network": network,
        "services": manifest_services,
        "sidecars": sidecars,
        "managedProfiles": managed_profiles,
        "hostRouterConfig": host_router_config.as_ref().map(|_| artifact_dir.join("host-router.json")),
        "hostUpstreams": host_upstreams,
        "ownershipLabels": labels,
        "sourceIdentities": source_identities,
        "externalInstances": bundle.spec.instances.iter().filter_map(|instance| {
            instance.external.as_ref().map(|host| json!({
                "instance": instance.name,
                "host": host,
                "ports": instance.expanded_external_ports(),
                "probed": instance.probe.is_some(),
            }))
        }).collect::<Vec<_>>(),
    });
    if !remote_projects.is_empty() {
        let manifest_remote_projects = remote_projects
            .iter()
            .map(|(name, remote)| {
                (
                    name.clone(),
                    json!({
                        "device": remote.device,
                        "composeProject": remote.compose_project,
                        "composeFile": remote.compose_file,
                        "services": remote.services,
                    }),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        manifest
            .as_object_mut()
            .expect("manifest is an object")
            .insert(
                "remoteProjects".into(),
                Value::Object(manifest_remote_projects),
            );
    }
    if let Some(overlay) = overlay {
        let object = manifest.as_object_mut().expect("manifest is an object");
        object.insert("origins".into(), json!(overlay.origins));
        object.insert("injectedFiles".into(), json!(overlay.files));
    }
    let manifest_json = serde_json::to_string_pretty(&manifest)?;

    let local_service_count = compose["services"]
        .as_object()
        .map_or(0, serde_json::Map::len);
    Ok(Plan {
        deployment: deployment.clone(),
        definition_hash,
        resource_hash,
        compose_project: project,
        artifact_dir,
        compose_yaml,
        local_service_count,
        remote_projects,
        resolved_deployment_yaml,
        manifest_json,
        route_configs,
        sidecars,
        managed_profiles,
        host_router_config,
        host_upstreams,
        source_identities,
        origins: overlay.map_or_else(Vec::new, |value| value.origins.clone()),
        injected_files: overlay.map_or_else(Vec::new, |value| value.files.clone()),
        warnings,
        external_probes,
        runtime_secrets: overlay.map_or_else(Vec::new, |value| {
            value
                .secret_environment
                .iter()
                .map(|((instance, key), reference)| RuntimeSecretPlan {
                    variable: overlay_secret_variable(instance, key),
                    reference: reference.clone(),
                })
                .collect()
        }),
        has_overrides: overlay.is_some(),
    })
}

fn managed_profiles(bundle: &Bundle) -> BTreeMap<String, ManagedProfilePlan> {
    const FIRST_PORT: u16 = 24_000;
    const PORT_COUNT: u16 = 8_000;
    let mut used = BTreeSet::new();
    let mut result = BTreeMap::new();
    for (ui, profile) in &bundle.spec.managed_profiles {
        let digest = Sha256::digest(format!("{}\0{ui}", bundle.metadata.name));
        let offset = u16::from_be_bytes([digest[0], digest[1]]) % PORT_COUNT;
        let mut port = FIRST_PORT + offset;
        while !used.insert(port) {
            port = FIRST_PORT + ((port - FIRST_PORT + 1) % PORT_COUNT);
        }
        result.insert(
            ui.clone(),
            ManagedProfilePlan {
                api_version: "switchyard.dev/managed-profile/v1alpha1".into(),
                deployment: bundle.metadata.name.clone(),
                ui: ui.clone(),
                route: profile.route.clone(),
                proxy_address: format!("127.0.0.1:{port}"),
                start_url: profile.start_url.clone(),
            },
        );
    }
    result
}

fn host_upstreams(
    bundle: &Bundle,
    devices: &BTreeMap<String, PlanningDevice>,
) -> BTreeMap<String, HostUpstreamPlan> {
    bundle
        .spec
        .host_upstreams
        .iter()
        .map(|(provider, upstream)| {
            (
                provider.clone(),
                HostUpstreamPlan {
                    compose_service: service_name_for(
                        &bundle.metadata.name,
                        &upstream.instance,
                        &upstream.service,
                    ),
                    container_port: upstream.port,
                    remote_address: bundle
                        .spec
                        .instances
                        .iter()
                        .find(|instance| instance.name == upstream.instance)
                        .and_then(|instance| instance.device.as_deref())
                        .filter(|device| *device != "local")
                        .and_then(|device| devices.get(device))
                        .map(|device| format!("{}:{}", device.host, upstream.port)),
                },
            )
        })
        .collect()
}

/// Returns whether a managed-profile start URL is a strict local HTTP URI.
pub fn is_local_http_url(value: &str) -> bool {
    local_http_target(value).is_some()
}

fn local_http_target(value: &str) -> Option<(String, u16)> {
    if value.chars().any(char::is_whitespace) || value.contains('#') {
        return None;
    }
    let uri = value.parse::<http::Uri>().ok()?;
    if uri.scheme_str() != Some("http") {
        return None;
    }
    let authority = uri.authority()?;
    if authority.as_str().contains('@') {
        return None;
    }
    let authority_text = authority.as_str();
    let explicit_port = if authority_text.starts_with('[') {
        authority_text
            .find(']')
            .is_some_and(|end| authority_text[end + 1..].starts_with(':'))
    } else {
        authority_text.contains(':')
    };
    if explicit_port && authority.port_u16().is_none() {
        return None;
    }
    let host = authority.host();
    let normalized = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host)
        .to_ascii_lowercase();
    let local = normalized == "localhost"
        || normalized.ends_with(".localhost")
        || normalized
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    let port = authority.port_u16().unwrap_or(80);
    (local && port != 0).then_some((normalized, port))
}

fn managed_profile_destinations(
    config: &router_config::RouterConfig,
    route_identity: &str,
    start_url: &str,
) -> Result<Vec<router_config::ListenerDestination>, String> {
    let start_target =
        local_http_target(start_url).ok_or("managed profile start URL is invalid")?;
    let identity = router_config::RouteSlotId::from(route_identity);
    let mut destinations = Vec::new();
    for route in &config.spec.browser_routes {
        if !matches!(
            &route.identity,
            router_config::BrowserIdentity::ProxyListener { listener }
                if listener == &identity
        ) {
            continue;
        }
        let sources = config
            .spec
            .listeners
            .iter()
            .filter(|listener| {
                listener.proxy_identity.is_none()
                    && listener.protocol == router_config::Protocol::Http
            })
            .flat_map(|listener| {
                listener
                    .destinations
                    .iter()
                    .filter(move |destination| destination.slot() == &route.destination)
                    .map(move |destination| (listener, destination))
            })
            .collect::<Vec<_>>();
        if sources.len() != 1 {
            return Err(format!(
                "proxy destination `{}` maps to {} cleartext HTTP source destinations; expected exactly one",
                route.destination,
                sources.len()
            ));
        }
        let (listener, destination) = sources[0];
        let host = match destination {
            router_config::ListenerDestination::CustomDomain { domain, .. } => domain,
            router_config::ListenerDestination::LegacyLocalhost { host, .. } => host,
            router_config::ListenerDestination::Loopback { .. }
            | router_config::ListenerDestination::ProxyTarget { .. } => {
                return Err(format!(
                    "proxy destination `{}` does not declare an exact local host",
                    route.destination
                ));
            }
        };
        let normalized = host.trim_end_matches('.').to_ascii_lowercase();
        let local = normalized == "localhost"
            || normalized.ends_with(".localhost")
            || normalized
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback());
        if !local {
            return Err(format!(
                "proxy destination `{}` host `{host}` is not local",
                route.destination
            ));
        }
        destinations.push(router_config::ListenerDestination::ProxyTarget {
            slot: route.destination.clone(),
            host: normalized,
            port: listener.bind.port,
        });
    }
    if destinations.is_empty() {
        return Err("managed profile route has no proxy-listener browser routes".into());
    }
    let start_matches = destinations
        .iter()
        .filter(|destination| {
            matches!(
                destination,
                router_config::ListenerDestination::ProxyTarget { host, port, .. }
                    if host == &start_target.0 && port == &start_target.1
            )
        })
        .count();
    if start_matches != 1 {
        return Err(format!(
            "managed profile start target `{}:{}` maps to {start_matches} declared proxy destinations; expected exactly one",
            start_target.0, start_target.1
        ));
    }
    Ok(destinations)
}

fn generate_host_router_config(
    bundle: &Bundle,
    groups: &ResolvedGroups,
    profiles: &BTreeMap<String, ManagedProfilePlan>,
    devices: &BTreeMap<String, PlanningDevice>,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let Some(mut config) = bundle.spec.host_router.clone() else {
        return Ok(None);
    };
    for provider in &mut config.spec.providers {
        let Some(upstream) = bundle.spec.host_upstreams.get(provider.id.as_str()) else {
            continue;
        };
        let Some(instance) = bundle
            .spec
            .instances
            .iter()
            .find(|instance| instance.name == upstream.instance)
        else {
            continue;
        };
        let Some(device) = instance
            .device
            .as_deref()
            .filter(|device| *device != "local")
            .and_then(|device| devices.get(device))
        else {
            continue;
        };
        provider.endpoint.host = device.host.clone();
        provider.endpoint.port = upstream.port;
    }
    let instances = bundle
        .spec
        .instances
        .iter()
        .map(|instance| (instance.name.as_str(), instance))
        .collect();
    let mut address_errors = Vec::new();
    apply_addresses(bundle, &instances, groups, &mut config, &mut address_errors);
    if !address_errors.is_empty() {
        return Err(address_errors
            .into_iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("; ")
            .into());
    }
    for profile in profiles.values() {
        let destinations =
            managed_profile_destinations(&config, &profile.route, &profile.start_url)?;
        let port = profile
            .proxy_address
            .rsplit_once(':')
            .and_then(|(_, port)| port.parse::<u16>().ok())
            .ok_or("planner produced an invalid managed proxy address")?;
        let credential_file = bundle
            .workspace_root
            .join(".switchyard/run")
            .join(&bundle.metadata.name)
            .join("managed-profiles")
            .join(format!("{}.credential", profile.ui));
        config.spec.listeners.push(router_config::Listener {
            consumer: None,
            bind: router_config::SocketAddress {
                host: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                port,
            },
            protocol: router_config::Protocol::Http,
            tls: None,
            destinations,
            proxy_identity: Some(router_config::BindingId::from(profile.route.as_str())),
            proxy_authentication: Some(router_config::ProxyAuthentication {
                scheme: router_config::ProxyAuthenticationScheme::Basic,
                credential_file,
            }),
        });
    }
    config.validate().map_err(|errors| {
        errors
            .into_iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("; ")
    })?;
    Ok(Some(serde_json::to_string_pretty(&config)?))
}

fn compose_namespace_service(
    name: &str,
    network: &str,
    labels: &BTreeMap<String, String>,
) -> Value {
    json!({
        "image": "alpine:3.22",
        "command": ["sleep", "infinity"],
        "restart": "unless-stopped",
        "networks": { network: { "aliases": [name] } },
        "labels": labels,
    })
}

fn compose_application(
    service: &Service,
    instance: &Instance,
    source: &Path,
    network: &str,
    labels: &BTreeMap<String, String>,
    bundle: &Bundle,
    block: &Block,
) -> Value {
    let mut value = serde_json::Map::new();
    value.insert("labels".into(), json!(labels));
    value.insert("networks".into(), json!([network]));
    if !matches!(
        &service.execution,
        Execution::Script {
            lifecycle: ScriptLifecycle::Task,
            ..
        }
    ) {
        value.insert("restart".into(), json!("unless-stopped"));
    }
    match &service.execution {
        Execution::Container {
            image,
            build,
            command,
            working_directory,
            environment,
        } => {
            if let Some(image) = image {
                value.insert("image".into(), json!(image));
            }
            if let Some(build) = build {
                let mut build_value = json!({ "context": source.join(&build.context) });
                if let Some(dockerfile) = &build.dockerfile {
                    build_value["dockerfile"] = json!(dockerfile);
                }
                value.insert("build".into(), build_value);
            }
            add_runtime_fields(
                &mut value,
                command,
                working_directory.as_deref(),
                environment,
                instance,
                bundle,
                block,
            );
        }
        Execution::Script {
            image,
            command,
            working_directory,
            source_mount,
            writable,
            environment,
            ..
        } => {
            value.insert("image".into(), json!(image));
            add_runtime_fields(
                &mut value,
                command,
                working_directory.as_deref(),
                environment,
                instance,
                bundle,
                block,
            );
            value.insert(
                "volumes".into(),
                json!([format!(
                    "{}:{}{}",
                    source.display(),
                    source_mount.display(),
                    if *writable { "" } else { ":ro" }
                )]),
            );
        }
        Execution::ProcessCompose {
            image,
            file,
            working_directory,
            source_mount,
            writable,
            environment,
        } => {
            value.insert("image".into(), json!(image));
            let command = vec![
                "process-compose".to_owned(),
                "--ordered-shutdown".to_owned(),
                "--no-server".to_owned(),
                "-t=false".to_owned(),
                "-f".to_owned(),
                source_mount.join(file).display().to_string(),
                "up".to_owned(),
            ];
            add_runtime_fields(
                &mut value,
                &command,
                working_directory.as_deref().or(Some(source_mount)),
                environment,
                instance,
                bundle,
                block,
            );
            value.insert(
                "volumes".into(),
                json!([format!(
                    "{}:{}{}",
                    source.display(),
                    source_mount.display(),
                    if *writable { "" } else { ":ro" }
                )]),
            );
        }
    }
    let mounts = value.entry("volumes").or_insert_with(|| json!([]));
    let mounts = mounts.as_array_mut().expect("volumes is an array");
    for mount in &service.volumes {
        mounts.push(json!(format!(
            "{}:{}{}",
            resource_name(&[&bundle.metadata.name, &instance.name, &mount.name]),
            mount.target.display(),
            if mount.read_only { ":ro" } else { "" }
        )));
    }
    if !service.publish.is_empty() {
        value.insert("ports".into(), compose_ports(&service.publish));
    }
    if let Some(probe) = &service.probe {
        value.insert("healthcheck".into(), compose_probe(probe));
    }
    Value::Object(value)
}

fn compose_ports(ports: &[u16]) -> Value {
    json!(
        ports
            .iter()
            .map(|port| format!("127.0.0.1::{port}"))
            .collect::<Vec<_>>()
    )
}

fn compose_remote_ports(ports: &[u16]) -> Value {
    json!(
        ports
            .iter()
            .map(|port| format!("{port}:{port}"))
            .collect::<Vec<_>>()
    )
}

fn add_runtime_fields(
    value: &mut serde_json::Map<String, Value>,
    command: &[String],
    working_directory: Option<&Path>,
    environment: &BTreeMap<String, String>,
    instance: &Instance,
    bundle: &Bundle,
    block: &Block,
) {
    if !command.is_empty() {
        value.insert("command".into(), json!(command));
    }
    if let Some(directory) = working_directory {
        value.insert("working_dir".into(), json!(directory));
    }
    let mut variables = block
        .parameters
        .iter()
        .filter_map(|(name, parameter)| {
            parameter
                .default
                .as_ref()
                .map(|value| (name.clone(), value.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    variables.extend(environment.clone());
    variables.extend(instance.parameters.clone());
    variables.extend(instance.environment.clone());
    variables.insert("SWITCHYARD_DEPLOYMENT".into(), bundle.metadata.name.clone());
    variables.insert("SWITCHYARD_INSTANCE".into(), instance.name.clone());
    if !variables.is_empty() {
        value.insert("environment".into(), json!(variables));
    }
}

fn apply_overlay_environment(
    service: &mut Value,
    overlay: Option<&OverlayResolution>,
    instance: &Instance,
) {
    let Some(environment) = service
        .as_object_mut()
        .and_then(|service| service.get_mut("environment"))
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    for key in &instance.environment_unset {
        environment.remove(key);
    }
    let Some(overlay) = overlay else { return };
    for (target, key) in overlay.secret_environment.keys() {
        if target == &instance.name {
            let variable = overlay_secret_variable(target, key);
            environment.insert(
                key.clone(),
                Value::String(format!("${{{variable}:?overlay secret is required}}")),
            );
        }
    }
}

fn overlay_secret_variable(instance: &str, key: &str) -> String {
    let digest = format!(
        "{:x}",
        Sha256::digest(format!("{instance}\0{key}").as_bytes())
    );
    format!(
        "SWITCHYARD_OVERLAY_SECRET_{}",
        digest[..16].to_ascii_uppercase()
    )
}

fn add_injected_mounts(
    service: &mut Value,
    overlay: Option<&OverlayResolution>,
    instance: &str,
    artifact_bind_dir: &Path,
) {
    let Some(overlay) = overlay else { return };
    let object = service.as_object_mut().expect("service is an object");
    let mounts = object.entry("volumes").or_insert_with(|| json!([]));
    let mounts = mounts.as_array_mut().expect("volumes is an array");
    for file in overlay
        .files
        .iter()
        .filter(|file| file.instance == instance)
    {
        mounts.push(json!(format!(
            "{}:{}:ro",
            artifact_bind_dir.join(&file.relative_path).display(),
            file.target.display()
        )));
    }
}

fn add_compose_dependencies(
    value: &mut serde_json::Map<String, Value>,
    bundle: &Bundle,
    instance: &Instance,
    service: &Service,
    routed_instances: &BTreeSet<&str>,
) {
    if service.depends_on.is_empty() {
        return;
    }
    let dependencies = service
        .depends_on
        .iter()
        .map(|(reference, condition)| {
            let (target_instance, target_service) = reference
                .split_once('/')
                .map_or((instance.name.as_str(), reference.as_str()), |parts| parts);
            let mut target =
                service_name_for(&bundle.metadata.name, target_instance, target_service);
            if routed_instances.contains(target_instance) {
                target = resource_name(&[&target, "app"]);
            }
            let condition = match condition {
                DependencyCondition::Started => "service_started",
                DependencyCondition::Healthy => "service_healthy",
                DependencyCondition::CompletedSuccessfully => "service_completed_successfully",
            };
            (target, json!({ "condition": condition }))
        })
        .collect::<serde_json::Map<_, _>>();
    value.insert("depends_on".into(), Value::Object(dependencies));
}

fn compose_probe(probe: &Probe) -> Value {
    let test = match probe {
        Probe::Http { path, port, https } => vec![
            "CMD-SHELL".to_owned(),
            format!(
                "wget -q --spider {}://127.0.0.1:{port}{path}",
                if *https { "https" } else { "http" }
            ),
        ],
        Probe::Tcp { port } => vec!["CMD-SHELL".to_owned(), format!("nc -z 127.0.0.1 {port}")],
        Probe::Command { command } => std::iter::once("CMD".to_owned())
            .chain(command.iter().cloned())
            .collect(),
    };
    json!({ "test": test, "interval": "2s", "timeout": "1s", "retries": 30 })
}

fn compose_sidecar(
    image: &str,
    namespace_service: &str,
    sidecar_name: &str,
    config_path: &Path,
    labels: &BTreeMap<String, String>,
    transparent: bool,
) -> Value {
    let mut depends_on = serde_json::Map::new();
    depends_on.insert(
        namespace_service.into(),
        json!({ "condition": "service_started" }),
    );
    let mut sidecar = json!({
        "image": image,
        "user": "${SWITCHYARD_UID:-1000}:${SWITCHYARD_GID:-1000}",
        "restart": "unless-stopped",
        "network_mode": format!("service:{namespace_service}"),
        "command": [
            "/usr/local/bin/switchyard-router",
            format!("/config/{}", config_path.file_name().unwrap_or_default().to_string_lossy()),
            "/tmp/switchyard-admin.socket",
        ],
        "environment": {
            "SWITCHYARD_ROUTER_TOKEN": "${SWITCHYARD_ROUTER_TOKEN:?set SWITCHYARD_ROUTER_TOKEN}"
        },
        "volumes": [
            format!("{}:/config/{}:ro", config_path.display(), config_path.file_name().unwrap_or_default().to_string_lossy()),
        ],
        "depends_on": depends_on,
        "healthcheck": {
            "test": ["CMD", "test", "-S", "/tmp/switchyard-admin.socket"],
            "interval": "1s",
            "timeout": "1s",
            "retries": 30,
        },
        "labels": labels,
        "container_name": sidecar_name,
    });
    if transparent {
        let object = sidecar.as_object_mut().expect("sidecar is an object");
        object.insert("user".into(), json!("0:0"));
        object.insert("cap_drop".into(), json!(["ALL"]));
        object.insert("cap_add".into(), json!(["NET_ADMIN"]));
        object.insert("security_opt".into(), json!(["no-new-privileges:true"]));
    }
    sidecar
}

const TRANSPARENT_INTERCEPTION_PORT: u16 = 65_535;

fn selected_group_for_instance<'a>(
    _bundle: &'a Bundle,
    groups: &'a ResolvedGroups,
    instance: &str,
) -> Option<&'a str> {
    let matching = groups
        .members
        .iter()
        .filter(|(_, members)| {
            members
                .iter()
                .any(|member| provider_reference(member).0 == instance)
        })
        .map(|(group, _)| group.as_str())
        .collect::<Vec<_>>();
    match matching.as_slice() {
        [group] => Some(*group),
        _ => None,
    }
}

fn transparent_members(
    bundle: &Bundle,
    instances: &BTreeMap<&str, &Instance>,
    groups: &ResolvedGroups,
    group: &str,
    devices: &BTreeMap<String, PlanningDevice>,
) -> Vec<Value> {
    groups
        .members
        .get(group)
        .into_iter()
        .flatten()
        .flat_map(|member| {
            let (instance_name, requested_service) = provider_reference(member);
            let Some(instance) = instances.get(instance_name) else {
                return Vec::new();
            };
            if let Some(host) = instance.external.as_deref() {
                return vec![json!({
                    "component": instance_name,
                    "host": host,
                    "ports": instance.expanded_external_ports(),
                })];
            }
            let Some(block) = bundle.spec.blocks.get(&instance.block) else {
                return Vec::new();
            };
            block
                .services
                .keys()
                .filter(|service| {
                    requested_service.is_none_or(|requested| requested == service.as_str())
                })
                .map(|service| {
                    let host = instance
                        .device
                        .as_deref()
                        .filter(|device| *device != "local")
                        .and_then(|device| devices.get(device))
                        .map_or_else(
                            || service_name_for(&bundle.metadata.name, instance_name, service),
                            |device| device.host.clone(),
                        );
                    json!({
                        "component": format!("{instance_name}/{service}"),
                        "host": host,
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn router_config(
    bundle: &Bundle,
    instances: &BTreeMap<&str, &Instance>,
    consumer: &Instance,
    transparent: TransparentRoute<'_>,
    devices: &BTreeMap<String, PlanningDevice>,
) -> Result<Value, Box<dyn std::error::Error>> {
    let transition = json!({ "strategy": "close" });
    let mut spec = json!({
        "snapshot": {
            "id": resource_name(&[&bundle.metadata.name, &consumer.name, "initial"]),
            "version": 1,
            "transitions": {
                "http": transition,
                "https": transition,
                "websocket": transition,
                "grpc": transition,
                "tcp": transition,
            }
        },
        "listeners": [],
        "providers": [],
        "routes": [],
    });
    if transparent.enabled {
        spec["transparentProxy"] = json!({
            "consumer": consumer.name,
            "port": TRANSPARENT_INTERCEPTION_PORT,
            "members": transparent.group.map_or_else(
                Vec::new,
                |group| transparent_members(bundle, instances, transparent.groups, group, devices)
            ),
            "connectTimeoutMs": 250,
        });
    }
    Ok(json!({
        "apiVersion": "switchyard.dev/router/v1alpha1",
        "kind": "RouterConfiguration",
        "metadata": { "deployment": bundle.metadata.name },
        "spec": spec,
    }))
}

fn ownership_labels(deployment: &str, resource_hash: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("dev.switchyard.deployment".into(), deployment.into()),
        ("dev.switchyard.managed".into(), "true".into()),
        ("dev.switchyard.resource-hash".into(), resource_hash.into()),
    ])
}

fn service_name_for(deployment: &str, instance: &str, service: &str) -> String {
    resource_name(&[deployment, instance, service])
}

fn resource_name(parts: &[&str]) -> String {
    let joined = parts.join("--");
    if joined.len() <= 63 {
        return joined;
    }
    let digest = format!("{:x}", Sha256::digest(joined.as_bytes()));
    format!("{}-{}", &joined[..54], &digest[..8])
}

fn remote_network_name(deployment: &str, device: &str) -> String {
    resource_name(&["sy", &format!("{deployment}-{device}"), "private"])
}
