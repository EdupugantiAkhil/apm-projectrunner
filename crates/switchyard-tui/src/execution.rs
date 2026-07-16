use std::{
    ffi::OsString,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::Sender,
    thread,
};

use crate::run_scripts::{RunScript, StructuredCommand};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OperationSpec {
    Structured {
        command: StructuredCommand,
        bundle: PathBuf,
        overlays: Vec<String>,
        variation: Option<String>,
        set: Vec<String>,
    },
    Bind {
        bundle: PathBuf,
        consumer: String,
        group: String,
    },
    Shell(String),
}

impl OperationSpec {
    pub(crate) fn direct(command: StructuredCommand, bundle: PathBuf) -> Self {
        Self::Structured {
            command,
            bundle,
            overlays: Vec::new(),
            variation: None,
            set: Vec::new(),
        }
    }

    pub(crate) fn from_script(script: &RunScript, bundle: PathBuf) -> Result<Self, String> {
        script.validate()?;
        if let Some(command) = script.command {
            Ok(Self::Structured {
                command,
                bundle,
                overlays: script.overlays.clone(),
                variation: script.variation.clone(),
                set: script.set.clone(),
            })
        } else {
            Ok(Self::Shell(
                script.shell.clone().expect("validated shell script"),
            ))
        }
    }

    pub(crate) fn bind(bundle: PathBuf, consumer: String, group: String) -> Self {
        Self::Bind {
            bundle,
            consumer,
            group,
        }
    }

    pub(crate) fn arguments(&self) -> Option<Vec<OsString>> {
        match self {
            Self::Structured {
                command,
                bundle,
                overlays,
                variation,
                set,
            } => {
                let mut args = vec![
                    OsString::from(command.as_str()),
                    bundle.as_os_str().to_owned(),
                ];
                for overlay in overlays {
                    args.extend([OsString::from("--with"), OsString::from(overlay)]);
                }
                if let Some(variation) = variation {
                    args.extend([OsString::from("--variation"), OsString::from(variation)]);
                }
                for value in set {
                    args.extend([OsString::from("--set"), OsString::from(value)]);
                }
                Some(args)
            }
            Self::Bind {
                bundle,
                consumer,
                group,
            } => Some(vec![
                OsString::from("bind"),
                bundle.as_os_str().to_owned(),
                OsString::from(consumer),
                OsString::from(group),
            ]),
            Self::Shell(_) => None,
        }
    }
}

#[derive(Debug)]
pub(crate) enum OperationEvent {
    Output(String),
    Finished { exit_code: i32 },
    Failed(String),
}

pub(crate) fn run(project: &Path, spec: OperationSpec, sender: &Sender<OperationEvent>) {
    let mut command = match &spec {
        OperationSpec::Structured { .. } | OperationSpec::Bind { .. } => {
            let executable = std::env::var_os("SWITCHYARD_BIN")
                .map(PathBuf::from)
                .or_else(|| std::env::current_exe().ok())
                .unwrap_or_else(|| PathBuf::from("switchyard"));
            let mut command = Command::new(executable);
            command.args(spec.arguments().expect("structured arguments"));
            command
        }
        OperationSpec::Shell(script) => {
            let shell = std::env::var_os("SHELL").unwrap_or_else(|| OsString::from("/bin/sh"));
            let mut command = Command::new(shell);
            command.args([OsString::from("-c"), OsString::from(script)]);
            command
        }
    };
    let child = command
        .current_dir(project)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(error) => {
            let _ = sender.send(OperationEvent::Failed(format!(
                "could not start operation: {error}"
            )));
            return;
        }
    };
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let stdout_sender = sender.clone();
    let stdout_thread = thread::spawn(move || stream(stdout, "", &stdout_sender));
    let stderr_sender = sender.clone();
    let stderr_thread = thread::spawn(move || stream(stderr, "stderr: ", &stderr_sender));
    let status = child.wait();
    let _ = stdout_thread.join();
    let _ = stderr_thread.join();
    match status {
        Ok(status) => {
            let _ = sender.send(OperationEvent::Finished {
                exit_code: status.code().unwrap_or(1),
            });
        }
        Err(error) => {
            let _ = sender.send(OperationEvent::Failed(format!(
                "operation wait failed: {error}"
            )));
        }
    }
}

fn stream(reader: impl std::io::Read, prefix: &str, sender: &Sender<OperationEvent>) {
    for line in BufReader::new(reader).lines() {
        let text = match line {
            Ok(line) => format!("{prefix}{line}"),
            Err(error) => format!("{prefix}<read error: {error}>"),
        };
        if sender.send(OperationEvent::Output(text)).is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structured_script_maps_to_typed_argv() {
        let script = RunScript {
            name: "dev".into(),
            description: None,
            command: Some(StructuredCommand::Up),
            overlays: vec!["a.yaml".into(), "b.yaml".into()],
            variation: Some("v1".into()),
            set: vec!["A=1".into(), "B=two words".into()],
            shell: None,
        };
        let spec = OperationSpec::from_script(&script, "deployment.yaml".into()).unwrap();
        assert_eq!(
            spec.arguments().unwrap(),
            [
                "up",
                "deployment.yaml",
                "--with",
                "a.yaml",
                "--with",
                "b.yaml",
                "--variation",
                "v1",
                "--set",
                "A=1",
                "--set",
                "B=two words"
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn pairing_maps_to_shell_free_bind_arguments() {
        let spec = OperationSpec::bind(
            "deployment.yaml".into(),
            "ui-a".into(),
            "backend-feature".into(),
        );
        assert_eq!(
            spec.arguments().unwrap(),
            ["bind", "deployment.yaml", "ui-a", "backend-feature"].map(OsString::from)
        );
    }
}
