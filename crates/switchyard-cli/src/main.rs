#![cfg(unix)]

mod admin;
mod browser;
mod cli;
mod host_runtime;
mod runtime;

use std::{env, fmt, fs, io, path::Path, process::ExitCode};

use cli::{CliCommand, USAGE};
use router_config::RouterConfig;
use runtime::{DeploymentStatus, DockerRuntime, DriftState, RuntimePlan};
use switchyard_planner::{Bundle, Plan};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("switchyard: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let command = cli::parse(env::args_os().skip(1))?;
    if command == CliCommand::Help {
        print!("{USAGE}");
        return Ok(());
    }
    let workspace_root = env::current_dir()?;
    let runtime = DockerRuntime::default();
    match command {
        CliCommand::Validate { bundle } => {
            let (_, plan) = load_and_plan(&bundle)?;
            println!(
                "deployment `{}` is valid (definition {})",
                plan.deployment, plan.definition_hash
            );
        }
        CliCommand::Plan { bundle } => {
            let (_, plan) = load_and_plan(&bundle)?;
            print_plan(&workspace_root, &plan)?;
        }
        CliCommand::Up { bundle } => {
            let (_, plan) = load_and_plan(&bundle)?;
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
            println!("deployment `{}` is healthy", plan.deployment);
        }
        CliCommand::Bind {
            bundle,
            consumer,
            group,
        } => {
            let bundle = load_bind_base(&workspace_root, &bundle)?;
            let mut plan = plan_with_binding(&bundle, &consumer, &group)?;
            apply_binding(&workspace_root, &mut plan, &consumer)?;
            switchyard_planner::write_plan(&workspace_root, &plan)?;
            println!("bound `{consumer}` to `{group}`");
        }
        CliCommand::Status { bundle, routes } => {
            let (_, plan) = load_and_plan(&bundle)?;
            let status = runtime.status(&runtime_plan(&workspace_root, &plan))?;
            print_status(&status);
            println!(
                "Host gateway: {}",
                host_runtime::HostRuntime::new(&workspace_root, &plan).status()?
            );
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
        CliCommand::Down { bundle } => {
            let (_, plan) = load_and_plan(&bundle)?;
            host_runtime::HostRuntime::new(&workspace_root, &plan).stop()?;
            runtime.down(&runtime_plan(&workspace_root, &plan))?;
            println!(
                "deployment `{}` stopped; volumes were preserved",
                plan.deployment
            );
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
        CliCommand::Help => unreachable!("handled before command dispatch"),
    }
    Ok(())
}

fn load_and_plan(path: &Path) -> Result<(Bundle, Plan), Box<dyn std::error::Error>> {
    let bundle = switchyard_planner::load_bundle(path)?;
    let plan = switchyard_planner::plan(&bundle).map_err(diagnostics)?;
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
) -> Result<(), Box<dyn std::error::Error>> {
    let encoded = plan.route_configs.get(consumer).cloned().ok_or_else(|| {
        MessageError(format!(
            "planner did not produce a route snapshot for consumer `{consumer}`"
        ))
    })?;
    let mut config: RouterConfig = serde_json::from_str(&encoded)?;
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
    let next_version = admin::current_version(&socket, &token)?
        .checked_add(1)
        .ok_or_else(|| MessageError("router snapshot version is exhausted".into()))?;
    config.spec.snapshot.version = next_version;
    config.spec.snapshot.id =
        router_config::RouteSnapshotId::new(format!("{consumer}-bind-{next_version}"));
    let acknowledgement = admin::apply_snapshot(&socket, &token, &config)?;
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
        let routes = admin::inspect_routes(&workspace_root.join(&sidecar.admin_socket), &token)?;
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
}
