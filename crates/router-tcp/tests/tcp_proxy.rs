use std::{
    fs, io,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use router_tcp::{
    TcpProxy, TcpProxyOptions, TcpTarget, TcpTelemetry, TcpTelemetrySnapshot, TransitionPolicy,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
    time::{sleep, timeout},
};

struct Backend {
    address: SocketAddr,
    task: JoinHandle<()>,
}

impl Drop for Backend {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn backend(identity: u8) -> Backend {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let (mut reader, mut writer) = stream.into_split();
                if writer.write_all(&[identity]).await.is_ok() {
                    let _ = tokio::io::copy(&mut reader, &mut writer).await;
                }
            });
        }
    });
    Backend { address, task }
}

fn options() -> TcpProxyOptions {
    TcpProxyOptions {
        connect_timeout: Duration::from_secs(1),
        idle_timeout: Duration::from_secs(5),
        shutdown_timeout: Duration::from_millis(100),
    }
}

async fn connect(proxy: &TcpProxy, identity: u8) -> TcpStream {
    let mut stream = TcpStream::connect(proxy.local_addr()).await.unwrap();
    let mut actual = [0];
    stream.read_exact(&mut actual).await.unwrap();
    assert_eq!(actual[0], identity);
    stream
}

async fn round_trip(stream: &mut TcpStream, byte: u8) {
    stream.write_all(&[byte]).await.unwrap();
    let mut actual = [0];
    stream.read_exact(&mut actual).await.unwrap();
    assert_eq!(actual[0], byte);
}

async fn assert_closed(stream: &mut TcpStream) {
    let mut byte = [0];
    let count = timeout(Duration::from_secs(1), stream.read(&mut byte))
        .await
        .expect("connection did not close")
        .unwrap_or(0);
    assert_eq!(count, 0);
}

async fn wait_for_telemetry(
    telemetry: &TcpTelemetry,
    expected: impl Fn(TcpTelemetrySnapshot) -> bool,
) -> TcpTelemetrySnapshot {
    timeout(Duration::from_secs(1), async {
        loop {
            let snapshot = telemetry.snapshot();
            if expected(snapshot) {
                return snapshot;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("telemetry was not updated")
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug)]
struct ResourceSample {
    fds: usize,
    rss_kb: usize,
}

#[cfg(target_os = "linux")]
fn resource_sample() -> ResourceSample {
    let fds = fs::read_dir("/proc/self/fd").unwrap().count();
    let status = fs::read_to_string("/proc/self/status").unwrap();
    let rss_kb = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|line| line.split_whitespace().next())
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    ResourceSample { fds, rss_kb }
}

#[tokio::test]
async fn reload_routes_only_new_connections_to_the_new_target() {
    let first = backend(b'A').await;
    let second = backend(b'B').await;
    let proxy = TcpProxy::bind(
        "127.0.0.1:0".parse().unwrap(),
        first.address.into(),
        options(),
    )
    .await
    .unwrap();
    let telemetry = proxy.telemetry();
    assert_eq!(telemetry.snapshot(), TcpTelemetrySnapshot::default());

    let mut old = connect(&proxy, b'A').await;
    assert_eq!(telemetry.snapshot().accepted_connections, 1);
    assert_eq!(telemetry.snapshot().active_connections, 1);
    proxy.reload(second.address.into(), TransitionPolicy::Pin);
    let mut new = connect(&proxy, b'B').await;

    round_trip(&mut old, 1).await;
    round_trip(&mut new, 2).await;
    assert_eq!(telemetry.snapshot().accepted_connections, 2);
    assert_eq!(telemetry.snapshot().active_connections, 2);
    proxy.shutdown().await.unwrap();
    assert_eq!(telemetry.snapshot().active_connections, 0);
}

#[tokio::test]
async fn close_policy_terminates_old_connections() {
    let first = backend(b'A').await;
    let second = backend(b'B').await;
    let proxy = TcpProxy::bind(
        "127.0.0.1:0".parse().unwrap(),
        first.address.into(),
        options(),
    )
    .await
    .unwrap();
    let mut old = connect(&proxy, b'A').await;

    proxy.reload(second.address.into(), TransitionPolicy::Close);

    assert_closed(&mut old).await;
    let _new = connect(&proxy, b'B').await;
    proxy.shutdown().await.unwrap();
}

#[tokio::test]
async fn drain_policy_allows_activity_until_its_deadline() {
    let first = backend(b'A').await;
    let second = backend(b'B').await;
    let proxy = TcpProxy::bind(
        "127.0.0.1:0".parse().unwrap(),
        first.address.into(),
        options(),
    )
    .await
    .unwrap();
    let mut old = connect(&proxy, b'A').await;

    proxy.reload(
        second.address.into(),
        TransitionPolicy::Drain(Duration::from_millis(150)),
    );
    round_trip(&mut old, 1).await;
    sleep(Duration::from_millis(175)).await;
    assert_closed(&mut old).await;
    proxy.shutdown().await.unwrap();
}

#[tokio::test]
async fn pin_policy_survives_later_route_changes() {
    let first = backend(b'A').await;
    let second = backend(b'B').await;
    let third = backend(b'C').await;
    let proxy = TcpProxy::bind(
        "127.0.0.1:0".parse().unwrap(),
        first.address.into(),
        options(),
    )
    .await
    .unwrap();
    let mut pinned = connect(&proxy, b'A').await;

    proxy.reload(second.address.into(), TransitionPolicy::Pin);
    proxy.reload(third.address.into(), TransitionPolicy::Close);

    round_trip(&mut pinned, 9).await;
    let _new = connect(&proxy, b'C').await;
    proxy.shutdown().await.unwrap();
}

#[tokio::test]
async fn idle_and_shutdown_timeouts_bound_long_lived_connections() -> io::Result<()> {
    let upstream = backend(b'A').await;
    let mut limits = options();
    limits.idle_timeout = Duration::from_millis(100);
    limits.shutdown_timeout = Duration::from_millis(75);
    let proxy = TcpProxy::bind(
        "127.0.0.1:0".parse().unwrap(),
        TcpTarget::from(upstream.address),
        limits,
    )
    .await?;
    let telemetry = proxy.telemetry();

    let mut idle = connect(&proxy, b'A').await;
    assert_closed(&mut idle).await;
    let after_idle = wait_for_telemetry(&telemetry, |snapshot| {
        snapshot.active_connections == 0 && snapshot.errors == 1
    })
    .await;
    assert_eq!(after_idle.accepted_connections, 1);

    let mut active = connect(&proxy, b'A').await;
    for byte in 0..3 {
        round_trip(&mut active, byte).await;
        if byte != 2 {
            sleep(Duration::from_millis(50)).await;
        }
    }
    proxy.shutdown().await?;
    assert_closed(&mut active).await;
    let after_shutdown = telemetry.snapshot();
    assert_eq!(after_shutdown.accepted_connections, 2);
    assert_eq!(after_shutdown.active_connections, 0);
    assert_eq!(after_shutdown.errors, 1);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "socket-bound reliability test; run via scripts/reliability.sh"]
async fn reload_storm_under_concurrent_clients_has_no_partial_tcp_responses_or_leaks() {
    let duration = std::env::var("SWITCHYARD_RELOAD_STORM_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(30));
    let clients = std::env::var("SWITCHYARD_CONCURRENCY")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(16);
    let first = backend(b'A').await;
    let second = backend(b'B').await;
    let proxy = Arc::new(
        TcpProxy::bind(
            "127.0.0.1:0".parse().unwrap(),
            first.address.into(),
            options(),
        )
        .await
        .unwrap(),
    );
    let telemetry = proxy.telemetry();

    let mut pinned = connect(&proxy, b'A').await;
    proxy.reload(second.address.into(), TransitionPolicy::Pin);
    round_trip(&mut pinned, 7).await;
    proxy.reload(first.address.into(), TransitionPolicy::Close);
    // A connection receives exactly one transition policy: the one declared by the
    // reload that changed its route. This connection was pinned then, so a later
    // `Close` reload must not retroactively terminate it — pin means the
    // connection lives on its existing target until it closes naturally.
    round_trip(&mut pinned, 9).await;
    drop(pinned);
    wait_for_telemetry(&telemetry, |snapshot| snapshot.active_connections == 0).await;

    #[cfg(target_os = "linux")]
    let warmup = resource_sample();
    let stop = Arc::new(AtomicBool::new(false));
    let invalid_responses = Arc::new(AtomicUsize::new(0));
    let incomplete_responses = Arc::new(AtomicUsize::new(0));
    let client_errors = Arc::new(AtomicUsize::new(0));
    let mut tasks = Vec::new();
    for _ in 0..clients {
        let proxy = Arc::clone(&proxy);
        let stop = Arc::clone(&stop);
        let invalid_responses = Arc::clone(&invalid_responses);
        let incomplete_responses = Arc::clone(&incomplete_responses);
        let client_errors = Arc::clone(&client_errors);
        tasks.push(tokio::spawn(async move {
            while !stop.load(Ordering::Relaxed) {
                match TcpStream::connect(proxy.local_addr()).await {
                    Ok(mut stream) => {
                        let mut identity = [0_u8];
                        if stream.read_exact(&mut identity).await.is_err() {
                            incomplete_responses.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                        if !matches!(identity[0], b'A' | b'B') {
                            invalid_responses.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                        let payload = identity[0].wrapping_add(1);
                        if stream.write_all(&[payload]).await.is_err() {
                            incomplete_responses.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                        let mut echoed = [0_u8];
                        if stream.read_exact(&mut echoed).await.is_err() || echoed[0] != payload {
                            incomplete_responses.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    Err(_) => {
                        client_errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }));
    }

    let deadline = Instant::now() + duration;
    let mut use_first = false;
    while Instant::now() < deadline {
        let target = if use_first {
            first.address
        } else {
            second.address
        };
        // Storm with Pin: existing connections must complete naturally against the
        // backend they started with, so every client exchange is required to finish
        // intact no matter how often the target flips. (Close terminates in-flight
        // connections by design — that behavior is covered by
        // `close_policy_terminates_old_connections` and the pre-storm sequence —
        // so a zero-incomplete assertion is only sound under Pin.)
        proxy.reload(target.into(), TransitionPolicy::Pin);
        use_first = !use_first;
        tokio::task::yield_now().await;
    }
    stop.store(true, Ordering::Relaxed);
    for task in tasks {
        task.await.unwrap();
    }
    wait_for_telemetry(&telemetry, |snapshot| snapshot.active_connections == 0).await;

    assert_eq!(invalid_responses.load(Ordering::Relaxed), 0);
    assert_eq!(incomplete_responses.load(Ordering::Relaxed), 0);
    assert_eq!(client_errors.load(Ordering::Relaxed), 0);
    assert_eq!(telemetry.snapshot().active_connections, 0);

    #[cfg(target_os = "linux")]
    {
        let end = resource_sample();
        // A leak is growth; warmup may have sampled a transient socket, so fewer
        // descriptors at the end is fine.
        assert!(
            end.fds <= warmup.fds,
            "TCP reload storm leaked file descriptors: {} -> {}",
            warmup.fds,
            end.fds
        );
        let rss_growth = end.rss_kb.saturating_sub(warmup.rss_kb);
        assert!(
            rss_growth <= 32 * 1024,
            "TCP reload storm RSS grew by {rss_growth} KiB"
        );
    }
    proxy.shutdown().await.unwrap();
}
