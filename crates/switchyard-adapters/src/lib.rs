//! Built-in adapters which express the existing Compose planner semantics through the SDK.
//!
//! These adapters validate and plan resources only. Execution remains owned by Switchyard's
//! existing generated-Compose runtime and host gateway.

use std::{collections::BTreeMap, path::Path, process::Command};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use switchyard_adapter_sdk::{
    Adapter, AdapterDeclaration, AdapterRegistry, CapabilityMetadata, ChildRelationship,
    ConfigurationExample, Diagnostic, ExecutionAdapter, ExecutionContext, ExecutionPlan, LogRecord,
    ObservedRoute, ObservedState, OperationEvent, ProbeAdapter, ProbeOutcome, Protocol,
    RecoveryLabels, ResourceClaim, RouteAdapter, RouteChange, RouteConnection, RouteHandle,
    RouteValidationContext, RuntimeHandle, SDK_CONTRACT_VERSION, SourceAdapter, SourceIdentity,
    SupervisorAdapter, schema_for, validate_schema,
};

const VERSION: &str = "0.1.0";

/// Builds the deterministic registry used by the planner and future control-plane clients.
#[must_use]
pub fn built_in_registry() -> AdapterRegistry {
    let mut registry = AdapterRegistry::new();
    registry
        .register_source(SourcePathAdapter)
        .expect("built-in source-path declaration is valid");
    registry
        .register_source(SourceGitAdapter)
        .expect("built-in source-git declaration is valid");
    registry
        .register_execution(ContainerExecutionAdapter)
        .expect("built-in execution-container declaration is valid");
    registry
        .register_execution(RunnerScriptExecutionAdapter)
        .expect("built-in execution-runner-script declaration is valid");
    registry
        .register_supervisor(ProcessComposeSupervisorAdapter)
        .expect("built-in supervisor-process-compose declaration is valid");
    registry
        .register_route(SwitchyardRouteAdapter)
        .expect("built-in route-switchyard declaration is valid");
    registry
        .register_probe(HealthProbeAdapter)
        .expect("built-in probe-health declaration is valid");
    registry
}

fn declaration(
    id: &str,
    protocols: Vec<Protocol>,
    live: bool,
    recovery: bool,
    features: &[&str],
) -> AdapterDeclaration {
    AdapterDeclaration {
        id: id.into(),
        version: VERSION.into(),
        sdk_contracts: vec![SDK_CONTRACT_VERSION.into()],
        capabilities: CapabilityMetadata {
            protocols,
            supports_live_update: live,
            supports_recovery: recovery,
            features: features.iter().map(|feature| (*feature).into()).collect(),
        },
    }
}

fn validate_typed<T: DeserializeOwned + JsonSchema>(configuration: &Value) -> Vec<Diagnostic> {
    let errors = validate_schema(&schema_for::<T>(), configuration);
    if !errors.is_empty() {
        return errors;
    }
    serde_json::from_value::<T>(configuration.clone())
        .err()
        .map(|error| {
            vec![Diagnostic::new(
                "adapter_config_deserialize",
                "$",
                error.to_string(),
            )]
        })
        .unwrap_or_default()
}

fn examples(valid: Value, invalid: Value) -> Vec<ConfigurationExample> {
    vec![
        ConfigurationExample {
            name: "valid".into(),
            configuration: valid,
            valid: true,
        },
        ConfigurationExample {
            name: "invalid".into(),
            configuration: invalid,
            valid: false,
        },
    ]
}

fn invalid_path(path: &str, field: &str) -> Option<Diagnostic> {
    let value = Path::new(path);
    (value.is_absolute() || value.components().any(|part| part.as_os_str() == "..")).then(|| {
        Diagnostic::new(
            "adapter_invalid_path",
            format!("$.{field}"),
            "path must stay within the selected source",
        )
    })
}

fn execution_plan(context: &ExecutionContext, kind: &str) -> ExecutionPlan {
    let commands = context
        .configuration
        .get("command")
        .and_then(Value::as_array)
        .map(|command| {
            vec![
                command
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect(),
            ]
        })
        .or_else(|| {
            context
                .configuration
                .get("file")
                .and_then(Value::as_str)
                .map(|file| {
                    vec![vec![
                        "process-compose".into(),
                        "-f".into(),
                        file.into(),
                        "up".into(),
                    ]]
                })
        })
        .unwrap_or_default();
    ExecutionPlan {
        resources: vec![json!({
            "kind": kind,
            "component": context.component,
            "configuration": context.configuration,
        })],
        commands,
        claims: vec![ResourceClaim {
            kind: "compose-service".into(),
            value: format!("{}--{}", context.deployment, context.component),
            exclusive: true,
        }],
    }
}

fn lifecycle_event(code: &str, state: ObservedState) -> Vec<OperationEvent> {
    vec![OperationEvent {
        code: code.into(),
        message: "operation delegated to the generated Compose runtime".into(),
        state: Some(state),
    }]
}

fn planning_handle(context: &ExecutionContext, adapter: &str) -> RuntimeHandle {
    RuntimeHandle(json!({
        "adapter": adapter,
        "deployment": context.deployment,
        "component": context.component,
    }))
}

fn recover_handle(
    labels: &RecoveryLabels,
    adapter: &str,
) -> Result<RuntimeHandle, Vec<Diagnostic>> {
    let Some(deployment) = labels.get("dev.switchyard.deployment") else {
        return Err(vec![Diagnostic::new(
            "adapter_recovery_label_missing",
            "$.dev.switchyard.deployment",
            "recovery requires the Switchyard deployment ownership label",
        )]);
    };
    Ok(RuntimeHandle(json!({
        "adapter": adapter,
        "deployment": deployment,
        "labels": labels,
    })))
}

macro_rules! execution_contract {
    ($adapter_id:literal, $config:ty, $resource_kind:literal) => {
        fn validate(&self, context: &ExecutionContext) -> Vec<Diagnostic> {
            self.validate_configuration(&context.configuration)
        }

        fn plan(&self, context: &ExecutionContext) -> Result<ExecutionPlan, Vec<Diagnostic>> {
            let errors = self.validate(context);
            if errors.is_empty() {
                Ok(execution_plan(context, $resource_kind))
            } else {
                Err(errors)
            }
        }

        fn prepare(
            &self,
            context: &ExecutionContext,
        ) -> Result<Vec<OperationEvent>, Vec<Diagnostic>> {
            let errors = self.validate(context);
            if errors.is_empty() {
                Ok(lifecycle_event("prepared", ObservedState::Preparing))
            } else {
                Err(errors)
            }
        }

        fn start(&self, context: &ExecutionContext) -> Result<RuntimeHandle, Vec<Diagnostic>> {
            let errors = self.validate(context);
            if errors.is_empty() {
                Ok(planning_handle(context, $adapter_id))
            } else {
                Err(errors)
            }
        }

        fn inspect(&self, _handle: &RuntimeHandle) -> Result<ObservedState, Vec<Diagnostic>> {
            Ok(ObservedState::Unknown)
        }

        fn logs(&self, _handle: &RuntimeHandle) -> Result<Vec<LogRecord>, Vec<Diagnostic>> {
            Ok(Vec::new())
        }

        fn stop(&self, _handle: &RuntimeHandle) -> Result<Vec<OperationEvent>, Vec<Diagnostic>> {
            Ok(lifecycle_event("stopped", ObservedState::Absent))
        }

        fn cleanup(&self, _handle: &RuntimeHandle) -> Result<Vec<OperationEvent>, Vec<Diagnostic>> {
            Ok(lifecycle_event("cleaned_up", ObservedState::Absent))
        }

        fn recover(&self, labels: &RecoveryLabels) -> Result<RuntimeHandle, Vec<Diagnostic>> {
            recover_handle(labels, $adapter_id)
        }
    };
}

/// Local directory source configuration.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourcePathConfig {
    /// Local directory path.
    #[schemars(length(min = 1))]
    pub path: String,
}

/// Non-mutating local-directory source adapter.
#[derive(Clone, Copy, Debug)]
pub struct SourcePathAdapter;

impl Adapter for SourcePathAdapter {
    fn declaration(&self) -> AdapterDeclaration {
        declaration(
            "source-path",
            Vec::new(),
            false,
            false,
            &["local-directory"],
        )
    }

    fn configuration_schema(&self) -> schemars::Schema {
        schema_for::<SourcePathConfig>()
    }

    fn configuration_examples(&self) -> Vec<ConfigurationExample> {
        examples(json!({"path": "/workspace/product"}), json!({"path": ""}))
    }

    fn validate_configuration(&self, configuration: &Value) -> Vec<Diagnostic> {
        validate_typed::<SourcePathConfig>(configuration)
    }

    fn example_handles(&self) -> Vec<Value> {
        vec![json!({"path": "/workspace/product"})]
    }
}

impl SourceAdapter for SourcePathAdapter {
    fn inspect(&self, configuration: &Value) -> Result<SourceIdentity, Vec<Diagnostic>> {
        let errors = self.validate_configuration(configuration);
        if !errors.is_empty() {
            return Err(errors);
        }
        let config: SourcePathConfig = serde_json::from_value(configuration.clone())
            .expect("validated source-path configuration deserializes");
        Ok(SourceIdentity {
            path: config.path,
            repository: None,
            r#ref: None,
            commit: None,
            dirty: None,
        })
    }
}

/// Existing Git repository or worktree source configuration.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceGitConfig {
    /// Selected worktree path.
    #[schemars(length(min = 1))]
    pub path: String,
    /// Repository root used to identify the worktree.
    #[schemars(length(min = 1))]
    pub repository: String,
    /// Requested branch, tag, or commit.
    #[schemars(length(min = 1))]
    pub r#ref: String,
}

/// Non-mutating Git/worktree source adapter.
#[derive(Clone, Copy, Debug)]
pub struct SourceGitAdapter;

impl Adapter for SourceGitAdapter {
    fn declaration(&self) -> AdapterDeclaration {
        declaration("source-git", Vec::new(), false, false, &["git", "worktree"])
    }

    fn configuration_schema(&self) -> schemars::Schema {
        schema_for::<SourceGitConfig>()
    }

    fn configuration_examples(&self) -> Vec<ConfigurationExample> {
        examples(
            json!({"path": "/worktrees/feature", "repository": "/code/product", "ref": "feature/api"}),
            json!({"path": "/worktrees/feature", "repository": "/code/product", "ref": ""}),
        )
    }

    fn validate_configuration(&self, configuration: &Value) -> Vec<Diagnostic> {
        validate_typed::<SourceGitConfig>(configuration)
    }

    fn example_handles(&self) -> Vec<Value> {
        vec![json!({"path": "/worktrees/feature", "ref": "feature/api"})]
    }
}

impl SourceAdapter for SourceGitAdapter {
    fn inspect(&self, configuration: &Value) -> Result<SourceIdentity, Vec<Diagnostic>> {
        let errors = self.validate_configuration(configuration);
        if !errors.is_empty() {
            return Err(errors);
        }
        let config: SourceGitConfig = serde_json::from_value(configuration.clone())
            .expect("validated source-git configuration deserializes");
        Ok(SourceIdentity {
            path: config.path,
            repository: Some(config.repository),
            r#ref: Some(config.r#ref),
            commit: git_output(
                configuration["path"].as_str().expect("path was validated"),
                &["rev-parse", "HEAD"],
            ),
            dirty: git_output(
                configuration["path"].as_str().expect("path was validated"),
                &["status", "--porcelain"],
            )
            .map(|output| !output.is_empty()),
        })
    }
}

fn git_output(path: &str, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(arguments)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Dockerfile build configuration matching the planner model.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContainerBuildConfig {
    /// Context relative to the selected source.
    #[schemars(length(min = 1))]
    pub context: String,
    /// Dockerfile relative to the build context or selected source.
    pub dockerfile: Option<String>,
}

/// Container execution configuration matching generated Compose semantics.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContainerExecutionConfig {
    /// Existing image reference.
    pub image: Option<String>,
    /// Optional source build.
    pub build: Option<ContainerBuildConfig>,
    /// Argument-array command override.
    #[serde(default)]
    pub command: Vec<String>,
    /// Container working directory.
    pub working_directory: Option<String>,
    /// Container environment.
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
}

/// Image or Dockerfile-backed execution adapter.
#[derive(Clone, Copy, Debug)]
pub struct ContainerExecutionAdapter;

impl Adapter for ContainerExecutionAdapter {
    fn declaration(&self) -> AdapterDeclaration {
        declaration(
            "execution-container",
            Vec::new(),
            false,
            true,
            &["compose", "image", "dockerfile"],
        )
    }

    fn configuration_schema(&self) -> schemars::Schema {
        schema_for::<ContainerExecutionConfig>()
    }

    fn configuration_examples(&self) -> Vec<ConfigurationExample> {
        examples(
            json!({"image": "example/api:local", "build": null, "command": [], "workingDirectory": null, "environment": {}}),
            json!({"image": null, "build": null, "command": [], "workingDirectory": null, "environment": {}}),
        )
    }

    fn validate_configuration(&self, configuration: &Value) -> Vec<Diagnostic> {
        let mut errors = validate_typed::<ContainerExecutionConfig>(configuration);
        if !errors.is_empty() {
            return errors;
        }
        let config: ContainerExecutionConfig = serde_json::from_value(configuration.clone())
            .expect("validated container configuration deserializes");
        if config.image.as_deref().is_none_or(str::is_empty) && config.build.is_none() {
            errors.push(Diagnostic::new(
                "adapter_missing_reference",
                "$",
                "container execution needs image or build",
            ));
        }
        if let Some(build) = config.build {
            if let Some(error) = invalid_path(&build.context, "build.context") {
                errors.push(error);
            }
            if let Some(dockerfile) = build.dockerfile {
                if let Some(error) = invalid_path(&dockerfile, "build.dockerfile") {
                    errors.push(error);
                }
            }
        }
        errors
    }

    fn example_handles(&self) -> Vec<Value> {
        vec![json!({"composeProject": "demo", "service": "demo--api"})]
    }
}

impl ExecutionAdapter for ContainerExecutionAdapter {
    execution_contract!("execution-container", ContainerExecutionConfig, "container");
}

/// Script lifecycle behavior.
#[derive(Clone, Copy, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ScriptLifecycle {
    /// Long-running service.
    #[default]
    Service,
    /// Finite task which must complete successfully.
    Task,
}

/// Runner-container script execution configuration.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunnerScriptConfig {
    /// Runner image containing the required toolchain.
    #[schemars(length(min = 1))]
    pub image: String,
    /// Script command and arguments.
    #[schemars(length(min = 1))]
    pub command: Vec<String>,
    /// Working directory inside the runner.
    pub working_directory: Option<String>,
    /// Selected source mount point.
    #[schemars(length(min = 1))]
    pub source_mount: String,
    /// Whether the source mount is writable.
    pub writable: bool,
    /// Runner environment.
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    /// Service or finite-task behavior.
    #[serde(default)]
    pub lifecycle: ScriptLifecycle,
}

/// Container-isolated script execution adapter.
#[derive(Clone, Copy, Debug)]
pub struct RunnerScriptExecutionAdapter;

impl Adapter for RunnerScriptExecutionAdapter {
    fn declaration(&self) -> AdapterDeclaration {
        declaration(
            "execution-runner-script",
            Vec::new(),
            false,
            true,
            &["compose", "runner-container", "service", "task"],
        )
    }

    fn configuration_schema(&self) -> schemars::Schema {
        schema_for::<RunnerScriptConfig>()
    }

    fn configuration_examples(&self) -> Vec<ConfigurationExample> {
        examples(
            json!({"image": "runner:local", "command": ["./start.sh"], "workingDirectory": null, "sourceMount": "/workspace", "writable": false, "environment": {}, "lifecycle": "service"}),
            json!({"image": "runner:local", "command": [], "workingDirectory": null, "sourceMount": "/workspace", "writable": false, "environment": {}, "lifecycle": "service"}),
        )
    }

    fn validate_configuration(&self, configuration: &Value) -> Vec<Diagnostic> {
        validate_typed::<RunnerScriptConfig>(configuration)
    }

    fn example_handles(&self) -> Vec<Value> {
        vec![json!({"composeProject": "demo", "service": "demo--runner"})]
    }
}

impl ExecutionAdapter for RunnerScriptExecutionAdapter {
    execution_contract!(
        "execution-runner-script",
        RunnerScriptConfig,
        "runner-script"
    );
}

/// Process Compose suite configuration inside a runner container.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessComposeConfig {
    /// Runner image containing Process Compose and child dependencies.
    #[schemars(length(min = 1))]
    pub image: String,
    /// Process Compose file relative to the source.
    #[schemars(length(min = 1))]
    pub file: String,
    /// Working directory inside the runner.
    pub working_directory: Option<String>,
    /// Selected source mount point.
    #[schemars(length(min = 1))]
    pub source_mount: String,
    /// Whether the source mount is writable.
    pub writable: bool,
    /// Runner environment.
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    /// Imported child dependency and readiness metadata.
    #[serde(default)]
    pub children: Vec<ProcessComposeChild>,
}

/// One imported Process Compose child relationship.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessComposeChild {
    /// Process name.
    #[schemars(length(min = 1))]
    pub name: String,
    /// Process names required first.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Whether Process Compose declares a readiness condition.
    #[serde(default)]
    pub readiness_required: bool,
}

/// Process Compose supervisor hosted inside the existing runner-container runtime.
#[derive(Clone, Copy, Debug)]
pub struct ProcessComposeSupervisorAdapter;

impl Adapter for ProcessComposeSupervisorAdapter {
    fn declaration(&self) -> AdapterDeclaration {
        declaration(
            "supervisor-process-compose",
            Vec::new(),
            false,
            true,
            &["compose", "runner-container", "child-readiness"],
        )
    }

    fn configuration_schema(&self) -> schemars::Schema {
        schema_for::<ProcessComposeConfig>()
    }

    fn configuration_examples(&self) -> Vec<ConfigurationExample> {
        examples(
            json!({"image": "ai-runner:local", "file": "process-compose.yaml", "workingDirectory": null, "sourceMount": "/workspace", "writable": false, "environment": {}, "children": [{"name": "api", "dependsOn": [], "readinessRequired": true}]}),
            json!({"image": "ai-runner:local", "file": "../process-compose.yaml", "workingDirectory": null, "sourceMount": "/workspace", "writable": false, "environment": {}, "children": []}),
        )
    }

    fn validate_configuration(&self, configuration: &Value) -> Vec<Diagnostic> {
        let mut errors = validate_typed::<ProcessComposeConfig>(configuration);
        if !errors.is_empty() {
            return errors;
        }
        let config: ProcessComposeConfig = serde_json::from_value(configuration.clone())
            .expect("validated Process Compose configuration deserializes");
        if let Some(error) = invalid_path(&config.file, "file") {
            errors.push(error);
        }
        errors
    }

    fn example_handles(&self) -> Vec<Value> {
        vec![json!({"composeProject": "demo", "suite": "ai"})]
    }
}

impl ExecutionAdapter for ProcessComposeSupervisorAdapter {
    execution_contract!(
        "supervisor-process-compose",
        ProcessComposeConfig,
        "process-compose"
    );
}

impl SupervisorAdapter for ProcessComposeSupervisorAdapter {
    fn child_relationships(
        &self,
        configuration: &Value,
    ) -> Result<Vec<ChildRelationship>, Vec<Diagnostic>> {
        let errors = self.validate_configuration(configuration);
        if !errors.is_empty() {
            return Err(errors);
        }
        let config: ProcessComposeConfig = serde_json::from_value(configuration.clone())
            .expect("validated Process Compose configuration deserializes");
        Ok(config
            .children
            .into_iter()
            .map(|child| ChildRelationship {
                child: child.name,
                depends_on: child.depends_on,
                readiness_required: child.readiness_required,
            })
            .collect())
    }
}

/// Switchyard route placement.
#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SwitchyardRouteMode {
    /// Sidecar sharing the unchanged consumer's network namespace.
    Sidecar,
    /// Native shared host gateway.
    HostGateway,
}

/// Switchyard HTTP-family and TCP route configuration.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SwitchyardRouteConfig {
    /// Placement used to realize the route.
    pub mode: SwitchyardRouteMode,
}

/// Sidecar and host-gateway route adapter for loopback HTTP-family and raw TCP slots.
#[derive(Clone, Copy, Debug)]
pub struct SwitchyardRouteAdapter;

impl Adapter for SwitchyardRouteAdapter {
    fn declaration(&self) -> AdapterDeclaration {
        declaration(
            "route-switchyard",
            vec![
                Protocol::Http,
                Protocol::Https,
                Protocol::Websocket,
                Protocol::Grpc,
                Protocol::Tcp,
            ],
            true,
            true,
            &["sidecar", "host-gateway", "loopback"],
        )
    }

    fn configuration_schema(&self) -> schemars::Schema {
        schema_for::<SwitchyardRouteConfig>()
    }

    fn configuration_examples(&self) -> Vec<ConfigurationExample> {
        examples(json!({"mode": "sidecar"}), json!({"mode": "environment"}))
    }

    fn validate_configuration(&self, configuration: &Value) -> Vec<Diagnostic> {
        validate_typed::<SwitchyardRouteConfig>(configuration)
    }

    fn example_handles(&self) -> Vec<Value> {
        vec![json!({"consumer": "ui-main", "slot": "api", "provider": "api-main"})]
    }
}

impl RouteAdapter for SwitchyardRouteAdapter {
    fn validate(&self, context: &RouteValidationContext) -> Vec<Diagnostic> {
        let mut errors = Vec::new();
        if context.slot.protocol != context.provider.protocol {
            errors.push(Diagnostic::new(
                "adapter_incompatible_protocol",
                "$.slot.protocol",
                "consumer and provider protocols differ",
            ));
        }
        if context.slot.port == 0 || context.provider.port == 0 {
            errors.push(Diagnostic::new(
                "adapter_invalid_port",
                "$",
                "route ports must be nonzero",
            ));
        }
        if !is_loopback(&context.slot.host) {
            errors.push(Diagnostic::new(
                "adapter_invalid_listener",
                "$.slot.host",
                "Switchyard route slots must use localhost or a loopback IP address",
            ));
        }
        errors
    }

    fn plan(&self, connection: &RouteConnection) -> Result<RouteChange, Vec<Diagnostic>> {
        let mut errors = self.validate_configuration(&connection.configuration);
        errors.extend(self.validate(&connection.route));
        if errors.is_empty() {
            Ok(RouteChange::Live)
        } else {
            Err(errors)
        }
    }

    fn apply(&self, connection: &RouteConnection) -> Result<RouteHandle, Vec<Diagnostic>> {
        self.plan(connection)?;
        Ok(RouteHandle(json!({
            "consumer": connection.route.consumer,
            "slot": connection.route.slot.name,
            "provider": connection.route.provider.name,
        })))
    }

    fn remove(&self, _handle: &RouteHandle) -> Result<Vec<OperationEvent>, Vec<Diagnostic>> {
        Ok(lifecycle_event("route_removed", ObservedState::Absent))
    }

    fn inspect(&self, handle: &RouteHandle) -> Result<ObservedRoute, Vec<Diagnostic>> {
        let provider = handle
            .0
            .get("provider")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "adapter_route_handle_invalid",
                    "$.provider",
                    "route handle has no provider",
                )]
            })?;
        Ok(ObservedRoute {
            provider: provider.into(),
            state: ObservedState::Ready,
        })
    }
}

fn is_loopback(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

/// Readiness and health probe configuration.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum HealthProbeConfig {
    /// HTTP GET readiness probe.
    Http {
        /// Request path.
        #[schemars(length(min = 1))]
        path: String,
        /// Target port.
        #[schemars(range(min = 1))]
        port: u16,
        /// Use HTTPS instead of HTTP.
        #[serde(default)]
        https: bool,
    },
    /// TCP connection probe.
    Tcp {
        /// Target port.
        #[schemars(range(min = 1))]
        port: u16,
    },
    /// Argument-array command probe.
    Command {
        /// Command and arguments.
        #[schemars(length(min = 1))]
        command: Vec<String>,
    },
}

/// HTTP, TCP, and command health-probe adapter.
#[derive(Clone, Copy, Debug)]
pub struct HealthProbeAdapter;

impl Adapter for HealthProbeAdapter {
    fn declaration(&self) -> AdapterDeclaration {
        declaration(
            "probe-health",
            vec![Protocol::Http, Protocol::Https, Protocol::Tcp],
            false,
            false,
            &["readiness", "health", "command"],
        )
    }

    fn configuration_schema(&self) -> schemars::Schema {
        schema_for::<HealthProbeConfig>()
    }

    fn configuration_examples(&self) -> Vec<ConfigurationExample> {
        examples(
            json!({"type": "http", "path": "/health", "port": 8080, "https": false}),
            json!({"type": "command", "command": []}),
        )
    }

    fn validate_configuration(&self, configuration: &Value) -> Vec<Diagnostic> {
        validate_typed::<HealthProbeConfig>(configuration)
    }

    fn example_handles(&self) -> Vec<Value> {
        vec![json!({"type": "http", "port": 8080})]
    }
}

impl ProbeAdapter for HealthProbeAdapter {
    fn validate(&self, configuration: &Value) -> Vec<Diagnostic> {
        self.validate_configuration(configuration)
    }

    fn observe(&self, configuration: &Value) -> Result<ProbeOutcome, Vec<Diagnostic>> {
        let errors = self.validate(configuration);
        if errors.is_empty() {
            Ok(ProbeOutcome {
                healthy: false,
                detail: "observation is delegated to the generated Compose healthcheck".into(),
            })
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests {
    use switchyard_adapter_sdk::{AdapterKind, RegisteredAdapter, conformance};

    use super::*;

    fn assert_conforms(kind: AdapterKind, id: &str) {
        let registry = built_in_registry();
        let adapter = registry
            .lookup(kind, id)
            .expect("built-in adapter is registered");
        conformance::assert_adapter(adapter.adapter());
        for handle in adapter.adapter().example_handles() {
            match adapter {
                RegisteredAdapter::Execution { .. } | RegisteredAdapter::Supervisor { .. } => {
                    conformance::check_runtime_handle(&RuntimeHandle(handle))
                        .expect("runtime handle round-trips");
                }
                RegisteredAdapter::Route { .. } => {
                    conformance::check_route_handle(&RouteHandle(handle))
                        .expect("route handle round-trips");
                }
                RegisteredAdapter::Source { .. } | RegisteredAdapter::Probe { .. } => {}
            }
        }
    }

    macro_rules! conformance_test {
        ($name:ident, $kind:expr, $id:literal) => {
            #[test]
            fn $name() {
                assert_conforms($kind, $id);
            }
        };
    }

    conformance_test!(source_path_conformance, AdapterKind::Source, "source-path");
    conformance_test!(source_git_conformance, AdapterKind::Source, "source-git");
    conformance_test!(
        execution_container_conformance,
        AdapterKind::Execution,
        "execution-container"
    );
    conformance_test!(
        execution_runner_script_conformance,
        AdapterKind::Execution,
        "execution-runner-script"
    );
    conformance_test!(
        supervisor_process_compose_conformance,
        AdapterKind::Supervisor,
        "supervisor-process-compose"
    );
    conformance_test!(
        route_switchyard_conformance,
        AdapterKind::Route,
        "route-switchyard"
    );
    conformance_test!(probe_health_conformance, AdapterKind::Probe, "probe-health");

    #[test]
    fn trusted_host_execution_is_explicitly_deferred() {
        assert!(
            built_in_registry()
                .lookup(AdapterKind::Execution, "execution-host")
                .is_none()
        );
    }
}
