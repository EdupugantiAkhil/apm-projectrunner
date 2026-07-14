//! Pingora-backed HTTP-family data plane.
//!
//! Pingora is an implementation detail: the public API accepts only Switchyard
//! configuration and routing types.

use std::{
    collections::{BTreeMap, VecDeque},
    fmt,
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use bytes::Bytes;
use pingora::{
    apps::HttpServerOptions,
    connectors::http::Connector,
    http::{RequestHeader, ResponseHeader},
    prelude::{Error, ErrorSource, ErrorType, HttpPeer, Result as PingoraResult},
    proxy::{FailToProxy, ProxyHttp, ProxyServiceBuilder, Session},
    server::{RunArgs, Server, ShutdownSignal, ShutdownSignalWatch, configuration::ServerConf},
};
use router_config::{
    ComponentId, HealthCheck, HealthCheckProtocol, IdentityPolicy, Listener, ListenerDestination,
    Protocol, ProxyAuthentication, ProxyAuthenticationScheme, RouteSlotId, UpstreamEndpoint,
};
use router_core::{
    BrowserLookup, BrowserRouteCandidate, CompiledSnapshot, LookupError, RouteEngine, RouteTarget,
};
use serde::Serialize;
use subtle::ConstantTimeEq;
use tokio::sync::Notify;

/// Operational limits which intentionally do not alter the versioned route contract.
#[derive(Clone, Debug)]
pub struct ProxyOptions {
    pub connect_timeout: Duration,
    pub total_connect_timeout: Duration,
    pub max_request_header_bytes: usize,
    pub max_request_body_bytes: usize,
}

impl Default for ProxyOptions {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(3),
            total_connect_timeout: Duration::from_secs(5),
            max_request_header_bytes: 64 * 1024,
            max_request_body_bytes: 16 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildErrorCode {
    UnsupportedListener,
    MissingDestination,
    InvalidTlsIdentity,
    InvalidProxyAuthentication,
    ServerInitialization,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BuildError {
    pub code: BuildErrorCode,
    pub message: String,
}

impl BuildError {
    fn new(code: BuildErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for BuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for BuildError {}

struct ProxyCredential {
    authorization: Box<[u8]>,
}

const MAX_PROXY_CREDENTIAL_BYTES: u64 = 256;

fn load_proxy_credential(
    authentication: &ProxyAuthentication,
) -> Result<ProxyCredential, BuildError> {
    match authentication.scheme {
        ProxyAuthenticationScheme::Basic => {}
    }
    let metadata = std::fs::symlink_metadata(&authentication.credential_file).map_err(|error| {
        BuildError::new(
            BuildErrorCode::InvalidProxyAuthentication,
            format!(
                "could not inspect proxy credential file {}: {error}",
                authentication.credential_file.display()
            ),
        )
    })?;
    if !metadata.file_type().is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_PROXY_CREDENTIAL_BYTES
    {
        return Err(BuildError::new(
            BuildErrorCode::InvalidProxyAuthentication,
            "proxy credential must be a regular file of 1 to 256 bytes",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.mode() & 0o777 != 0o600 {
            return Err(BuildError::new(
                BuildErrorCode::InvalidProxyAuthentication,
                "proxy credential file must have mode 0600",
            ));
        }
    }
    let file = std::fs::File::open(&authentication.credential_file).map_err(|error| {
        BuildError::new(
            BuildErrorCode::InvalidProxyAuthentication,
            format!(
                "could not read proxy credential file {}: {error}",
                authentication.credential_file.display()
            ),
        )
    })?;
    let mut token = Vec::with_capacity(metadata.len() as usize);
    use std::io::Read as _;
    file.take(MAX_PROXY_CREDENTIAL_BYTES + 1)
        .read_to_end(&mut token)
        .map_err(|error| {
            BuildError::new(
                BuildErrorCode::InvalidProxyAuthentication,
                format!(
                    "could not read proxy credential file {}: {error}",
                    authentication.credential_file.display()
                ),
            )
        })?;
    if token.len() as u64 > MAX_PROXY_CREDENTIAL_BYTES {
        token.fill(0);
        return Err(BuildError::new(
            BuildErrorCode::InvalidProxyAuthentication,
            "proxy credential must be a regular file of 1 to 256 bytes",
        ));
    }
    if token.last() == Some(&b'\n') {
        token.pop();
        if token.last() == Some(&b'\r') {
            token.pop();
        }
    }
    if token.is_empty() || token.iter().any(|byte| matches!(byte, b'\r' | b'\n')) {
        token.fill(0);
        return Err(BuildError::new(
            BuildErrorCode::InvalidProxyAuthentication,
            "proxy credential file must contain one non-empty token",
        ));
    }

    let mut cleartext = b"switchyard:".to_vec();
    cleartext.extend_from_slice(&token);
    token.fill(0);
    let encoded = BASE64_STANDARD.encode(&cleartext);
    cleartext.fill(0);
    Ok(ProxyCredential {
        authorization: format!("Basic {encoded}").into_bytes().into_boxed_slice(),
    })
}

/// Loads the exact Basic authorization value used by authenticated proxy listeners.
pub fn proxy_authorization(authentication: &ProxyAuthentication) -> Result<Box<[u8]>, BuildError> {
    load_proxy_credential(authentication).map(|credential| credential.authorization)
}

/// A cheap snapshot of data-plane counters.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct MetricsSnapshot {
    pub requests: u64,
    pub errors: u64,
    pub active_requests: usize,
}

/// Header-free, structured events retained by [`DataPlaneTelemetry`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DataPlaneEvent {
    Routing {
        provider: ComponentId,
        snapshot_version: u64,
    },
    Rejection {
        status: u16,
        code: String,
    },
    Access {
        provider: Option<ComponentId>,
        succeeded: bool,
    },
}

struct TelemetryInner {
    requests: AtomicU64,
    errors: AtomicU64,
    active_requests: AtomicUsize,
    event_capacity: usize,
    events: Mutex<VecDeque<DataPlaneEvent>>,
}

/// Cloneable metrics and bounded event handle for the HTTP-family data plane.
#[derive(Clone)]
pub struct DataPlaneTelemetry(Arc<TelemetryInner>);

impl DataPlaneTelemetry {
    pub fn new(event_capacity: usize) -> Self {
        Self(Arc::new(TelemetryInner {
            requests: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            active_requests: AtomicUsize::new(0),
            event_capacity,
            events: Mutex::new(VecDeque::with_capacity(event_capacity)),
        }))
    }

    pub fn metrics(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            requests: self.0.requests.load(Ordering::Relaxed),
            errors: self.0.errors.load(Ordering::Relaxed),
            active_requests: self.0.active_requests.load(Ordering::Relaxed),
        }
    }

    pub fn events(&self) -> Vec<DataPlaneEvent> {
        self.0
            .events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .cloned()
            .collect()
    }

    fn request_started(&self) {
        self.0.requests.fetch_add(1, Ordering::Relaxed);
        self.0.active_requests.fetch_add(1, Ordering::Relaxed);
    }

    fn request_finished(&self) {
        self.0.active_requests.fetch_sub(1, Ordering::Relaxed);
    }

    fn error(&self) {
        self.0.errors.fetch_add(1, Ordering::Relaxed);
    }

    fn record(&self, event: DataPlaneEvent) {
        if self.0.event_capacity == 0 {
            return;
        }
        let mut events = self
            .0
            .events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if events.len() == self.0.event_capacity {
            events.pop_front();
        }
        events.push_back(event);
    }
}

impl Default for DataPlaneTelemetry {
    fn default() -> Self {
        Self::new(256)
    }
}

/// A Pingora-backed collection of HTTP-family listeners.
pub struct HttpDataPlane {
    engine: Arc<RouteEngine>,
    listeners: Vec<Listener>,
    proxy_credentials: Vec<Option<ProxyCredential>>,
    identity: IdentityPolicy,
    options: ProxyOptions,
    telemetry: DataPlaneTelemetry,
}

impl HttpDataPlane {
    pub fn new(
        engine: Arc<RouteEngine>,
        listeners: Vec<Listener>,
        identity: IdentityPolicy,
        options: ProxyOptions,
    ) -> Result<Self, BuildError> {
        let mut proxy_credentials = Vec::with_capacity(listeners.len());
        for listener in &listeners {
            if !matches!(
                listener.protocol,
                Protocol::Http | Protocol::Https | Protocol::Websocket | Protocol::Grpc
            ) {
                return Err(BuildError::new(
                    BuildErrorCode::UnsupportedListener,
                    format!(
                        "{}:{} is not an HTTP-family listener",
                        listener.bind.host, listener.bind.port
                    ),
                ));
            }
            if listener.destinations.is_empty() {
                return Err(BuildError::new(
                    BuildErrorCode::MissingDestination,
                    format!(
                        "{}:{} has no destinations",
                        listener.bind.host, listener.bind.port
                    ),
                ));
            }
            if matches!(listener.protocol, Protocol::Https) != listener.tls.is_some() {
                return Err(BuildError::new(
                    BuildErrorCode::UnsupportedListener,
                    "HTTPS listeners require a TLS identity and non-HTTPS listeners must not declare one",
                ));
            }
            proxy_credentials.push(
                listener
                    .proxy_authentication
                    .as_ref()
                    .map(load_proxy_credential)
                    .transpose()?,
            );
        }
        Ok(Self {
            engine,
            listeners,
            proxy_credentials,
            identity,
            options,
            telemetry: DataPlaneTelemetry::default(),
        })
    }

    pub fn telemetry(&self) -> DataPlaneTelemetry {
        self.telemetry.clone()
    }

    pub fn with_telemetry(mut self, telemetry: DataPlaneTelemetry) -> Self {
        self.telemetry = telemetry;
        self
    }

    /// Starts the listeners on a background thread.
    pub fn spawn(self) -> Result<RunningHttpDataPlane, BuildError> {
        let conf = ServerConf {
            threads: 1,
            grace_period_seconds: Some(0),
            graceful_shutdown_timeout_seconds: Some(0),
            ..ServerConf::default()
        };
        let mut server = Server::new_with_opt_and_conf(None, conf);
        let addresses = self
            .listeners
            .iter()
            .map(|listener| SocketAddr::new(listener.bind.host, listener.bind.port))
            .collect::<Vec<_>>();

        for (index, (listener, proxy_credential)) in self
            .listeners
            .into_iter()
            .zip(self.proxy_credentials)
            .enumerate()
        {
            let app = SwitchyardProxy {
                engine: self.engine.clone(),
                listener: listener.clone(),
                proxy_credential,
                identity: self.identity.clone(),
                options: self.options.clone(),
                telemetry: self.telemetry.clone(),
            };
            let mut server_options = HttpServerOptions::default();
            server_options.h2c = matches!(listener.protocol, Protocol::Grpc);
            let mut service = ProxyServiceBuilder::new(&server.configuration, app)
                .name(format!("switchyard-http-{index}"))
                .server_options(server_options)
                .build();
            let address = format!("{}:{}", listener.bind.host, listener.bind.port);
            if let Some(tls) = &listener.tls {
                let certificate = tls.certificate.to_str().ok_or_else(|| {
                    BuildError::new(
                        BuildErrorCode::ServerInitialization,
                        "TLS certificate path must be valid UTF-8",
                    )
                })?;
                let private_key = tls.private_key.to_str().ok_or_else(|| {
                    BuildError::new(
                        BuildErrorCode::ServerInitialization,
                        "TLS private key path must be valid UTF-8",
                    )
                })?;
                service
                    .add_tls(&address, certificate, private_key)
                    .map_err(|error| {
                        BuildError::new(BuildErrorCode::ServerInitialization, error.to_string())
                    })?;
            } else {
                service.add_tcp(&address);
            }
            server.add_service(service);
        }

        server.bootstrap();
        let shutdown = Arc::new(Notify::new());
        let signal = NotifyShutdown(shutdown.clone());
        let join = thread::Builder::new()
            .name("switchyard-pingora".into())
            .spawn(move || {
                server.run(RunArgs {
                    #[cfg(unix)]
                    shutdown_signal: Box::new(signal),
                });
            })
            .map_err(|error| {
                BuildError::new(BuildErrorCode::ServerInitialization, error.to_string())
            })?;

        Ok(RunningHttpDataPlane {
            addresses,
            shutdown,
            join: Some(join),
            telemetry: self.telemetry,
        })
    }
}

/// Handle used to observe startup and stop a running data plane.
pub struct RunningHttpDataPlane {
    addresses: Vec<SocketAddr>,
    shutdown: Arc<Notify>,
    join: Option<thread::JoinHandle<()>>,
    telemetry: DataPlaneTelemetry,
}

impl RunningHttpDataPlane {
    pub fn wait_ready(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        self.addresses.iter().all(|address| {
            loop {
                if std::net::TcpStream::connect_timeout(address, Duration::from_millis(25)).is_ok()
                {
                    break true;
                }
                if Instant::now() >= deadline {
                    break false;
                }
                thread::sleep(Duration::from_millis(10));
            }
        })
    }

    pub fn shutdown(mut self) {
        self.stop();
    }

    pub fn telemetry(&self) -> DataPlaneTelemetry {
        self.telemetry.clone()
    }

    fn stop(&mut self) {
        // `notify_one` retains a permit if Pingora has opened its listener but
        // has not started polling the shutdown future yet.
        self.shutdown.notify_one();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for RunningHttpDataPlane {
    fn drop(&mut self) {
        self.stop();
    }
}

struct NotifyShutdown(Arc<Notify>);

#[async_trait]
impl ShutdownSignalWatch for NotifyShutdown {
    async fn recv(&self) -> ShutdownSignal {
        self.0.notified().await;
        ShutdownSignal::FastShutdown
    }
}

#[derive(Default)]
struct RequestContext {
    snapshot: Option<Arc<CompiledSnapshot>>,
    target: Option<RouteTarget>,
    cors_origin: Option<String>,
    browser_routed: bool,
    body_bytes: usize,
    rejection: Option<ProxyRejection>,
    started: bool,
    error_counted: bool,
}

struct SwitchyardProxy {
    engine: Arc<RouteEngine>,
    listener: Listener,
    proxy_credential: Option<ProxyCredential>,
    identity: IdentityPolicy,
    options: ProxyOptions,
    telemetry: DataPlaneTelemetry,
}

impl SwitchyardProxy {
    async fn reject_proxy_authentication(
        &self,
        session: &mut Session,
        ctx: &mut RequestContext,
    ) -> PingoraResult<bool> {
        self.telemetry.error();
        ctx.error_counted = true;
        self.telemetry.record(DataPlaneEvent::Rejection {
            status: 407,
            code: "proxy_authentication_required".into(),
        });
        respond_proxy_authentication_required(session).await?;
        Ok(true)
    }

    async fn reject(
        &self,
        session: &mut Session,
        ctx: &mut RequestContext,
        status: u16,
        code: &'static str,
        message: &str,
    ) -> PingoraResult<bool> {
        self.telemetry.error();
        ctx.error_counted = true;
        self.telemetry.record(DataPlaneEvent::Rejection {
            status,
            code: code.into(),
        });
        respond_json(session, status, code, message, ctx.cors_origin.as_deref()).await?;
        Ok(true)
    }

    async fn reject_browser_route(
        &self,
        session: &mut Session,
        ctx: &mut RequestContext,
        status: u16,
        code: &'static str,
        message: &str,
        candidates: &[BrowserRouteCandidate],
    ) -> PingoraResult<bool> {
        self.telemetry.error();
        ctx.error_counted = true;
        self.telemetry.record(DataPlaneEvent::Rejection {
            status,
            code: code.into(),
        });
        respond_route_json(
            session,
            status,
            code,
            message,
            candidates,
            ctx.cors_origin.as_deref(),
        )
        .await?;
        Ok(true)
    }
}

#[async_trait]
impl ProxyHttp for SwitchyardProxy {
    type CTX = RequestContext;

    fn new_ctx(&self) -> Self::CTX {
        RequestContext::default()
    }

    async fn request_filter(
        &self,
        session: &mut Session,
        ctx: &mut Self::CTX,
    ) -> PingoraResult<bool> {
        self.telemetry.request_started();
        ctx.started = true;
        if request_header_bytes(session.req_header()) > self.options.max_request_header_bytes {
            return self
                .reject(
                    session,
                    ctx,
                    431,
                    "request_headers_too_large",
                    "request headers exceed the configured limit",
                )
                .await;
        }
        if let Some(credential) = &self.proxy_credential {
            let supplied = session.req_header().headers.get("proxy-authorization");
            let authorized = header_count(session.req_header(), "proxy-authorization") == 1
                && supplied.is_some_and(|value| {
                    bool::from(value.as_bytes().ct_eq(credential.authorization.as_ref()))
                });
            if !authorized {
                return self.reject_proxy_authentication(session, ctx).await;
            }
        }

        let Some(slot) = destination_slot(&self.listener, session.req_header()) else {
            return self
                .reject(
                    session,
                    ctx,
                    404,
                    "route_not_found",
                    "the request does not match a listener destination",
                )
                .await;
        };
        let snapshot = self.engine.snapshot();
        let target = if let Some(consumer) = &self.listener.consumer {
            snapshot.lookup_consumer(consumer, slot).cloned()
        } else {
            ctx.browser_routed = true;
            if header_count(session.req_header(), &self.identity.explicit_header) > 1
                || header_count(session.req_header(), "origin") > 1
            {
                let candidates = snapshot.browser_candidates(slot);
                return self
                    .reject_browser_route(
                        session,
                        ctx,
                        400,
                        "conflicting_route_identity",
                        "routing identity headers must occur at most once",
                        &candidates,
                    )
                    .await;
            }
            let explicit = header_text(session.req_header(), &self.identity.explicit_header);
            let origin = header_text(session.req_header(), "origin");
            ctx.cors_origin = origin
                .filter(|origin| {
                    snapshot
                        .lookup_browser(BrowserLookup {
                            destination: slot,
                            explicit_header: None,
                            origin: Some(origin),
                            proxy_listener: None,
                        })
                        .is_ok()
                })
                .map(str::to_owned);
            if explicit.is_some() && !self.listener.bind.host.is_loopback() {
                let candidates = snapshot.browser_candidates(slot);
                return self
                    .reject_browser_route(
                        session,
                        ctx,
                        403,
                        "untrusted_identity_header",
                        "the explicit route header is accepted only on loopback listeners",
                        &candidates,
                    )
                    .await;
            }
            let proxy_listener = self
                .listener
                .proxy_identity
                .as_ref()
                .map(|identity| RouteSlotId::from(identity.as_str()));
            match snapshot.lookup_browser(BrowserLookup {
                destination: slot,
                explicit_header: explicit,
                origin,
                proxy_listener: proxy_listener.as_ref(),
            }) {
                Ok(target) => Some(target.clone()),
                Err(error) => {
                    let (status, code) = match error {
                        LookupError::MissingIdentity => (400, "missing_route_identity"),
                        LookupError::UnknownIdentity if explicit.is_some() => {
                            (403, "unknown_route_identity")
                        }
                        LookupError::UnknownIdentity if origin.is_some() => {
                            (403, "disallowed_origin")
                        }
                        LookupError::UnknownIdentity => (403, "unknown_route_identity"),
                        LookupError::ConflictingIdentity => (400, "conflicting_route_identity"),
                    };
                    let candidates = snapshot.browser_candidates(slot);
                    return self
                        .reject_browser_route(
                            session,
                            ctx,
                            status,
                            code,
                            &error.to_string(),
                            &candidates,
                        )
                        .await;
                }
            }
        };
        let Some(target) = target else {
            return self
                .reject(
                    session,
                    ctx,
                    404,
                    "route_not_found",
                    "no route is configured for this request",
                )
                .await;
        };

        self.telemetry.record(DataPlaneEvent::Routing {
            provider: target.provider.clone(),
            snapshot_version: snapshot.version(),
        });

        ctx.snapshot = Some(snapshot);
        ctx.target = Some(target);
        if ctx.browser_routed && is_cors_preflight(session.req_header()) {
            if ctx.cors_origin.is_none() {
                return self
                    .reject(
                        session,
                        ctx,
                        403,
                        "disallowed_origin",
                        "CORS preflight Origin is not configured for this destination",
                    )
                    .await;
            }
            let preflight = match parse_cors_preflight(session.req_header()) {
                Ok(preflight) => preflight,
                Err(message) => {
                    return self
                        .reject(session, ctx, 400, "invalid_cors_preflight", message)
                        .await;
                }
            };
            let origin = ctx
                .cors_origin
                .as_deref()
                .expect("a routed CORS preflight has an Origin");
            respond_preflight(session, origin, &preflight).await?;
            return Ok(true);
        }
        let target = ctx.target.as_ref().expect("route target was just stored");
        if let Some(check) = &target.health_check {
            if let Err(error) = probe_endpoint(&target.endpoint, check).await {
                return self
                    .reject(session, ctx, 503, "provider_unhealthy", &error.message)
                    .await;
            }
        }
        Ok(false)
    }

    async fn upstream_peer(
        &self,
        _session: &mut Session,
        ctx: &mut Self::CTX,
    ) -> PingoraResult<Box<HttpPeer>> {
        let target = ctx.target.as_ref().ok_or_else(|| {
            Error::explain(
                ErrorType::InternalError,
                "route target missing after request filter",
            )
        })?;
        let tls = matches!(target.endpoint.protocol, Protocol::Https);
        let mut peer = HttpPeer::new(
            (&*target.endpoint.host, target.endpoint.port),
            tls,
            target.endpoint.host.clone(),
        );
        peer.options.connection_timeout = Some(self.options.connect_timeout);
        peer.options.total_connection_timeout = Some(self.options.total_connect_timeout);
        if matches!(target.endpoint.protocol, Protocol::Grpc) {
            peer.options.set_http_version(2, 2);
        }
        Ok(Box::new(peer))
    }

    async fn upstream_request_filter(
        &self,
        session: &mut Session,
        request: &mut RequestHeader,
        ctx: &mut Self::CTX,
    ) -> PingoraResult<()> {
        let preserve_identity = !self.identity.strip_before_forwarding
            && ctx
                .target
                .as_ref()
                .is_some_and(|target| target.receive_identity_header);
        if !preserve_identity {
            request.remove_header(&self.identity.explicit_header);
        }
        request.remove_header("proxy-authorization");
        if let Some(host) = header_text(session.req_header(), "host") {
            request.insert_header("x-forwarded-host", host)?;
        }
        request.insert_header(
            "x-forwarded-proto",
            if self.listener.tls.is_some() {
                "https"
            } else {
                "http"
            },
        )?;
        if let Some(address) = session.as_downstream().client_addr() {
            if let Some(inet) = address.as_inet() {
                request.append_header("x-forwarded-for", inet.ip().to_string())?;
            }
        }
        Ok(())
    }

    async fn response_filter(
        &self,
        _session: &mut Session,
        response: &mut ResponseHeader,
        ctx: &mut Self::CTX,
    ) -> PingoraResult<()> {
        if ctx.browser_routed {
            apply_cors_response_headers(response, ctx.cors_origin.as_deref())?;
        }
        Ok(())
    }

    async fn request_body_filter(
        &self,
        _session: &mut Session,
        body: &mut Option<Bytes>,
        _end_of_stream: bool,
        ctx: &mut Self::CTX,
    ) -> PingoraResult<()> {
        ctx.body_bytes = ctx
            .body_bytes
            .saturating_add(body.as_ref().map_or(0, Bytes::len));
        if ctx.body_bytes > self.options.max_request_body_bytes {
            ctx.rejection = Some(ProxyRejection {
                status: 413,
                code: "request_body_too_large",
                message: "request body exceeds the configured limit".into(),
            });
            return Err(Error::explain(
                ErrorType::HTTPStatus(413),
                "request body exceeds configured limit",
            ));
        }
        Ok(())
    }

    async fn fail_to_proxy(
        &self,
        session: &mut Session,
        error: &Error,
        ctx: &mut Self::CTX,
    ) -> FailToProxy {
        let rejection = ctx.rejection.take().unwrap_or_else(|| {
            if error.esource() == &ErrorSource::Upstream {
                ProxyRejection {
                    status: 502,
                    code: "upstream_unavailable",
                    message: "the selected provider could not be reached".into(),
                }
            } else {
                ProxyRejection {
                    status: 500,
                    code: "proxy_error",
                    message: "the request could not be proxied".into(),
                }
            }
        });
        if !ctx.error_counted {
            self.telemetry.error();
            ctx.error_counted = true;
        }
        self.telemetry.record(DataPlaneEvent::Rejection {
            status: rejection.status,
            code: rejection.code.into(),
        });
        let _ = respond_json(
            session,
            rejection.status,
            rejection.code,
            &rejection.message,
            ctx.cors_origin.as_deref(),
        )
        .await;
        FailToProxy {
            error_code: rejection.status,
            can_reuse_downstream: false,
        }
    }

    async fn logging(&self, _session: &mut Session, error: Option<&Error>, ctx: &mut Self::CTX) {
        if error.is_some() && !ctx.error_counted {
            self.telemetry.error();
            ctx.error_counted = true;
        }
        if ctx.started {
            self.telemetry.request_finished();
            ctx.started = false;
        }
        self.telemetry.record(DataPlaneEvent::Access {
            provider: ctx.target.as_ref().map(|target| target.provider.clone()),
            succeeded: error.is_none() && !ctx.error_counted,
        });
    }
}

struct ProxyRejection {
    status: u16,
    code: &'static str,
    message: String,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    code: &'a str,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    candidates: Option<Vec<RouteCandidateBody<'a>>>,
}

#[derive(Serialize)]
struct RouteCandidateBody<'a> {
    identity: &'a str,
    provider: &'a ComponentId,
}

async fn respond_proxy_authentication_required(session: &mut Session) -> PingoraResult<()> {
    let body = serde_json::to_vec(&ErrorBody {
        code: "proxy_authentication_required",
        message: "valid managed-profile proxy credentials are required",
        candidates: None,
    })
    .map_err(|error| Error::because(ErrorType::InternalError, "serialize proxy error", error))?;
    let mut header = ResponseHeader::build(407, Some(4))?;
    header.insert_header("content-type", "application/json")?;
    header.insert_header("content-length", body.len().to_string())?;
    header.insert_header("proxy-authenticate", "Basic realm=\"Switchyard\"")?;
    session
        .write_response_header(Box::new(header), false)
        .await?;
    session
        .write_response_body(Some(Bytes::from(body)), true)
        .await
}

async fn respond_json(
    session: &mut Session,
    status: u16,
    code: &str,
    message: &str,
    allowed_origin: Option<&str>,
) -> PingoraResult<()> {
    respond_error_json(
        session,
        status,
        ErrorBody {
            code,
            message,
            candidates: None,
        },
        allowed_origin,
    )
    .await
}

async fn respond_route_json(
    session: &mut Session,
    status: u16,
    code: &str,
    message: &str,
    candidates: &[BrowserRouteCandidate],
    allowed_origin: Option<&str>,
) -> PingoraResult<()> {
    respond_error_json(
        session,
        status,
        ErrorBody {
            code,
            message,
            candidates: Some(
                candidates
                    .iter()
                    .map(|candidate| RouteCandidateBody {
                        identity: &candidate.identity,
                        provider: &candidate.provider,
                    })
                    .collect(),
            ),
        },
        allowed_origin,
    )
    .await
}

async fn respond_error_json(
    session: &mut Session,
    status: u16,
    error_body: ErrorBody<'_>,
    allowed_origin: Option<&str>,
) -> PingoraResult<()> {
    let body = serde_json::to_vec(&error_body).map_err(|error| {
        Error::because(ErrorType::InternalError, "serialize proxy error", error)
    })?;
    let mut header = ResponseHeader::build(status, Some(5))?;
    header.insert_header("content-type", "application/json")?;
    header.insert_header("content-length", body.len().to_string())?;
    if let Some(origin) = allowed_origin {
        header.insert_header("access-control-allow-origin", origin)?;
        header.insert_header("vary", "Origin")?;
    }
    session
        .write_response_header(Box::new(header), false)
        .await?;
    session
        .write_response_body(Some(Bytes::from(body)), true)
        .await
}

fn is_cors_preflight(request: &RequestHeader) -> bool {
    request.method.as_str() == "OPTIONS"
        && header_text(request, "origin").is_some()
        && header_text(request, "access-control-request-method").is_some()
}

struct CorsPreflight {
    requested_method: String,
    requested_headers: Option<String>,
    private_network: bool,
}

fn parse_cors_preflight(request: &RequestHeader) -> Result<CorsPreflight, &'static str> {
    if header_count(request, "access-control-request-method") != 1
        || header_count(request, "access-control-request-headers") > 1
        || header_count(request, "access-control-request-private-network") > 1
    {
        return Err("CORS preflight headers must occur at most once");
    }
    let requested_method = header_text(request, "access-control-request-method")
        .filter(|value| value.len() <= 32 && is_http_token(value))
        .ok_or("CORS preflight method must be a valid HTTP token of at most 32 bytes")?;
    let requested_headers = header_text(request, "access-control-request-headers");
    if requested_headers.is_some_and(|value| {
        value.len() > 1024
            || value.split(',').count() > 32
            || value.split(',').any(|name| !is_http_token(name.trim()))
    }) {
        return Err("CORS preflight may request at most 32 valid header names in 1024 bytes");
    }
    let private_network = match header_text(request, "access-control-request-private-network") {
        None => false,
        Some(value) if value.eq_ignore_ascii_case("true") => true,
        Some(_) => return Err("private-network preflight must request the value `true`"),
    };
    Ok(CorsPreflight {
        requested_method: requested_method.to_owned(),
        requested_headers: requested_headers.map(str::to_owned),
        private_network,
    })
}

fn is_http_token(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

/// Reflect only bounded, syntactically valid preflight fields after exact Origin and route checks.
async fn respond_preflight(
    session: &mut Session,
    origin: &str,
    preflight: &CorsPreflight,
) -> PingoraResult<()> {
    let mut header = ResponseHeader::build(204, Some(7))?;
    header.insert_header("content-length", "0")?;
    header.insert_header("access-control-allow-origin", origin)?;
    header.insert_header("access-control-allow-methods", &preflight.requested_method)?;
    if let Some(requested_headers) = &preflight.requested_headers {
        header.insert_header("access-control-allow-headers", requested_headers)?;
    }
    if preflight.private_network {
        header.insert_header("access-control-allow-private-network", "true")?;
    }
    header.insert_header(
        "vary",
        "Origin, Access-Control-Request-Method, Access-Control-Request-Headers, Access-Control-Request-Private-Network",
    )?;
    session.write_response_header(Box::new(header), true).await
}

fn apply_cors_response_headers(
    response: &mut ResponseHeader,
    allowed_origin: Option<&str>,
) -> PingoraResult<()> {
    for name in [
        "access-control-allow-origin",
        "access-control-allow-methods",
        "access-control-allow-headers",
        "access-control-allow-private-network",
        "access-control-allow-credentials",
        "access-control-expose-headers",
        "access-control-max-age",
    ] {
        response.remove_header(name);
    }
    if let Some(origin) = allowed_origin {
        response.insert_header("access-control-allow-origin", origin)?;
    }
    let vary_has_origin = response
        .headers
        .get_all("vary")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|value| value.trim().eq_ignore_ascii_case("origin"));
    if !vary_has_origin {
        response.append_header("vary", "Origin")?;
    }
    Ok(())
}

fn destination_slot<'a>(
    listener: &'a Listener,
    request: &RequestHeader,
) -> Option<&'a RouteSlotId> {
    let host = header_text(request, "host")
        .and_then(|value| value.split(':').next())
        .unwrap_or_default();
    listener
        .destinations
        .iter()
        .find(|destination| match destination {
            ListenerDestination::CustomDomain { domain, .. } => domain.eq_ignore_ascii_case(host),
            ListenerDestination::LegacyLocalhost { host: expected, .. } => {
                expected.eq_ignore_ascii_case(host)
            }
            ListenerDestination::Loopback { .. } => listener.destinations.len() == 1,
            ListenerDestination::ProxyTarget { host: expected, .. } => {
                expected.eq_ignore_ascii_case(host)
            }
        })
        .map(ListenerDestination::slot)
}

fn header_text<'a>(request: &'a RequestHeader, name: &str) -> Option<&'a str> {
    request.headers.get(name)?.to_str().ok()
}

fn header_count(request: &RequestHeader, name: &str) -> usize {
    request.headers.get_all(name).iter().count()
}

fn request_header_bytes(request: &RequestHeader) -> usize {
    request.method.as_str().len()
        + request.uri.to_string().len()
        + request
            .headers
            .iter()
            .map(|(name, value)| name.as_str().len() + value.as_bytes().len() + 4)
            .sum::<usize>()
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct HealthError {
    pub message: String,
}

impl fmt::Display for HealthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for HealthError {}

/// Runs one configured provider probe. A successful HTTP probe must return 2xx.
pub async fn probe_endpoint(
    endpoint: &UpstreamEndpoint,
    check: &HealthCheck,
) -> Result<(), HealthError> {
    let timeout = Duration::from_millis(check.timeout_ms);
    let address = format!("{}:{}", endpoint.host, endpoint.port);
    if matches!(check.protocol, HealthCheckProtocol::Tcp) {
        return tokio::time::timeout(timeout, tokio::net::TcpStream::connect(&address))
            .await
            .map_err(|_| health_error("health check timed out"))?
            .map(|_| ())
            .map_err(|error| health_error(error.to_string()));
    }

    let tls = matches!(check.protocol, HealthCheckProtocol::Https);
    let connector = Connector::new(None);
    let peer = HttpPeer::new((&*endpoint.host, endpoint.port), tls, endpoint.host.clone());
    let path = check.path.as_deref().unwrap_or("/");
    tokio::time::timeout(timeout, async {
        let (mut session, _) = connector
            .get_http_session(&peer)
            .await
            .map_err(|error| health_error(error.to_string()))?;
        let mut request = RequestHeader::build("GET", path.as_bytes(), None)
            .map_err(|error| health_error(error.to_string()))?;
        request
            .insert_header("host", &endpoint.host)
            .map_err(|error| health_error(error.to_string()))?;
        session
            .write_request_header(Box::new(request))
            .await
            .map_err(|error| health_error(error.to_string()))?;
        session
            .finish_request_body()
            .await
            .map_err(|error| health_error(error.to_string()))?;
        session
            .read_response_header()
            .await
            .map_err(|error| health_error(error.to_string()))?;
        let status = session
            .response_header()
            .map(|header| header.status.as_u16())
            .ok_or_else(|| health_error("health check returned no response"))?;
        if (200..300).contains(&status) {
            Ok(())
        } else {
            Err(health_error(format!("health check returned HTTP {status}")))
        }
    })
    .await
    .map_err(|_| health_error("health check timed out"))?
}

fn health_error(message: impl Into<String>) -> HealthError {
    HealthError {
        message: message.into(),
    }
}

/// Probes all health-checked targets in the active snapshot once.
pub async fn readiness(
    targets: impl IntoIterator<Item = (ComponentId, UpstreamEndpoint, Option<HealthCheck>)>,
) -> BTreeMap<ComponentId, Result<(), HealthError>> {
    let mut result = BTreeMap::new();
    for (provider, endpoint, check) in targets {
        let status = match check {
            Some(check) => probe_endpoint(&endpoint, &check).await,
            None => Ok(()),
        };
        result.insert(provider, status);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    use router_config::{InstanceId, SocketAddress};

    fn listener(destination: ListenerDestination) -> Listener {
        Listener {
            consumer: Some(InstanceId::from("consumer")),
            bind: SocketAddress {
                host: IpAddr::V4(Ipv4Addr::LOCALHOST),
                port: 8080,
            },
            protocol: Protocol::Http,
            tls: None,
            destinations: vec![destination],
            proxy_identity: None,
            proxy_authentication: None,
        }
    }

    #[test]
    fn loopback_listener_selects_its_only_slot() {
        let listener = listener(ListenerDestination::Loopback {
            slot: RouteSlotId::from("api"),
        });
        let request = RequestHeader::build("GET", b"/", None).unwrap();
        assert_eq!(
            destination_slot(&listener, &request).unwrap().as_str(),
            "api"
        );
    }

    #[test]
    fn host_destination_is_exact_and_case_insensitive() {
        let listener = listener(ListenerDestination::CustomDomain {
            slot: RouteSlotId::from("ui"),
            domain: "app.test".into(),
        });
        let mut request = RequestHeader::build("GET", b"/", None).unwrap();
        request.insert_header("host", "APP.TEST:8080").unwrap();
        assert_eq!(
            destination_slot(&listener, &request).unwrap().as_str(),
            "ui"
        );
    }

    #[test]
    fn telemetry_keeps_only_the_newest_bounded_events() {
        let telemetry = DataPlaneTelemetry::new(2);
        for status in [400, 404, 503] {
            telemetry.record(DataPlaneEvent::Rejection {
                status,
                code: "rejected".into(),
            });
        }
        assert_eq!(
            telemetry
                .events()
                .iter()
                .map(|event| match event {
                    DataPlaneEvent::Rejection { status, .. } => *status,
                    _ => unreachable!(),
                })
                .collect::<Vec<_>>(),
            [404, 503]
        );
    }

    #[cfg(unix)]
    #[test]
    fn proxy_credentials_reject_symlinks_and_oversized_files() {
        use std::{
            os::unix::fs::{PermissionsExt, symlink},
            time::{SystemTime, UNIX_EPOCH},
        };

        let directory = std::env::temp_dir().join(format!(
            "switchyard-invalid-proxy-auth-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let target = directory.join("target");
        std::fs::write(&target, b"token").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        let credential_file = directory.join("credential");
        symlink(&target, &credential_file).unwrap();
        let authentication = ProxyAuthentication {
            scheme: ProxyAuthenticationScheme::Basic,
            credential_file: credential_file.clone(),
        };
        let Err(error) = load_proxy_credential(&authentication) else {
            panic!("proxy credential symlink was accepted");
        };
        assert_eq!(error.code, BuildErrorCode::InvalidProxyAuthentication);

        std::fs::remove_file(&credential_file).unwrap();
        std::fs::write(&credential_file, vec![b'x'; 257]).unwrap();
        std::fs::set_permissions(&credential_file, std::fs::Permissions::from_mode(0o600)).unwrap();
        let Err(error) = load_proxy_credential(&authentication) else {
            panic!("oversized proxy credential was accepted");
        };
        assert_eq!(error.code, BuildErrorCode::InvalidProxyAuthentication);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
