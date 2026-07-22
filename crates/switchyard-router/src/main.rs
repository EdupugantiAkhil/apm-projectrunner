use std::{
    env,
    io::{self, Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::ExitCode,
    time::Duration,
};

const MAX_ADMIN_FRAME_BYTES: u64 = 1024 * 1024;

use switchyard_router::{
    AdminOptions, RouterProcess,
    host_gateway::{
        cleanup_certificates, cleanup_proxy_credentials, ensure_certificates,
        ensure_proxy_credentials, exposure_summary, preflight, trust_guidance,
    },
};

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("switchyard-router: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = env::args_os().skip(1).collect::<Vec<_>>();
    if arguments.len() == 2 && arguments[0] == "admin-client" {
        run_admin_client(Path::new(&arguments[1]))?;
        return Ok(());
    }
    if arguments.len() == 3 && arguments[0] == "certificates" && arguments[1] == "trust" {
        let config = read_config(&arguments[2]).await?;
        println!("{}", trust_guidance(&config));
        return Ok(());
    }
    if arguments.len() == 3 && arguments[0] == "certificates" && arguments[1] == "cleanup" {
        let config = read_config(&arguments[2]).await?;
        for path in cleanup_certificates(&config)? {
            println!("removed {}", path.display());
        }
        for path in cleanup_proxy_credentials(&config)? {
            println!("removed {}", path.display());
        }
        return Ok(());
    }
    let (host_mode, config_path, socket_path) = match arguments.as_slice() {
        [mode, config, socket] if mode == "host" => (true, config, socket),
        [mode, config, socket] if mode == "sidecar" => (false, config, socket),
        // Backwards-compatible Phase 1/2 sidecar invocation.
        [config, socket] => (false, config, socket),
        _ => return Err(usage().into()),
    };
    // Validate authentication before host preflight can create or renew any
    // managed certificate or proxy credential. Certificate maintenance commands
    // return above and intentionally remain tokenless.
    let token =
        env::var("SWITCHYARD_ROUTER_TOKEN").map_err(|_| "SWITCHYARD_ROUTER_TOKEN must be set")?;
    let config = read_config(config_path).await?;
    if host_mode {
        preflight(&config)?;
        let exposure = exposure_summary(&config)?;
        if exposure.mode == router_config::GatewayExposureMode::Lan {
            eprintln!(
                "{}",
                serde_json::json!({
                    "event": "lan_exposure_warning",
                    "level": "warning",
                    "mode": "lan",
                    "exposedAddresses": exposure.exposed_addresses,
                })
            );
        }
        let report = ensure_certificates(&config)?;
        for path in ensure_proxy_credentials(&config)? {
            eprintln!("generated managed-profile credential {}", path.display());
        }
        for path in report.generated {
            eprintln!("generated local certificate {}", path.display());
        }
        for path in report.renewed {
            eprintln!("renewed local certificate {}", path.display());
        }
    }
    let process = RouterProcess::start(
        config,
        AdminOptions {
            socket_path: PathBuf::from(socket_path),
            token,
        },
    )
    .await?;

    let shutdown = process.shutdown_handle();
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::spawn(async move {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
        shutdown.request();
    });
    process.wait().await?;
    Ok(())
}

async fn read_config(
    path: &std::ffi::OsStr,
) -> Result<router_config::RouterConfig, Box<dyn std::error::Error>> {
    let bytes = tokio::fs::read(path).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn usage() -> &'static str {
    "usage:\n  switchyard-router [sidecar] <config.json> <admin.socket>\n  switchyard-router host <config.json> <admin.socket>\n  switchyard-router admin-client <admin.socket>\n  switchyard-router certificates trust|cleanup <config.json>"
}

fn run_admin_client(socket: &Path) -> io::Result<()> {
    let mut request = Vec::new();
    io::stdin()
        .lock()
        .take(MAX_ADMIN_FRAME_BYTES + 2)
        .read_to_end(&mut request)?;
    if request.len() > MAX_ADMIN_FRAME_BYTES as usize + 1 || !request.ends_with(b"\n") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "administration request must be one newline-terminated frame of at most 1 MiB",
        ));
    }

    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    stream.write_all(&request)?;
    stream.shutdown(std::net::Shutdown::Write)?;

    let mut response = Vec::new();
    stream
        .take(MAX_ADMIN_FRAME_BYTES + 2)
        .read_to_end(&mut response)?;
    if response.len() > MAX_ADMIN_FRAME_BYTES as usize + 1 || !response.ends_with(b"\n") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "administration response must be one newline-terminated frame of at most 1 MiB",
        ));
    }
    io::stdout().lock().write_all(&response)
}
