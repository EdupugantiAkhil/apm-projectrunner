//! Deterministic, side-effect-free deployment planning.

mod model;

pub use model::*;

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use switchyard_adapter_sdk::{
    AdapterKind, ConsumerSlot, Diagnostic as AdapterDiagnostic, Protocol as AdapterProtocol,
    ProviderCapability, RegisteredAdapter, RouteValidationContext,
};
use switchyard_adapters::built_in_registry;

#[derive(Debug)]
pub enum PlannerError {
    Io(io::Error),
    Yaml(serde_yaml::Error),
}

impl fmt::Display for PlannerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "could not read deployment: {error}"),
            Self::Yaml(error) => write!(f, "invalid deployment YAML: {error}"),
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
    MissingProvider,
    IncompatibleProtocol,
    IncompleteGroup,
    BackendGroupInvariant,
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub deployment: String,
    pub definition_hash: String,
    pub resource_hash: String,
    pub compose_project: String,
    pub artifact_dir: PathBuf,
    pub compose_yaml: String,
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
}

/// Loads one self-contained deployment bundle without changing runtime state.
pub fn load_bundle(path: &Path) -> Result<Bundle, PlannerError> {
    let input = fs::read_to_string(path)?;
    let mut bundle: Bundle = serde_yaml::from_str(&input)?;
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
    let resolved_groups = validate(bundle)?;
    generate(bundle, &resolved_groups).map_err(|error| {
        vec![Diagnostic::new(
            DiagnosticCode::InvalidPath,
            "$",
            error.to_string(),
        )]
    })
}

/// Replans with one service-group selection changed atomically.
pub fn plan_with_binding(
    bundle: &Bundle,
    consumer: &str,
    group: &str,
) -> Result<Plan, Vec<Diagnostic>> {
    let mut updated = bundle.clone();
    updated
        .spec
        .bindings
        .insert(consumer.to_owned(), group.to_owned());
    for route in updated
        .spec
        .ui_routes
        .values_mut()
        .filter(|route| route.backend == consumer)
    {
        route.downstream_group = group.to_owned();
    }
    plan(&updated)
}

/// Atomically writes disposable artifacts beneath `.switchyard/generated/<deployment>`.
pub fn write_plan(workspace_root: &Path, plan: &Plan) -> io::Result<PathBuf> {
    let artifact_dir = workspace_root.join(&plan.artifact_dir);
    fs::create_dir_all(artifact_dir.join("routes"))?;
    write_atomic(
        &artifact_dir.join("compose.yaml"),
        plan.compose_yaml.as_bytes(),
    )?;
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

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, path)
}

fn validate(
    bundle: &Bundle,
) -> Result<BTreeMap<String, BTreeMap<String, String>>, Vec<Diagnostic>> {
    let mut errors = Vec::new();
    if bundle.api_version != API_VERSION || bundle.kind != KIND {
        errors.push(Diagnostic::new(
            DiagnosticCode::UnsupportedSchema,
            "apiVersion",
            format!("expected {API_VERSION} kind {KIND}"),
        ));
    }
    validate_name(&bundle.metadata.name, "metadata.name", &mut errors);

    let adapters = built_in_registry();

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
        let path = resolve_path(&bundle.definition_dir, &source.path);
        if !path.is_dir() {
            errors.push(Diagnostic::new(
                DiagnosticCode::InvalidPath,
                format!("spec.sources.{name}.path"),
                format!("source directory does not exist: {}", path.display()),
            ));
        }
        if matches!(source.r#type, SourceType::Worktree) {
            match &source.repository {
                Some(repository) if resolve_path(&bundle.definition_dir, repository).is_dir() => {}
                Some(_) => errors.push(Diagnostic::new(
                    DiagnosticCode::InvalidPath,
                    format!("spec.sources.{name}"),
                    "worktree source needs an existing repository directory",
                )),
                None => {}
            }
        }
    }

    for (block_name, block) in &bundle.spec.blocks {
        validate_name(block_name, format!("spec.blocks.{block_name}"), &mut errors);
        if block.services.is_empty() {
            errors.push(Diagnostic::new(
                DiagnosticCode::MissingReference,
                format!("spec.blocks.{block_name}.services"),
                "a block must contain at least one service",
            ));
        }
        for (service_name, service) in &block.services {
            validate_name(
                service_name,
                format!("spec.blocks.{block_name}.services.{service_name}"),
                &mut errors,
            );
            validate_execution(block_name, service_name, service, &adapters, &mut errors);
            if let Some(probe) = &service.probe {
                validate_probe(block_name, service_name, probe, &adapters, &mut errors);
            }
            validate_route_slots(block_name, service_name, service, &adapters, &mut errors);
            for slot in service.provides.keys().chain(service.consumes.keys()) {
                validate_name(
                    slot,
                    format!("spec.blocks.{block_name}.services.{service_name}.{slot}"),
                    &mut errors,
                );
            }
            for volume in &service.volumes {
                validate_name(
                    &volume.name,
                    format!("spec.blocks.{block_name}.services.{service_name}.volumes"),
                    &mut errors,
                );
                if !volume.target.is_absolute() {
                    errors.push(Diagnostic::new(
                        DiagnosticCode::InvalidPath,
                        format!("spec.blocks.{block_name}.services.{service_name}.volumes"),
                        "volume target must be an absolute container path",
                    ));
                }
            }
        }
        validate_local_dependencies(block_name, block, &mut errors);
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
        let Some(block) = bundle.spec.blocks.get(&instance.block) else {
            errors.push(Diagnostic::new(
                DiagnosticCode::MissingReference,
                format!("{path}.block"),
                format!("unknown block {}", instance.block),
            ));
            continue;
        };
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
        validate_listener_conflicts(instance, block, &mut errors);
        if let Some(source) = bundle.spec.sources.get(&instance.source) {
            let source_path = resolve_path(&bundle.definition_dir, &source.path);
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

    let resolved_groups = resolve_groups(&bundle.spec.groups, &mut errors);
    validate_expanded_dependencies(bundle, &instances, &mut errors);
    validate_routes(bundle, &instances, &resolved_groups, &adapters, &mut errors);
    validate_ui_routes(bundle, &instances, &resolved_groups, &mut errors);
    for (ui, profile) in &bundle.spec.managed_profiles {
        let path = format!("spec.managedProfiles.{ui}");
        validate_name(ui, &path, &mut errors);
        validate_name(&profile.route, format!("{path}.route"), &mut errors);
        if !instances.contains_key(ui.as_str()) {
            errors.push(Diagnostic::new(
                DiagnosticCode::MissingReference,
                &path,
                "managed profile UI must name a declared instance",
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
        let Some(host_router) = &bundle.spec.host_router else {
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
    if let Some(host_router) = &bundle.spec.host_router {
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
        Ok(resolved_groups)
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
    match source.r#type {
        SourceType::Path => (
            "source-path",
            json!({ "path": source.path.to_string_lossy() }),
        ),
        SourceType::Worktree => (
            "source-git",
            json!({
                "path": source.path.to_string_lossy(),
                "repository": source.repository.as_ref().map(|path| path.to_string_lossy()),
                "ref": source.r#ref,
            }),
        ),
    }
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

fn validate_route_slots(
    block_name: &str,
    service_name: &str,
    service: &Service,
    adapters: &switchyard_adapter_sdk::AdapterRegistry,
    errors: &mut Vec<Diagnostic>,
) {
    let path = format!("spec.blocks.{block_name}.services.{service_name}");
    let Some(RegisteredAdapter::Route { adapter, common }) =
        adapters.lookup(AdapterKind::Route, "route-switchyard")
    else {
        errors.push(Diagnostic::new(
            DiagnosticCode::UnsupportedSchema,
            path,
            "built-in adapter route-switchyard is not registered",
        ));
        return;
    };
    extend_adapter_diagnostics(
        errors,
        common.validate_configuration(&json!({ "mode": "sidecar" })),
        DiagnosticCode::InvalidPath,
        &path,
    );
    for (slot_name, slot) in &service.consumes {
        let context = RouteValidationContext {
            consumer: service_name.into(),
            slot: ConsumerSlot {
                name: slot_name.clone(),
                protocol: adapter_protocol(slot.protocol),
                host: slot.address.host.clone(),
                port: slot.address.port,
            },
            provider: ProviderCapability {
                name: "validation-placeholder".into(),
                protocol: adapter_protocol(slot.protocol),
                port: 1,
            },
        };
        extend_adapter_diagnostics(
            errors,
            adapter.validate(&context),
            DiagnosticCode::InvalidPath,
            &format!("{path}.consumes.{slot_name}"),
        );
    }
    let declaration = common.declaration();
    let supported = &declaration.capabilities.protocols;
    for (capability_name, capability) in &service.provides {
        if capability.port == 0 {
            errors.push(Diagnostic::new(
                DiagnosticCode::InvalidPath,
                format!("{path}.provides.{capability_name}.port"),
                "route ports must be nonzero",
            ));
        }
        if !supported.contains(&adapter_protocol(capability.protocol)) {
            errors.push(Diagnostic::new(
                DiagnosticCode::IncompatibleProtocol,
                format!("{path}.provides.{capability_name}.protocol"),
                "route-switchyard does not support this provider protocol",
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

fn validate_listener_conflicts(instance: &Instance, block: &Block, errors: &mut Vec<Diagnostic>) {
    let mut listeners = BTreeMap::new();
    let mut consumer_services = BTreeSet::new();
    for (service_name, service) in &block.services {
        if !service.consumes.is_empty() {
            consumer_services.insert(service_name);
        }
        for (slot, route) in &service.consumes {
            let key = (&route.address.host, route.address.port);
            if let Some(first) = listeners.insert(key, slot) {
                errors.push(Diagnostic::new(
                    DiagnosticCode::ListenerConflict,
                    format!("spec.instances.{}.consumes.{slot}", instance.name),
                    format!("listener conflicts with slot {first}"),
                ));
            }
        }
    }
    if consumer_services.len() > 1 {
        errors.push(Diagnostic::new(
            DiagnosticCode::ListenerConflict,
            format!("spec.instances.{}", instance.name),
            "one sidecar cannot share more than one application network namespace",
        ));
    }
}

fn resolve_groups(
    groups: &BTreeMap<String, ServiceGroup>,
    errors: &mut Vec<Diagnostic>,
) -> BTreeMap<String, BTreeMap<String, String>> {
    fn resolve_one(
        name: &str,
        groups: &BTreeMap<String, ServiceGroup>,
        stack: &mut BTreeSet<String>,
        resolved: &mut BTreeMap<String, BTreeMap<String, String>>,
        errors: &mut Vec<Diagnostic>,
    ) -> BTreeMap<String, String> {
        if let Some(group) = resolved.get(name) {
            return group.clone();
        }
        if !stack.insert(name.to_owned()) {
            errors.push(Diagnostic::new(
                DiagnosticCode::DependencyCycle,
                format!("spec.groups.{name}.extends"),
                "service-group inheritance cycle detected",
            ));
            return BTreeMap::new();
        }
        let Some(group) = groups.get(name) else {
            errors.push(Diagnostic::new(
                DiagnosticCode::MissingReference,
                format!("spec.groups.{name}"),
                "unknown service group",
            ));
            return BTreeMap::new();
        };
        let mut providers = group
            .extends
            .as_deref()
            .map(|parent| resolve_one(parent, groups, stack, resolved, errors))
            .unwrap_or_default();
        providers.extend(group.providers.clone());
        stack.remove(name);
        resolved.insert(name.to_owned(), providers.clone());
        providers
    }

    let mut resolved = BTreeMap::new();
    for name in groups.keys() {
        validate_name(name, format!("spec.groups.{name}"), errors);
        resolve_one(name, groups, &mut BTreeSet::new(), &mut resolved, errors);
    }
    resolved
}

fn validate_routes(
    bundle: &Bundle,
    instances: &BTreeMap<&str, &Instance>,
    groups: &BTreeMap<String, BTreeMap<String, String>>,
    adapters: &switchyard_adapter_sdk::AdapterRegistry,
    errors: &mut Vec<Diagnostic>,
) {
    for (consumer, group) in &bundle.spec.bindings {
        if !instances.contains_key(consumer.as_str()) {
            errors.push(Diagnostic::new(
                DiagnosticCode::MissingReference,
                format!("spec.bindings.{consumer}"),
                "binding consumer does not exist",
            ));
        }
        if !groups.contains_key(group) {
            errors.push(Diagnostic::new(
                DiagnosticCode::MissingReference,
                format!("spec.bindings.{consumer}"),
                format!("service group {group} does not exist"),
            ));
        }
    }
    for (consumer, instance) in instances {
        let block = &bundle.spec.blocks[&instance.block];
        if !block
            .services
            .values()
            .any(|service| !service.consumes.is_empty())
        {
            continue;
        }
        let mut selected = bundle
            .spec
            .bindings
            .get(*consumer)
            .and_then(|group| groups.get(group))
            .cloned()
            .unwrap_or_default();
        if let Some(routes) = bundle.spec.routes.get(*consumer) {
            selected.extend(routes.clone());
        }
        for (slot, route_slot) in block
            .services
            .values()
            .flat_map(|service| &service.consumes)
        {
            let Some(provider_ref) = selected.get(slot) else {
                errors.push(Diagnostic::new(
                    DiagnosticCode::IncompleteGroup,
                    format!("spec.routes.{consumer}.{slot}"),
                    "consumer route slot has no selected provider",
                ));
                continue;
            };
            match provider_for(bundle, instances, provider_ref, slot) {
                Ok((_, capability)) => {
                    let path = format!("spec.routes.{consumer}.{slot}");
                    match adapters.lookup(AdapterKind::Route, "route-switchyard") {
                        Some(RegisteredAdapter::Route { adapter, .. }) => {
                            let context = RouteValidationContext {
                                consumer: (*consumer).into(),
                                slot: ConsumerSlot {
                                    name: slot.clone(),
                                    protocol: adapter_protocol(route_slot.protocol),
                                    host: route_slot.address.host.clone(),
                                    port: route_slot.address.port,
                                },
                                provider: ProviderCapability {
                                    name: provider_ref.clone(),
                                    protocol: adapter_protocol(capability.protocol),
                                    port: capability.port,
                                },
                            };
                            for diagnostic in adapter.validate(&context) {
                                if diagnostic.code == "adapter_incompatible_protocol" {
                                    errors.push(Diagnostic::new(
                                        DiagnosticCode::IncompatibleProtocol,
                                        &path,
                                        diagnostic.message,
                                    ));
                                }
                            }
                        }
                        _ => errors.push(Diagnostic::new(
                            DiagnosticCode::UnsupportedSchema,
                            path,
                            "built-in adapter route-switchyard is not registered",
                        )),
                    }
                }
                Err(message) => errors.push(Diagnostic::new(
                    DiagnosticCode::MissingProvider,
                    format!("spec.routes.{consumer}.{slot}"),
                    message,
                )),
            }
        }
    }
}

fn adapter_protocol(protocol: Protocol) -> AdapterProtocol {
    match protocol {
        Protocol::Http => AdapterProtocol::Http,
        Protocol::Https => AdapterProtocol::Https,
        Protocol::Websocket => AdapterProtocol::Websocket,
        Protocol::Grpc => AdapterProtocol::Grpc,
        Protocol::Tcp => AdapterProtocol::Tcp,
    }
}

fn validate_ui_routes(
    bundle: &Bundle,
    instances: &BTreeMap<&str, &Instance>,
    groups: &BTreeMap<String, BTreeMap<String, String>>,
    errors: &mut Vec<Diagnostic>,
) {
    let mut backend_requirements = BTreeMap::<&str, (&str, &str)>::new();
    for (ui, route) in &bundle.spec.ui_routes {
        let path = format!("spec.uiRoutes.{ui}");
        validate_name(ui, &path, errors);
        if !instances.contains_key(ui.as_str()) {
            errors.push(Diagnostic::new(
                DiagnosticCode::MissingReference,
                &path,
                "UI route names an unknown UI instance",
            ));
        }
        let backend = instances.get(route.backend.as_str());
        if backend.is_none() {
            errors.push(Diagnostic::new(
                DiagnosticCode::MissingReference,
                format!("{path}.backend"),
                format!("backend instance {} does not exist", route.backend),
            ));
        }
        if !groups.contains_key(&route.downstream_group) {
            errors.push(Diagnostic::new(
                DiagnosticCode::MissingReference,
                format!("{path}.downstreamGroup"),
                format!("service group {} does not exist", route.downstream_group),
            ));
        }

        if let Some((first_ui, first_group)) =
            backend_requirements.insert(&route.backend, (ui, &route.downstream_group))
        {
            if first_group != route.downstream_group {
                errors.push(Diagnostic::new(
                    DiagnosticCode::BackendGroupInvariant,
                    format!("{path}.downstreamGroup"),
                    format!(
                        "UI `{ui}` requests backend `{}` with group `{}`, but UI `{first_ui}` requests the same backend with group `{first_group}`; duplicate the backend instance (the copies may select the same source) because one backend cannot infer per-request downstream context",
                        route.backend, route.downstream_group
                    ),
                ));
            }
        }

        if backend.is_some() && groups.contains_key(&route.downstream_group) {
            match bundle.spec.bindings.get(&route.backend) {
                Some(selected) if selected == &route.downstream_group => {}
                Some(selected) => errors.push(Diagnostic::new(
                    DiagnosticCode::BackendGroupInvariant,
                    format!("{path}.downstreamGroup"),
                    format!(
                        "UI `{ui}` requests backend `{}` with group `{}`, but that backend is bound to `{selected}`; duplicate the backend instance to select a different downstream group",
                        route.backend, route.downstream_group
                    ),
                )),
                None => errors.push(Diagnostic::new(
                    DiagnosticCode::BackendGroupInvariant,
                    format!("{path}.backend"),
                    format!(
                        "backend `{}` has no complete downstream group binding",
                        route.backend
                    ),
                )),
            }
        }

        validate_ui_host_route(bundle, ui, route, &path, errors);
    }
}

fn validate_ui_host_route(
    bundle: &Bundle,
    ui: &str,
    route: &UiRoute,
    path: &str,
    errors: &mut Vec<Diagnostic>,
) {
    let Some(host_router) = &bundle.spec.host_router else {
        errors.push(Diagnostic::new(
            DiagnosticCode::MissingReference,
            path,
            "UI routes require spec.hostRouter browser routing",
        ));
        return;
    };
    let matching = host_router
        .spec
        .browser_routes
        .iter()
        .filter(|candidate| {
            matches!(
                &candidate.identity,
                router_config::BrowserIdentity::Origin { origin } if origin == &route.origin
            )
        })
        .collect::<Vec<_>>();
    if matching.len() != 1 {
        errors.push(Diagnostic::new(
            DiagnosticCode::MissingReference,
            format!("{path}.origin"),
            format!(
                "UI `{ui}` origin `{}` matches {} host-router browser routes; expected exactly one",
                route.origin,
                matching.len()
            ),
        ));
        return;
    }
    let provider = matching[0].provider.as_str();
    let mapped_backend = bundle
        .spec
        .host_upstreams
        .get(provider)
        .map(|upstream| upstream.instance.as_str());
    if mapped_backend != Some(route.backend.as_str()) {
        errors.push(Diagnostic::new(
            DiagnosticCode::MissingReference,
            format!("{path}.backend"),
            format!(
                "origin `{}` selects host provider `{provider}`, which does not map to backend `{}`",
                route.origin, route.backend
            ),
        ));
    }
}

fn validate_expanded_dependencies(
    bundle: &Bundle,
    instances: &BTreeMap<&str, &Instance>,
    errors: &mut Vec<Diagnostic>,
) {
    let mut graph = BTreeMap::<String, Vec<String>>::new();
    for instance in instances.values() {
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

fn provider_for<'a>(
    bundle: &'a Bundle,
    instances: &BTreeMap<&str, &'a Instance>,
    provider_ref: &str,
    slot: &str,
) -> Result<(&'a str, &'a Capability), String> {
    let (instance_name, requested_service) = provider_ref
        .split_once('/')
        .map_or((provider_ref, None), |(instance, service)| {
            (instance, Some(service))
        });
    let instance = instances
        .get(instance_name)
        .ok_or_else(|| format!("provider instance {instance_name} does not exist"))?;
    let block = &bundle.spec.blocks[&instance.block];
    let mut matches = block.services.iter().filter(|(name, service)| {
        requested_service.is_none_or(|requested| requested == name.as_str())
            && service.provides.contains_key(slot)
    });
    let Some((service, definition)) = matches.next() else {
        return Err(format!("{provider_ref} does not provide capability {slot}"));
    };
    if matches.next().is_some() {
        return Err(format!(
            "{provider_ref} is ambiguous; select an instance/service"
        ));
    }
    Ok((service, &definition.provides[slot]))
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
    groups: &BTreeMap<String, BTreeMap<String, String>>,
) -> Result<Plan, Box<dyn std::error::Error>> {
    let deployment = &bundle.metadata.name;
    let project = resource_name(&["sy", deployment]);
    let network = resource_name(&["sy", deployment, "private"]);
    let artifact_dir = PathBuf::from(".switchyard/generated").join(deployment);
    let runtime_dir = PathBuf::from(".switchyard/run").join(deployment);
    let artifact_bind_dir = bundle.workspace_root.join(&artifact_dir);
    let runtime_bind_dir = bundle.workspace_root.join(&runtime_dir);
    let definition_bytes = serde_json::to_vec(bundle)?;
    let definition_hash = format!("{:x}", Sha256::digest(definition_bytes));
    let mut resource_definition = bundle.clone();
    resource_definition.spec.bindings.clear();
    resource_definition.spec.routes.clear();
    resource_definition.spec.ui_routes.clear();
    resource_definition.spec.managed_profiles.clear();
    resource_definition.spec.host_router = None;
    resource_definition.spec.host_upstreams.clear();
    let resource_hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&resource_definition)?)
    );
    let labels = ownership_labels(deployment, &resource_hash);
    let instances = bundle
        .spec
        .instances
        .iter()
        .map(|instance| (instance.name.as_str(), instance))
        .collect::<BTreeMap<_, _>>();

    let mut services = serde_json::Map::new();
    let mut volumes = serde_json::Map::new();
    let mut manifest_services = Vec::new();
    let mut route_configs = BTreeMap::new();
    let mut sidecars = BTreeMap::new();
    let managed_profiles = managed_profiles(bundle);
    let host_router_config = generate_host_router_config(bundle, &managed_profiles)?;
    let host_upstreams = host_upstreams(bundle);

    for instance in &bundle.spec.instances {
        let mut instance_labels = labels.clone();
        instance_labels.insert("dev.switchyard.instance".into(), instance.name.clone());
        let block = &bundle.spec.blocks[&instance.block];
        let source = resolve_path(
            &bundle.definition_dir,
            &bundle.spec.sources[&instance.source].path,
        );
        let consumer_service = block
            .services
            .iter()
            .find(|(_, service)| !service.consumes.is_empty())
            .map(|(name, _)| name.as_str());
        for (service_name, service) in &block.services {
            let base_name = service_name_for(deployment, &instance.name, service_name);
            let is_consumer = consumer_service == Some(service_name.as_str());
            if is_consumer {
                let mut namespace =
                    compose_namespace_service(&base_name, &network, &instance_labels);
                if !service.publish.is_empty() {
                    namespace
                        .as_object_mut()
                        .expect("namespace is an object")
                        .insert("ports".into(), compose_ports(&service.publish));
                }
                services.insert(base_name.clone(), namespace);
                let app_name = resource_name(&[&base_name, "app"]);
                let sidecar_name = resource_name(&[&base_name, "router"]);
                let mut app = compose_application(
                    service,
                    instance,
                    &source,
                    &network,
                    &instance_labels,
                    bundle,
                    block,
                );
                let app_object = app.as_object_mut().expect("service is an object");
                add_compose_dependencies(app_object, bundle, instance, service);
                app_object.remove("networks");
                app_object.remove("ports");
                app_object.insert(
                    "network_mode".into(),
                    Value::String(format!("service:{base_name}")),
                );
                app_object
                    .entry("depends_on")
                    .or_insert_with(|| json!({}))
                    .as_object_mut()
                    .expect("depends_on is an object")
                    .insert(
                        sidecar_name.clone(),
                        json!({ "condition": "service_healthy" }),
                    );
                services.insert(app_name.clone(), app);

                let selected = selected_routes(bundle, groups, &instance.name);
                let config = router_config(bundle, &instances, instance, block, &selected)?;
                let provider_dependencies =
                    provider_dependencies(bundle, &instances, block, &selected)?;
                let config_path = artifact_dir
                    .join("routes")
                    .join(format!("{}.json", instance.name));
                let admin_socket = runtime_dir.join(format!("{}.socket", instance.name));
                let sidecar = compose_sidecar(
                    &bundle.spec.router_image,
                    &base_name,
                    &sidecar_name,
                    &artifact_bind_dir
                        .join("routes")
                        .join(format!("{}.json", instance.name)),
                    &runtime_bind_dir,
                    &provider_dependencies,
                    &instance_labels,
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
                manifest_services.push(json!({
                    "instance": instance.name,
                    "component": service_name,
                    "service": app_name,
                    "namespaceService": base_name,
                    "sidecar": sidecar_name,
                    "labels": instance_labels,
                }));
            } else {
                let mut app = compose_application(
                    service,
                    instance,
                    &source,
                    &network,
                    &instance_labels,
                    bundle,
                    block,
                );
                add_compose_dependencies(
                    app.as_object_mut().expect("service is an object"),
                    bundle,
                    instance,
                    service,
                );
                services.insert(base_name.clone(), app);
                manifest_services.push(json!({
                    "instance": instance.name,
                    "component": service_name,
                    "service": base_name,
                    "labels": instance_labels,
                }));
            }
            for mount in &service.volumes {
                let volume_name = resource_name(&[deployment, &instance.name, &mount.name]);
                volumes.insert(
                    volume_name,
                    json!({ "labels": instance_labels, "name": resource_name(&["sy", deployment, &instance.name, &mount.name]) }),
                );
            }
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
    let mut resolved = bundle.clone();
    for source in resolved.spec.sources.values_mut() {
        source.path = resolve_path(&bundle.definition_dir, &source.path);
    }
    let resolved_deployment_yaml = serde_yaml::to_string(&resolved)?;
    let manifest = json!({
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
    });
    let manifest_json = serde_json::to_string_pretty(&manifest)?;

    Ok(Plan {
        deployment: deployment.clone(),
        definition_hash,
        resource_hash,
        compose_project: project,
        artifact_dir,
        compose_yaml,
        resolved_deployment_yaml,
        manifest_json,
        route_configs,
        sidecars,
        managed_profiles,
        host_router_config,
        host_upstreams,
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

fn host_upstreams(bundle: &Bundle) -> BTreeMap<String, HostUpstreamPlan> {
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
    profiles: &BTreeMap<String, ManagedProfilePlan>,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let Some(mut config) = bundle.spec.host_router.clone() else {
        return Ok(None);
    };
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
    variables.extend(instance.parameters.clone());
    variables.extend(environment.clone());
    variables.insert("SWITCHYARD_DEPLOYMENT".into(), bundle.metadata.name.clone());
    variables.insert("SWITCHYARD_INSTANCE".into(), instance.name.clone());
    if !variables.is_empty() {
        value.insert("environment".into(), json!(variables));
    }
}

fn add_compose_dependencies(
    value: &mut serde_json::Map<String, Value>,
    bundle: &Bundle,
    instance: &Instance,
    service: &Service,
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
            if bundle
                .spec
                .instances
                .iter()
                .find(|candidate| candidate.name == target_instance)
                .and_then(|candidate| bundle.spec.blocks.get(&candidate.block))
                .and_then(|block| block.services.get(target_service))
                .is_some_and(|target| !target.consumes.is_empty())
            {
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
    runtime_dir: &Path,
    providers: &BTreeMap<String, bool>,
    labels: &BTreeMap<String, String>,
) -> Value {
    let mut depends_on = serde_json::Map::new();
    depends_on.insert(
        namespace_service.into(),
        json!({ "condition": "service_started" }),
    );
    for (provider, healthy) in providers {
        depends_on.insert(
            provider.clone(),
            json!({
                "condition": if *healthy {
                    "service_healthy"
                } else {
                    "service_started"
                }
            }),
        );
    }
    json!({
        "image": image,
        "user": "${SWITCHYARD_UID:-1000}:${SWITCHYARD_GID:-1000}",
        "restart": "unless-stopped",
        "network_mode": format!("service:{namespace_service}"),
        "command": [
            "/usr/local/bin/switchyard-router",
            format!("/config/{}", config_path.file_name().unwrap_or_default().to_string_lossy()),
            format!("/run/switchyard/{}.socket", config_path.file_stem().unwrap_or_default().to_string_lossy()),
        ],
        "environment": {
            "SWITCHYARD_ROUTER_TOKEN": "${SWITCHYARD_ROUTER_TOKEN:?set SWITCHYARD_ROUTER_TOKEN}"
        },
        "volumes": [
            format!("{}:/config/{}:ro", config_path.display(), config_path.file_name().unwrap_or_default().to_string_lossy()),
            format!("{}:/run/switchyard", runtime_dir.display()),
        ],
        "depends_on": depends_on,
        "healthcheck": {
            "test": ["CMD", "test", "-S", format!("/run/switchyard/{}.socket", config_path.file_stem().unwrap_or_default().to_string_lossy())],
            "interval": "1s",
            "timeout": "1s",
            "retries": 30,
        },
        "labels": labels,
        "container_name": sidecar_name,
    })
}

fn provider_dependencies(
    bundle: &Bundle,
    instances: &BTreeMap<&str, &Instance>,
    block: &Block,
    selected: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, bool>, io::Error> {
    let mut dependencies = BTreeMap::new();
    for slot in block
        .services
        .values()
        .flat_map(|service| service.consumes.keys())
    {
        let provider_ref = &selected[slot];
        let (provider_service, _) =
            provider_for(bundle, instances, provider_ref, slot).map_err(io::Error::other)?;
        let provider_instance = provider_ref.split('/').next().unwrap_or(provider_ref);
        let provider_definition =
            &bundle.spec.blocks[&instances[provider_instance].block].services[provider_service];
        let base = service_name_for(&bundle.metadata.name, provider_instance, provider_service);
        let dependency = if provider_definition.consumes.is_empty() {
            base
        } else {
            resource_name(&[&base, "app"])
        };
        dependencies.insert(dependency, provider_definition.probe.is_some());
    }
    Ok(dependencies)
}

fn selected_routes(
    bundle: &Bundle,
    groups: &BTreeMap<String, BTreeMap<String, String>>,
    consumer: &str,
) -> BTreeMap<String, String> {
    let mut selected = bundle
        .spec
        .bindings
        .get(consumer)
        .and_then(|group| groups.get(group))
        .cloned()
        .unwrap_or_default();
    if let Some(routes) = bundle.spec.routes.get(consumer) {
        selected.extend(routes.clone());
    }
    selected
}

fn router_config(
    bundle: &Bundle,
    instances: &BTreeMap<&str, &Instance>,
    consumer: &Instance,
    block: &Block,
    selected: &BTreeMap<String, String>,
) -> Result<Value, Box<dyn std::error::Error>> {
    let mut listeners = Vec::new();
    let mut providers = Vec::new();
    let mut routes = Vec::new();
    for (slot, route_slot) in block
        .services
        .values()
        .flat_map(|service| &service.consumes)
    {
        let provider_ref = &selected[slot];
        let (provider_service, capability) =
            provider_for(bundle, instances, provider_ref, slot).map_err(io::Error::other)?;
        let provider_instance = provider_ref.split('/').next().unwrap_or(provider_ref);
        let dns = service_name_for(&bundle.metadata.name, provider_instance, provider_service);
        let provider_id = format!("{provider_instance}/{provider_service}--{slot}");
        let provider_definition =
            &bundle.spec.blocks[&instances[provider_instance].block].services[provider_service];
        listeners.push(json!({
            "consumer": consumer.name,
            "bind": { "host": route_slot.address.host, "port": route_slot.address.port },
            "protocol": protocol_name(route_slot.protocol),
            "destinations": [{ "kind": "loopback", "slot": slot }],
        }));
        let mut provider = json!({
            "id": provider_id,
            "endpoint": {
                "protocol": protocol_name(capability.protocol),
                "host": dns,
                "port": capability.port,
            }
        });
        if let Some(health_check) = provider_router_health(provider_definition.probe.as_ref()) {
            provider["healthCheck"] = health_check;
        }
        providers.push(provider);
        routes.push(json!({ "consumer": consumer.name, "slot": slot, "provider": provider_id }));
    }
    let transition = json!({ "strategy": "close" });
    Ok(json!({
        "apiVersion": "switchyard.dev/router/v1alpha1",
        "kind": "RouterConfiguration",
        "metadata": { "deployment": bundle.metadata.name },
        "spec": {
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
            "listeners": listeners,
            "providers": providers,
            "routes": routes,
        }
    }))
}

fn provider_router_health(probe: Option<&Probe>) -> Option<Value> {
    match probe? {
        Probe::Http { path, https, .. } => Some(json!({
            "protocol": if *https { "https" } else { "http" },
            "path": path,
            "intervalMs": 1000,
            "timeoutMs": 500,
        })),
        Probe::Tcp { .. } => Some(json!({
            "protocol": "tcp",
            "intervalMs": 1000,
            "timeoutMs": 500,
        })),
        Probe::Command { .. } => None,
    }
}

fn protocol_name(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::Http => "http",
        Protocol::Https => "https",
        Protocol::Websocket => "websocket",
        Protocol::Grpc => "grpc",
        Protocol::Tcp => "tcp",
    }
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
