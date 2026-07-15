//! The `switchyard.dev/router/v1alpha1` router configuration schema.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
};

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

        if self.spec.exposure.as_ref().is_some_and(|exposure| {
            exposure.mode == GatewayExposureMode::Lan && !exposure.acknowledge_lan_exposure_risk
        }) {
            errors.push(ValidationError::new(
                ValidationCode::LanExposureRiskNotAcknowledged,
                "spec.exposure.acknowledgeLanExposureRisk",
                "LAN exposure requires acknowledgeLanExposureRisk: true",
            ));
        }

        if self.spec.exposure.as_ref().is_some_and(|exposure| {
            exposure.publish_tailscale
                && (exposure.mode != GatewayExposureMode::Lan
                    || !exposure.acknowledge_lan_exposure_risk)
        }) {
            errors.push(ValidationError::new(
                ValidationCode::TailscalePublicationRequiresLanExposure,
                "spec.exposure.publishTailscale",
                "Tailscale publication requires mode: lan and acknowledgeLanExposureRisk: true",
            ));
        }

        let mut listener_keys = BTreeMap::new();
        let mut slots = BTreeMap::new();
        for (index, listener) in self.spec.listeners.iter().enumerate() {
            let path = format!("spec.listeners[{index}]");
            if !listener.bind.host.is_loopback() && !self.spec.lan_exposure_acknowledged() {
                errors.push(ValidationError::new(
                    ValidationCode::LanExposureNotEnabled,
                    &format!("{path}.bind.host"),
                    "non-loopback listener binds require acknowledged LAN exposure",
                ));
            }
            if listener.bind.port == 0 {
                errors.push(ValidationError::new(
                    ValidationCode::InvalidListener,
                    &format!("{path}.bind.port"),
                    "listener port must be nonzero",
                ));
            }
            if listener.destinations.is_empty() {
                errors.push(ValidationError::new(
                    ValidationCode::InvalidListener,
                    &format!("{path}.destinations"),
                    "listener must declare at least one destination",
                ));
            }
            if matches!(listener.protocol, Protocol::Https) != listener.tls.is_some() {
                errors.push(ValidationError::new(
                    ValidationCode::InvalidListener,
                    &format!("{path}.tls"),
                    "HTTPS listeners require TLS and non-HTTPS listeners must not declare TLS",
                ));
            }
            if let Some(tls) = &listener.tls {
                if tls.certificate.as_os_str().is_empty() || tls.private_key.as_os_str().is_empty()
                {
                    errors.push(ValidationError::new(
                        ValidationCode::InvalidListener,
                        &format!("{path}.tls"),
                        "TLS certificate and private-key paths must be nonempty",
                    ));
                }
            }
            if let Some(authentication) = &listener.proxy_authentication {
                if authentication.credential_file.as_os_str().is_empty() {
                    errors.push(ValidationError::new(
                        ValidationCode::InvalidListener,
                        &format!("{path}.proxyAuthentication.credentialFile"),
                        "proxy credential path must be nonempty",
                    ));
                }
                if listener.protocol != Protocol::Http {
                    errors.push(ValidationError::new(
                        ValidationCode::InvalidListener,
                        &format!("{path}.proxyAuthentication"),
                        "proxy authentication requires a cleartext HTTP listener",
                    ));
                }
            }
            if listener.proxy_identity.is_some() && listener.proxy_authentication.is_none() {
                errors.push(ValidationError::new(
                    ValidationCode::InvalidListener,
                    &format!("{path}.proxyAuthentication"),
                    "managed proxy listeners require proxy authentication",
                ));
            }
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
                match (listener.proxy_identity.is_some(), destination) {
                    (true, ListenerDestination::ProxyTarget { host, port, .. }) => {
                        if *port == 0 || !is_local_host(host) {
                            errors.push(ValidationError::new(
                                ValidationCode::InvalidListener,
                                &format!("{path}.destinations"),
                                "managed proxy targets require a nonzero port and local host",
                            ));
                        }
                    }
                    (true, _) => errors.push(ValidationError::new(
                        ValidationCode::InvalidListener,
                        &format!("{path}.destinations"),
                        "managed proxy listeners require exact host-and-port proxy targets",
                    )),
                    (false, ListenerDestination::ProxyTarget { .. }) => {
                        errors.push(ValidationError::new(
                            ValidationCode::InvalidListener,
                            &format!("{path}.destinations"),
                            "proxy targets are valid only on managed proxy listeners",
                        ));
                    }
                    (false, _) => {}
                }
            }
        }

        let mut providers = BTreeMap::new();
        for (index, provider) in self.spec.providers.iter().enumerate() {
            let path = format!("spec.providers[{index}]");
            if self.spec.exposure_mode() == GatewayExposureMode::Lan
                && !is_loopback_host(&provider.endpoint.host)
            {
                errors.push(ValidationError::new(
                    ValidationCode::UnsafeLanProvider,
                    &format!("{path}.endpoint.host"),
                    "LAN exposure does not permit non-loopback provider upstreams",
                ));
            }
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
                BrowserIdentity::ExplicitHeader { value } => {
                    validate_route_header_value(
                        value.as_str(),
                        &format!("{path}.identity.value"),
                        &mut errors,
                    );
                    format!("header:{value}")
                }
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
    /// Host-gateway listener exposure. Omission preserves the secure loopback-only default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exposure: Option<GatewayExposure>,
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayExposure {
    pub mode: GatewayExposureMode,
    /// LAN exposure must remain a deliberate, reviewable acknowledgement in desired state.
    #[serde(default, skip_serializing_if = "is_false")]
    pub acknowledge_lan_exposure_risk: bool,
    /// Opts into advisory verification of private tailnet reachability.
    #[serde(default, skip_serializing_if = "is_false")]
    pub publish_tailscale: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayExposureMode {
    Loopback,
    Lan,
}

/// Effective listener exposure after wildcard binds are expanded to local interfaces.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayExposureSummary {
    pub mode: GatewayExposureMode,
    pub exposed_addresses: Vec<SocketAddr>,
}

impl fmt::Display for GatewayExposureSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mode = match self.mode {
            GatewayExposureMode::Loopback => "loopback",
            GatewayExposureMode::Lan => "LAN",
        };
        write!(formatter, "{mode}")?;
        if !self.exposed_addresses.is_empty() {
            write!(
                formatter,
                " ({})",
                self.exposed_addresses
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )?;
        }
        Ok(())
    }
}

impl RouterSpec {
    pub fn exposure_mode(&self) -> GatewayExposureMode {
        self.exposure
            .as_ref()
            .map_or(GatewayExposureMode::Loopback, |exposure| exposure.mode)
    }

    pub fn lan_exposure_acknowledged(&self) -> bool {
        self.exposure.as_ref().is_some_and(|exposure| {
            exposure.mode == GatewayExposureMode::Lan && exposure.acknowledge_lan_exposure_risk
        })
    }

    pub fn needs_interface_enumeration(&self) -> bool {
        self.exposure_mode() == GatewayExposureMode::Lan
            && self
                .listeners
                .iter()
                .any(|listener| listener.bind.host.is_unspecified())
    }

    pub fn exposure_summary(&self, interfaces: &[IpAddr]) -> GatewayExposureSummary {
        let mode = self.exposure_mode();
        let mut addresses = BTreeSet::new();
        for listener in &self.listeners {
            if mode == GatewayExposureMode::Lan && listener.bind.host.is_unspecified() {
                for address in interfaces {
                    if address.is_ipv4() == listener.bind.host.is_ipv4() {
                        addresses.insert(SocketAddr::new(*address, listener.bind.port));
                    }
                }
            } else {
                addresses.insert(SocketAddr::new(listener.bind.host, listener.bind.port));
            }
        }
        GatewayExposureSummary {
            mode,
            exposed_addresses: addresses.into_iter().collect(),
        }
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_authentication: Option<ProxyAuthentication>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProxyAuthentication {
    pub scheme: ProxyAuthenticationScheme,
    pub credential_file: PathBuf,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyAuthenticationScheme {
    Basic,
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
    CustomDomain {
        slot: RouteSlotId,
        domain: String,
    },
    LegacyLocalhost {
        slot: RouteSlotId,
        host: String,
    },
    Loopback {
        slot: RouteSlotId,
    },
    ProxyTarget {
        slot: RouteSlotId,
        host: String,
        port: u16,
    },
}

impl ListenerDestination {
    pub fn slot(&self) -> &RouteSlotId {
        match self {
            Self::CustomDomain { slot, .. }
            | Self::LegacyLocalhost { slot, .. }
            | Self::Loopback { slot }
            | Self::ProxyTarget { slot, .. } => slot,
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
    /// Allows this provider to receive the internal browser route header when the
    /// router-wide stripping policy is also disabled.
    #[serde(default, skip_serializing_if = "is_false")]
    pub receive_identity_header: bool,
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
    InvalidListener,
    DuplicateIdentifier,
    DuplicateListener,
    MissingProvider,
    MissingGroup,
    MissingRouteSlot,
    IncompatibleProtocol,
    IncompleteGroup,
    AmbiguousRoute,
    LanExposureNotEnabled,
    LanExposureRiskNotAcknowledged,
    TailscalePublicationRequiresLanExposure,
    UnsafeLanProvider,
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

fn is_local_host(host: &str) -> bool {
    let normalized = host.trim_end_matches('.').to_ascii_lowercase();
    normalized == "localhost"
        || normalized.ends_with(".localhost")
        || normalized
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
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

fn validate_route_header_value(value: &str, path: &str, errors: &mut Vec<ValidationError>) {
    let bytes = value.as_bytes();
    let valid = (1..=128).contains(&bytes.len())
        && bytes
            .first()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes[1..].iter().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'.' | b'_' | b':' | b'-')
        });
    if !valid {
        errors.push(ValidationError::new(
            ValidationCode::InvalidIdentifier,
            path,
            "route header value must be 1-128 lowercase ASCII letters, digits, `.`, `_`, `:`, or `-`, starting with a letter or digit",
        ));
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}
