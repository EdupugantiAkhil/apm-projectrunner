//! Project run-action storage, validation, acknowledgement, and execution.

use std::{
    ffi::OsString,
    fmt, fs, io,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::Sender,
    thread,
};

use serde::{Deserialize, Serialize};

pub const FILE_NAME: &str = ".apmpr/run-scripts.yaml";
const SHELL_NOTICE_FILE: &str = ".apmpr/shell-run-notice-acknowledged";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StructuredCommand {
    Up,
    Down,
    Plan,
    Status,
}

impl StructuredCommand {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
            Self::Plan => "plan",
            Self::Status => "status",
        }
    }

    pub const fn next(self) -> Self {
        match self {
            Self::Up => Self::Down,
            Self::Down => Self::Plan,
            Self::Plan => Self::Status,
            Self::Status => Self::Up,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunScript {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<StructuredCommand>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overlays: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variation: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub set: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
}

impl RunScript {
    pub fn validate(&self) -> Result<(), String> {
        validate_name(&self.name)?;
        match (self.command, self.shell.as_deref()) {
            (Some(_), None) => {}
            (None, Some(shell)) if !shell.trim().is_empty() => {
                if !self.overlays.is_empty() || self.variation.is_some() || !self.set.is_empty() {
                    return Err("overlays, variation, and set require a structured command".into());
                }
            }
            (Some(_), Some(_)) => {
                return Err("choose a structured command or shell, not both".into());
            }
            _ => return Err("choose a structured command or enter a shell command".into()),
        }
        if self.set.iter().any(|value| {
            value
                .split_once('=')
                .is_none_or(|(key, _)| key.trim().is_empty())
        }) {
            return Err("each set entry must be KEY=VALUE".into());
        }
        Ok(())
    }

    pub const fn is_shell(&self) -> bool {
        self.shell.is_some()
    }
}

pub fn validate_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("name is required".into());
    }
    if name.chars().any(char::is_control) {
        return Err("name may not contain control characters".into());
    }
    Ok(())
}

#[derive(Debug)]
pub enum RunActionError {
    Io { path: PathBuf, source: io::Error },
    InvalidFile { path: PathBuf, message: String },
    InvalidAction { name: String, message: String },
    DuplicateName { name: String },
    NotFound { name: String },
}

impl RunActionError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Io { .. } => "run_actions_io",
            Self::InvalidFile { .. } => "run_actions_invalid",
            Self::InvalidAction { .. } => "run_action_invalid",
            Self::DuplicateName { .. } => "run_action_exists",
            Self::NotFound { .. } => "run_action_not_found",
        }
    }
}

impl fmt::Display for RunActionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(formatter, "could not access {}: {source}", path.display())
            }
            Self::InvalidFile { path, message } => {
                write!(formatter, "invalid {}: {message}", path.display())
            }
            Self::InvalidAction { name, message } => {
                write!(formatter, "invalid run action `{name}`: {message}")
            }
            Self::DuplicateName { name } => {
                write!(formatter, "run action name `{name}` already exists")
            }
            Self::NotFound { name } => write!(formatter, "run action `{name}` was not found"),
        }
    }
}

impl std::error::Error for RunActionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub fn load_result(project: &Path) -> Result<Vec<RunScript>, RunActionError> {
    let path = project.join(FILE_NAME);
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(RunActionError::Io { path, source }),
    };
    let scripts: Vec<RunScript> =
        serde_yaml::from_str(&contents).map_err(|error| RunActionError::InvalidFile {
            path: path.clone(),
            message: error.to_string(),
        })?;
    validate_collection(&scripts, &path)?;
    Ok(scripts)
}

fn validate_collection(scripts: &[RunScript], path: &Path) -> Result<(), RunActionError> {
    for (index, script) in scripts.iter().enumerate() {
        script
            .validate()
            .map_err(|message| RunActionError::InvalidFile {
                path: path.to_path_buf(),
                message: format!("invalid script {}: {message}", index + 1),
            })?;
        if scripts[..index]
            .iter()
            .any(|other| other.name == script.name)
        {
            return Err(RunActionError::InvalidFile {
                path: path.to_path_buf(),
                message: format!("duplicate script name `{}`", script.name),
            });
        }
    }
    Ok(())
}

pub fn load(project: &Path) -> (Vec<RunScript>, Option<String>) {
    match load_result(project) {
        Ok(scripts) => (scripts, None),
        Err(error) => (Vec::new(), Some(error.to_string())),
    }
}

pub fn save(project: &Path, scripts: &[RunScript]) -> Result<(), String> {
    save_result(project, scripts).map_err(|error| error.to_string())
}

pub fn save_result(project: &Path, scripts: &[RunScript]) -> Result<(), RunActionError> {
    let path = project.join(FILE_NAME);
    validate_collection(scripts, &path)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| RunActionError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let contents = serde_yaml::to_string(scripts).map_err(|error| RunActionError::InvalidFile {
        path: path.clone(),
        message: error.to_string(),
    })?;
    fs::write(&path, contents).map_err(|source| RunActionError::Io { path, source })
}

pub fn create(project: &Path, script: RunScript) -> Result<Vec<RunScript>, RunActionError> {
    script
        .validate()
        .map_err(|message| RunActionError::InvalidAction {
            name: script.name.clone(),
            message,
        })?;
    let mut scripts = load_result(project)?;
    if scripts.iter().any(|existing| existing.name == script.name) {
        return Err(RunActionError::DuplicateName { name: script.name });
    }
    scripts.push(script);
    save_result(project, &scripts)?;
    Ok(scripts)
}

pub fn update(
    project: &Path,
    existing_name: &str,
    script: RunScript,
) -> Result<Vec<RunScript>, RunActionError> {
    script
        .validate()
        .map_err(|message| RunActionError::InvalidAction {
            name: script.name.clone(),
            message,
        })?;
    let mut scripts = load_result(project)?;
    let index = scripts
        .iter()
        .position(|existing| existing.name == existing_name)
        .ok_or_else(|| RunActionError::NotFound {
            name: existing_name.into(),
        })?;
    if scripts
        .iter()
        .enumerate()
        .any(|(other_index, existing)| other_index != index && existing.name == script.name)
    {
        return Err(RunActionError::DuplicateName { name: script.name });
    }
    scripts[index] = script;
    save_result(project, &scripts)?;
    Ok(scripts)
}

pub fn delete(project: &Path, name: &str) -> Result<Vec<RunScript>, RunActionError> {
    let mut scripts = load_result(project)?;
    let index = scripts
        .iter()
        .position(|script| script.name == name)
        .ok_or_else(|| RunActionError::NotFound { name: name.into() })?;
    scripts.remove(index);
    save_result(project, &scripts)?;
    Ok(scripts)
}

pub fn shell_notice_acknowledged(project: &Path) -> bool {
    project.join(SHELL_NOTICE_FILE).is_file()
}

pub fn acknowledge_shell_notice(project: &Path) -> Result<(), String> {
    let path = project.join(SHELL_NOTICE_FILE);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(path, b"Shell run-script warning acknowledged.\n").map_err(|error| error.to_string())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OperationSpec {
    Structured {
        command: StructuredCommand,
        bundle: PathBuf,
        overlays: Vec<String>,
        variation: Option<String>,
        set: Vec<String>,
    },
    Membership {
        bundle: PathBuf,
        instance: String,
        group: String,
    },
    Shell(String),
}

impl OperationSpec {
    pub fn direct(command: StructuredCommand, bundle: PathBuf) -> Self {
        Self::Structured {
            command,
            bundle,
            overlays: Vec::new(),
            variation: None,
            set: Vec::new(),
        }
    }

    pub fn from_script(script: &RunScript, bundle: PathBuf) -> Result<Self, String> {
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

    pub fn membership(bundle: PathBuf, instance: String, group: String) -> Self {
        Self::Membership {
            bundle,
            instance,
            group,
        }
    }

    pub fn arguments(&self) -> Option<Vec<OsString>> {
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
            Self::Membership {
                bundle,
                instance,
                group,
            } => Some(vec![
                OsString::from("move"),
                bundle.as_os_str().to_owned(),
                OsString::from(instance),
                OsString::from(group),
            ]),
            Self::Shell(_) => None,
        }
    }

    /// Builds the exact child command used by interactive clients.
    pub fn process_command(&self) -> Command {
        self.process_command_with(None)
    }

    /// Builds the same child command while allowing an embedding daemon to supply its
    /// configured APM ProjectRunner CLI path for structured actions.
    pub fn process_command_with(&self, structured_program: Option<&Path>) -> Command {
        match self {
            Self::Structured { .. } | Self::Membership { .. } => {
                let executable = structured_program
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| {
                        std::env::var_os("APMPR_BIN")
                            .map(PathBuf::from)
                            .or_else(|| std::env::current_exe().ok())
                            .unwrap_or_else(|| PathBuf::from("apmpr"))
                    });
                let mut command = Command::new(executable);
                command.args(self.arguments().expect("structured arguments"));
                command
            }
            Self::Shell(script) => {
                let shell = std::env::var_os("SHELL").unwrap_or_else(|| OsString::from("/bin/sh"));
                let mut command = Command::new(shell);
                command.args([OsString::from("-c"), OsString::from(script)]);
                command
            }
        }
    }
}

#[derive(Debug)]
pub enum OperationEvent {
    Output(String),
    Finished { exit_code: i32 },
    Failed(String),
}

pub fn run(project: &Path, spec: OperationSpec, sender: &Sender<OperationEvent>) {
    let child = spec
        .process_command()
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

fn stream(reader: impl io::Read, prefix: &str, sender: &Sender<OperationEvent>) {
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
    use tempfile::TempDir;

    fn structured(name: &str) -> RunScript {
        RunScript {
            name: name.into(),
            description: Some("starts the dev topology".into()),
            command: Some(StructuredCommand::Up),
            overlays: vec!["overlays/dev.yaml".into()],
            variation: Some("fast".into()),
            set: vec!["API_PORT=9000".into()],
            shell: None,
        }
    }

    #[test]
    fn file_round_trips_and_crud_preserves_validation() {
        let root = TempDir::new().unwrap();
        let original = structured("dev up");
        save(root.path(), std::slice::from_ref(&original)).unwrap();
        assert_eq!(load(root.path()), (vec![original], None));

        create(root.path(), structured("plan")).unwrap();
        let mut replacement = structured("renamed");
        replacement.command = Some(StructuredCommand::Plan);
        update(root.path(), "plan", replacement.clone()).unwrap();
        assert_eq!(load_result(root.path()).unwrap()[1], replacement);
        delete(root.path(), "renamed").unwrap();
        assert_eq!(load_result(root.path()).unwrap().len(), 1);
    }

    #[test]
    fn malformed_file_is_a_visible_load_error() {
        let root = TempDir::new().unwrap();
        fs::create_dir_all(root.path().join(".apmpr")).unwrap();
        fs::write(root.path().join(FILE_NAME), "- name: [not valid").unwrap();
        let (scripts, error) = load(root.path());
        assert!(scripts.is_empty());
        assert!(error.unwrap().contains("invalid"));
    }

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
    fn membership_maps_to_shell_free_move_arguments() {
        let spec = OperationSpec::membership(
            "deployment.yaml".into(),
            "ui-a".into(),
            "backend-feature".into(),
        );
        assert_eq!(
            spec.arguments().unwrap(),
            ["move", "deployment.yaml", "ui-a", "backend-feature"].map(OsString::from)
        );
    }
}
