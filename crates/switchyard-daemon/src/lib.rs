//! Long-running, loopback-only Switchyard control plane and versioned API.

#![cfg(unix)]

pub mod client;
pub mod contract;
pub mod device;
pub mod server;

pub use server::{DaemonConfig, DaemonError, RunningDaemon};

/// Runs the real daemon until SIGINT, SIGTERM, or an authenticated stop request.
pub fn run_blocking(config: DaemonConfig) -> Result<(), DaemonError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(DaemonError::Io)?;
    runtime.block_on(async move {
        let daemon = server::start(config).await?;
        let shutdown = daemon.shutdown.clone();
        tokio::spawn(async move {
            #[cfg(unix)]
            {
                let mut terminate =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                        .expect("SIGTERM handler installs");
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {},
                    _ = terminate.recv() => {},
                }
            }
            shutdown.send_replace(true);
        });
        daemon
            .task
            .await
            .map_err(|error| DaemonError::Message(error.to_string()))?
    })
}
