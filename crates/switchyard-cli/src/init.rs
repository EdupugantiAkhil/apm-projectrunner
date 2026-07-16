use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
};

const PROJECT_NAME: &str = "{{project_name}}";
const FILES: &[(&str, &str)] = &[
    (
        "deployment.yaml",
        include_str!("../templates/init/deployment.yaml"),
    ),
    (
        "overlays/dev.yaml",
        include_str!("../templates/init/overlays/dev.yaml"),
    ),
    ("README.md", include_str!("../templates/init/README.md")),
    (".gitignore", include_str!("../templates/init/gitignore")),
];

#[derive(Debug)]
pub struct Scaffold {
    pub directory: PathBuf,
    pub deployment: PathBuf,
    pub project_name: String,
}

#[derive(Debug)]
pub enum InitError {
    Io(io::Error),
    Conflicts(Vec<PathBuf>),
    InvalidName(String),
    MissingDirectoryName(PathBuf),
}

impl fmt::Display for InitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::Conflicts(paths) => {
                writeln!(formatter, "refusing to overwrite existing scaffold files:")?;
                for path in paths {
                    writeln!(formatter, "  {}", path.display())?;
                }
                write!(formatter, "rerun with --force to overwrite them")
            }
            Self::InvalidName(name) => write!(
                formatter,
                "invalid project name `{name}`; names must be lowercase DNS labels of at most 63 characters"
            ),
            Self::MissingDirectoryName(path) => write!(
                formatter,
                "could not derive a project name from directory {}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for InitError {}

impl From<io::Error> for InitError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub fn scaffold(
    directory: &Path,
    requested_name: Option<&str>,
    force: bool,
) -> Result<Scaffold, InitError> {
    fs::create_dir_all(directory)?;
    let directory = directory.canonicalize()?;
    let project_name = match requested_name {
        Some(name) if valid_metadata_name(name) => name.to_owned(),
        Some(name) => return Err(InitError::InvalidName(name.to_owned())),
        None => default_project_name(&directory)?,
    };

    let conflicts = FILES
        .iter()
        .map(|(relative, _)| PathBuf::from(relative))
        .filter(|relative| directory.join(relative).exists())
        .collect::<Vec<_>>();
    if !force && !conflicts.is_empty() {
        return Err(InitError::Conflicts(conflicts));
    }

    for (relative, template) in FILES {
        let path = directory.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, template.replace(PROJECT_NAME, &project_name))?;
    }

    Ok(Scaffold {
        deployment: directory.join("deployment.yaml"),
        directory,
        project_name,
    })
}

fn default_project_name(directory: &Path) -> Result<String, InitError> {
    let basename = directory
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| InitError::MissingDirectoryName(directory.to_owned()))?;
    let mut name = String::new();
    let mut previous_hyphen = false;
    for character in basename.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_lowercase() || character.is_ascii_digit() {
            name.push(character);
            previous_hyphen = false;
        } else if !name.is_empty() && !previous_hyphen {
            name.push('-');
            previous_hyphen = true;
        }
    }
    while name.ends_with('-') {
        name.pop();
    }
    if name.is_empty() {
        name.push_str("project");
    } else if !name.starts_with(|character: char| character.is_ascii_lowercase()) {
        name.insert_str(0, "project-");
    }
    name.truncate(63);
    while name.ends_with('-') {
        name.pop();
    }
    Ok(name)
}

fn valid_metadata_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 63
        && name.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit() && index > 0
                || byte == b'-' && index > 0
        })
        && !name.ends_with('-')
}
