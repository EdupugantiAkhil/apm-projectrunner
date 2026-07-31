//! Raw TCP forwarding for Switchyard.
//!
//! [`TcpProxy`] owns one listener. Runtime orchestration can compose as many listeners
//! as it needs and atomically reload each listener's target without depending on any
//! router configuration types.

use std::{
    collections::BTreeSet,
    fmt, io,
    net::SocketAddr,
    sync::{
        Arc, Mutex as StdMutex, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Mutex, watch},
    task::{JoinHandle, JoinSet},
    time::{Instant, timeout},
};

/// Packet mark used by the transparent proxy for loopback connections that
/// must bypass namespace interception.
pub const TRANSPARENT_BYPASS_MARK: u32 = 0x5359;
/// Namespace-internal port used by sidecars to exchange passive listener
/// observations. It is never application-facing.
pub const TRANSPARENT_REGISTRY_PORT: u16 = 65_534;

/// One ordered candidate for transparent same-port forwarding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransparentTarget {
    component: Arc<str>,
    host: Arc<str>,
    local: bool,
    declared_ports: Option<Arc<BTreeSet<u16>>>,
}

impl TransparentTarget {
    pub fn new(component: impl Into<Arc<str>>, host: impl Into<Arc<str>>, local: bool) -> Self {
        Self {
            component: component.into(),
            host: host.into(),
            local,
            declared_ports: None,
        }
    }

    pub fn with_declared_ports(mut self, ports: impl IntoIterator<Item = u16>) -> Self {
        self.declared_ports = Some(Arc::new(ports.into_iter().collect()));
        self
    }

    pub fn component(&self) -> &str {
        &self.component
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn is_local(&self) -> bool {
        self.local
    }

    fn declares_port(&self, port: u16) -> Option<bool> {
        self.declared_ports
            .as_ref()
            .map(|ports| ports.contains(&port))
    }
}

#[derive(Debug)]
struct TransparentState {
    targets: Vec<TransparentTarget>,
}

/// A single listener receiving namespace-redirected loopback connections.
///
/// It recovers the original destination port and tries active group members in
/// authored priority order.
pub struct TransparentTcpProxy {
    local_addr: SocketAddr,
    state: Arc<StdMutex<TransparentState>>,
    shutdown: watch::Sender<bool>,
    task: Mutex<Option<JoinHandle<io::Result<()>>>>,
}

impl TransparentTcpProxy {
    pub async fn bind(
        bind: SocketAddr,
        targets: Vec<TransparentTarget>,
        registry_excluded_ports: BTreeSet<u16>,
        connect_timeout: Duration,
    ) -> io::Result<Self> {
        if !cfg!(target_os = "linux") {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "transparent localhost interception requires Linux",
            ));
        }
        let listener = TcpListener::bind(bind).await?;
        Self::from_listener(listener, targets, registry_excluded_ports, connect_timeout)
    }

    /// Binds an IPv6-only listener so an IPv4 wildcard listener may own the
    /// same reserved interception port.
    pub fn bind_v6_only(
        bind: SocketAddr,
        targets: Vec<TransparentTarget>,
        registry_excluded_ports: BTreeSet<u16>,
        connect_timeout: Duration,
    ) -> io::Result<Self> {
        if !cfg!(target_os = "linux") {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "transparent localhost interception requires Linux",
            ));
        }
        use socket2::{Domain, Protocol as SocketProtocol, Socket, Type};
        let socket = Socket::new(Domain::IPV6, Type::STREAM, Some(SocketProtocol::TCP))?;
        socket.set_only_v6(true)?;
        socket.set_reuse_address(true)?;
        socket.set_nonblocking(true)?;
        socket.bind(&bind.into())?;
        socket.listen(1024)?;
        let listener = TcpListener::from_std(socket.into())?;
        Self::from_listener(listener, targets, registry_excluded_ports, connect_timeout)
    }

    fn from_listener(
        listener: TcpListener,
        targets: Vec<TransparentTarget>,
        registry_excluded_ports: BTreeSet<u16>,
        connect_timeout: Duration,
    ) -> io::Result<Self> {
        let local_addr = listener.local_addr()?;
        let state = Arc::new(StdMutex::new(TransparentState { targets }));
        let (shutdown, shutdown_rx) = watch::channel(false);
        let task = tokio::spawn(run_transparent_listener(
            listener,
            Arc::clone(&state),
            Arc::new(registry_excluded_ports),
            shutdown_rx,
            connect_timeout,
        ));
        Ok(Self {
            local_addr,
            state,
            shutdown,
            task: Mutex::new(Some(task)),
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn reload(&self, targets: Vec<TransparentTarget>) {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .targets = targets;
    }

    pub async fn shutdown(&self) -> io::Result<()> {
        self.shutdown.send_replace(true);
        let Some(task) = self.task.lock().await.take() else {
            return Ok(());
        };
        join_result(task.await)
    }
}

impl Drop for TransparentTcpProxy {
    fn drop(&mut self) {
        self.shutdown.send_replace(true);
        if let Some(task) = self.task.get_mut().take() {
            task.abort();
        }
    }
}

async fn run_transparent_listener(
    listener: TcpListener,
    state: Arc<StdMutex<TransparentState>>,
    registry_excluded_ports: Arc<BTreeSet<u16>>,
    mut shutdown: watch::Receiver<bool>,
    connect_timeout: Duration,
) -> io::Result<()> {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
            accepted = listener.accept() => {
                let (client, _) = accepted?;
                let targets = state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .targets
                    .clone();
                let registry_excluded_ports = Arc::clone(&registry_excluded_ports);
                connections.spawn(async move {
                    let _ = forward_transparent(
                        client,
                        targets,
                        &registry_excluded_ports,
                        connect_timeout,
                    )
                    .await;
                });
            }
        }
    }
    connections.abort_all();
    while connections.join_next().await.is_some() {}
    Ok(())
}

async fn forward_transparent(
    mut client: TcpStream,
    targets: Vec<TransparentTarget>,
    registry_excluded_ports: &BTreeSet<u16>,
    connect_timeout: Duration,
) -> io::Result<()> {
    let original = original_destination(&client)?;
    let port = original.port();
    if !original.ip().is_loopback() && port == TRANSPARENT_REGISTRY_PORT {
        let ports = listening_ports(registry_excluded_ports)?;
        let encoded = ports
            .into_iter()
            .map(|port| port.to_string())
            .collect::<Vec<_>>()
            .join(",");
        client.write_all(encoded.as_bytes()).await?;
        client.shutdown().await?;
        return Ok(());
    }
    let local_destinations = if original.is_ipv4() {
        [
            SocketAddr::from(([127, 0, 0, 1], port)),
            SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], port)),
        ]
    } else {
        [
            SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], port)),
            SocketAddr::from(([127, 0, 0, 1], port)),
        ]
    };

    // Connections redirected from the deployment bridge are attempts to reach
    // this member specifically. Only locally-originated loopback connections
    // may fall through to other ordered group members.
    if !original.ip().is_loopback() {
        for local_destination in local_destinations {
            if let Ok(mut local) = marked_loopback_connect(local_destination, connect_timeout).await
            {
                tokio::io::copy_bidirectional(&mut client, &mut local).await?;
                return Ok(());
            }
        }
        return Err(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            format!("this group member does not listen on port {port}"),
        ));
    }

    let observed = observe_targets(&targets, port, registry_excluded_ports, connect_timeout).await;
    let active = observed
        .iter()
        .enumerate()
        .filter_map(|(index, listening)| listening.then_some(index))
        .collect::<Vec<_>>();
    if active.len() > 1 {
        eprintln!(
            "switchyard: port {port} has {} active group members: {}; routing to {}, the first listed",
            active.len(),
            active
                .iter()
                .map(|index| targets[*index].component())
                .collect::<Vec<_>>()
                .join(", "),
            targets[active[0]].component(),
        );
    }
    for index in active {
        let target = &targets[index];
        if let Ok(mut upstream) = connect_target(target, port, connect_timeout).await {
            tokio::io::copy_bidirectional(&mut client, &mut upstream).await?;
            return Ok(());
        }
    }
    for (index, target) in targets.into_iter().enumerate() {
        if observed.get(index).copied().unwrap_or(false) {
            continue;
        }
        if target.declares_port(port) == Some(false) {
            continue;
        }
        if let Ok(mut upstream) = connect_target(&target, port, connect_timeout).await {
            tokio::io::copy_bidirectional(&mut client, &mut upstream).await?;
            return Ok(());
        }
    }
    // An instance with no active routing group must retain its own localhost.
    // This also covers the short startup interval before its registry endpoint
    // is reachable.
    for local_destination in local_destinations {
        if let Ok(mut local) = marked_loopback_connect(local_destination, connect_timeout).await {
            tokio::io::copy_bidirectional(&mut client, &mut local).await?;
            return Ok(());
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("no active group member listens on port {port}"),
    ))
}

async fn observe_targets(
    targets: &[TransparentTarget],
    port: u16,
    registry_excluded_ports: &BTreeSet<u16>,
    connect_timeout: Duration,
) -> Vec<bool> {
    let mut checks = JoinSet::new();
    for (index, target) in targets.iter().enumerate() {
        let host = Arc::clone(&target.host);
        let local = target.local;
        let declared_ports = target.declared_ports.clone();
        let registry_excluded_ports = registry_excluded_ports.clone();
        checks.spawn(async move {
            let ports = if let Some(ports) = declared_ports {
                Some((*ports).clone())
            } else if local {
                listening_ports(&registry_excluded_ports).ok()
            } else {
                observed_listeners(&host, connect_timeout).await
            };
            (index, ports.is_some_and(|ports| ports.contains(&port)))
        });
    }
    let mut observed = vec![false; targets.len()];
    while let Some(Ok((index, listening))) = checks.join_next().await {
        observed[index] = listening;
    }
    observed
}

async fn connect_target(
    target: &TransparentTarget,
    port: u16,
    connect_timeout: Duration,
) -> io::Result<TcpStream> {
    if target.local {
        for destination in [
            SocketAddr::from(([127, 0, 0, 1], port)),
            SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], port)),
        ] {
            if let Ok(stream) = marked_loopback_connect(destination, connect_timeout).await {
                return Ok(stream);
            }
        }
        return Err(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            format!("local group member does not listen on port {port}"),
        ));
    }
    timeout(
        connect_timeout,
        TcpStream::connect((target.host.as_ref(), port)),
    )
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "group member connect timed out"))?
}

async fn observed_listeners(host: &str, connect_timeout: Duration) -> Option<BTreeSet<u16>> {
    let mut stream = timeout(
        connect_timeout,
        TcpStream::connect((host, TRANSPARENT_REGISTRY_PORT)),
    )
    .await
    .ok()?
    .ok()?;
    let mut encoded = Vec::new();
    timeout(connect_timeout, stream.read_to_end(&mut encoded))
        .await
        .ok()?
        .ok()?;
    let value = std::str::from_utf8(&encoded).ok()?;
    value
        .split(',')
        .filter(|port| !port.is_empty())
        .map(str::parse)
        .collect::<Result<BTreeSet<_>, _>>()
        .ok()
}

#[cfg(target_os = "linux")]
fn listening_ports(excluded: &BTreeSet<u16>) -> io::Result<BTreeSet<u16>> {
    let mut ports = BTreeSet::new();
    for path in ["/proc/net/tcp", "/proc/net/tcp6"] {
        let table = std::fs::read_to_string(path)?;
        for line in table.lines().skip(1) {
            let columns = line.split_whitespace().collect::<Vec<_>>();
            if columns.len() < 4 || columns[3] != "0A" {
                continue;
            }
            let Some((_, encoded_port)) = columns[1].rsplit_once(':') else {
                continue;
            };
            if let Ok(port) = u16::from_str_radix(encoded_port, 16)
                && port != TRANSPARENT_REGISTRY_PORT
                && !excluded.contains(&port)
            {
                ports.insert(port);
            }
        }
    }
    Ok(ports)
}

#[cfg(not(target_os = "linux"))]
fn listening_ports(_excluded: &BTreeSet<u16>) -> io::Result<BTreeSet<u16>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "transparent localhost interception requires Linux",
    ))
}

#[cfg(target_os = "linux")]
fn original_destination(stream: &TcpStream) -> io::Result<SocketAddr> {
    let socket = socket2::SockRef::from(stream);
    let original = if stream.local_addr()?.is_ipv6() {
        socket.original_dst_v6()?
    } else {
        socket.original_dst_v4()?
    };
    original.as_socket().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "original destination is not an IP socket address",
        )
    })
}

#[cfg(not(target_os = "linux"))]
fn original_destination(_stream: &TcpStream) -> io::Result<SocketAddr> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "transparent localhost interception requires Linux",
    ))
}

#[cfg(target_os = "linux")]
async fn marked_loopback_connect(
    destination: SocketAddr,
    connect_timeout: Duration,
) -> io::Result<TcpStream> {
    use socket2::{Domain, Protocol as SocketProtocol, Socket, Type};

    let socket = Socket::new(
        Domain::for_address(destination),
        Type::STREAM,
        Some(SocketProtocol::TCP),
    )?;
    socket.set_mark(TRANSPARENT_BYPASS_MARK)?;
    socket.set_nonblocking(true)?;
    match socket.connect(&destination.into()) {
        Ok(()) => {}
        Err(error)
            if matches!(
                error.raw_os_error(),
                Some(libc::EINPROGRESS) | Some(libc::EALREADY)
            ) => {}
        Err(error) => return Err(error),
    }
    let stream = TcpStream::from_std(socket.into())?;
    timeout(connect_timeout, stream.writable())
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "local connect timed out"))??;
    if let Some(error) = stream.take_error()? {
        return Err(error);
    }
    Ok(stream)
}

#[cfg(not(target_os = "linux"))]
async fn marked_loopback_connect(
    _destination: SocketAddr,
    _connect_timeout: Duration,
) -> io::Result<TcpStream> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "transparent localhost interception requires Linux",
    ))
}

/// The upstream endpoint selected for new connections.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TcpTarget {
    host: Arc<str>,
    port: u16,
}

impl TcpTarget {
    pub fn new(host: impl Into<Arc<str>>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
        }
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

impl From<SocketAddr> for TcpTarget {
    fn from(address: SocketAddr) -> Self {
        Self::new(address.ip().to_string(), address.port())
    }
}

impl fmt::Display for TcpTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.host, self.port)
    }
}

/// What a reload does to connections using the previous target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransitionPolicy {
    /// Close existing connections promptly.
    Close,
    /// Let existing connections finish, but only for the given duration.
    Drain(Duration),
    /// Keep existing connections on their original target until they finish naturally.
    Pin,
}

/// Per-listener transport limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TcpProxyOptions {
    pub connect_timeout: Duration,
    pub idle_timeout: Duration,
    pub shutdown_timeout: Duration,
}

/// A cheap, cloneable view of one listener's counters.
#[derive(Clone, Debug, Default)]
pub struct TcpTelemetry {
    counters: Arc<TcpCounters>,
}

impl TcpTelemetry {
    pub fn snapshot(&self) -> TcpTelemetrySnapshot {
        TcpTelemetrySnapshot {
            accepted_connections: self.counters.accepted.load(Ordering::Relaxed),
            active_connections: self.counters.active.load(Ordering::Relaxed),
            errors: self.counters.errors.load(Ordering::Relaxed),
        }
    }
}

/// A point-in-time copy of a listener's counters.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TcpTelemetrySnapshot {
    pub accepted_connections: u64,
    pub active_connections: u64,
    pub errors: u64,
}

#[derive(Debug, Default)]
struct TcpCounters {
    accepted: AtomicU64,
    active: AtomicU64,
    errors: AtomicU64,
}

struct ActiveConnection(Arc<TcpCounters>);

impl Drop for ActiveConnection {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Default for TcpProxyOptions {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            idle_timeout: Duration::from_secs(5 * 60),
            shutdown_timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Debug)]
struct RouteState {
    target: TcpTarget,
    waiting: Vec<Weak<watch::Sender<Option<TransitionPolicy>>>>,
}

/// A running TCP listener.
pub struct TcpProxy {
    local_addr: SocketAddr,
    route: Arc<StdMutex<RouteState>>,
    shutdown: watch::Sender<bool>,
    task: Mutex<Option<JoinHandle<io::Result<()>>>>,
    shutdown_timeout: Duration,
    telemetry: TcpTelemetry,
}

impl TcpProxy {
    /// Binds and starts a listener. Port zero may be used to request an ephemeral port.
    pub async fn bind(
        bind: SocketAddr,
        target: TcpTarget,
        options: TcpProxyOptions,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind(bind).await?;
        let local_addr = listener.local_addr()?;
        let initial = RouteState {
            target,
            waiting: Vec::new(),
        };
        let route = Arc::new(StdMutex::new(initial));
        let telemetry = TcpTelemetry::default();
        let (shutdown, shutdown_rx) = watch::channel(false);
        let task = tokio::spawn(run_listener(
            listener,
            Arc::clone(&route),
            shutdown_rx,
            options,
            Arc::clone(&telemetry.counters),
        ));

        Ok(Self {
            local_addr,
            route,
            shutdown,
            task: Mutex::new(Some(task)),
            shutdown_timeout: options.shutdown_timeout,
            telemetry,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn telemetry(&self) -> TcpTelemetry {
        self.telemetry.clone()
    }

    /// Routes new connections to `target` and applies `policy` to older connections.
    pub fn reload(&self, target: TcpTarget, policy: TransitionPolicy) {
        let waiting = {
            let mut route = self.route.lock().unwrap_or_else(|error| error.into_inner());
            route.target = target;
            std::mem::take(&mut route.waiting)
        };
        for transition in waiting.into_iter().filter_map(|sender| sender.upgrade()) {
            transition.send_replace(Some(policy));
        }
    }

    /// Stops accepting, then waits for active connections up to the shutdown timeout.
    ///
    /// Calling this more than once is harmless.
    pub async fn shutdown(&self) -> io::Result<()> {
        self.shutdown.send_replace(true);
        let Some(mut task) = self.task.lock().await.take() else {
            return Ok(());
        };

        match timeout(self.shutdown_timeout, &mut task).await {
            Ok(result) => join_result(result),
            Err(_) => {
                task.abort();
                let _ = task.await;
                Ok(())
            }
        }
    }
}

impl Drop for TcpProxy {
    fn drop(&mut self) {
        self.shutdown.send_replace(true);
        if let Some(task) = self.task.get_mut().take() {
            task.abort();
        }
    }
}

fn join_result(result: Result<io::Result<()>, tokio::task::JoinError>) -> io::Result<()> {
    result.map_err(io::Error::other)?
}

async fn run_listener(
    listener: TcpListener,
    route: Arc<StdMutex<RouteState>>,
    mut shutdown: watch::Receiver<bool>,
    options: TcpProxyOptions,
    counters: Arc<TcpCounters>,
) -> io::Result<()> {
    let mut connections = JoinSet::new();

    loop {
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            Some(result) = connections.join_next(), if !connections.is_empty() => {
                // Connection-local transport failures must not stop the listener.
                let _ = result;
            }
            accepted = listener.accept() => {
                let (client, _) = match accepted {
                    Ok(connection) => connection,
                    Err(error) => {
                        counters.errors.fetch_add(1, Ordering::Relaxed);
                        return Err(error);
                    }
                };
                counters.accepted.fetch_add(1, Ordering::Relaxed);
                counters.active.fetch_add(1, Ordering::Relaxed);
                let active = ActiveConnection(Arc::clone(&counters));
                let (transition_tx, transition_rx) = watch::channel(None);
                let transition_tx = Arc::new(transition_tx);
                let target = {
                    let mut route = route.lock().unwrap_or_else(|error| error.into_inner());
                    route.waiting.retain(|sender| sender.strong_count() != 0);
                    route.waiting.push(Arc::downgrade(&transition_tx));
                    route.target.clone()
                };
                connections.spawn(proxy_connection(
                    client,
                    target,
                    transition_rx,
                    transition_tx,
                    options,
                    Arc::clone(&counters),
                    active,
                ));
            }
        }
    }

    drop(listener);
    while connections.join_next().await.is_some() {}
    Ok(())
}

async fn proxy_connection(
    mut client: TcpStream,
    target: TcpTarget,
    mut transition: watch::Receiver<Option<TransitionPolicy>>,
    _transition_guard: Arc<watch::Sender<Option<TransitionPolicy>>>,
    options: TcpProxyOptions,
    counters: Arc<TcpCounters>,
    _active: ActiveConnection,
) {
    let connected = timeout(
        options.connect_timeout,
        TcpStream::connect((target.host(), target.port())),
    )
    .await;
    let Ok(Ok(mut upstream)) = connected else {
        counters.errors.fetch_add(1, Ordering::Relaxed);
        return;
    };

    let forwarding = forward_until_idle(&mut client, &mut upstream, options.idle_timeout);
    tokio::pin!(forwarding);
    tokio::select! {
        result = &mut forwarding => {
            if result.is_err() {
                counters.errors.fetch_add(1, Ordering::Relaxed);
            }
        }
        changed = transition.changed() => {
            if changed.is_err() {
                return;
            }
            let policy = *transition.borrow_and_update();
            match policy {
                Some(TransitionPolicy::Close) | None => {}
                Some(TransitionPolicy::Drain(duration)) => {
                    tokio::select! {
                        result = &mut forwarding => {
                            if result.is_err() {
                                counters.errors.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        _ = tokio::time::sleep_until(Instant::now() + duration) => {}
                    }
                }
                Some(TransitionPolicy::Pin) => {
                    forwarding.await.ok();
                }
            }
        }
    }
}

async fn forward_until_idle<A, B>(left: &mut A, right: &mut B, idle: Duration) -> io::Result<()>
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    let (left_read, left_write) = tokio::io::split(left);
    let (right_read, right_write) = tokio::io::split(right);
    let (activity_tx, mut activity_rx) = watch::channel(0_u64);

    let left_to_right = copy_direction(left_read, right_write, activity_tx.clone());
    let right_to_left = copy_direction(right_read, left_write, activity_tx);
    tokio::pin!(left_to_right, right_to_left);
    let mut left_done = false;
    let mut right_done = false;

    loop {
        let idle_sleep = tokio::time::sleep(idle);
        tokio::pin!(idle_sleep);
        tokio::select! {
            result = &mut left_to_right, if !left_done => {
                result?;
                left_done = true;
            }
            result = &mut right_to_left, if !right_done => {
                result?;
                right_done = true;
            }
            changed = activity_rx.changed() => {
                if changed.is_err() {
                    return Ok(());
                }
            }
            _ = &mut idle_sleep => {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "TCP connection was idle"));
            }
        }
        if left_done && right_done {
            return Ok(());
        }
    }
}

async fn copy_direction<R, W>(
    mut reader: R,
    mut writer: W,
    activity: watch::Sender<u64>,
) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            return writer.shutdown().await;
        }
        writer.write_all(&buffer[..count]).await?;
        activity.send_modify(|generation| *generation = generation.wrapping_add(1));
    }
}

#[cfg(test)]
mod transparent_tests {
    use super::*;

    #[tokio::test]
    async fn declared_external_ports_are_candidates_and_connect_port_for_port() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let target =
            TransparentTarget::new("staging-es", "127.0.0.1", false).with_declared_ports([port]);
        let observed = observe_targets(
            std::slice::from_ref(&target),
            port,
            &BTreeSet::new(),
            Duration::from_secs(1),
        )
        .await;
        assert_eq!(observed, [true]);
        assert_eq!(target.declares_port(port), Some(true));
        assert_eq!(target.declares_port(port.saturating_sub(1)), Some(false));

        let accepted = tokio::spawn(async move { listener.accept().await.unwrap().1 });
        let stream = connect_target(&target, port, Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(stream.peer_addr().unwrap().port(), port);
        accepted.await.unwrap();
    }
}
