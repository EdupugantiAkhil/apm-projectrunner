//! Immutable route compilation and atomic snapshot replacement.

use std::{
    collections::BTreeMap,
    error::Error,
    fmt,
    sync::{Arc, RwLock},
};

use router_config::{
    BindingId, BrowserIdentity, ComponentId, ConnectionTransitionPolicy, GroupId, HealthCheck,
    InstanceId, Protocol, RouteSlotId, RouteSnapshotId, RouterConfig, UpstreamEndpoint,
    ValidationError,
};
use sha2::{Digest, Sha256};

/// A provider selected by a compiled route.
#[derive(Clone, Debug, PartialEq)]
pub struct RouteTarget {
    pub provider: ComponentId,
    pub endpoint: UpstreamEndpoint,
    pub health_check: Option<HealthCheck>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum BrowserKey {
    Header(BindingId),
    Origin(String),
    Proxy(RouteSlotId),
}

/// Immutable routing data used by all data planes.
#[derive(Clone, Debug)]
pub struct CompiledSnapshot {
    config: RouterConfig,
    checksum: String,
    consumer_routes: BTreeMap<(InstanceId, RouteSlotId), RouteTarget>,
    browser_routes: BTreeMap<(RouteSlotId, BrowserKey), RouteTarget>,
}

impl CompiledSnapshot {
    pub fn version(&self) -> u64 {
        self.config.spec.snapshot.version
    }

    pub fn id(&self) -> &RouteSnapshotId {
        &self.config.spec.snapshot.id
    }

    pub fn checksum(&self) -> &str {
        &self.checksum
    }

    pub fn config(&self) -> &RouterConfig {
        &self.config
    }

    pub fn transition(&self, protocol: Protocol) -> &ConnectionTransitionPolicy {
        let policies = &self.config.spec.snapshot.transitions;
        match protocol {
            Protocol::Http => &policies.http,
            Protocol::Https => &policies.https,
            Protocol::Websocket => &policies.websocket,
            Protocol::Grpc => &policies.grpc,
            Protocol::Tcp => &policies.tcp,
        }
    }

    pub fn lookup_consumer(
        &self,
        consumer: &InstanceId,
        slot: &RouteSlotId,
    ) -> Option<&RouteTarget> {
        self.consumer_routes.get(&(consumer.clone(), slot.clone()))
    }

    /// Resolves browser identity in the fixed header, Origin, proxy-listener order.
    /// If a higher-precedence identity is supplied but unknown, lookup fails closed.
    pub fn lookup_browser(&self, lookup: BrowserLookup<'_>) -> Result<&RouteTarget, LookupError> {
        let key = if let Some(header) = lookup.explicit_header {
            BrowserKey::Header(BindingId::from(header))
        } else if let Some(origin) = lookup.origin {
            BrowserKey::Origin(origin.to_owned())
        } else if let Some(proxy) = lookup.proxy_listener {
            BrowserKey::Proxy(proxy.clone())
        } else {
            return Err(LookupError::MissingIdentity);
        };

        self.browser_routes
            .get(&(lookup.destination.clone(), key))
            .ok_or(LookupError::UnknownIdentity)
    }

    pub fn consumer_route_count(&self) -> usize {
        self.consumer_routes.len()
    }

    pub fn browser_route_count(&self) -> usize {
        self.browser_routes.len()
    }
}

pub struct BrowserLookup<'a> {
    pub destination: &'a RouteSlotId,
    pub explicit_header: Option<&'a str>,
    pub origin: Option<&'a str>,
    pub proxy_listener: Option<&'a RouteSlotId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LookupError {
    MissingIdentity,
    UnknownIdentity,
}

impl fmt::Display for LookupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingIdentity => formatter.write_str("request has no usable routing identity"),
            Self::UnknownIdentity => formatter.write_str("request routing identity is unknown"),
        }
    }
}

impl Error for LookupError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivationStatus {
    Activated,
    RejectedStale,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplyAck {
    pub version: u64,
    pub checksum: String,
    pub status: ActivationStatus,
}

#[derive(Debug)]
pub enum ApplyError {
    Decode(serde_json::Error),
    Invalid(Vec<ValidationError>),
    LockPoisoned,
}

impl fmt::Display for ApplyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(error) => write!(formatter, "configuration could not be decoded: {error}"),
            Self::Invalid(errors) => write!(
                formatter,
                "configuration is invalid ({} error(s))",
                errors.len()
            ),
            Self::LockPoisoned => formatter.write_str("route snapshot lock is poisoned"),
        }
    }
}

impl Error for ApplyError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Decode(error) => Some(error),
            Self::Invalid(_) | Self::LockPoisoned => None,
        }
    }
}

struct SnapshotPair {
    current: Arc<CompiledSnapshot>,
    previous: Option<Arc<CompiledSnapshot>>,
}

/// Thread-safe owner of the active and immediately previous snapshots.
pub struct RouteEngine {
    snapshots: RwLock<SnapshotPair>,
}

impl RouteEngine {
    pub fn new(config: RouterConfig) -> Result<Self, ApplyError> {
        let current = Arc::new(compile(config)?);
        Ok(Self {
            snapshots: RwLock::new(SnapshotPair {
                current,
                previous: None,
            }),
        })
    }

    pub fn from_json(input: &[u8]) -> Result<Self, ApplyError> {
        let config = serde_json::from_slice(input).map_err(ApplyError::Decode)?;
        Self::new(config)
    }

    pub fn apply_json(&self, input: &[u8]) -> Result<ApplyAck, ApplyError> {
        let config = serde_json::from_slice(input).map_err(ApplyError::Decode)?;
        self.apply(config)
    }

    pub fn apply(&self, config: RouterConfig) -> Result<ApplyAck, ApplyError> {
        // Compile completely before acquiring the write lock. Invalid input can never
        // expose a partial route table.
        let candidate = Arc::new(compile(config)?);
        let mut pair = self
            .snapshots
            .write()
            .map_err(|_| ApplyError::LockPoisoned)?;
        if candidate.version() <= pair.current.version() {
            return Ok(ApplyAck {
                version: candidate.version(),
                checksum: candidate.checksum().to_owned(),
                status: ActivationStatus::RejectedStale,
            });
        }

        let previous = std::mem::replace(&mut pair.current, candidate);
        pair.previous = Some(previous);
        Ok(ApplyAck {
            version: pair.current.version(),
            checksum: pair.current.checksum().to_owned(),
            status: ActivationStatus::Activated,
        })
    }

    pub fn snapshot(&self) -> Arc<CompiledSnapshot> {
        let pair = self
            .snapshots
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Arc::clone(&pair.current)
    }

    pub fn previous_snapshot(&self) -> Option<Arc<CompiledSnapshot>> {
        let pair = self
            .snapshots
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        pair.previous.as_ref().map(Arc::clone)
    }
}

fn compile(config: RouterConfig) -> Result<CompiledSnapshot, ApplyError> {
    config.validate().map_err(ApplyError::Invalid)?;

    let providers: BTreeMap<_, _> = config
        .spec
        .providers
        .iter()
        .map(|provider| (provider.id.clone(), provider))
        .collect();
    let groups: BTreeMap<GroupId, _> = config
        .spec
        .groups
        .iter()
        .map(|group| (group.id.clone(), group))
        .collect();
    let mut consumer_routes = BTreeMap::new();

    for binding in &config.spec.bindings {
        let group = &groups[&binding.group];
        for (slot, provider_id) in &group.providers {
            consumer_routes.insert(
                (binding.consumer.clone(), slot.clone()),
                target(provider_id, &providers),
            );
        }
    }
    // Explicit direct routes intentionally take precedence over group expansion.
    for route in &config.spec.routes {
        consumer_routes.insert(
            (route.consumer.clone(), route.slot.clone()),
            target(&route.provider, &providers),
        );
    }

    let mut browser_routes = BTreeMap::new();
    for route in &config.spec.browser_routes {
        let identity = match &route.identity {
            BrowserIdentity::ExplicitHeader { value } => BrowserKey::Header(value.clone()),
            BrowserIdentity::Origin { origin } => BrowserKey::Origin(origin.clone()),
            BrowserIdentity::ProxyListener { listener } => BrowserKey::Proxy(listener.clone()),
        };
        browser_routes.insert(
            (route.destination.clone(), identity),
            target(&route.provider, &providers),
        );
    }

    let encoded = serde_json::to_vec(&config).expect("RouterConfig serialization is infallible");
    let checksum = format!("{:x}", Sha256::digest(encoded));
    Ok(CompiledSnapshot {
        config,
        checksum,
        consumer_routes,
        browser_routes,
    })
}

fn target(
    provider_id: &ComponentId,
    providers: &BTreeMap<ComponentId, &router_config::Provider>,
) -> RouteTarget {
    let provider = providers[provider_id];
    RouteTarget {
        provider: provider.id.clone(),
        endpoint: provider.endpoint.clone(),
        health_check: provider.health_check.clone(),
    }
}
