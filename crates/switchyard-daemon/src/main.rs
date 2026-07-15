#![cfg(unix)]

use std::{env, net::SocketAddr, path::PathBuf, process::ExitCode};

use switchyard_daemon::{DaemonConfig, run_blocking};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("switchyard-daemon: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let project_root = env::current_dir()?;
    let cli_program = env::var_os("SWITCHYARD_CLI")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("switchyard"));
    let mut config = DaemonConfig::new(project_root, cli_program);
    if let Some(bind) = env::var_os("SWITCHYARD_DAEMON_BIND") {
        config.bind = bind.to_string_lossy().parse::<SocketAddr>()?;
    }
    if let Some(limit) = env::var_os("SWITCHYARD_DAEMON_MAX_HEAVY") {
        config.max_heavy_operations = limit.to_string_lossy().parse()?;
    }
    if let Some(path) = env::var_os("SWITCHYARD_GUI_DIST") {
        config.gui_dist = PathBuf::from(path);
    }
    run_blocking(config)?;
    Ok(())
}
