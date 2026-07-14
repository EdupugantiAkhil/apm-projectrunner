use std::{env, path::PathBuf, process::ExitCode};

use switchyard_router::{AdminOptions, RouterProcess};

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
    let mut arguments = env::args_os().skip(1);
    let config_path = arguments
        .next()
        .ok_or("usage: switchyard-router <config.json> <admin.socket>")?;
    let socket_path = arguments
        .next()
        .ok_or("usage: switchyard-router <config.json> <admin.socket>")?;
    if arguments.next().is_some() {
        return Err("usage: switchyard-router <config.json> <admin.socket>".into());
    }
    let token =
        env::var("SWITCHYARD_ROUTER_TOKEN").map_err(|_| "SWITCHYARD_ROUTER_TOKEN must be set")?;
    let bytes = tokio::fs::read(config_path).await?;
    let config = serde_json::from_slice(&bytes)?;
    let process = RouterProcess::start(
        config,
        AdminOptions {
            socket_path: PathBuf::from(socket_path),
            token,
        },
    )
    .await?;

    let shutdown = process.shutdown_handle();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            shutdown.request();
        }
    });
    process.wait().await?;
    Ok(())
}
