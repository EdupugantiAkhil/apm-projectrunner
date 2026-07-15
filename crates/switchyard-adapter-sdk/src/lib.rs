//! Framework-neutral contracts, registry, and conformance helpers for Switchyard adapters.
//!
//! Adapter configuration and recovery handles cross the SDK boundary as JSON values. This keeps
//! the core independent of adapter implementation types while schemas and adapter-side
//! deserialization retain strong validation.

use std::{collections::BTreeMap, fmt, sync::Arc};

use schemars::{JsonSchema, Schema, SchemaGenerator, generate::SchemaSettings};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The current adapter SDK contract version.
pub const SDK_CONTRACT_VERSION: &str = "switchyard.dev/adapter-sdk/v1alpha1";

/// A stable machine-readable diagnostic emitted across an adapter boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    /// Stable code suitable for programmatic handling.
    pub code: String,
    /// JSON-style path to the invalid input.
    pub path: String,
    /// Human-readable explanation.
    pub message: String,
}

impl Diagnostic {
    /// Constructs one adapter diagnostic.
    #[must_use]
    pub fn new(
        code: impl Into<String>,
        path: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            path: path.into(),
            message: message.into(),
        }
    }
}

/// The independently registered kind of an adapter.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AdapterKind {
    /// Supplies and inspects source files.
    Source,
    /// Plans and controls one executable component.
    Execution,
    /// Coordinates a suite of child processes.
    Supervisor,
    /// Connects a consumer slot to a provider.
    Route,
    /// Observes readiness or health.
    Probe,
}

/// Protocol metadata understood by the built-in routing model.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    /// Clear-text HTTP.
    Http,
    /// HTTP over TLS.
    Https,
    /// WebSocket, including its HTTP upgrade handshake.
    Websocket,
    /// gRPC over HTTP/2.
    Grpc,
    /// Raw TCP.
    Tcp,
    /// An adapter-defined protocol not interpreted by the SDK.
    Custom(String),
}

/// Capabilities published for discovery and schema-driven user interfaces.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityMetadata {
    /// Protocols accepted or provided by this adapter.
    #[serde(default)]
    pub protocols: Vec<Protocol>,
    /// Whether an active connection can be changed without restarting its consumer.
    #[serde(default)]
    pub supports_live_update: bool,
    /// Whether handles can be reconstructed from durable ownership labels.
    #[serde(default)]
    pub supports_recovery: bool,
    /// Additional stable feature names understood by clients.
    #[serde(default)]
    pub features: Vec<String>,
}

/// Compatibility and discovery declaration supplied by every adapter.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdapterDeclaration {
    /// Stable adapter identifier.
    pub id: String,
    /// Adapter implementation version in semantic-version form.
    pub version: String,
    /// SDK contract versions this adapter implements.
    pub sdk_contracts: Vec<String>,
    /// Capabilities used for discovery and form presentation.
    pub capabilities: CapabilityMetadata,
}

impl AdapterDeclaration {
    /// Returns whether this declaration supports the SDK used by this process.
    #[must_use]
    pub fn supports_current_sdk(&self) -> bool {
        self.sdk_contracts
            .iter()
            .any(|version| version == SDK_CONTRACT_VERSION)
    }
}

/// One JSON example shipped by an adapter.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigurationExample {
    /// Short stable example name.
    pub name: String,
    /// Example configuration document.
    pub configuration: Value,
    /// Whether the example is expected to pass schema and semantic validation.
    pub valid: bool,
}

/// An opaque, serializable execution recovery handle.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(transparent)]
pub struct RuntimeHandle(pub Value);

/// An opaque, serializable route recovery handle.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(transparent)]
pub struct RouteHandle(pub Value);

/// State normalized across all execution mechanisms.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservedState {
    /// No owned resources are present.
    Absent,
    /// Resources are being prepared.
    Preparing,
    /// The component is starting but not ready.
    Starting,
    /// The component is ready.
    Ready,
    /// The component is stopping.
    Stopping,
    /// The component exited successfully.
    Completed,
    /// The component failed or drifted.
    Failed,
    /// The adapter cannot determine the state.
    Unknown,
}

/// A normalized lifecycle event.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationEvent {
    /// Stable event type.
    pub code: String,
    /// Human-readable event summary.
    pub message: String,
    /// State after the event, when known.
    pub state: Option<ObservedState>,
}

/// A normalized log record returned by an execution adapter.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LogRecord {
    /// Adapter-defined source, such as a child service name.
    pub source: String,
    /// Log payload without presentation formatting.
    pub message: String,
}

/// A claimed resource which the runtime must own and recover safely.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceClaim {
    /// Claim class, such as `compose-project`, `port`, or `volume`.
    pub kind: String,
    /// Stable value unique within its claim class.
    pub value: String,
    /// Whether two active instances may share the resource.
    pub exclusive: bool,
}

/// Planning output consumed by an existing runtime implementation.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionPlan {
    /// Declarative resources for the runtime generator.
    #[serde(default)]
    pub resources: Vec<Value>,
    /// Commands represented as argument arrays, never shell strings.
    #[serde(default)]
    pub commands: Vec<Vec<String>>,
    /// Ownership and collision claims.
    #[serde(default)]
    pub claims: Vec<ResourceClaim>,
}

/// Context supplied while validating or planning execution.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionContext {
    /// Adapter-specific configuration.
    pub configuration: Value,
    /// Stable deployment name.
    pub deployment: String,
    /// Namespaced component name.
    pub component: String,
    /// Selected source path, if the execution mechanism uses files.
    pub source_path: Option<String>,
}

/// Labels available to recovery without relying on in-memory state.
pub type RecoveryLabels = BTreeMap<String, String>;

/// Identity returned by a non-mutating source inspection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceIdentity {
    /// Resolved source path.
    pub path: String,
    /// Repository root when available.
    pub repository: Option<String>,
    /// Requested branch, tag, or revision when available.
    pub r#ref: Option<String>,
    /// Resolved commit identifier when available.
    pub commit: Option<String>,
    /// Dirty state when it can be determined without mutation.
    pub dirty: Option<bool>,
}

/// A provider capability used for route compatibility checks.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderCapability {
    /// User-defined capability name.
    pub name: String,
    /// Transport protocol.
    pub protocol: Protocol,
    /// Provider endpoint port.
    pub port: u16,
}

/// A consumer route slot used for route compatibility checks.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConsumerSlot {
    /// User-defined slot name.
    pub name: String,
    /// Required transport protocol.
    pub protocol: Protocol,
    /// Loopback address used by the unchanged consumer.
    pub host: String,
    /// Loopback port used by the unchanged consumer.
    pub port: u16,
}

/// Complete route validation input.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteValidationContext {
    /// Consumer instance name.
    pub consumer: String,
    /// Consumer slot.
    pub slot: ConsumerSlot,
    /// Selected provider capability.
    pub provider: ProviderCapability,
}

/// Planned disruption required to change one connection.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteChange {
    /// Apply atomically without restarting the consumer.
    Live,
    /// Restart the consumer or route process.
    Restart,
    /// Rebuild runtime resources before restart.
    Rebuild,
}

/// A connection passed to route planning and application.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteConnection {
    /// Adapter-specific route configuration.
    pub configuration: Value,
    /// Compatibility input for this connection.
    pub route: RouteValidationContext,
}

/// Observed target for an applied route.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservedRoute {
    /// Current provider identity.
    pub provider: String,
    /// Current normalized state.
    pub state: ObservedState,
}

/// Probe outcome normalized across probe mechanisms.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProbeOutcome {
    /// Whether the target satisfies the probe.
    pub healthy: bool,
    /// Human-readable detail suitable for an operation view.
    pub detail: String,
}

/// Common discovery, schema, and validation behavior shared by every adapter kind.
pub trait Adapter: Send + Sync {
    /// Returns the compatibility declaration.
    fn declaration(&self) -> AdapterDeclaration;
    /// Returns a draft 2020-12 JSON Schema for user configuration.
    fn configuration_schema(&self) -> Schema;
    /// Returns at least one valid and one invalid configuration example.
    fn configuration_examples(&self) -> Vec<ConfigurationExample>;
    /// Deserializes and semantically validates adapter configuration.
    fn validate_configuration(&self, configuration: &Value) -> Vec<Diagnostic>;
    /// Returns representative opaque handles used by the conformance suite.
    fn example_handles(&self) -> Vec<Value>;
}

/// Non-mutating source inspection contract.
pub trait SourceAdapter: Adapter {
    /// Resolves available path, repository, ref, commit, and dirty identity fields.
    fn inspect(&self, configuration: &Value) -> Result<SourceIdentity, Vec<Diagnostic>>;
}

/// Complete execution control contract.
pub trait ExecutionAdapter: Adapter {
    /// Validates execution in deployment context.
    fn validate(&self, context: &ExecutionContext) -> Vec<Diagnostic>;
    /// Emits resources, commands, and ownership claims for the selected runtime.
    fn plan(&self, context: &ExecutionContext) -> Result<ExecutionPlan, Vec<Diagnostic>>;
    /// Performs or describes preparation operations.
    fn prepare(&self, context: &ExecutionContext) -> Result<Vec<OperationEvent>, Vec<Diagnostic>>;
    /// Starts the component and returns an opaque recovery handle.
    fn start(&self, context: &ExecutionContext) -> Result<RuntimeHandle, Vec<Diagnostic>>;
    /// Inspects a previously returned handle.
    fn inspect(&self, handle: &RuntimeHandle) -> Result<ObservedState, Vec<Diagnostic>>;
    /// Reads normalized log records from a handle.
    fn logs(&self, handle: &RuntimeHandle) -> Result<Vec<LogRecord>, Vec<Diagnostic>>;
    /// Stops owned runtime resources.
    fn stop(&self, handle: &RuntimeHandle) -> Result<Vec<OperationEvent>, Vec<Diagnostic>>;
    /// Removes disposable resources owned by the handle.
    fn cleanup(&self, handle: &RuntimeHandle) -> Result<Vec<OperationEvent>, Vec<Diagnostic>>;
    /// Reconstructs a handle from durable ownership labels.
    fn recover(&self, labels: &RecoveryLabels) -> Result<RuntimeHandle, Vec<Diagnostic>>;
}

/// Process-suite coordination layered on the execution contract.
pub trait SupervisorAdapter: ExecutionAdapter {
    /// Returns dependency and readiness relationships between imported children.
    fn child_relationships(
        &self,
        configuration: &Value,
    ) -> Result<Vec<ChildRelationship>, Vec<Diagnostic>>;
}

/// Dependency and readiness metadata for one supervised child.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChildRelationship {
    /// Child process name.
    pub child: String,
    /// Children which must become ready first.
    pub depends_on: Vec<String>,
    /// Whether this child has an explicit readiness condition.
    pub readiness_required: bool,
}

/// Live route control contract.
pub trait RouteAdapter: Adapter {
    /// Checks consumer, slot, and provider compatibility.
    fn validate(&self, context: &RouteValidationContext) -> Vec<Diagnostic>;
    /// Predicts whether applying a connection is live, restart, or rebuild.
    fn plan(&self, connection: &RouteConnection) -> Result<RouteChange, Vec<Diagnostic>>;
    /// Applies a connection and returns an opaque route handle.
    fn apply(&self, connection: &RouteConnection) -> Result<RouteHandle, Vec<Diagnostic>>;
    /// Removes an applied connection.
    fn remove(&self, handle: &RouteHandle) -> Result<Vec<OperationEvent>, Vec<Diagnostic>>;
    /// Inspects the current provider target.
    fn inspect(&self, handle: &RouteHandle) -> Result<ObservedRoute, Vec<Diagnostic>>;
}

/// Readiness and health observation contract.
pub trait ProbeAdapter: Adapter {
    /// Validates a probe document in context.
    fn validate(&self, configuration: &Value) -> Vec<Diagnostic>;
    /// Executes or plans one normalized observation.
    fn observe(&self, configuration: &Value) -> Result<ProbeOutcome, Vec<Diagnostic>>;
}

/// Generates a draft 2020-12 schema for an adapter configuration type.
#[must_use]
pub fn schema_for<T: JsonSchema>() -> Schema {
    SchemaGenerator::new(SchemaSettings::draft2020_12()).into_root_schema_for::<T>()
}

/// Validates one JSON value using a draft 2020-12 schema and stable diagnostics.
#[must_use]
pub fn validate_schema(schema: &Schema, instance: &Value) -> Vec<Diagnostic> {
    let validator = match jsonschema::draft202012::options().build(schema.as_value()) {
        Ok(validator) => validator,
        Err(error) => {
            return vec![Diagnostic::new(
                "adapter_schema_invalid",
                "$",
                error.to_string(),
            )];
        }
    };
    validator
        .iter_errors(instance)
        .map(|error| {
            let path = error.instance_path().to_string();
            Diagnostic::new(
                "adapter_config_schema",
                if path.is_empty() { "$" } else { &path },
                error.to_string(),
            )
        })
        .collect()
}

/// A registered adapter preserving its kind-specific interface.
#[derive(Clone)]
pub enum RegisteredAdapter {
    /// Source adapter entry.
    Source {
        /// Common discovery interface.
        common: Arc<dyn Adapter>,
        /// Kind-specific source interface.
        adapter: Arc<dyn SourceAdapter>,
    },
    /// Execution adapter entry.
    Execution {
        /// Common discovery interface.
        common: Arc<dyn Adapter>,
        /// Kind-specific execution interface.
        adapter: Arc<dyn ExecutionAdapter>,
    },
    /// Supervisor adapter entry.
    Supervisor {
        /// Common discovery interface.
        common: Arc<dyn Adapter>,
        /// Kind-specific supervisor interface.
        adapter: Arc<dyn SupervisorAdapter>,
    },
    /// Route adapter entry.
    Route {
        /// Common discovery interface.
        common: Arc<dyn Adapter>,
        /// Kind-specific route interface.
        adapter: Arc<dyn RouteAdapter>,
    },
    /// Probe adapter entry.
    Probe {
        /// Common discovery interface.
        common: Arc<dyn Adapter>,
        /// Kind-specific probe interface.
        adapter: Arc<dyn ProbeAdapter>,
    },
}

impl RegisteredAdapter {
    /// Returns this entry's kind.
    #[must_use]
    pub fn kind(&self) -> AdapterKind {
        match self {
            Self::Source { .. } => AdapterKind::Source,
            Self::Execution { .. } => AdapterKind::Execution,
            Self::Supervisor { .. } => AdapterKind::Supervisor,
            Self::Route { .. } => AdapterKind::Route,
            Self::Probe { .. } => AdapterKind::Probe,
        }
    }

    /// Returns the common adapter interface.
    #[must_use]
    pub fn adapter(&self) -> &dyn Adapter {
        match self {
            Self::Source { common, .. }
            | Self::Execution { common, .. }
            | Self::Supervisor { common, .. }
            | Self::Route { common, .. }
            | Self::Probe { common, .. } => common.as_ref(),
        }
    }
}

/// Stable registry failure code.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistryErrorCode {
    /// Adapter identifier is empty or malformed.
    InvalidAdapterId,
    /// Adapter version is not semantic-version shaped.
    InvalidAdapterVersion,
    /// The current SDK contract is not declared.
    IncompatibleSdkContract,
    /// An identical kind, id, and version is already present.
    DuplicateAdapter,
}

/// Registration error with a stable machine-readable code.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryError {
    /// Stable error code.
    pub code: RegistryErrorCode,
    /// Human-readable detail.
    pub message: String,
}

impl fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for RegistryError {}

/// Discovery metadata returned by registry listing.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterMetadata {
    /// Registered adapter kind.
    pub kind: AdapterKind,
    /// Compatibility and capability declaration.
    pub declaration: AdapterDeclaration,
    /// Draft 2020-12 user-configuration schema.
    pub configuration_schema: Value,
}

/// In-process registry keyed by adapter kind, identifier, and semantic version.
#[derive(Default)]
pub struct AdapterRegistry {
    entries: BTreeMap<(AdapterKind, String, String), RegisteredAdapter>,
}

impl AdapterRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers one source adapter.
    pub fn register_source<A: SourceAdapter + 'static>(
        &mut self,
        adapter: A,
    ) -> Result<(), RegistryError> {
        let adapter = Arc::new(adapter);
        self.register(RegisteredAdapter::Source {
            common: adapter.clone(),
            adapter,
        })
    }

    /// Registers one execution adapter.
    pub fn register_execution<A: ExecutionAdapter + 'static>(
        &mut self,
        adapter: A,
    ) -> Result<(), RegistryError> {
        let adapter = Arc::new(adapter);
        self.register(RegisteredAdapter::Execution {
            common: adapter.clone(),
            adapter,
        })
    }

    /// Registers one supervisor adapter.
    pub fn register_supervisor<A: SupervisorAdapter + 'static>(
        &mut self,
        adapter: A,
    ) -> Result<(), RegistryError> {
        let adapter = Arc::new(adapter);
        self.register(RegisteredAdapter::Supervisor {
            common: adapter.clone(),
            adapter,
        })
    }

    /// Registers one route adapter.
    pub fn register_route<A: RouteAdapter + 'static>(
        &mut self,
        adapter: A,
    ) -> Result<(), RegistryError> {
        let adapter = Arc::new(adapter);
        self.register(RegisteredAdapter::Route {
            common: adapter.clone(),
            adapter,
        })
    }

    /// Registers one probe adapter.
    pub fn register_probe<A: ProbeAdapter + 'static>(
        &mut self,
        adapter: A,
    ) -> Result<(), RegistryError> {
        let adapter = Arc::new(adapter);
        self.register(RegisteredAdapter::Probe {
            common: adapter.clone(),
            adapter,
        })
    }

    fn register(&mut self, adapter: RegisteredAdapter) -> Result<(), RegistryError> {
        let declaration = adapter.adapter().declaration();
        validate_declaration(&declaration)?;
        let key = (
            adapter.kind(),
            declaration.id.clone(),
            declaration.version.clone(),
        );
        if self.entries.contains_key(&key) {
            return Err(RegistryError {
                code: RegistryErrorCode::DuplicateAdapter,
                message: format!(
                    "{} {} is already registered as {:?}",
                    declaration.id,
                    declaration.version,
                    adapter.kind()
                ),
            });
        }
        self.entries.insert(key, adapter);
        Ok(())
    }

    /// Looks up the highest registered semantic version for a kind and identifier.
    #[must_use]
    pub fn lookup(&self, kind: AdapterKind, id: &str) -> Option<&RegisteredAdapter> {
        self.entries
            .iter()
            .filter(|((entry_kind, entry_id, _), _)| *entry_kind == kind && entry_id == id)
            .max_by(|((_, _, left), _), ((_, _, right), _)| compare_semantic_versions(left, right))
            .map(|(_, adapter)| adapter)
    }

    /// Lists capability and schema metadata in deterministic key order.
    #[must_use]
    pub fn list(&self) -> Vec<AdapterMetadata> {
        self.entries
            .values()
            .map(|entry| AdapterMetadata {
                kind: entry.kind(),
                declaration: entry.adapter().declaration(),
                configuration_schema: entry.adapter().configuration_schema().to_value(),
            })
            .collect()
    }
}

fn validate_declaration(declaration: &AdapterDeclaration) -> Result<(), RegistryError> {
    if declaration.id.is_empty()
        || !declaration
            .id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(RegistryError {
            code: RegistryErrorCode::InvalidAdapterId,
            message: format!("invalid adapter id `{}`", declaration.id),
        });
    }
    if !is_semantic_version(&declaration.version) {
        return Err(RegistryError {
            code: RegistryErrorCode::InvalidAdapterVersion,
            message: format!("invalid semantic version `{}`", declaration.version),
        });
    }
    if !declaration.supports_current_sdk() {
        return Err(RegistryError {
            code: RegistryErrorCode::IncompatibleSdkContract,
            message: format!("adapter does not support {SDK_CONTRACT_VERSION}"),
        });
    }
    Ok(())
}

fn is_semantic_version(value: &str) -> bool {
    let (without_build, build) = value
        .split_once('+')
        .map_or((value, None), |(core, suffix)| (core, Some(suffix)));
    let (core, prerelease) = without_build
        .split_once('-')
        .map_or((without_build, None), |(core, suffix)| (core, Some(suffix)));
    let mut parts = core.split('.');
    let valid_number = |part: Option<&str>| {
        part.is_some_and(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && (part == "0" || !part.starts_with('0'))
        })
    };
    let core_valid = valid_number(parts.next())
        && valid_number(parts.next())
        && valid_number(parts.next())
        && parts.next().is_none();
    let identifiers_valid = |suffix: Option<&str>| {
        suffix.is_none_or(|suffix| {
            !suffix.is_empty()
                && suffix.split('.').all(|identifier| {
                    !identifier.is_empty()
                        && identifier
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                })
        })
    };
    core_valid && identifiers_valid(prerelease) && identifiers_valid(build)
}

fn compare_semantic_versions(left: &str, right: &str) -> std::cmp::Ordering {
    fn core(version: &str) -> [u64; 3] {
        let version = version.split(['-', '+']).next().unwrap_or(version);
        let mut parts = version.split('.').map(|part| {
            part.parse::<u64>()
                .expect("registered version was validated")
        });
        [
            parts.next().expect("validated major version"),
            parts.next().expect("validated minor version"),
            parts.next().expect("validated patch version"),
        ]
    }
    fn prerelease(version: &str) -> Option<&str> {
        version
            .split_once('-')
            .map(|(_, suffix)| suffix.split('+').next().unwrap_or(suffix))
    }
    fn compare_prerelease(left: &str, right: &str) -> std::cmp::Ordering {
        use std::cmp::Ordering;

        let mut left = left.split('.');
        let mut right = right.split('.');
        loop {
            match (left.next(), right.next()) {
                (None, None) => return Ordering::Equal,
                (None, Some(_)) => return Ordering::Less,
                (Some(_), None) => return Ordering::Greater,
                (Some(left), Some(right)) => {
                    let ordering = match (left.parse::<u64>(), right.parse::<u64>()) {
                        (Ok(left), Ok(right)) => left.cmp(&right),
                        (Ok(_), Err(_)) => Ordering::Less,
                        (Err(_), Ok(_)) => Ordering::Greater,
                        (Err(_), Err(_)) => left.cmp(right),
                    };
                    if ordering != Ordering::Equal {
                        return ordering;
                    }
                }
            }
        }
    }
    core(left)
        .cmp(&core(right))
        .then_with(|| match (prerelease(left), prerelease(right)) {
            (None, None) => std::cmp::Ordering::Equal,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (Some(_), None) => std::cmp::Ordering::Less,
            (Some(left), Some(right)) => compare_prerelease(left, right),
        })
}

/// Public adapter conformance helpers.
pub mod conformance {
    use serde::de::DeserializeOwned;

    use super::*;

    /// Stable conformance failure returned to adapter-owned test suites.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct ConformanceFailure {
        /// Name of the failed conformance rule.
        pub check: &'static str,
        /// Human-readable evidence.
        pub detail: String,
    }

    /// Runs common schema, examples, declaration, determinism, and handle checks.
    #[must_use]
    pub fn check_adapter(adapter: &dyn Adapter) -> Vec<ConformanceFailure> {
        let mut failures = Vec::new();
        let schema = adapter.configuration_schema();
        if let Err(error) = jsonschema::draft202012::options().build(schema.as_value()) {
            failures.push(failure("schema_compiles", error));
            return failures;
        }
        if schema
            .as_object()
            .and_then(|object| object.get("$schema"))
            .and_then(Value::as_str)
            != Some("https://json-schema.org/draft/2020-12/schema")
        {
            failures.push(ConformanceFailure {
                check: "schema_draft",
                detail: "schema must declare draft 2020-12".into(),
            });
        }
        let examples = adapter.configuration_examples();
        if !examples.iter().any(|example| example.valid)
            || !examples.iter().any(|example| !example.valid)
        {
            failures.push(ConformanceFailure {
                check: "examples_present",
                detail: "at least one valid and one invalid example are required".into(),
            });
        }
        for example in examples {
            let first = adapter.validate_configuration(&example.configuration);
            let second = adapter.validate_configuration(&example.configuration);
            if first != second {
                failures.push(ConformanceFailure {
                    check: "validation_deterministic",
                    detail: example.name.clone(),
                });
            }
            if example.valid && !first.is_empty() {
                failures.push(ConformanceFailure {
                    check: "valid_example",
                    detail: format!("{}: {first:?}", example.name),
                });
            }
            if !example.valid && first.is_empty() {
                failures.push(ConformanceFailure {
                    check: "invalid_example",
                    detail: example.name,
                });
            }
        }
        if let Err(error) = validate_declaration(&adapter.declaration()) {
            failures.push(failure("compatibility", error));
        }
        let capabilities = adapter.declaration().capabilities;
        if has_duplicates(&capabilities.protocols)
            || has_duplicates(&capabilities.features)
            || capabilities.features.iter().any(String::is_empty)
            || capabilities
                .protocols
                .iter()
                .any(|protocol| matches!(protocol, Protocol::Custom(name) if name.is_empty()))
            || (capabilities.supports_live_update && capabilities.protocols.is_empty())
        {
            failures.push(ConformanceFailure {
                check: "capabilities_consistent",
                detail: "protocol and feature declarations are internally inconsistent".into(),
            });
        }
        for handle in adapter.example_handles() {
            match serde_json::to_string(&handle)
                .and_then(|encoded| serde_json::from_str::<Value>(&encoded))
            {
                Ok(decoded) if decoded == handle => {}
                Ok(_) => failures.push(ConformanceFailure {
                    check: "handle_round_trip",
                    detail: "decoded handle differs".into(),
                }),
                Err(error) => failures.push(failure("handle_round_trip", error)),
            }
        }
        failures
    }

    /// Asserts all common conformance checks, producing readable test output.
    pub fn assert_adapter(adapter: &dyn Adapter) {
        let failures = check_adapter(adapter);
        assert!(
            failures.is_empty(),
            "adapter conformance failures: {failures:#?}"
        );
    }

    /// Checks that an opaque execution handle survives serialization unchanged.
    pub fn check_runtime_handle(handle: &RuntimeHandle) -> Result<(), ConformanceFailure> {
        round_trip(handle, "runtime_handle_round_trip")
    }

    /// Checks that an opaque route handle survives serialization unchanged.
    pub fn check_route_handle(handle: &RouteHandle) -> Result<(), ConformanceFailure> {
        round_trip(handle, "route_handle_round_trip")
    }

    fn round_trip<T>(value: &T, check: &'static str) -> Result<(), ConformanceFailure>
    where
        T: Serialize + DeserializeOwned + PartialEq,
    {
        let encoded = serde_json::to_string(value).map_err(|error| failure(check, error))?;
        let decoded = serde_json::from_str::<T>(&encoded).map_err(|error| failure(check, error))?;
        if &decoded == value {
            Ok(())
        } else {
            Err(ConformanceFailure {
                check,
                detail: "decoded handle differs".into(),
            })
        }
    }

    fn failure(check: &'static str, error: impl fmt::Display) -> ConformanceFailure {
        ConformanceFailure {
            check,
            detail: error.to_string(),
        }
    }

    fn has_duplicates<T: Ord + Clone>(values: &[T]) -> bool {
        let mut values = values.to_vec();
        values.sort();
        values.windows(2).any(|pair| pair[0] == pair[1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_versions_are_checked_without_accepting_partial_versions() {
        assert!(is_semantic_version("1.2.3"));
        assert!(is_semantic_version("1.2.3-alpha.1+build"));
        assert!(!is_semantic_version("1.2"));
        assert!(!is_semantic_version("01.2.3"));
        assert!(!is_semantic_version("1.2.3-"));
        assert!(!is_semantic_version("1.2.3+"));
        assert_eq!(
            compare_semantic_versions("1.10.0", "1.9.0"),
            std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn compatibility_failures_have_stable_registry_codes() {
        let declaration = AdapterDeclaration {
            id: "example".into(),
            version: "1.0.0".into(),
            sdk_contracts: vec!["switchyard.dev/adapter-sdk/v2".into()],
            capabilities: CapabilityMetadata::default(),
        };
        assert_eq!(
            validate_declaration(&declaration).unwrap_err().code,
            RegistryErrorCode::IncompatibleSdkContract
        );

        let mut invalid_version = declaration;
        invalid_version.sdk_contracts = vec![SDK_CONTRACT_VERSION.into()];
        invalid_version.version = "1.0".into();
        assert_eq!(
            validate_declaration(&invalid_version).unwrap_err().code,
            RegistryErrorCode::InvalidAdapterVersion
        );
    }
}
