//! The `switchyard.dev/router/v1alpha1` router configuration schema.

use std::{collections::BTreeMap, fmt, net::IpAddr, path::PathBuf};

use serde::{Deserialize, Serialize};

pub const API_VERSION: &str = "switchyard.dev/router/v1alpha1";
pub const KIND: &str = "RouterConfiguration";

macro_rules! identifier {
    ($name:ident) => {
        #[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self::new(value)
            }
        }
    };
}

identifier!(DeploymentId);
identifier!(InstanceId);
identifier!(ComponentId);
identifier!(GroupId);
identifier!(BindingId);
identifier!(RouteSlotId);
identifier!(RouteSnapshotId);

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouterConfig {
    pub api_version: String,
    pub kind: String,
    pub metadata: ConfigMetadata,
    pub spec: RouterSpec,
}

impl RouterConfig {
    /// Validates references and invariants without performing any I/O.
    pub fn validate(&self) -> Result<(), Vec<ValidationError>> {
        let mut errors = Vec::new();

        if self.api_version != API_VERSION || self.kind != KIND {
            errors.push(ValidationError::new(
                ValidationCode::UnsupportedSchema,
                "apiVersion",
                "configuration must use the supported router schema",
            ));
        }

        validate_identifier(
            self.metadata.deployment.as_str(),
            false,
            "metadata.deployment",
            &mut errors,
        );
        validate_identifier(
            self.spec.snapshot.id.as_str(),
            false,
            "spec.snapshot.id",
            &mut errors,
        );

        let mut listener_keys = BTreeMap::new();
        let mut slots = BTreeMap::new();
        for (index, listener) in self.spec.listeners.iter().enumerate() {
            let path = format!("spec.listeners[{index}]");
            if let Some(consumer) = &listener.consumer {
                validate_identifier(
                    consumer.as_str(),
                    false,
                    &format!("{path}.consumer"),
                    &mut errors,
                );
            }
            let key = (listener.consumer.clone(), listener.bind.clone());
            if let Some(first) = listener_keys.insert(key, path.clone()) {
                errors.push(
                    ValidationError::new(
                        ValidationCode::DuplicateListener,
                        &path,
                        "listener address is already in use in this network namespace",
                    )
                    .context("first", first),
                );
            }
            for destination in &listener.destinations {
                let slot = destination.slot();
                validate_identifier(
                    slot.as_str(),
                    false,
                    &format!("{path}.destinations"),
                    &mut errors,
                );
                match slots.insert(slot.clone(), (listener.protocol, path.clone())) {
                    Some(first) if first.0 != listener.protocol => errors.push(
                        ValidationError::new(
                            ValidationCode::DuplicateIdentifier,
                            &path,
                            "route slot is declared with conflicting protocols",
                        )
                        .context("slot", slot.to_string())
                        .context("first", first.1),
                    ),
                    _ => {}
                }
            }
        }

        let mut providers = BTreeMap::new();
        for (index, provider) in self.spec.providers.iter().enumerate() {
            let path = format!("spec.providers[{index}]");
            validate_identifier(
                provider.id.as_str(),
                true,
                &format!("{path}.id"),
                &mut errors,
            );
            if providers.insert(provider.id.clone(), provider).is_some() {
                errors.push(ValidationError::new(
                    ValidationCode::DuplicateIdentifier,
                    &format!("{path}.id"),
                    "provider identifier is declared more than once",
                ));
            }
        }

        let mut groups = BTreeMap::new();
        for (index, group) in self.spec.groups.iter().enumerate() {
            let path = format!("spec.groups[{index}]");
            validate_identifier(group.id.as_str(), false, &format!("{path}.id"), &mut errors);
            if groups.insert(group.id.clone(), group).is_some() {
                errors.push(ValidationError::new(
                    ValidationCode::DuplicateIdentifier,
                    &format!("{path}.id"),
                    "group identifier is declared more than once",
                ));
            }
            for (slot, provider) in &group.providers {
                validate_provider_reference(&path, slot, provider, &slots, &providers, &mut errors);
            }
        }

        let mut bound_consumers = BTreeMap::new();
        for (index, binding) in self.spec.bindings.iter().enumerate() {
            let path = format!("spec.bindings[{index}]");
            validate_identifier(
                binding.id.as_str(),
                false,
                &format!("{path}.id"),
                &mut errors,
            );
            validate_identifier(
                binding.consumer.as_str(),
                false,
                &format!("{path}.consumer"),
                &mut errors,
            );
            if let Some(first) = bound_consumers.insert(binding.consumer.clone(), path.clone()) {
                errors.push(
                    ValidationError::new(
                        ValidationCode::AmbiguousRoute,
                        &path,
                        "consumer has more than one selected service group",
                    )
                    .context("consumer", binding.consumer.to_string())
                    .context("first", first),
                );
            }
            match groups.get(&binding.group) {
                None => errors.push(
                    ValidationError::new(
                        ValidationCode::MissingGroup,
                        &format!("{path}.group"),
                        "binding refers to an unknown group",
                    )
                    .context("group", binding.group.to_string()),
                ),
                Some(group) => {
                    let missing: Vec<_> = binding
                        .required_slots
                        .iter()
                        .filter(|slot| !group.providers.contains_key(*slot))
                        .map(ToString::to_string)
                        .collect();
                    if !missing.is_empty() {
                        errors.push(
                            ValidationError::new(
                                ValidationCode::IncompleteGroup,
                                &format!("{path}.requiredSlots"),
                                "selected group does not provide every required slot",
                            )
                            .context("group", binding.group.to_string())
                            .context("missingSlots", missing.join(",")),
                        );
                    }
                }
            }
        }

        let mut direct_routes = BTreeMap::new();
        for (index, route) in self.spec.routes.iter().enumerate() {
            let path = format!("spec.routes[{index}]");
            if let Some(first) =
                direct_routes.insert((route.consumer.clone(), route.slot.clone()), path.clone())
            {
                errors.push(
                    ValidationError::new(
                        ValidationCode::AmbiguousRoute,
                        &path,
                        "consumer route is declared more than once",
                    )
                    .context("consumer", route.consumer.to_string())
                    .context("slot", route.slot.to_string())
                    .context("first", first),
                );
            }
            validate_provider_reference(
                &path,
                &route.slot,
                &route.provider,
                &slots,
                &providers,
                &mut errors,
            );
        }
        let mut browser_routes = BTreeMap::new();
        for (index, route) in self.spec.browser_routes.iter().enumerate() {
            let path = format!("spec.browserRoutes[{index}]");
            let identity = match &route.identity {
                BrowserIdentity::ExplicitHeader { value } => format!("header:{value}"),
                BrowserIdentity::Origin { origin } => format!("origin:{origin}"),
                BrowserIdentity::ProxyListener { listener } => format!("proxy:{listener}"),
            };
            if let Some(first) =
                browser_routes.insert((route.destination.clone(), identity.clone()), path.clone())
            {
                errors.push(
                    ValidationError::new(
                        ValidationCode::AmbiguousRoute,
                        &path,
                        "browser identity route is declared more than once",
                    )
                    .context("destination", route.destination.to_string())
                    .context("identity", identity)
                    .context("first", first),
                );
            }
            validate_provider_reference(
                &path,
                &route.destination,
                &route.provider,
                &slots,
                &providers,
                &mut errors,
            );
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigMetadata {
    pub deployment: DeploymentId,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouterSpec {
    pub snapshot: RouteSnapshot,
    #[serde(default)]
    pub listeners: Vec<Listener>,
    #[serde(default)]
    pub providers: Vec<Provider>,
    #[serde(default)]
    pub groups: Vec<ServiceGroup>,
    #[serde(default)]
    pub bindings: Vec<Binding>,
    #[serde(default)]
    pub routes: Vec<Route>,
    #[serde(default)]
    pub browser_routes: Vec<BrowserRoute>,
    #[serde(default)]
    pub identity: IdentityPolicy,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteSnapshot {
    pub id: RouteSnapshotId,
    pub version: u64,
    pub transitions: ConnectionTransitionPolicies,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectionTransitionPolicies {
    pub http: ConnectionTransitionPolicy,
    pub https: ConnectionTransitionPolicy,
    pub websocket: ConnectionTransitionPolicy,
    pub grpc: ConnectionTransitionPolicy,
    pub tcp: ConnectionTransitionPolicy,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(
    tag = "strategy",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ConnectionTransitionPolicy {
    Close,
    Drain { timeout_ms: u64 },
    Pin,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    Http,
    Https,
    Websocket,
    Grpc,
    Tcp,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SocketAddress {
    pub host: IpAddr,
    pub port: u16,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Listener {
    /// Isolates identical sidecar addresses belonging to different consumers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consumer: Option<InstanceId>,
    pub bind: SocketAddress,
    pub protocol: Protocol,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls: Option<TlsIdentity>,
    pub destinations: Vec<ListenerDestination>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_identity: Option<BindingId>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TlsIdentity {
    pub certificate: PathBuf,
    pub private_key: PathBuf,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ListenerDestination {
    CustomDomain { slot: RouteSlotId, domain: String },
    LegacyLocalhost { slot: RouteSlotId, host: String },
    Loopback { slot: RouteSlotId },
}

impl ListenerDestination {
    pub fn slot(&self) -> &RouteSlotId {
        match self {
            Self::CustomDomain { slot, .. }
            | Self::LegacyLocalhost { slot, .. }
            | Self::Loopback { slot } => slot,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Provider {
    pub id: ComponentId,
    pub endpoint: UpstreamEndpoint,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_check: Option<HealthCheck>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpstreamEndpoint {
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HealthCheck {
    pub protocol: HealthCheckProtocol,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub interval_ms: u64,
    pub timeout_ms: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthCheckProtocol {
    Http,
    Https,
    Tcp,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceGroup {
    pub id: GroupId,
    pub providers: BTreeMap<RouteSlotId, ComponentId>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Binding {
    pub id: BindingId,
    pub consumer: InstanceId,
    pub group: GroupId,
    pub required_slots: Vec<RouteSlotId>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Route {
    pub consumer: InstanceId,
    pub slot: RouteSlotId,
    pub provider: ComponentId,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserRoute {
    pub identity: BrowserIdentity,
    pub destination: RouteSlotId,
    pub provider: ComponentId,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserIdentity {
    ExplicitHeader { value: BindingId },
    Origin { origin: String },
    ProxyListener { listener: RouteSlotId },
}

/// Browser identities are always evaluated in this order; it is not configurable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrowserIdentitySource {
    ExplicitHeader,
    Origin,
    ProxyListener,
}

impl BrowserIdentitySource {
    pub const PRECEDENCE: [Self; 3] = [Self::ExplicitHeader, Self::Origin, Self::ProxyListener];
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IdentityPolicy {
    pub explicit_header: String,
    pub strip_before_forwarding: bool,
}

impl Default for IdentityPolicy {
    fn default() -> Self {
        Self {
            explicit_header: "X-Switchyard-Route".into(),
            strip_before_forwarding: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationCode {
    UnsupportedSchema,
    InvalidIdentifier,
    DuplicateIdentifier,
    DuplicateListener,
    MissingProvider,
    MissingGroup,
    MissingRouteSlot,
    IncompatibleProtocol,
    IncompleteGroup,
    AmbiguousRoute,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidationError {
    pub code: ValidationCode,
    pub path: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub context: BTreeMap<String, String>,
}

impl ValidationError {
    fn new(code: ValidationCode, path: &str, message: &str) -> Self {
        Self {
            code,
            path: path.into(),
            message: message.into(),
            context: BTreeMap::new(),
        }
    }

    fn context(mut self, key: &str, value: impl Into<String>) -> Self {
        self.context.insert(key.into(), value.into());
        self
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:?} at {}: {}",
            self.code, self.path, self.message
        )
    }
}

fn validate_provider_reference(
    path: &str,
    slot: &RouteSlotId,
    provider_id: &ComponentId,
    slots: &BTreeMap<RouteSlotId, (Protocol, String)>,
    providers: &BTreeMap<ComponentId, &Provider>,
    errors: &mut Vec<ValidationError>,
) {
    let Some((listener_protocol, _)) = slots.get(slot) else {
        errors.push(
            ValidationError::new(
                ValidationCode::MissingRouteSlot,
                path,
                "route refers to an unknown listener destination",
            )
            .context("slot", slot.to_string()),
        );
        return;
    };
    let Some(provider) = providers.get(provider_id) else {
        errors.push(
            ValidationError::new(
                ValidationCode::MissingProvider,
                path,
                "route refers to an unknown provider",
            )
            .context("provider", provider_id.to_string()),
        );
        return;
    };
    if !protocols_compatible(*listener_protocol, provider.endpoint.protocol) {
        errors.push(
            ValidationError::new(
                ValidationCode::IncompatibleProtocol,
                path,
                "listener and provider protocols are incompatible",
            )
            .context("listenerProtocol", format!("{listener_protocol:?}"))
            .context(
                "providerProtocol",
                format!("{:?}", provider.endpoint.protocol),
            ),
        );
    }
}

fn protocols_compatible(listener: Protocol, provider: Protocol) -> bool {
    listener == provider
        || matches!(
            (listener, provider),
            (
                Protocol::Http | Protocol::Https,
                Protocol::Http | Protocol::Https
            )
        )
}

fn validate_identifier(
    value: &str,
    allow_path: bool,
    path: &str,
    errors: &mut Vec<ValidationError>,
) {
    let valid_segment = |segment: &str| {
        !segment.is_empty()
            && segment.len() <= 63
            && segment
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            && segment
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
            && segment
                .as_bytes()
                .last()
                .is_some_and(u8::is_ascii_alphanumeric)
    };
    let valid = if allow_path {
        value.split('/').all(valid_segment) && value.matches('/').count() <= 1
    } else {
        valid_segment(value)
    };
    if !valid {
        errors.push(
            ValidationError::new(
                ValidationCode::InvalidIdentifier,
                path,
                "identifier must contain lowercase letters, digits, or interior hyphens",
            )
            .context("value", value),
        );
    }
}
