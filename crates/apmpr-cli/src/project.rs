use std::{
    fmt, fs, io,
    io::Write,
    path::{Path, PathBuf},
};

use apmpr_sources::SourceManager;
use apmpr_state::{
    PROJECT_API_VERSION, PROJECT_MARKER_PATH, ProjectMetadata, StateStore, load_project_metadata,
};

use crate::init::{default_project_name, valid_metadata_name};

#[derive(Debug)]
pub struct Registration {
    pub root: PathBuf,
    pub metadata: ProjectMetadata,
    pub source_name: String,
    pub already_registered: bool,
}

#[derive(Debug)]
pub enum ProjectError {
    Io(io::Error),
    Message(String),
    State(apmpr_state::StateError),
    Source(apmpr_sources::SourceError),
}

impl fmt::Display for ProjectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::Message(message) => message.fmt(formatter),
            Self::State(error) => error.fmt(formatter),
            Self::Source(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ProjectError {}
impl From<io::Error> for ProjectError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}
impl From<apmpr_state::StateError> for ProjectError {
    fn from(value: apmpr_state::StateError) -> Self {
        Self::State(value)
    }
}
impl From<apmpr_sources::SourceError> for ProjectError {
    fn from(value: apmpr_sources::SourceError) -> Self {
        Self::Source(value)
    }
}

pub fn register(
    directory: &Path,
    requested_name: Option<&str>,
) -> Result<Registration, ProjectError> {
    if !directory.is_dir() {
        return Err(ProjectError::Message(format!(
            "project folder `{}` does not exist or is not a directory",
            directory.display()
        )));
    }
    let root = directory.canonicalize()?;
    let name = match requested_name {
        Some(name) if valid_metadata_name(name) => name.to_owned(),
        Some(name) => {
            return Err(ProjectError::Message(format!(
                "invalid project name `{name}`; names must be lowercase DNS labels of at most 63 characters"
            )));
        }
        None => {
            default_project_name(&root).map_err(|error| ProjectError::Message(error.to_string()))?
        }
    };
    let metadata = ProjectMetadata {
        api_version: PROJECT_API_VERSION.into(),
        name: name.clone(),
    };
    let existing = load_project_metadata(&root)?;
    if let Some(existing) = &existing {
        if existing != &metadata {
            return Err(ProjectError::Message(format!(
                "folder is already registered as APM ProjectRunner project `{}`",
                existing.name
            )));
        }
    }

    fs::create_dir_all(root.join(".apmpr"))?;
    let (store, _) = StateStore::open(root.join(".apmpr/state.sqlite3"))?;
    let manager = SourceManager::new(&root);
    let sources = manager.list(&store)?;
    let (source_name, register_source) =
        if let Some(source) = sources.iter().find(|source| source.source.path == root) {
            (source.source.name.clone(), false)
        } else {
            if let Some(source) = sources.iter().find(|source| source.source.name == name) {
                return Err(ProjectError::Message(format!(
                    "source name `{name}` is already registered for `{}`",
                    source.source.path.display()
                )));
            }
            (name.clone(), true)
        };
    let deployments = root.join("deployments");
    let created_deployments = !deployments.exists();
    fs::create_dir_all(&deployments)?;

    let mut created_marker = None;
    if existing.is_none() {
        let path = root.join(PROJECT_MARKER_PATH);
        let temporary = path.with_extension(format!("json.tmp-{}", std::process::id()));
        let mut contents = serde_json::to_vec_pretty(&metadata)
            .map_err(|error| ProjectError::Message(error.to_string()))?;
        contents.push(b'\n');
        let write_result = (|| -> io::Result<()> {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(&contents)?;
            file.sync_all()?;
            fs::rename(&temporary, &path)
        })();
        if let Err(error) = write_result {
            let _ = fs::remove_file(&temporary);
            if created_deployments {
                let _ = fs::remove_dir(&deployments);
            }
            return Err(error.into());
        }
        created_marker = Some(path);
    }
    if register_source {
        if let Err(error) = manager.register_unmanaged(&store, &name, &root) {
            if let Some(path) = created_marker {
                let _ = fs::remove_file(path);
            }
            if created_deployments {
                let _ = fs::remove_dir(&deployments);
            }
            return Err(error.into());
        }
    }
    Ok(Registration {
        root,
        metadata,
        source_name,
        already_registered: existing.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn adopts_existing_folder_without_touching_its_files_and_is_idempotent() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("Existing App");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("keep.txt"), "mine").unwrap();

        let first = register(&root, Some("existing-app")).unwrap();
        assert!(!first.already_registered);
        assert_eq!(fs::read_to_string(root.join("keep.txt")).unwrap(), "mine");
        assert!(root.join("deployments").is_dir());
        assert_eq!(load_project_metadata(&root).unwrap(), Some(first.metadata));

        let second = register(&root, Some("existing-app")).unwrap();
        assert!(second.already_registered);
        assert_eq!(second.source_name, "existing-app");
    }
}
