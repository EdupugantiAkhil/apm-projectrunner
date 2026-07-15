use std::{ffi::OsString, fmt, path::PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CliCommand {
    Validate {
        bundle: PathBuf,
    },
    Plan {
        bundle: PathBuf,
    },
    Up {
        bundle: PathBuf,
    },
    Bind {
        bundle: PathBuf,
        consumer: String,
        group: String,
    },
    Status {
        bundle: PathBuf,
        routes: bool,
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
    },
    Cleanup {
        bundle: PathBuf,
        confirmed: bool,
    },
    DaemonRun,
    DaemonStatus,
    DaemonStop,
    Help,
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
  switchyard plan <deployment.yaml>
  switchyard up <deployment.yaml>
  switchyard bind <deployment.yaml> <consumer> <group>
  switchyard status <deployment.yaml> [--routes]
  switchyard routes <deployment.yaml>
  switchyard logs <deployment.yaml> [instance[/service]]
  switchyard open <deployment.yaml> <ui>
  switchyard down <deployment.yaml>
  switchyard cleanup <deployment.yaml> --yes
  switchyard daemon run
  switchyard daemon status
  switchyard daemon stop
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
        "validate" if rest.len() == 1 => Ok(CliCommand::Validate { bundle: bundle()? }),
        "plan" if rest.len() == 1 => Ok(CliCommand::Plan { bundle: bundle()? }),
        "up" if rest.len() == 1 => Ok(CliCommand::Up { bundle: bundle()? }),
        "bind" if rest.len() == 3 => Ok(CliCommand::Bind {
            bundle: bundle()?,
            consumer: rest[1].clone(),
            group: rest[2].clone(),
        }),
        "status" if rest.len() == 1 => Ok(CliCommand::Status {
            bundle: bundle()?,
            routes: false,
        }),
        "status" if rest.len() == 2 && rest[1] == "--routes" => Ok(CliCommand::Status {
            bundle: bundle()?,
            routes: true,
        }),
        "routes" if rest.len() == 1 => Ok(CliCommand::Routes { bundle: bundle()? }),
        "logs" if (1..=2).contains(&rest.len()) => Ok(CliCommand::Logs {
            bundle: bundle()?,
            target: rest.get(1).cloned(),
        }),
        "open" if rest.len() == 2 => Ok(CliCommand::Open {
            bundle: bundle()?,
            ui: rest[1].clone(),
        }),
        "down" if rest.len() == 1 => Ok(CliCommand::Down { bundle: bundle()? }),
        "cleanup" if rest.len() == 1 => Ok(CliCommand::Cleanup {
            bundle: bundle()?,
            confirmed: false,
        }),
        "cleanup" if rest.len() == 2 && rest[1] == "--yes" => Ok(CliCommand::Cleanup {
            bundle: bundle()?,
            confirmed: true,
        }),
        "daemon" if rest == ["run"] => Ok(CliCommand::DaemonRun),
        "daemon" if rest == ["status"] => Ok(CliCommand::DaemonStatus),
        "daemon" if rest == ["stop"] => Ok(CliCommand::DaemonStop),
        _ => Err(UsageError(format!(
            "invalid {command} arguments\n\n{USAGE}"
        ))),
    }
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
            }
        );
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
}
