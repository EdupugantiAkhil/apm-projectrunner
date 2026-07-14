//! Process assembly and a local, authenticated administration channel.

#![cfg(unix)]

mod forward_proxy;
pub mod host_gateway;

use std::{
    collections::{BTreeMap, VecDeque},
    fmt, io,
    net::SocketAddr,
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use router_config::{ConnectionTransitionPolicy, Listener, Protocol, RouteSlotId, RouterConfig};
use router_core::{ActivationStatus, BrowserLookup, RouteEngine};
use router_pingora::{
    DataPlaneTelemetry, HttpDataPlane, ProxyOptions, RunningHttpDataPlane, readiness,
};
use router_tcp::{TcpProxy, TcpProxyOptions, TcpTarget, TransitionPolicy};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    sync::{Mutex as AsyncMutex, watch},
    task::JoinHandle,
};

const MAX_FRAME_BYTES: usize = 1024 * 1024;
const MAX_EVENTS: usize = 256;

#[derive(Clone, Debug)]
pub struct AdminOptions {
    pub socket_path: PathBuf,
    pub token: String,
}

#[derive(Debug)]
pub enum ProcessError {
    Io(io::Error),
    Configuration(String),
    Http(router_pingora::BuildError),
}

impl fmt::Display for ProcessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "router I/O failed: {error}"),
            Self::Configuration(message) => {
                write!(formatter, "router configuration failed: {message}")
            }
            Self::Http(error) => write!(formatter, "HTTP data plane failed: {error}"),
        }
    }
}

impl std::error::Error for ProcessError {}

impl From<io::Error> for ProcessError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<router_pingora::BuildError> for ProcessError {
    fn from(error: router_pingora::BuildError) -> Self {
        Self::Http(error)
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Access,
    RoutingDecision,
    Health,
    Reload,
    Rejection,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterEvent {
    sequence: u64,
    timestamp_ms: u64,
    kind: EventKind,
    outcome: String,
    fields: BTreeMap<String, Value>,
}

#[derive(Default)]
struct Counters {
    requests: AtomicU64,
    connections: AtomicU64,
    errors: AtomicU64,
}

struct Observability {
    counters: Counters,
    sequence: AtomicU64,
    events: Mutex<VecDeque<RouterEvent>>,
}

impl Observability {
    fn new() -> Self {
        Self {
            counters: Counters::default(),
            sequence: AtomicU64::new(0),
            events: Mutex::new(VecDeque::with_capacity(MAX_EVENTS)),
        }
    }

    fn emit(&self, kind: EventKind, outcome: &str, fields: BTreeMap<String, Value>) {
        let fields = fields
            .into_iter()
            .map(|(key, value)| {
                let value = if sensitive_key(&key) {
                    Value::String("[REDACTED]".into())
                } else {
                    redact_value(value)
                };
                (key, value)
            })
            .collect();
        let event = RouterEvent {
            sequence: self.sequence.fetch_add(1, Ordering::Relaxed) + 1,
            timestamp_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX),
            kind,
            outcome: outcome.into(),
            fields,
        };
        let mut events = self
            .events
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if events.len() == MAX_EVENTS {
            events.pop_front();
        }
        events.push_back(event);
    }

    fn events(&self) -> Vec<RouterEvent> {
        self.events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .cloned()
            .collect()
    }
}

struct TcpBinding {
    listener: Listener,
    proxy: Arc<TcpProxy>,
}

struct ControlState {
    engine: Arc<RouteEngine>,
    initial_listeners: Vec<Listener>,
    initial_identity: router_config::IdentityPolicy,
    tcp: Vec<TcpBinding>,
    http_telemetry: DataPlaneTelemetry,
    apply_lock: AsyncMutex<()>,
    observations: Observability,
    shutdown: watch::Sender<bool>,
}

/// A complete router process. Dropping it requests shutdown; [`wait`](Self::wait)
/// performs an orderly data-plane shutdown.
pub struct RouterProcess {
    state: Arc<ControlState>,
    http: Option<RunningHttpDataPlane>,
    forward_proxies: Vec<forward_proxy::ForwardProxyBinding>,
    admin_task: Option<JoinHandle<io::Result<()>>>,
    admin_path: PathBuf,
    shutdown_rx: watch::Receiver<bool>,
}

/// Clonable trigger for signal handlers and embedding processes.
#[derive(Clone)]
pub struct ShutdownHandle(watch::Sender<bool>);

impl ShutdownHandle {
    pub fn request(&self) {
        self.0.send_replace(true);
    }
}

impl RouterProcess {
    pub async fn start(config: RouterConfig, admin: AdminOptions) -> Result<Self, ProcessError> {
        if admin.token.is_empty() {
            return Err(ProcessError::Configuration(
                "administration token must not be empty".into(),
            ));
        }
        validate_runtime_config(&config)?;
        let engine = Arc::new(
            RouteEngine::new(config.clone())
                .map_err(|error| ProcessError::Configuration(error.to_string()))?,
        );

        let http_listeners = config
            .spec
            .listeners
            .iter()
            .filter(|listener| {
                listener.protocol != Protocol::Tcp && listener.proxy_identity.is_none()
            })
            .cloned()
            .collect::<Vec<_>>();
        let (http, http_telemetry) = if http_listeners.is_empty() {
            (None, DataPlaneTelemetry::default())
        } else {
            let data_plane = HttpDataPlane::new(
                Arc::clone(&engine),
                http_listeners,
                config.spec.identity.clone(),
                ProxyOptions::default(),
            )?;
            let telemetry = data_plane.telemetry();
            (Some(data_plane.spawn()?), telemetry)
        };

        let (shutdown, shutdown_rx) = watch::channel(false);
        let mut forward_proxies = Vec::new();
        for listener in config
            .spec
            .listeners
            .iter()
            .filter(|listener| listener.proxy_identity.is_some())
        {
            forward_proxies.push(
                forward_proxy::ForwardProxyBinding::bind(
                    listener.clone(),
                    Arc::clone(&engine),
                    shutdown.subscribe(),
                )
                .await?,
            );
        }

        let mut tcp = Vec::new();
        for listener in config
            .spec
            .listeners
            .iter()
            .filter(|listener| listener.protocol == Protocol::Tcp)
        {
            let target = tcp_target(&engine, listener)?;
            let bind = SocketAddr::new(listener.bind.host, listener.bind.port);
            let proxy = TcpProxy::bind(bind, target, TcpProxyOptions::default()).await?;
            tcp.push(TcpBinding {
                listener: listener.clone(),
                proxy: Arc::new(proxy),
            });
        }

        let listener = bind_admin(&admin.socket_path).await?;
        let state = Arc::new(ControlState {
            engine,
            initial_listeners: config.spec.listeners,
            initial_identity: config.spec.identity,
            tcp,
            http_telemetry,
            apply_lock: AsyncMutex::new(()),
            observations: Observability::new(),
            shutdown,
        });
        let task_state = Arc::clone(&state);
        let token = Arc::<str>::from(admin.token);
        let admin_task = tokio::spawn(run_admin(listener, token, task_state));
        Ok(Self {
            state,
            http,
            forward_proxies,
            admin_task: Some(admin_task),
            admin_path: admin.socket_path,
            shutdown_rx,
        })
    }

    pub fn socket_path(&self) -> &Path {
        &self.admin_path
    }

    pub fn request_shutdown(&self) {
        self.state.shutdown.send_replace(true);
    }

    pub fn shutdown_handle(&self) -> ShutdownHandle {
        ShutdownHandle(self.state.shutdown.clone())
    }

    pub async fn wait(mut self) -> Result<(), ProcessError> {
        if !*self.shutdown_rx.borrow() {
            let _ = self.shutdown_rx.changed().await;
        }
        self.state.shutdown.send_replace(true);

        for binding in &self.state.tcp {
            binding.proxy.shutdown().await?;
        }
        for binding in self.forward_proxies.drain(..) {
            binding.wait().await?;
        }
        if let Some(http) = self.http.take() {
            tokio::task::spawn_blocking(move || http.shutdown())
                .await
                .map_err(|error| ProcessError::Configuration(error.to_string()))?;
        }
        if let Some(admin_task) = self.admin_task.take() {
            match admin_task.await {
                Ok(result) => result?,
                Err(error) if error.is_cancelled() => {}
                Err(error) => return Err(ProcessError::Configuration(error.to_string())),
            }
        }
        let _ = std::fs::remove_file(&self.admin_path);
        Ok(())
    }
}

impl Drop for RouterProcess {
    fn drop(&mut self) {
        self.state.shutdown.send_replace(true);
    }
}

async fn bind_admin(path: &Path) -> io::Result<UnixListener> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => {
            if UnixStream::connect(path).await.is_ok() {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    format!(
                        "administration socket is already active: {}",
                        path.display()
                    ),
                ));
            }
            std::fs::remove_file(path)?;
        }
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("administration path is not a socket: {}", path.display()),
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let listener = UnixListener::bind(path)?;
    if let Err(error) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
        let _ = std::fs::remove_file(path);
        return Err(error);
    }
    Ok(listener)
}

async fn run_admin(
    listener: UnixListener,
    token: Arc<str>,
    state: Arc<ControlState>,
) -> io::Result<()> {
    let mut shutdown = state.shutdown.subscribe();
    loop {
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Ok(());
                }
            }
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                state.observations.counters.connections.fetch_add(1, Ordering::Relaxed);
                let state = Arc::clone(&state);
                let token = Arc::clone(&token);
                tokio::spawn(async move {
                    if serve_connection(stream, &token, &state).await.is_err() {
                        state.observations.counters.errors.fetch_add(1, Ordering::Relaxed);
                    }
                });
            }
        }
    }
}

#[derive(Deserialize)]
struct AdminRequest {
    token: String,
    #[serde(flatten)]
    command: AdminCommand,
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "kebab-case")]
enum AdminCommand {
    Validate { config: RouterConfig },
    Apply { config: RouterConfig },
    CurrentVersion,
    Routes,
    Health,
    Drain,
    Counters,
    Events,
}

async fn serve_connection(
    mut stream: UnixStream,
    expected_token: &str,
    state: &Arc<ControlState>,
) -> io::Result<()> {
    let frame = match read_frame(&mut stream).await {
        Ok(frame) => frame,
        Err(error) => {
            state
                .observations
                .counters
                .errors
                .fetch_add(1, Ordering::Relaxed);
            write_response(
                &mut stream,
                false,
                json!({"code": "invalid_frame", "message": error.to_string()}),
            )
            .await?;
            return Ok(());
        }
    };
    state
        .observations
        .counters
        .requests
        .fetch_add(1, Ordering::Relaxed);
    let request: AdminRequest = match serde_json::from_slice(&frame) {
        Ok(request) => request,
        Err(error) => {
            state
                .observations
                .counters
                .errors
                .fetch_add(1, Ordering::Relaxed);
            rejection(state, "invalid_request", "decode");
            write_response(
                &mut stream,
                false,
                json!({"code": "invalid_request", "message": error.to_string()}),
            )
            .await?;
            return Ok(());
        }
    };
    if !constant_time_eq(request.token.as_bytes(), expected_token.as_bytes()) {
        state
            .observations
            .counters
            .errors
            .fetch_add(1, Ordering::Relaxed);
        rejection(state, "unauthorized", "authentication");
        write_response(
            &mut stream,
            false,
            json!({"code": "unauthorized", "message": "invalid administration token"}),
        )
        .await?;
        return Ok(());
    }

    let operation = command_name(&request.command);
    let (ok, body, drain) = execute(request.command, state).await;
    let mut fields = BTreeMap::new();
    fields.insert("operation".into(), Value::String(operation.into()));
    state
        .observations
        .emit(EventKind::Access, if ok { "ok" } else { "error" }, fields);
    if !ok {
        state
            .observations
            .counters
            .errors
            .fetch_add(1, Ordering::Relaxed);
    }
    write_response(&mut stream, ok, body).await?;
    if drain {
        state.state_shutdown();
    }
    Ok(())
}

impl ControlState {
    fn state_shutdown(&self) {
        self.shutdown.send_replace(true);
    }
}

fn command_name(command: &AdminCommand) -> &'static str {
    match command {
        AdminCommand::Validate { .. } => "validate",
        AdminCommand::Apply { .. } => "apply",
        AdminCommand::CurrentVersion => "current-version",
        AdminCommand::Routes => "routes",
        AdminCommand::Health => "health",
        AdminCommand::Drain => "drain",
        AdminCommand::Counters => "counters",
        AdminCommand::Events => "events",
    }
}

async fn execute(command: AdminCommand, state: &Arc<ControlState>) -> (bool, Value, bool) {
    match command {
        AdminCommand::Validate { config } => match validate_candidate(state, &config) {
            Ok(engine) => (true, snapshot_identity(&engine), false),
            Err(message) => {
                rejection(state, "invalid_configuration", "validate");
                (
                    false,
                    json!({"code": "invalid_configuration", "message": message}),
                    false,
                )
            }
        },
        AdminCommand::Apply { config } => {
            let _guard = state.apply_lock.lock().await;
            let candidate = match validate_candidate(state, &config) {
                Ok(candidate) => candidate,
                Err(message) => {
                    rejection(state, "invalid_configuration", "apply");
                    return (
                        false,
                        json!({"code": "invalid_configuration", "message": message}),
                        false,
                    );
                }
            };
            let active_version = state.engine.snapshot().version();
            if candidate.snapshot().version() > active_version {
                let targets = candidate
                    .snapshot()
                    .config()
                    .spec
                    .providers
                    .iter()
                    .map(|provider| {
                        (
                            provider.id.clone(),
                            provider.endpoint.clone(),
                            provider.health_check.clone(),
                        )
                    })
                    .collect::<Vec<_>>();
                let failed = readiness(targets)
                    .await
                    .into_iter()
                    .filter_map(|(provider, result)| {
                        result
                            .err()
                            .map(|error| (provider.to_string(), error.message))
                    })
                    .collect::<BTreeMap<_, _>>();
                if !failed.is_empty() {
                    let mut fields = BTreeMap::new();
                    fields.insert("activeVersion".into(), json!(active_version));
                    fields.insert(
                        "candidateVersion".into(),
                        json!(candidate.snapshot().version()),
                    );
                    fields.insert("providers".into(), json!(failed));
                    state.observations.emit(
                        EventKind::Health,
                        "candidate_unhealthy",
                        fields.clone(),
                    );
                    state
                        .observations
                        .emit(EventKind::Reload, "rolled_back", fields);
                    rejection(state, "provider_unhealthy", "apply");
                    return (
                        false,
                        json!({
                            "code": "provider_unhealthy",
                            "message": "candidate snapshot failed provider readiness; the previous snapshot remains active",
                            "status": "rolled_back",
                            "activeVersion": active_version,
                            "candidateVersion": candidate.snapshot().version(),
                            "providers": failed,
                        }),
                        false,
                    );
                }
            }
            match state.engine.apply(config) {
                Ok(ack) => {
                    if ack.status == ActivationStatus::Activated {
                        let snapshot = state.engine.snapshot();
                        for binding in &state.tcp {
                            match tcp_target(&state.engine, &binding.listener) {
                                Ok(target) => binding.proxy.reload(
                                    target,
                                    transition_policy(snapshot.transition(Protocol::Tcp)),
                                ),
                                Err(error) => {
                                    rejection(state, "tcp_reload_failed", "apply");
                                    return (
                                        false,
                                        json!({"code": "tcp_reload_failed", "message": error.to_string()}),
                                        false,
                                    );
                                }
                            }
                        }
                    }
                    let status = match ack.status {
                        ActivationStatus::Activated => "activated",
                        ActivationStatus::RejectedStale => "rejected_stale",
                    };
                    let mut fields = BTreeMap::new();
                    fields.insert("version".into(), json!(ack.version));
                    state.observations.emit(EventKind::Reload, status, fields);
                    (
                        true,
                        json!({"version": ack.version, "checksum": ack.checksum, "status": status}),
                        false,
                    )
                }
                Err(error) => {
                    rejection(state, "apply_failed", "apply");
                    (
                        false,
                        json!({"code": "apply_failed", "message": error.to_string()}),
                        false,
                    )
                }
            }
        }
        AdminCommand::CurrentVersion => (true, snapshot_identity(&state.engine), false),
        AdminCommand::Routes => {
            let routes = inspect_routes(&state.engine);
            state
                .observations
                .emit(EventKind::RoutingDecision, "inspected", BTreeMap::new());
            (true, routes, false)
        }
        AdminCommand::Health => {
            let snapshot = state.engine.snapshot();
            let targets = snapshot
                .config()
                .spec
                .providers
                .iter()
                .map(|provider| {
                    (
                        provider.id.clone(),
                        provider.endpoint.clone(),
                        provider.health_check.clone(),
                    )
                })
                .collect::<Vec<_>>();
            let checks = readiness(targets).await;
            let mut result = BTreeMap::new();
            let mut healthy = true;
            for (provider, check) in checks {
                let value = match check {
                    Ok(()) => json!({"healthy": true}),
                    Err(error) => {
                        healthy = false;
                        json!({"healthy": false, "message": error.message})
                    }
                };
                result.insert(provider.to_string(), value);
            }
            state.observations.emit(
                EventKind::Health,
                if healthy { "healthy" } else { "unhealthy" },
                BTreeMap::new(),
            );
            (
                true,
                json!({"healthy": healthy, "providers": result}),
                false,
            )
        }
        AdminCommand::Drain => (true, json!({"status": "draining"}), true),
        AdminCommand::Counters => (true, counters(state), false),
        AdminCommand::Events => (true, events(state), false),
    }
}

fn counters(state: &ControlState) -> Value {
    let http = state.http_telemetry.metrics();
    let mut tcp_accepted = 0;
    let mut tcp_active = 0;
    let mut tcp_errors = 0;
    for binding in &state.tcp {
        let snapshot = binding.proxy.telemetry().snapshot();
        tcp_accepted += snapshot.accepted_connections;
        tcp_active += snapshot.active_connections;
        tcp_errors += snapshot.errors;
    }
    json!({
        "dataPlane": {
            "httpRequests": http.requests,
            "httpActiveRequests": http.active_requests,
            "httpErrors": http.errors,
            "tcpAcceptedConnections": tcp_accepted,
            "tcpActiveConnections": tcp_active,
            "tcpErrors": tcp_errors,
        },
        "activeSnapshotVersion": state.engine.snapshot().version(),
        "adminRequests": state.observations.counters.requests.load(Ordering::Relaxed),
        "adminConnections": state.observations.counters.connections.load(Ordering::Relaxed),
        "adminErrors": state.observations.counters.errors.load(Ordering::Relaxed),
    })
}

fn events(state: &ControlState) -> Value {
    json!({
        "controlEvents": state.observations.events(),
        "httpEvents": state.http_telemetry.events(),
    })
}

fn validate_candidate(state: &ControlState, config: &RouterConfig) -> Result<RouteEngine, String> {
    if config.spec.listeners != state.initial_listeners
        || config.spec.identity != state.initial_identity
    {
        return Err("listener and identity changes require a process restart".into());
    }
    validate_runtime_config(config).map_err(|error| error.to_string())?;
    RouteEngine::new(config.clone()).map_err(|error| error.to_string())
}

fn validate_runtime_config(config: &RouterConfig) -> Result<(), ProcessError> {
    let engine = Arc::new(
        RouteEngine::new(config.clone())
            .map_err(|error| ProcessError::Configuration(error.to_string()))?,
    );
    let http = config
        .spec
        .listeners
        .iter()
        .filter(|listener| listener.protocol != Protocol::Tcp && listener.proxy_identity.is_none())
        .cloned()
        .collect::<Vec<_>>();
    if !http.is_empty() {
        HttpDataPlane::new(
            engine.clone(),
            http,
            config.spec.identity.clone(),
            ProxyOptions::default(),
        )?;
    }
    for listener in config
        .spec
        .listeners
        .iter()
        .filter(|listener| listener.protocol == Protocol::Tcp)
    {
        tcp_target(&engine, listener)?;
    }
    Ok(())
}

fn tcp_target(engine: &RouteEngine, listener: &Listener) -> Result<TcpTarget, ProcessError> {
    if listener.destinations.len() != 1 {
        return Err(ProcessError::Configuration(
            "a TCP listener must declare exactly one destination".into(),
        ));
    }
    let slot = listener.destinations[0].slot();
    let snapshot = engine.snapshot();
    let target = if let Some(consumer) = &listener.consumer {
        snapshot.lookup_consumer(consumer, slot).cloned()
    } else {
        let proxy = listener
            .proxy_identity
            .as_ref()
            .map(|identity| RouteSlotId::from(identity.as_str()));
        snapshot
            .lookup_browser(BrowserLookup {
                destination: slot,
                explicit_header: None,
                origin: None,
                proxy_listener: proxy.as_ref(),
            })
            .ok()
            .cloned()
    }
    .ok_or_else(|| {
        ProcessError::Configuration(format!("TCP listener route for slot {slot} is unresolved"))
    })?;
    if target.endpoint.protocol != Protocol::Tcp {
        return Err(ProcessError::Configuration(format!(
            "TCP slot {slot} selected a non-TCP provider"
        )));
    }
    Ok(TcpTarget::new(target.endpoint.host, target.endpoint.port))
}

fn transition_policy(policy: &ConnectionTransitionPolicy) -> TransitionPolicy {
    match policy {
        ConnectionTransitionPolicy::Close => TransitionPolicy::Close,
        ConnectionTransitionPolicy::Drain { timeout_ms } => {
            TransitionPolicy::Drain(Duration::from_millis(*timeout_ms))
        }
        ConnectionTransitionPolicy::Pin => TransitionPolicy::Pin,
    }
}

fn snapshot_identity(engine: &RouteEngine) -> Value {
    let snapshot = engine.snapshot();
    json!({
        "id": snapshot.id().as_str(),
        "version": snapshot.version(),
        "checksum": snapshot.checksum(),
    })
}

fn inspect_routes(engine: &RouteEngine) -> Value {
    let snapshot = engine.snapshot();
    let mut routes = Vec::new();
    for listener in &snapshot.config().spec.listeners {
        let Some(consumer) = &listener.consumer else {
            continue;
        };
        for destination in &listener.destinations {
            let slot = destination.slot();
            if let Some(target) = snapshot.lookup_consumer(consumer, slot) {
                routes.push(json!({
                    "consumer": consumer.as_str(),
                    "slot": slot.as_str(),
                    "provider": target.provider.as_str(),
                    "endpoint": {
                        "protocol": target.endpoint.protocol,
                        "host": target.endpoint.host,
                        "port": target.endpoint.port,
                    },
                }));
            }
        }
    }
    json!({
        "version": snapshot.version(),
        "checksum": snapshot.checksum(),
        "consumerRoutes": routes,
        "browserRoutes": snapshot.config().spec.browser_routes,
    })
}

fn rejection(state: &ControlState, code: &str, operation: &str) {
    let mut fields = BTreeMap::new();
    fields.insert("code".into(), Value::String(code.into()));
    fields.insert("operation".into(), Value::String(operation.into()));
    state
        .observations
        .emit(EventKind::Rejection, "rejected", fields);
}

async fn read_frame(stream: &mut UnixStream) -> io::Result<Vec<u8>> {
    let mut frame = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        let read = stream.read(&mut byte).await?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "request must end with a newline",
            ));
        }
        if byte[0] == b'\n' {
            return Ok(frame);
        }
        if frame.len() == MAX_FRAME_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "administration frame is too large",
            ));
        }
        frame.push(byte[0]);
    }
}

async fn write_response(stream: &mut UnixStream, ok: bool, body: Value) -> io::Result<()> {
    let response = if ok {
        json!({"ok": true, "result": body})
    } else {
        json!({"ok": false, "error": body})
    };
    let mut encoded = serde_json::to_vec(&response).map_err(io::Error::other)?;
    encoded.push(b'\n');
    stream.write_all(&encoded).await
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let max = left.len().max(right.len());
    let mut difference = left.len() ^ right.len();
    for index in 0..max {
        difference |= usize::from(*left.get(index).unwrap_or(&0) ^ *right.get(index).unwrap_or(&0));
    }
    difference == 0
}

fn sensitive_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase();
    [
        "authorization",
        "cookie",
        "password",
        "secret",
        "token",
        "privatekey",
        "private_key",
    ]
    .iter()
    .any(|sensitive| normalized.contains(sensitive))
}

fn redact_value(value: Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .map(|(key, value)| {
                    let value = if sensitive_key(&key) {
                        Value::String("[REDACTED]".into())
                    } else {
                        redact_value(value)
                    };
                    (key, value)
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.into_iter().map(redact_value).collect()),
        value => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_comparison_and_redaction_are_safe() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secrex"));
        assert!(!constant_time_eq(b"secret", b"short"));
        let redacted = redact_value(
            json!({"token": "secret", "nested": {"Authorization": "bearer"}, "safe": 1}),
        );
        assert_eq!(redacted["token"], "[REDACTED]");
        assert_eq!(redacted["nested"]["Authorization"], "[REDACTED]");
        assert_eq!(redacted["safe"], 1);
    }
}
