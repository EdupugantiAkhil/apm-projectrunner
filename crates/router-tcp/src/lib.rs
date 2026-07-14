//! Raw TCP forwarding for Switchyard.
//!
//! [`TcpProxy`] owns one listener. Runtime orchestration can compose as many listeners
//! as it needs and atomically reload each listener's target without depending on any
//! router configuration types.

use std::{
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
