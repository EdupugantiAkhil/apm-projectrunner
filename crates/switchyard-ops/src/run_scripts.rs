use std::{fs, io, path::Path};

use serde::{Deserialize, Serialize};

pub const FILE_NAME: &str = ".switchyard/run-scripts.yaml";
const SHELL_NOTICE_FILE: &str = ".switchyard/shell-run-notice-acknowledged";

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

pub fn load(project: &Path) -> (Vec<RunScript>, Option<String>) {
    let path = project.join(FILE_NAME);
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return (Vec::new(), None),
        Err(error) => {
            return (
                Vec::new(),
                Some(format!("could not read {}: {error}", path.display())),
            );
        }
    };
    let scripts: Vec<RunScript> = match serde_yaml::from_str(&contents) {
        Ok(scripts) => scripts,
        Err(error) => {
            return (
                Vec::new(),
                Some(format!("invalid {}: {error}", path.display())),
            );
        }
    };
    for (index, script) in scripts.iter().enumerate() {
        if let Err(error) = script.validate() {
            return (
                Vec::new(),
                Some(format!(
                    "invalid script {} in {}: {error}",
                    index + 1,
                    path.display()
                )),
            );
        }
        if scripts[..index]
            .iter()
            .any(|other| other.name == script.name)
        {
            return (
                Vec::new(),
                Some(format!(
                    "duplicate script name `{}` in {}",
                    script.name,
                    path.display()
                )),
            );
        }
    }
    (scripts, None)
}

pub fn save(project: &Path, scripts: &[RunScript]) -> Result<(), String> {
    let path = project.join(FILE_NAME);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let contents = serde_yaml::to_string(scripts).map_err(|error| error.to_string())?;
    fs::write(path, contents).map_err(|error| error.to_string())
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

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temp_project() -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("switchyard-tui-scripts-{nonce}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn file_round_trips() {
        let root = temp_project();
        let scripts = vec![RunScript {
            name: "dev up".into(),
            description: Some("starts the dev topology".into()),
            command: Some(StructuredCommand::Up),
            overlays: vec!["overlays/dev.yaml".into()],
            variation: Some("fast".into()),
            set: vec!["API_PORT=9000".into()],
            shell: None,
        }];
        save(&root, &scripts).unwrap();
        assert_eq!(load(&root), (scripts, None));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_file_is_a_visible_load_error() {
        let root = temp_project();
        fs::create_dir_all(root.join(".switchyard")).unwrap();
        fs::write(root.join(FILE_NAME), "- name: [not valid").unwrap();
        let (scripts, error) = load(&root);
        assert!(scripts.is_empty());
        assert!(error.unwrap().contains("invalid"));
        fs::remove_dir_all(root).unwrap();
    }
}
