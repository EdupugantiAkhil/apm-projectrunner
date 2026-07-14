use std::{env, path::PathBuf, process::ExitCode};

use switchyard_router::{
    AdminOptions, RouterProcess,
    host_gateway::{
        cleanup_certificates, cleanup_proxy_credentials, ensure_certificates,
        ensure_proxy_credentials, preflight, trust_guidance,
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
    "usage:\n  switchyard-router [sidecar] <config.json> <admin.socket>\n  switchyard-router host <config.json> <admin.socket>\n  switchyard-router certificates trust|cleanup <config.json>"
}
