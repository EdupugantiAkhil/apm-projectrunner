#![cfg(unix)]

mod browser;
mod cli;
mod host_runtime;
mod lan_preflight;
mod runtime;
mod tailscale_publication;

use std::{
    env, fmt, fs, io,
    net::{SocketAddr, ToSocketAddrs},
    path::Path,
    process::{Command, ExitCode, Stdio},
};

use cli::{CliCommand, DeploymentOptions, USAGE};
use router_config::RouterConfig;
use runtime::{DeploymentStatus, DockerRuntime, DriftState, RuntimePlan};
use switchyard_planner::{Bundle, Plan};

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("switchyard: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode, Box<dyn std::error::Error>> {
    let command = cli::parse(env::args_os().skip(1))?;
    if command == CliCommand::Help {
        print!("{USAGE}");
        return Ok(ExitCode::SUCCESS);
    }
    let workspace_root = env::current_dir()?;
    if let Some(code) = handle_daemon_command(&workspace_root, &command)? {
        return Ok(code);
    }
    if let Some(code) = handle_source_command(&workspace_root, &command)? {
        return Ok(code);
    }
    if env::var_os("SWITCHYARD_BYPASS_DAEMON").is_none() && daemon_compatible(&command) {
        let (kind, request) = daemon_request(&command);
        match switchyard_daemon::client::execute_if_running(&workspace_root, kind, &request)? {
            switchyard_daemon::client::DaemonExecution::NotRunning => {}
            switchyard_daemon::client::DaemonExecution::Completed(operation) => {
                let result = operation.result.ok_or_else(|| {
                    MessageError("daemon completed the operation without a command result".into())
                })?;
                print!("{}", result.stdout);
                eprint!("{}", result.stderr);
                if matches!(
                    command,
                    CliCommand::Status { .. } | CliCommand::Routes { .. }
                ) {
                    if let Some(routes) = switchyard_daemon::client::deployment_routes(
                        &workspace_root,
                        &operation.deployment,
                    )? {
                        print_route_versions(&routes);
                    }
                }
                let code = u8::try_from(result.exit_code).unwrap_or(1);
                return Ok(ExitCode::from(code));
            }
        }
    }
    let runtime = DockerRuntime::default();
    match command {
        CliCommand::Validate { bundle } => {
            let (_, plan) = load_and_plan(&bundle)?;
            println!(
                "deployment `{}` is valid (definition {})",
                plan.deployment, plan.definition_hash
            );
        }
        CliCommand::Plan { bundle, options } => {
            let (_, plan) = load_and_plan_options(&bundle, &options)?;
            print_plan(&workspace_root, &plan)?;
        }
        CliCommand::Up { bundle, options } => {
            let (_, plan) = load_and_plan_options(&bundle, &options)?;
            let runtime_plan = runtime_plan(&workspace_root, &plan);
            refuse_runtime_drift(&runtime.status(&runtime_plan)?)?;
            let host_runtime = host_runtime::HostRuntime::new(&workspace_root, &plan);
            let host_needs_token = host_runtime.requires_token_for_start()?;
            if (!plan.sidecars.is_empty() || host_needs_token)
                && env::var_os("SWITCHYARD_ROUTER_TOKEN").is_none()
            {
                return Err(MessageError(
                    "SWITCHYARD_ROUTER_TOKEN must be set when starting routers".into(),
                )
                .into());
            }
            for sidecar in plan.sidecars.values() {
                if let Some(parent) = workspace_root.join(&sidecar.admin_socket).parent() {
                    fs::create_dir_all(parent)?;
                }
            }
            let artifact_dir = switchyard_planner::write_plan(&workspace_root, &plan)?;
            println!("wrote {}", artifact_dir.display());
            println!("building `{}`", plan.deployment);
            runtime.up(&runtime_plan)?;
            let host = host_runtime.start().map_err(|error| {
                    MessageError(format!(
                        "{error}; Compose resources may still be running for inspection—run `switchyard down {}` or `switchyard cleanup {} --yes`",
                        bundle.display(),
                        bundle.display()
                    ))
                })?;
            println!("host gateway: {host}");
            let mdns = lan_preflight::LanRuntime::new(&workspace_root, &plan).start()?;
            print_mdns_status(&mdns);
            let tailscale =
                tailscale_publication::TailscaleRuntime::new(&workspace_root, &plan).start()?;
            print_tailscale_status(&tailscale);
            println!("deployment `{}` is healthy", plan.deployment);
        }
        CliCommand::Bind {
            bundle,
            consumer,
            group,
            transition,
        } => {
            let bundle = load_bind_base(&workspace_root, &bundle)?;
            let mut plan = plan_with_binding(&bundle, &consumer, &group)?;
            apply_binding(&workspace_root, &mut plan, &consumer, transition)?;
            switchyard_planner::write_plan(&workspace_root, &plan)?;
            println!("bound `{consumer}` to `{group}`");
        }
        CliCommand::Status {
            bundle,
            routes,
            options,
        } => {
            let (_, plan) = load_and_plan_options(&bundle, &options)?;
            let status = runtime.status(&runtime_plan(&workspace_root, &plan))?;
            print_status(&status);
            println!(
                "Host gateway: {}",
                host_runtime::HostRuntime::new(&workspace_root, &plan).status()?
            );
            print_mdns_status(&lan_preflight::LanRuntime::new(&workspace_root, &plan).status()?);
            print_tailscale_status(
                &tailscale_publication::TailscaleRuntime::new(&workspace_root, &plan).status()?,
            );
            print_source_identities(&plan);
            if routes {
                print_routes(&workspace_root, &plan)?;
            }
        }
        CliCommand::Routes { bundle } => {
            let (_, plan) = load_and_plan(&bundle)?;
            print_routes(&workspace_root, &plan)?;
        }
        CliCommand::Logs { bundle, target } => {
            let (_, plan) = load_and_plan(&bundle)?;
            let services = target
                .as_deref()
                .map(|target| log_targets(&plan, target))
                .transpose()?
                .unwrap_or_default();
            runtime.logs(&runtime_plan(&workspace_root, &plan), &services)?;
        }
        CliCommand::Open { bundle, ui } => {
            let (_, plan) = load_and_plan(&bundle)?;
            let profile = browser::load_managed_profile(
                &workspace_root,
                &plan.artifact_dir,
                &plan.deployment,
                &ui,
            )?;
            let profile_dir = browser::open_managed_profile(&workspace_root, &profile)?;
            println!(
                "opened `{ui}` through route `{}` using profile {}",
                profile.route,
                profile_dir.display()
            );
        }
        CliCommand::Down { bundle, options } => {
            let (_, plan) = load_and_plan_options(&bundle, &options)?;
            host_runtime::HostRuntime::new(&workspace_root, &plan).stop()?;
            runtime.down(&runtime_plan(&workspace_root, &plan))?;
            println!(
                "deployment `{}` stopped; volumes were preserved",
                plan.deployment
            );
        }
        CliCommand::OverlayValidate { overlay } => {
            let overlay = switchyard_planner::load_overlay(&overlay)?;
            switchyard_planner::validate_overlay(&overlay).map_err(diagnostics)?;
            println!("overlay `{}` is valid", overlay.metadata.name);
        }
        CliCommand::OverlayDiff { bundle, options } => {
            let (_, plan) = load_and_plan_options(&bundle, &options)?;
            print_overlay_diff(&workspace_root, &plan)?;
        }
        CliCommand::Cleanup { bundle, confirmed } => {
            let (_, plan) = load_and_plan(&bundle)?;
            if !confirmed {
                runtime.cleanup(&runtime_plan(&workspace_root, &plan), false)?;
                unreachable!("unconfirmed cleanup always returns an error");
            }
            host_runtime::HostRuntime::new(&workspace_root, &plan).cleanup()?;
            runtime.cleanup(&runtime_plan(&workspace_root, &plan), true)?;
            println!(
                "deployment `{}` stopped and its owned volumes were deleted",
                plan.deployment
            );
        }
        CliCommand::Help
        | CliCommand::DaemonRun
        | CliCommand::DaemonStatus
        | CliCommand::DaemonStop
        | CliCommand::OperationCancel { .. }
        | CliCommand::Gui
        | CliCommand::SourceList { .. }
        | CliCommand::SourceRegister { .. }
        | CliCommand::SourceDeregister { .. }
        | CliCommand::WorktreeCreate { .. }
        | CliCommand::WorktreeRemove { .. } => unreachable!("handled before command dispatch"),
    }
    Ok(ExitCode::SUCCESS)
}

fn print_mdns_status(status: &lan_preflight::MdnsStatus) {
    if status.is_empty() {
        println!("mDNS publication: not configured");
        return;
    }
    for publication in &status.publications {
        println!(
            "mDNS publication: {} {} -> {} ({})",
            publication.outcome, publication.name, publication.address, publication.detail
        );
    }
    for check in &status.checks {
        println!(
            "LAN check [{}] {}: {}",
            check.outcome, check.name, check.detail
        );
    }
}

fn print_tailscale_status(status: &tailscale_publication::TailscaleStatus) {
    if !status.configured {
        println!("tailnet publication: not configured");
        return;
    }
    if let Some(record) = &status.record {
        println!(
            "tailnet publication: {} via {} on ports {}",
            record.names.join(", "),
            record.addresses.join(", "),
            record
                .ports
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        );
    } else {
        println!("tailnet publication: unavailable");
    }
    for check in &status.checks {
        println!(
            "tailnet check [{}] {}: {}",
            check.outcome, check.name, check.detail
        );
    }
}

fn daemon_compatible(command: &CliCommand) -> bool {
    match command {
        CliCommand::Plan { options, .. }
        | CliCommand::Up { options, .. }
        | CliCommand::Down { options, .. }
        | CliCommand::Status { options, .. } => options == &DeploymentOptions::default(),
        CliCommand::OverlayValidate { .. } | CliCommand::OverlayDiff { .. } => false,
        _ => true,
    }
}

fn handle_source_command(
    workspace_root: &Path,
    command: &CliCommand,
) -> Result<Option<ExitCode>, Box<dyn std::error::Error>> {
    use switchyard_daemon::contract::{CreateWorktreeRequestV1, RegisterSourceRequestV1};
    let bypass = env::var_os("SWITCHYARD_BYPASS_DAEMON").is_some();
    let state = || {
        switchyard_state::StateStore::open(workspace_root.join(".switchyard/state.sqlite3"))
            .map(|value| value.0)
    };
    let manager = switchyard_sources::SourceManager::new(workspace_root);
    match command {
        CliCommand::SourceList { json } => {
            let daemon_sources = if !bypass {
                switchyard_daemon::client::sources(workspace_root)?
            } else {
                None
            };
            let sources = match daemon_sources {
                Some(sources) => sources,
                None => manager.list(&state()?)?,
            };
            print_sources(&sources, *json)?;
        }
        CliCommand::SourceRegister { name, path } => {
            let path = absolute_from(workspace_root, path);
            let request = RegisterSourceRequestV1 {
                name: name.clone(),
                path: path.clone(),
            };
            let source = if !bypass {
                switchyard_daemon::client::register_source(workspace_root, &request)?
            } else {
                None
            }
            .map_or_else(
                || {
                    let store = state()?;
                    let source = manager.register_unmanaged(&store, name, &path)?;
                    let inspection = manager.inspect(&source.path, source.requested_ref.as_deref());
                    Ok::<_, Box<dyn std::error::Error>>(
                        switchyard_sources::RegisteredSourceInspection { source, inspection },
                    )
                },
                Ok,
            )?;
            println!(
                "registered unmanaged source `{}` at {}",
                source.source.name,
                source.source.path.display()
            );
        }
        CliCommand::SourceDeregister { name } => {
            if bypass
                || switchyard_daemon::client::deregister_source(workspace_root, name)?.is_none()
            {
                manager.deregister(&state()?, name)?;
            }
            println!("deregistered source `{name}`; no files were changed");
        }
        CliCommand::WorktreeCreate {
            repository,
            r#ref,
            path,
            name,
        } => {
            let name = name.clone().unwrap_or_else(|| sanitize_source_name(r#ref));
            let path = path
                .as_ref()
                .map(|path| absolute_from(workspace_root, path));
            let request = CreateWorktreeRequestV1 {
                repository: repository.clone(),
                r#ref: r#ref.clone(),
                path: path.clone(),
                name: Some(name.clone()),
            };
            let source = if !bypass {
                switchyard_daemon::client::create_worktree(workspace_root, &request)?
            } else {
                None
            }
            .map_or_else(
                || {
                    let store = state()?;
                    let source = manager.create_worktree(
                        &store,
                        repository,
                        r#ref,
                        &name,
                        path.as_deref(),
                    )?;
                    let inspection = manager.inspect(&source.path, source.requested_ref.as_deref());
                    Ok::<_, Box<dyn std::error::Error>>(
                        switchyard_sources::RegisteredSourceInspection { source, inspection },
                    )
                },
                Ok,
            )?;
            println!(
                "created managed worktree `{}` at {}",
                source.source.name,
                source.source.path.display()
            );
        }
        CliCommand::WorktreeRemove { name, allow_dirty } => {
            let daemon_dirty = if !bypass {
                switchyard_daemon::client::remove_worktree(workspace_root, name, *allow_dirty)?
            } else {
                None
            };
            let dirty = match daemon_dirty {
                Some(dirty) => dirty,
                None => manager.remove(&state()?, name, *allow_dirty)?,
            };
            println!(
                "removed managed worktree `{name}` (staged={}, unstaged={}, untracked={})",
                dirty.staged, dirty.unstaged, dirty.untracked
            );
        }
        _ => return Ok(None),
    }
    Ok(Some(ExitCode::SUCCESS))
}

fn print_sources(
    sources: &[switchyard_sources::RegisteredSourceInspection],
    json: bool,
) -> Result<(), serde_json::Error> {
    if json {
        println!("{}", serde_json::to_string_pretty(sources)?);
        return Ok(());
    }
    println!("NAME\tKIND\tPATH\tREF\tCOMMIT\tDIRTY\tAHEAD/BEHIND");
    for entry in sources {
        let inspection = &entry.inspection;
        let commit = inspection
            .identity
            .commit
            .as_deref()
            .map(|value| &value[..value.len().min(12)])
            .unwrap_or("-");
        let reference = inspection
            .branch
            .as_deref()
            .or(inspection.identity.r#ref.as_deref())
            .unwrap_or("-");
        let dirty = inspection
            .identity
            .dirty
            .map_or("?", |value| if value { "*" } else { "-" });
        let ahead_behind = match (inspection.ahead, inspection.behind) {
            (Some(ahead), Some(behind)) => format!("{ahead}/{behind}"),
            _ => "-".into(),
        };
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            entry.source.name,
            match entry.source.kind {
                switchyard_state::RegisteredSourceKind::Managed => "managed",
                switchyard_state::RegisteredSourceKind::Unmanaged => "unmanaged",
            },
            entry.source.path.display(),
            reference,
            commit,
            dirty,
            ahead_behind
        );
    }
    Ok(())
}

fn absolute_from(root: &Path, path: &Path) -> std::path::PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        root.join(path)
    }
}

fn sanitize_source_name(reference: &str) -> String {
    reference
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn print_route_versions(routes: &switchyard_daemon::contract::DeploymentRoutesV1) {
    if routes.bindings.is_empty() {
        return;
    }
    println!("Route versions:");
    for binding in &routes.bindings {
        let version =
            |value: Option<i64>| value.map_or_else(|| "-".into(), |value| value.to_string());
        println!(
            "  {} {} desired={} current={} previous={} observed={} status={}",
            binding.router,
            binding.binding,
            version(binding.desired_version),
            version(binding.current_version),
            version(binding.previous_version),
            version(binding.observed_version),
            binding.status,
        );
    }
}

fn print_source_identities(plan: &Plan) {
    if plan.source_identities.is_empty() {
        return;
    }
    println!("Source identities (plan time):");
    for (instance, identity) in &plan.source_identities {
        println!(
            "  {} path={} repository={} ref={} commit={} dirty={}",
            instance,
            identity.path,
            identity.repository.as_deref().unwrap_or("-"),
            identity.r#ref.as_deref().unwrap_or("-"),
            identity.commit.as_deref().unwrap_or("-"),
            identity
                .dirty
                .map_or("unknown", |dirty| if dirty { "yes" } else { "no" }),
        );
    }
}

fn handle_daemon_command(
    workspace_root: &Path,
    command: &CliCommand,
) -> Result<Option<ExitCode>, Box<dyn std::error::Error>> {
    match command {
        CliCommand::DaemonRun => {
            let mut config = switchyard_daemon::DaemonConfig::new(
                workspace_root.to_owned(),
                env::current_exe()?,
            );
            if let Some(bind) = env::var_os("SWITCHYARD_DAEMON_BIND") {
                config.bind = bind.to_string_lossy().parse::<SocketAddr>()?;
            }
            if let Some(limit) = env::var_os("SWITCHYARD_DAEMON_MAX_HEAVY") {
                config.max_heavy_operations = limit.to_string_lossy().parse()?;
            }
            if let Some(path) = env::var_os("SWITCHYARD_GUI_DIST") {
                config.gui_dist = path.into();
            }
            switchyard_daemon::run_blocking(config)?;
            Ok(Some(ExitCode::SUCCESS))
        }
        CliCommand::DaemonStatus => {
            match switchyard_daemon::client::daemon_status(workspace_root)? {
                Some(status) => println!(
                    "daemon running (API {}, pid {}, active {}, heavy limit {})",
                    status.api_version,
                    status.pid,
                    status.active_operations,
                    status.max_heavy_operations
                ),
                None => println!("daemon not running"),
            }
            Ok(Some(ExitCode::SUCCESS))
        }
        CliCommand::OperationCancel { id } => {
            match switchyard_daemon::client::cancel_operation(workspace_root, id)? {
                Some(operation) => println!(
                    "operation {} ({}) is now {:?}",
                    operation.id,
                    operation.kind.segment(),
                    operation.status
                ),
                None => println!(
                    "daemon not running; operations only exist while the daemon is running"
                ),
            }
            Ok(Some(ExitCode::SUCCESS))
        }
        CliCommand::DaemonStop => {
            if switchyard_daemon::client::daemon_stop(workspace_root)? {
                println!("daemon stop requested");
            } else {
                println!("daemon not running");
            }
            Ok(Some(ExitCode::SUCCESS))
        }
        CliCommand::Gui => {
            let url = gui_url(workspace_root)?;
            println!("{url}");
            let opener = if cfg!(target_os = "macos") {
                "open"
            } else {
                "xdg-open"
            };
            let _ = Command::new(opener)
                .arg(&url)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            Ok(Some(ExitCode::SUCCESS))
        }
        _ => Ok(None),
    }
}

fn gui_url(workspace_root: &Path) -> Result<String, MessageError> {
    let discovery = switchyard_daemon::client::load_discovery(workspace_root)
        .map_err(|error| MessageError(error.to_string()))?
        .ok_or_else(|| {
            MessageError("daemon not running; start it with `switchyard daemon run`".into())
        })?;
    let port = discovery
        .address
        .to_socket_addrs()
        .map_err(|error| MessageError(format!("invalid daemon address: {error}")))?
        .next()
        .ok_or_else(|| MessageError("daemon address did not resolve".into()))?
        .port();
    Ok(format!(
        "http://127.0.0.1:{port}/gui/#token={}",
        discovery.token
    ))
}

fn daemon_request(
    command: &CliCommand,
) -> (
    switchyard_daemon::contract::CommandKind,
    switchyard_daemon::contract::CommandRequestV1,
) {
    use switchyard_daemon::contract::{CommandKind, CommandRequestV1};
    let empty = |bundle| CommandRequestV1 {
        bundle,
        consumer: None,
        group: None,
        transition: None,
        target: None,
        ui: None,
        routes: false,
        confirmed: false,
    };
    match command {
        CliCommand::Validate { bundle } => (CommandKind::Validate, empty(bundle.clone())),
        CliCommand::Plan { bundle, .. } => (CommandKind::Plan, empty(bundle.clone())),
        CliCommand::Up { bundle, .. } => (CommandKind::Apply, empty(bundle.clone())),
        CliCommand::Bind {
            bundle,
            consumer,
            group,
            transition,
        } => {
            let mut request = empty(bundle.clone());
            request.consumer = Some(consumer.clone());
            request.group = Some(group.clone());
            request.transition = transition.map(|transition| match transition {
                cli::TransitionArgument::Close => {
                    switchyard_daemon::contract::TransitionPolicyV1::Close
                }
                cli::TransitionArgument::Drain { timeout_ms } => {
                    switchyard_daemon::contract::TransitionPolicyV1::Drain { timeout_ms }
                }
                cli::TransitionArgument::Pin => {
                    switchyard_daemon::contract::TransitionPolicyV1::Pin
                }
            });
            (CommandKind::Bind, request)
        }
        CliCommand::Status { bundle, routes, .. } => {
            let mut request = empty(bundle.clone());
            request.routes = *routes;
            (CommandKind::Status, request)
        }
        CliCommand::Routes { bundle } => (CommandKind::Routes, empty(bundle.clone())),
        CliCommand::Logs { bundle, target } => {
            let mut request = empty(bundle.clone());
            request.target = target.clone();
            (CommandKind::Logs, request)
        }
        CliCommand::Open { bundle, ui } => {
            let mut request = empty(bundle.clone());
            request.ui = Some(ui.clone());
            (CommandKind::Open, request)
        }
        CliCommand::Down { bundle, .. } => (CommandKind::Down, empty(bundle.clone())),
        CliCommand::Cleanup { bundle, confirmed } => {
            let mut request = empty(bundle.clone());
            request.confirmed = *confirmed;
            (CommandKind::Cleanup, request)
        }
        CliCommand::Help
        | CliCommand::DaemonRun
        | CliCommand::DaemonStatus
        | CliCommand::DaemonStop
        | CliCommand::OperationCancel { .. }
        | CliCommand::Gui
        | CliCommand::SourceList { .. }
        | CliCommand::SourceRegister { .. }
        | CliCommand::SourceDeregister { .. }
        | CliCommand::WorktreeCreate { .. }
        | CliCommand::WorktreeRemove { .. }
        | CliCommand::OverlayValidate { .. }
        | CliCommand::OverlayDiff { .. } => unreachable!("not delegated"),
    }
}

fn load_and_plan(path: &Path) -> Result<(Bundle, Plan), Box<dyn std::error::Error>> {
    load_and_plan_options(path, &DeploymentOptions::default())
}

fn load_and_plan_options(
    path: &Path,
    options: &DeploymentOptions,
) -> Result<(Bundle, Plan), Box<dyn std::error::Error>> {
    let bundle = switchyard_planner::load_bundle(path)?;
    let options = switchyard_planner::OverlayOptions {
        overlays: options.overlays.clone(),
        variation: options.variation.clone(),
        set: options.set.iter().cloned().collect(),
    };
    let plan = switchyard_planner::plan_with_overlays(&bundle, &options).map_err(diagnostics)?;
    Ok((bundle, plan))
}

fn plan_with_binding(
    bundle: &Bundle,
    consumer: &str,
    group: &str,
) -> Result<Plan, Box<dyn std::error::Error>> {
    switchyard_planner::plan_with_binding(bundle, consumer, group)
        .map_err(diagnostics)
        .map_err(Into::into)
}

fn load_bind_base(
    workspace_root: &Path,
    bundle_path: &Path,
) -> Result<Bundle, Box<dyn std::error::Error>> {
    let mut authored = switchyard_planner::load_bundle(bundle_path)?;
    let authored_plan = switchyard_planner::plan(&authored).map_err(diagnostics)?;
    let artifact_dir = workspace_root.join(&authored_plan.artifact_dir);
    let manifest_path = artifact_dir.join("manifest.json");
    let resolved_path = artifact_dir.join("resolved-deployment.yaml");
    if !manifest_path.exists() && !resolved_path.exists() {
        return Ok(authored);
    }
    let manifest: serde_json::Value = serde_json::from_slice(&fs::read(&manifest_path)?)?;
    let applied_deployment = manifest["deployment"].as_str();
    let applied_resource_hash = manifest["resourceHash"].as_str();
    if applied_deployment != Some(authored_plan.deployment.as_str())
        || applied_resource_hash != Some(authored_plan.resource_hash.as_str())
    {
        return Err(MessageError(
            "generated bind state does not match this deployment; run status and reconcile drift"
                .into(),
        )
        .into());
    }
    let resolved = switchyard_planner::load_bundle(&resolved_path)?;
    if resolved.metadata.name != authored.metadata.name {
        return Err(MessageError("resolved deployment identity does not match".into()).into());
    }
    authored.spec.bindings = resolved.spec.bindings;
    authored.spec.routes = resolved.spec.routes;
    authored.spec.ui_routes = resolved.spec.ui_routes;
    Ok(authored)
}

fn diagnostics(diagnostics: Vec<switchyard_planner::Diagnostic>) -> MessageError {
    MessageError(
        diagnostics
            .into_iter()
            .map(|diagnostic| diagnostic.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn runtime_plan(workspace_root: &Path, plan: &Plan) -> RuntimePlan {
    RuntimePlan {
        deployment: plan.deployment.clone(),
        compose_project: plan.compose_project.clone(),
        project_directory: workspace_root.to_owned(),
        artifact_dir: workspace_root.join(&plan.artifact_dir),
        requires_router_token: !plan.sidecars.is_empty(),
        runtime_secrets: plan.runtime_secrets.clone(),
    }
}

fn print_plan(workspace_root: &Path, plan: &Plan) -> io::Result<()> {
    let manifest_path = workspace_root
        .join(&plan.artifact_dir)
        .join("manifest.json");
    let mutation = match fs::read_to_string(manifest_path) {
        Ok(current) if current == plan.manifest_json => "no generated artifact changes",
        Ok(_) => "replace generated deployment artifacts",
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            "create generated deployment artifacts"
        }
        Err(error) => return Err(error),
    };
    println!("Deployment: {}", plan.deployment);
    println!("Mutation: {mutation}");
    println!("Compose project: {}", plan.compose_project);
    println!("Artifact directory: {}", plan.artifact_dir.display());
    if plan.has_overrides {
        print_impacts(workspace_root, plan)?;
        print_origins(plan);
    }
    println!("\nGenerated Compose:\n{}", plan.compose_yaml.trim_end());
    if plan.route_configs.is_empty() {
        println!("\nRoutes: none");
    } else {
        println!("\nRoute snapshots:");
        for (consumer, config) in &plan.route_configs {
            println!("\n[{consumer}]\n{}", config.trim_end());
        }
    }
    Ok(())
}

fn print_overlay_diff(workspace_root: &Path, plan: &Plan) -> io::Result<()> {
    println!("Deployment: {}", plan.deployment);
    print_impacts(workspace_root, plan)?;
    print_origins(plan);
    Ok(())
}

fn print_impacts(workspace_root: &Path, plan: &Plan) -> io::Result<()> {
    let changes = switchyard_planner::classify_changes(workspace_root, plan)?;
    if changes.is_empty() {
        println!("Impact: none");
    } else {
        println!("Impact:");
        for change in changes {
            println!(
                "  {}: {}",
                change.service,
                format!("{:?}", change.impact).to_ascii_lowercase()
            );
        }
    }
    Ok(())
}

fn print_origins(plan: &Plan) {
    if plan.origins.is_empty() {
        return;
    }
    println!("Origins:");
    for origin in &plan.origins {
        println!(
            "  {}/{} {}={}  ← {}",
            origin.instance, origin.category, origin.key, origin.value, origin.layer
        );
        for shadowed in &origin.shadowed {
            println!(
                "    warning: shadows {} from {}",
                shadowed.value, shadowed.layer
            );
        }
    }
}

fn refuse_runtime_drift(status: &DeploymentStatus) -> Result<(), MessageError> {
    let has_active_topology = status
        .resources
        .iter()
        .any(|resource| resource.kind != runtime::ResourceKind::Volume);
    if !has_active_topology {
        return Ok(());
    }
    match status.drift {
        DriftState::NotRunning | DriftState::InSync => Ok(()),
        DriftState::Drifted | DriftState::Unknown => Err(MessageError(format!(
            "runtime drift detected for `{}`: {}; run `switchyard status` and reconcile it before up",
            status.deployment, status.detail
        ))),
    }
}

fn print_status(status: &DeploymentStatus) {
    println!("Deployment: {}", status.deployment);
    println!("Drift: {:?} ({})", status.drift, status.detail);
    if status.resources.is_empty() {
        println!("Resources: none");
        return;
    }
    println!("Resources:");
    for resource in &status.resources {
        println!(
            "  {:9} {:32} {}",
            resource.kind,
            resource.name,
            resource.state.as_deref().unwrap_or("present")
        );
    }
}

fn apply_binding(
    workspace_root: &Path,
    plan: &mut Plan,
    consumer: &str,
    transition: Option<cli::TransitionArgument>,
) -> Result<(), Box<dyn std::error::Error>> {
    let encoded = plan.route_configs.get(consumer).cloned().ok_or_else(|| {
        MessageError(format!(
            "planner did not produce a route snapshot for consumer `{consumer}`"
        ))
    })?;
    let mut config: RouterConfig = serde_json::from_str(&encoded)?;
    if let Some(transition) = transition {
        let policy = match transition {
            cli::TransitionArgument::Close => router_config::ConnectionTransitionPolicy::Close,
            cli::TransitionArgument::Drain { timeout_ms } => {
                router_config::ConnectionTransitionPolicy::Drain { timeout_ms }
            }
            cli::TransitionArgument::Pin => router_config::ConnectionTransitionPolicy::Pin,
        };
        config.spec.snapshot.transitions = router_config::ConnectionTransitionPolicies {
            http: policy.clone(),
            https: policy.clone(),
            websocket: policy.clone(),
            grpc: policy.clone(),
            tcp: policy,
        };
    }
    config.validate().map_err(|errors| {
        MessageError(
            errors
                .into_iter()
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        )
    })?;
    let sidecar = plan.sidecars.get(consumer).ok_or_else(|| {
        MessageError(format!(
            "no router sidecar exists for consumer `{consumer}`"
        ))
    })?;
    let token = env::var("SWITCHYARD_ROUTER_TOKEN")
        .map_err(|_| MessageError("SWITCHYARD_ROUTER_TOKEN must be set for bind".into()))?;
    let socket = workspace_root.join(&sidecar.admin_socket);
    let next_version = switchyard_router_admin::current_snapshot(&socket, &token)?
        .version
        .checked_add(1)
        .ok_or_else(|| MessageError("router snapshot version is exhausted".into()))?;
    config.spec.snapshot.version = next_version;
    config.spec.snapshot.id =
        router_config::RouteSnapshotId::new(format!("{consumer}-bind-{next_version}"));
    let acknowledgement = switchyard_router_admin::apply_snapshot(&socket, &token, &config)?;
    plan.route_configs
        .insert(consumer.to_owned(), serde_json::to_string_pretty(&config)?);
    println!(
        "router acknowledgement: {}",
        serde_json::to_string(&acknowledgement)?
    );
    Ok(())
}

fn print_routes(workspace_root: &Path, plan: &Plan) -> Result<(), Box<dyn std::error::Error>> {
    if plan.sidecars.is_empty() {
        println!("Routes: none");
        return Ok(());
    }
    let token = env::var("SWITCHYARD_ROUTER_TOKEN").map_err(|_| {
        MessageError("SWITCHYARD_ROUTER_TOKEN must be set to inspect routes".into())
    })?;
    for (consumer, sidecar) in &plan.sidecars {
        let routes = switchyard_router_admin::inspect_routes(
            &workspace_root.join(&sidecar.admin_socket),
            &token,
        )?;
        println!("[{consumer}]\n{}", serde_json::to_string_pretty(&routes)?);
    }
    Ok(())
}

fn log_targets(plan: &Plan, target: &str) -> Result<Vec<String>, MessageError> {
    let (instance, component) = target
        .split_once('/')
        .map_or((target, None), |(instance, component)| {
            (instance, Some(component))
        });
    let manifest: serde_json::Value = serde_json::from_str(&plan.manifest_json)
        .map_err(|error| MessageError(error.to_string()))?;
    let mut services = Vec::new();
    for entry in manifest["services"].as_array().into_iter().flatten() {
        if entry["instance"].as_str() != Some(instance)
            || component.is_some() && entry["component"].as_str() != component
        {
            continue;
        }
        if let Some(service) = entry["service"].as_str() {
            services.push(service.to_owned());
        }
        if component.is_none() {
            if let Some(sidecar) = entry["sidecar"].as_str() {
                services.push(sidecar.to_owned());
            }
        }
    }
    services.sort();
    services.dedup();
    if services.is_empty() {
        return Err(MessageError(format!(
            "no generated service matches log target `{target}`"
        )));
    }
    Ok(services)
}

#[derive(Debug)]
struct MessageError(String);

impl fmt::Display for MessageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for MessageError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_target_resolves_all_instance_services_from_manifest() {
        let plan: Plan = serde_json::from_value(serde_json::json!({
            "deployment": "demo",
            "definitionHash": "definition",
            "resourceHash": "resources",
            "composeProject": "sy--demo",
            "artifactDir": ".switchyard/generated/demo",
            "composeYaml": "",
            "resolvedDeploymentYaml": "",
            "manifestJson": "{\"services\":[{\"instance\":\"backend\",\"component\":\"api\",\"service\":\"demo--backend--api--app\",\"sidecar\":\"demo--backend--api--router\"}]}",
            "routeConfigs": {},
            "sidecars": {}
        }))
        .unwrap();
        assert_eq!(
            log_targets(&plan, "backend").unwrap(),
            vec![
                "demo--backend--api--app".to_owned(),
                "demo--backend--api--router".to_owned()
            ]
        );
    }

    #[test]
    fn drift_blocks_up_without_mutating_it() {
        let status = DeploymentStatus {
            deployment: "demo".into(),
            drift: DriftState::Drifted,
            detail: "hash mismatch".into(),
            resources: vec![runtime::OwnedResource {
                kind: runtime::ResourceKind::Container,
                id: "id".into(),
                name: "demo".into(),
                labels: Default::default(),
                state: None,
            }],
        };
        assert!(refuse_runtime_drift(&status).is_err());
    }

    #[test]
    fn gui_without_daemon_returns_actionable_error() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(
            gui_url(temp.path()).unwrap_err().to_string(),
            "daemon not running; start it with `switchyard daemon run`"
        );
    }
}
