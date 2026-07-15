use std::{ffi::OsString, fmt, path::PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CliCommand {
    BundleExport {
        deployment: PathBuf,
        overlays: Vec<PathBuf>,
        output: Option<PathBuf>,
    },
    BundleImport {
        bundle: PathBuf,
        into: PathBuf,
        force: bool,
    },
    Diagnostics {
        deployment: PathBuf,
        output: Option<PathBuf>,
    },
    Validate {
        bundle: PathBuf,
    },
    Plan {
        bundle: PathBuf,
        options: DeploymentOptions,
    },
    Up {
        bundle: PathBuf,
        options: DeploymentOptions,
    },
    Bind {
        bundle: PathBuf,
        consumer: String,
        group: String,
        transition: Option<TransitionArgument>,
    },
    Status {
        bundle: PathBuf,
        routes: bool,
        options: DeploymentOptions,
    },
    Routes {
        bundle: PathBuf,
    },
    Logs {
        bundle: PathBuf,
        target: Option<String>,
    },
    Open {
        bundle: PathBuf,
        ui: String,
    },
    Down {
        bundle: PathBuf,
        options: DeploymentOptions,
    },
    OverlayValidate {
        overlay: PathBuf,
    },
    OverlayDiff {
        bundle: PathBuf,
        options: DeploymentOptions,
    },
    Cleanup {
        bundle: PathBuf,
        confirmed: bool,
    },
    DaemonRun,
    DaemonStatus,
    DaemonStop,
    OperationCancel {
        id: String,
    },
    Gui,
    SourceList {
        json: bool,
    },
    SourceRegister {
        name: String,
        path: PathBuf,
    },
    SourceDeregister {
        name: String,
    },
    WorktreeCreate {
        repository: String,
        r#ref: String,
        path: Option<PathBuf>,
        name: Option<String>,
    },
    WorktreeRemove {
        name: String,
        allow_dirty: bool,
    },
    Help,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeploymentOptions {
    pub overlays: Vec<PathBuf>,
    pub variation: Option<String>,
    pub set: Vec<(String, String)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransitionArgument {
    Close,
    Drain { timeout_ms: u64 },
    Pin,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UsageError(String);

impl fmt::Display for UsageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for UsageError {}

pub const USAGE: &str = "\
Usage:
  switchyard validate <deployment.yaml>
  switchyard plan <deployment.yaml> [--with <overlay.yaml>]... [--variation <name>] [--set KEY=VALUE]...
  switchyard up <deployment.yaml> [--with <overlay.yaml>]... [--variation <name>] [--set KEY=VALUE]...
  switchyard bundle export <deployment.yaml> [--with <overlay.yaml>]... [--output <file>]
  switchyard bundle import <bundle-file> --into <directory> [--force]
  switchyard diagnostics <deployment.yaml> [--output <path>]
  switchyard overlay validate <overlay.yaml>
  switchyard overlay diff <deployment.yaml> --with <overlay.yaml> [--with <overlay.yaml>]... [--variation <name>] [--set KEY=VALUE]...
  switchyard bind <deployment.yaml> <consumer> <group> [--transition close|drain|pin] [--drain-timeout-ms <ms>]
  switchyard status <deployment.yaml> [--routes] [--with <overlay.yaml>]... [--variation <name>] [--set KEY=VALUE]...
  switchyard routes <deployment.yaml>
  switchyard logs <deployment.yaml> [instance[/service]]
  switchyard open <deployment.yaml> <ui>
  switchyard down <deployment.yaml> [--with <overlay.yaml>]... [--variation <name>] [--set KEY=VALUE]...
  switchyard cleanup <deployment.yaml> --yes
  switchyard daemon run
  switchyard daemon status
  switchyard daemon stop
  switchyard operation cancel <operation-id>
  switchyard gui
  switchyard source list [--json]
  switchyard source register <name> <path>
  switchyard source deregister <name>
  switchyard worktree create <repository-source> <ref> [--path <path>] [--name <name>]
  switchyard worktree remove <name> [--allow-dirty]
";

pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<CliCommand, UsageError> {
    let arguments = arguments
        .into_iter()
        .map(|argument| {
            argument
                .into_string()
                .map_err(|_| UsageError("arguments must be valid UTF-8".into()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let Some(command) = arguments.first().map(String::as_str) else {
        return Ok(CliCommand::Help);
    };
    if matches!(command, "help" | "-h" | "--help") {
        return Ok(CliCommand::Help);
    }
    let rest = &arguments[1..];
    let bundle = || {
        rest.first()
            .map(PathBuf::from)
            .ok_or_else(|| UsageError(format!("{command} requires a deployment YAML path")))
    };
    match command {
        "bundle" if rest.len() >= 2 && rest[0] == "export" => parse_bundle_export(rest),
        "bundle" if rest.len() >= 3 && rest[0] == "import" => parse_bundle_import(rest),
        "diagnostics" if !rest.is_empty() => parse_diagnostics(rest),
        "validate" if rest.len() == 1 => Ok(CliCommand::Validate { bundle: bundle()? }),
        "plan" if !rest.is_empty() => {
            let (bundle, options, _) = parse_deployment_options(rest, false)?;
            Ok(CliCommand::Plan { bundle, options })
        }
        "up" if !rest.is_empty() => {
            let (bundle, options, _) = parse_deployment_options(rest, false)?;
            Ok(CliCommand::Up { bundle, options })
        }
        "bind" if rest.len() >= 3 => parse_bind(rest),
        "status" if !rest.is_empty() => {
            let (bundle, options, routes) = parse_deployment_options(rest, true)?;
            Ok(CliCommand::Status {
                bundle,
                routes,
                options,
            })
        }
        "routes" if rest.len() == 1 => Ok(CliCommand::Routes { bundle: bundle()? }),
        "logs" if (1..=2).contains(&rest.len()) => Ok(CliCommand::Logs {
            bundle: bundle()?,
            target: rest.get(1).cloned(),
        }),
        "open" if rest.len() == 2 => Ok(CliCommand::Open {
            bundle: bundle()?,
            ui: rest[1].clone(),
        }),
        "down" if !rest.is_empty() => {
            let (bundle, options, _) = parse_deployment_options(rest, false)?;
            Ok(CliCommand::Down { bundle, options })
        }
        "overlay" if rest.len() == 2 && rest[0] == "validate" => Ok(CliCommand::OverlayValidate {
            overlay: PathBuf::from(&rest[1]),
        }),
        "overlay" if rest.len() >= 4 && rest[0] == "diff" => {
            let (bundle, options, _) = parse_deployment_options(&rest[1..], false)?;
            if options.overlays.is_empty() {
                return Err(UsageError(
                    "overlay diff requires at least one --with overlay".into(),
                ));
            }
            Ok(CliCommand::OverlayDiff { bundle, options })
        }
        "cleanup" if rest.len() == 1 => Ok(CliCommand::Cleanup {
            bundle: bundle()?,
            confirmed: false,
        }),
        "cleanup" if rest.len() == 2 && rest[1] == "--yes" => Ok(CliCommand::Cleanup {
            bundle: bundle()?,
            confirmed: true,
        }),
        "operation" if rest.len() == 2 && rest[0] == "cancel" => Ok(CliCommand::OperationCancel {
            id: rest[1].clone(),
        }),
        "daemon" if rest == ["run"] => Ok(CliCommand::DaemonRun),
        "daemon" if rest == ["status"] => Ok(CliCommand::DaemonStatus),
        "daemon" if rest == ["stop"] => Ok(CliCommand::DaemonStop),
        "gui" if rest.is_empty() => Ok(CliCommand::Gui),
        "source" if rest == ["list"] => Ok(CliCommand::SourceList { json: false }),
        "source" if rest == ["list", "--json"] => Ok(CliCommand::SourceList { json: true }),
        "source" if rest.len() == 3 && rest[0] == "register" => Ok(CliCommand::SourceRegister {
            name: rest[1].clone(),
            path: PathBuf::from(&rest[2]),
        }),
        "source" if rest.len() == 2 && rest[0] == "deregister" => {
            Ok(CliCommand::SourceDeregister {
                name: rest[1].clone(),
            })
        }
        "worktree" if rest.len() >= 3 && rest[0] == "create" => parse_worktree_create(rest),
        "worktree" if rest.len() == 2 && rest[0] == "remove" => Ok(CliCommand::WorktreeRemove {
            name: rest[1].clone(),
            allow_dirty: false,
        }),
        "worktree" if rest.len() == 3 && rest[0] == "remove" && rest[2] == "--allow-dirty" => {
            Ok(CliCommand::WorktreeRemove {
                name: rest[1].clone(),
                allow_dirty: true,
            })
        }
        _ => Err(UsageError(format!(
            "invalid {command} arguments\n\n{USAGE}"
        ))),
    }
}

fn parse_diagnostics(rest: &[String]) -> Result<CliCommand, UsageError> {
    let deployment = PathBuf::from(&rest[0]);
    let output = match rest {
        [_] => None,
        [_, flag, path] if flag == "--output" => Some(PathBuf::from(path)),
        _ => {
            return Err(UsageError(format!(
                "invalid diagnostics arguments\n\n{USAGE}"
            )));
        }
    };
    Ok(CliCommand::Diagnostics { deployment, output })
}

fn parse_bundle_export(rest: &[String]) -> Result<CliCommand, UsageError> {
    let deployment = PathBuf::from(&rest[1]);
    let mut overlays = Vec::new();
    let mut output = None;
    let mut index = 2;
    while index < rest.len() {
        match rest[index].as_str() {
            "--with" if index + 1 < rest.len() => {
                overlays.push(PathBuf::from(&rest[index + 1]));
                index += 2;
            }
            "--output" if index + 1 < rest.len() && output.is_none() => {
                output = Some(PathBuf::from(&rest[index + 1]));
                index += 2;
            }
            _ => {
                return Err(UsageError(format!(
                    "invalid bundle export arguments\n\n{USAGE}"
                )));
            }
        }
    }
    Ok(CliCommand::BundleExport {
        deployment,
        overlays,
        output,
    })
}

fn parse_bundle_import(rest: &[String]) -> Result<CliCommand, UsageError> {
    let bundle = PathBuf::from(&rest[1]);
    let mut into = None;
    let mut force = false;
    let mut index = 2;
    while index < rest.len() {
        match rest[index].as_str() {
            "--into" if index + 1 < rest.len() && into.is_none() => {
                into = Some(PathBuf::from(&rest[index + 1]));
                index += 2;
            }
            "--force" if !force => {
                force = true;
                index += 1;
            }
            _ => {
                return Err(UsageError(format!(
                    "invalid bundle import arguments\n\n{USAGE}"
                )));
            }
        }
    }
    let into = into.ok_or_else(|| UsageError("bundle import requires --into".into()))?;
    Ok(CliCommand::BundleImport {
        bundle,
        into,
        force,
    })
}

fn parse_deployment_options(
    rest: &[String],
    allow_routes: bool,
) -> Result<(PathBuf, DeploymentOptions, bool), UsageError> {
    let bundle = rest
        .first()
        .map(PathBuf::from)
        .ok_or_else(|| UsageError("deployment YAML path is required".into()))?;
    let mut options = DeploymentOptions::default();
    let mut routes = false;
    let mut index = 1;
    while index < rest.len() {
        match rest[index].as_str() {
            "--with" if index + 1 < rest.len() => {
                options.overlays.push(PathBuf::from(&rest[index + 1]));
                index += 2;
            }
            "--variation" if index + 1 < rest.len() && options.variation.is_none() => {
                options.variation = Some(rest[index + 1].clone());
                index += 2;
            }
            "--set" if index + 1 < rest.len() => {
                let Some((key, value)) = rest[index + 1].split_once('=') else {
                    return Err(UsageError("--set requires KEY=VALUE".into()));
                };
                options.set.push((key.into(), value.into()));
                index += 2;
            }
            "--routes" if allow_routes && !routes => {
                routes = true;
                index += 1;
            }
            _ => return Err(UsageError(format!("invalid deployment options\n\n{USAGE}"))),
        }
    }
    Ok((bundle, options, routes))
}

fn parse_worktree_create(rest: &[String]) -> Result<CliCommand, UsageError> {
    let mut path = None;
    let mut name = None;
    let mut index = 3;
    while index < rest.len() {
        match rest[index].as_str() {
            "--path" if index + 1 < rest.len() => {
                path = Some(PathBuf::from(&rest[index + 1]));
                index += 2;
            }
            "--name" if index + 1 < rest.len() => {
                name = Some(rest[index + 1].clone());
                index += 2;
            }
            _ => {
                return Err(UsageError(format!(
                    "invalid worktree create arguments\n\n{USAGE}"
                )));
            }
        }
    }
    Ok(CliCommand::WorktreeCreate {
        repository: rest[1].clone(),
        r#ref: rest[2].clone(),
        path,
        name,
    })
}

fn parse_bind(rest: &[String]) -> Result<CliCommand, UsageError> {
    let mut strategy = None;
    let mut timeout = None;
    let mut index = 3;
    while index < rest.len() {
        match rest[index].as_str() {
            "--transition" if index + 1 < rest.len() => {
                strategy = Some(rest[index + 1].as_str());
                index += 2;
            }
            "--drain-timeout-ms" if index + 1 < rest.len() => {
                timeout = Some(rest[index + 1].parse::<u64>().map_err(|_| {
                    UsageError("--drain-timeout-ms must be an unsigned integer".into())
                })?);
                index += 2;
            }
            _ => return Err(UsageError(format!("invalid bind arguments\n\n{USAGE}"))),
        }
    }
    let transition = match strategy {
        None if timeout.is_none() => None,
        Some("close") if timeout.is_none() => Some(TransitionArgument::Close),
        Some("pin") if timeout.is_none() => Some(TransitionArgument::Pin),
        Some("drain") => Some(TransitionArgument::Drain {
            timeout_ms: timeout.unwrap_or(30_000),
        }),
        None => {
            return Err(UsageError(
                "--drain-timeout-ms requires --transition drain".into(),
            ));
        }
        Some(_) => {
            return Err(UsageError(
                "--transition must be close, drain, or pin".into(),
            ));
        }
    };
    Ok(CliCommand::Bind {
        bundle: PathBuf::from(&rest[0]),
        consumer: rest[1].clone(),
        group: rest[2].clone(),
        transition,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_bind_without_optional_ambiguity() {
        assert_eq!(
            parse(args(&["bind", "demo.yaml", "backend", "ai-feature"])).unwrap(),
            CliCommand::Bind {
                bundle: "demo.yaml".into(),
                consumer: "backend".into(),
                group: "ai-feature".into(),
                transition: None,
            }
        );
    }

    #[test]
    fn parses_bundle_commands() {
        assert_eq!(
            parse(args(&[
                "bundle",
                "export",
                "demo.yaml",
                "--with",
                "overlay.yaml",
                "--output",
                "demo.switchyard-bundle.json"
            ]))
            .unwrap(),
            CliCommand::BundleExport {
                deployment: "demo.yaml".into(),
                overlays: vec!["overlay.yaml".into()],
                output: Some("demo.switchyard-bundle.json".into()),
            }
        );
        assert_eq!(
            parse(args(&[
                "bundle",
                "import",
                "demo.switchyard-bundle.json",
                "--into",
                "imported",
                "--force"
            ]))
            .unwrap(),
            CliCommand::BundleImport {
                bundle: "demo.switchyard-bundle.json".into(),
                into: "imported".into(),
                force: true,
            }
        );
        assert!(parse(args(&["bundle", "import", "demo.switchyard-bundle.json"])).is_err());
    }

    #[test]
    fn parses_diagnostics_output() {
        assert_eq!(
            parse(args(&[
                "diagnostics",
                "demo.yaml",
                "--output",
                "report.json"
            ]))
            .unwrap(),
            CliCommand::Diagnostics {
                deployment: "demo.yaml".into(),
                output: Some("report.json".into()),
            }
        );
        assert!(parse(args(&["diagnostics", "demo.yaml", "--output"])).is_err());
    }

    #[test]
    fn parses_drain_policy() {
        assert!(matches!(
            parse(args(&[
                "bind",
                "demo.yaml",
                "backend",
                "base",
                "--transition",
                "drain",
                "--drain-timeout-ms",
                "2500"
            ]))
            .unwrap(),
            CliCommand::Bind {
                transition: Some(TransitionArgument::Drain { timeout_ms: 2500 }),
                ..
            }
        ));
    }

    #[test]
    fn cleanup_is_not_confirmed_by_default() {
        assert_eq!(
            parse(args(&["cleanup", "demo.yaml"])).unwrap(),
            CliCommand::Cleanup {
                bundle: "demo.yaml".into(),
                confirmed: false,
            }
        );
    }

    #[test]
    fn rejects_volume_deletion_through_down() {
        assert!(parse(args(&["down", "demo.yaml", "--volumes"])).is_err());
    }

    #[test]
    fn parses_managed_profile_open() {
        assert_eq!(
            parse(args(&["open", "demo.yaml", "ui-1"])).unwrap(),
            CliCommand::Open {
                bundle: "demo.yaml".into(),
                ui: "ui-1".into(),
            }
        );
    }

    #[test]
    fn parses_source_and_worktree_commands() {
        assert_eq!(
            parse(args(&["source", "list", "--json"])).unwrap(),
            CliCommand::SourceList { json: true }
        );
        assert_eq!(
            parse(args(&["source", "register", "repo", "/code/repo"])).unwrap(),
            CliCommand::SourceRegister {
                name: "repo".into(),
                path: "/code/repo".into()
            }
        );
        assert_eq!(
            parse(args(&[
                "worktree",
                "create",
                "repo",
                "feature/x",
                "--name",
                "feature-x",
                "--path",
                ".switchyard/worktrees/x"
            ]))
            .unwrap(),
            CliCommand::WorktreeCreate {
                repository: "repo".into(),
                r#ref: "feature/x".into(),
                path: Some(".switchyard/worktrees/x".into()),
                name: Some("feature-x".into())
            }
        );
        assert_eq!(
            parse(args(&["worktree", "remove", "feature-x", "--allow-dirty"])).unwrap(),
            CliCommand::WorktreeRemove {
                name: "feature-x".into(),
                allow_dirty: true
            }
        );
    }

    #[test]
    fn parses_operation_cancel() {
        assert_eq!(
            parse(args(&["operation", "cancel", "op-42"])).unwrap(),
            CliCommand::OperationCancel { id: "op-42".into() }
        );
        assert!(parse(args(&["operation", "cancel"])).is_err());
        assert!(parse(args(&["operation", "cancel", "op-42", "extra"])).is_err());
    }

    #[test]
    fn parses_gui_without_arguments() {
        assert_eq!(parse(args(&["gui"])).unwrap(), CliCommand::Gui);
        assert!(parse(args(&["gui", "extra"])).is_err());
    }

    #[test]
    fn parses_overlay_and_variation_options_in_order() {
        assert_eq!(
            parse(args(&[
                "plan",
                "demo.yaml",
                "--with",
                "one.yaml",
                "--with",
                "two.yaml",
                "--variation",
                "mongo",
                "--set",
                "LOG_LEVEL=trace"
            ]))
            .unwrap(),
            CliCommand::Plan {
                bundle: "demo.yaml".into(),
                options: DeploymentOptions {
                    overlays: vec!["one.yaml".into(), "two.yaml".into()],
                    variation: Some("mongo".into()),
                    set: vec![("LOG_LEVEL".into(), "trace".into())],
                },
            }
        );
        assert!(matches!(
            parse(args(&["overlay", "validate", "one.yaml"])).unwrap(),
            CliCommand::OverlayValidate { .. }
        ));
        assert!(parse(args(&["overlay", "diff", "demo.yaml"])).is_err());
    }
}
