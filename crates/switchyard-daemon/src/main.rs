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
    if let Some(argument) = env::args_os().nth(1) {
        if argument == "--version" || argument == "-V" {
            println!("switchyard-daemon {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        if argument == "--help" || argument == "-h" {
            println!("usage: switchyard-daemon [--help|--version]");
            return Ok(());
        }
        return Err("usage: switchyard-daemon [--help|--version]".into());
    }
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
    } else if let Some(path) = installed_gui_dist() {
        config.gui_dist = path;
    }
    run_blocking(config)?;
    Ok(())
}

fn installed_gui_dist() -> Option<PathBuf> {
    let executable = env::current_exe().ok()?;
    let prefix = executable.parent()?.parent()?;
    let candidate = prefix.join("share/switchyard/web");
    candidate.is_dir().then_some(candidate)
}
