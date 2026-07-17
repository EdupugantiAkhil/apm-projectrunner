use std::{
    collections::BTreeMap,
    fmt, fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use switchyard_planner::{Block, Diagnostic};
use switchyard_state::StateStore;

use crate::{
    profiles::{ProfileOrigin, ProfileTrust, list_profiles, load_profile_block},
    projections::SourceChoice,
};

#[derive(Clone, Debug)]
pub struct CreateInstanceRequest {
    pub name: String,
    pub profile: String,
    pub profile_origin: ProfileOrigin,
    pub source: String,
    pub device: String,
    pub parameters: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstancePreview {
    pub draft: String,
    pub expanded_services: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreatedInstance {
    pub name: String,
    pub expanded_services: Vec<String>,
    pub materialized_profile: bool,
    pub declared_source: bool,
}

#[derive(Debug)]
pub enum CreateInstanceError {
    Io { path: PathBuf, source: io::Error },
    InvalidRequest(String),
    Profile(String),
    Source(String),
    Definition(String),
    Validation(Vec<Diagnostic>),
}

impl fmt::Display for CreateInstanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(formatter, "could not update {}: {source}", path.display())
            }
            Self::InvalidRequest(message)
            | Self::Profile(message)
            | Self::Source(message)
            | Self::Definition(message) => formatter.write_str(message),
            Self::Validation(diagnostics) => formatter.write_str(
                &diagnostics
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; "),
            ),
        }
    }
}

impl std::error::Error for CreateInstanceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

struct DraftContext {
    preview: InstancePreview,
    materialized_profile: bool,
    declared_source: bool,
}

/// Builds and plans an authored deployment draft without writing it.
pub fn preview_instance(
    project_dir: &Path,
    definition: &Path,
    request: &CreateInstanceRequest,
) -> Result<InstancePreview, CreateInstanceError> {
    Ok(build_draft(project_dir, definition, request)?.preview)
}

/// Validates and atomically appends an instance to an authored deployment.
pub fn create_instance(
    project_dir: &Path,
    definition: &Path,
    request: &CreateInstanceRequest,
) -> Result<CreatedInstance, CreateInstanceError> {
    let context = build_draft(project_dir, definition, request)?;
    if !context.preview.diagnostics.is_empty() {
        return Err(CreateInstanceError::Validation(context.preview.diagnostics));
    }
    replace_definition(definition, &context.preview.draft)?;
    Ok(CreatedInstance {
        name: request.name.trim().into(),
        expanded_services: context.preview.expanded_services,
        materialized_profile: context.materialized_profile,
        declared_source: context.declared_source,
    })
}

fn build_draft(
    project_dir: &Path,
    definition: &Path,
    request: &CreateInstanceRequest,
) -> Result<DraftContext, CreateInstanceError> {
    validate_request(request)?;
    let input = fs::read_to_string(definition).map_err(|source| CreateInstanceError::Io {
        path: definition.into(),
        source,
    })?;
    let bundle = switchyard_planner::load_bundle(definition)
        .map_err(|error| CreateInstanceError::Definition(error.to_string()))?;
    let listing = list_profiles(project_dir, definition)
        .map_err(|error| CreateInstanceError::Profile(error.to_string()))?;
    let row = listing
        .rows
        .iter()
        .find(|row| row.name == request.profile && row.origin == request.profile_origin)
        .ok_or_else(|| {
            CreateInstanceError::Profile(format!(
                "startup profile `{}` is no longer available",
                request.profile
            ))
        })?;
    if !matches!(row.trust, ProfileTrust::Trusted | ProfileTrust::Imported) {
        return Err(CreateInstanceError::Profile(format!(
            "startup profile `{}` must be reviewed/imported in the Profiles view first",
            request.profile
        )));
    }
    let block = load_profile_block(
        project_dir,
        definition,
        &request.profile,
        &request.profile_origin,
    )
    .map_err(|error| CreateInstanceError::Profile(error.to_string()))?;
    reject_unknown_parameters(&block, &request.parameters)?;
    let source = resolve_source(project_dir, &bundle, &request.source)?;

    let had_trailing_newline = input.ends_with('\n');
    let mut lines = input.lines().map(str::to_owned).collect::<Vec<_>>();
    let declared_source = !source.declared;
    if declared_source {
        insert_spec_section(
            &mut lines,
            "sources",
            vec![source_definition_line(&source)?],
        )?;
    }
    let materialized_profile = !bundle.spec.blocks.contains_key(&request.profile);
    if materialized_profile {
        insert_spec_section(
            &mut lines,
            "blocks",
            block_definition_lines(&request.profile, &block)?,
        )?;
    }
    insert_spec_section(&mut lines, "instances", instance_definition_lines(request)?)?;
    let mut draft = lines.join("\n");
    if had_trailing_newline {
        draft.push('\n');
    }

    let draft_bundle = switchyard_planner::load_bundle_from_str(&draft, definition)
        .map_err(|error| CreateInstanceError::Definition(error.to_string()))?;
    let expanded_services = draft_bundle
        .spec
        .blocks
        .get(&request.profile)
        .map(|block| {
            block
                .services
                .keys()
                .map(|service| {
                    format!(
                        "{}--{}--{service}",
                        draft_bundle.metadata.name,
                        request.name.trim()
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let diagnostics = switchyard_planner::plan(&draft_bundle)
        .err()
        .unwrap_or_default();
    Ok(DraftContext {
        preview: InstancePreview {
            draft,
            expanded_services,
            diagnostics,
        },
        materialized_profile,
        declared_source,
    })
}

fn validate_request(request: &CreateInstanceRequest) -> Result<(), CreateInstanceError> {
    if request.device.trim().is_empty() {
        return Err(CreateInstanceError::InvalidRequest(
            "device cannot be empty".into(),
        ));
    }
    Ok(())
}

fn reject_unknown_parameters(
    block: &Block,
    values: &BTreeMap<String, String>,
) -> Result<(), CreateInstanceError> {
    if let Some(name) = values
        .keys()
        .find(|name| !block.parameters.contains_key(*name))
    {
        return Err(CreateInstanceError::InvalidRequest(format!(
            "startup profile does not declare parameter `{name}`"
        )));
    }
    Ok(())
}

fn resolve_source(
    project_dir: &Path,
    bundle: &switchyard_planner::Bundle,
    name: &str,
) -> Result<SourceChoice, CreateInstanceError> {
    if let Some(source) = bundle.spec.sources.get(name) {
        return Ok(SourceChoice {
            name: name.into(),
            path: source.path.clone(),
            declared: true,
            worktree: matches!(source.r#type, switchyard_planner::SourceType::Worktree),
            repository: source.repository.clone(),
            requested_ref: source.r#ref.clone(),
        });
    }
    let store = StateStore::open(project_dir.join(".switchyard/state.sqlite3"))
        .map_err(|error| CreateInstanceError::Source(error.to_string()))?
        .0;
    let source = store
        .source(name)
        .map_err(|error| CreateInstanceError::Source(error.to_string()))?
        .ok_or_else(|| CreateInstanceError::Source(format!("source `{name}` is not registered")))?;
    Ok(SourceChoice {
        name: source.name,
        path: source.path,
        declared: false,
        worktree: source.repository_path.is_some(),
        repository: source.repository_path,
        requested_ref: source.requested_ref,
    })
}

fn yaml_scalar(value: &str) -> Result<String, CreateInstanceError> {
    serde_yaml::to_string(value)
        .map(|encoded| encoded.trim().to_owned())
        .map_err(|error| CreateInstanceError::Definition(format!("could not encode YAML: {error}")))
}

fn source_definition_line(source: &SourceChoice) -> Result<String, CreateInstanceError> {
    let path = yaml_scalar(&source.path.display().to_string())?;
    if !source.worktree {
        return Ok(format!("    {}: {{ path: {path} }}", source.name));
    }
    let repository = source.repository.as_ref().ok_or_else(|| {
        CreateInstanceError::Source(format!(
            "worktree source `{}` has no repository path",
            source.name
        ))
    })?;
    let repository = yaml_scalar(&repository.display().to_string())?;
    let reference = source
        .requested_ref
        .as_deref()
        .map(yaml_scalar)
        .transpose()?
        .map_or_else(String::new, |value| format!(", ref: {value}"));
    Ok(format!(
        "    {}: {{ type: worktree, repository: {repository}, path: {path}{reference} }}",
        source.name
    ))
}

fn block_definition_lines(name: &str, block: &Block) -> Result<Vec<String>, CreateInstanceError> {
    let value = serde_yaml::to_value(block).map_err(|error| {
        CreateInstanceError::Definition(format!("could not encode startup profile: {error}"))
    })?;
    let value = prune_empty_yaml(value).unwrap_or(serde_yaml::Value::Mapping(Default::default()));
    let body = serde_yaml::to_string(&value).map_err(|error| {
        CreateInstanceError::Definition(format!("could not encode startup profile: {error}"))
    })?;
    let mut lines = vec![format!("    {name}:")];
    lines.extend(body.trim_end().lines().map(|line| format!("      {line}")));
    Ok(lines)
}

/// Drops nulls and empty maps/sequences so materialized profiles stay readable.
/// The planner deserializes the pruned and unpruned forms identically because all
/// pruned fields are `#[serde(default)]`.
fn prune_empty_yaml(value: serde_yaml::Value) -> Option<serde_yaml::Value> {
    match value {
        serde_yaml::Value::Null => None,
        serde_yaml::Value::Mapping(mapping) => {
            let pruned: serde_yaml::Mapping = mapping
                .into_iter()
                .filter_map(|(key, value)| Some((key, prune_empty_yaml(value)?)))
                .collect();
            if pruned.is_empty() {
                None
            } else {
                Some(serde_yaml::Value::Mapping(pruned))
            }
        }
        serde_yaml::Value::Sequence(sequence) => {
            let pruned: Vec<_> = sequence.into_iter().filter_map(prune_empty_yaml).collect();
            if pruned.is_empty() {
                None
            } else {
                Some(serde_yaml::Value::Sequence(pruned))
            }
        }
        scalar => Some(scalar),
    }
}

fn instance_definition_lines(
    request: &CreateInstanceRequest,
) -> Result<Vec<String>, CreateInstanceError> {
    let mut lines = vec![
        format!("    - name: {}", yaml_scalar(request.name.trim())?),
        format!("      block: {}", request.profile),
        format!("      source: {}", request.source),
        format!("      device: {}", yaml_scalar(request.device.trim())?),
    ];
    let parameters = request
        .parameters
        .iter()
        .filter(|(_, value)| !value.is_empty())
        .collect::<Vec<_>>();
    if !parameters.is_empty() {
        lines.push("      parameters:".into());
        for (name, value) in parameters {
            lines.push(format!("        {name}: {}", yaml_scalar(value)?));
        }
    }
    Ok(lines)
}

pub(crate) fn insert_spec_section(
    lines: &mut Vec<String>,
    section: &str,
    additions: Vec<String>,
) -> Result<(), CreateInstanceError> {
    let marker = format!("  {section}:");
    let start = lines
        .iter()
        .position(|line| line == &marker)
        .ok_or_else(|| {
            CreateInstanceError::Definition(format!(
                "cannot add interactively: `spec.{section}` must use an indented YAML block"
            ))
        })?;
    let mut end = start + 1;
    while end < lines.len() {
        let line = &lines[end];
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            end += 1;
            continue;
        }
        if line.len() - line.trim_start().len() <= 2 {
            break;
        }
        end += 1;
    }
    lines.splice(end..end, additions);
    Ok(())
}

fn replace_definition(definition: &Path, output: &str) -> Result<(), CreateInstanceError> {
    let parent = definition.parent().ok_or_else(|| {
        CreateInstanceError::Definition("deployment definition has no parent directory".into())
    })?;
    let filename = definition
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("deployment.yaml");
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| CreateInstanceError::Definition("system clock is before Unix epoch".into()))?
        .as_millis();
    let temporary = parent.join(format!(
        ".{filename}.switchyard-ops-{}-{millis}",
        std::process::id()
    ));
    let permissions = fs::metadata(definition)
        .map_err(|source| CreateInstanceError::Io {
            path: definition.into(),
            source,
        })?
        .permissions();
    let result = (|| {
        fs::write(&temporary, output).map_err(|source| CreateInstanceError::Io {
            path: temporary.clone(),
            source,
        })?;
        fs::set_permissions(&temporary, permissions).map_err(|source| CreateInstanceError::Io {
            path: temporary.clone(),
            source,
        })?;
        fs::rename(&temporary, definition).map_err(|source| CreateInstanceError::Io {
            path: definition.into(),
            source,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn materialized_profile_yaml_omits_nulls_and_empty_collections() {
        let block: Block = serde_yaml::from_str(
            "services:\n  web:\n    execution: { type: container, image: example/api:1 }\n",
        )
        .unwrap();
        let lines = block_definition_lines("demo-api", &block).unwrap();
        let text = lines.join("\n");
        assert!(
            !text.contains("null"),
            "pruned YAML still contains null: {text}"
        );
        assert!(
            !text.contains("{}"),
            "pruned YAML still contains {{}}: {text}"
        );
        assert!(text.contains("image: example/api:1"));
    }

    use switchyard_state::{RegisteredSource, RegisteredSourceKind};
    use tempfile::TempDir;

    const PROJECT_BLOCK: &str = r#"    api:
      parameters:
        LOG_LEVEL: { required: true }
        PORT: { default: "8080" }
      services:
        web:
          execution: { type: container, image: example/api:1 }
"#;

    fn definition(root: &Path, block: &str) -> PathBuf {
        let path = root.join("deployment.yaml");
        fs::write(
            &path,
            format!(
                r#"apiVersion: switchyard.dev/v1alpha1
kind: Deployment
metadata: {{ name: demo }}
spec:
  sources:
    checkout: {{ path: . }}
  blocks:
{block}  instances:
  groups:
  bindings:
  routes:
"#
            ),
        )
        .unwrap();
        StateStore::open(root.join(".switchyard/state.sqlite3")).unwrap();
        path
    }

    fn request(origin: ProfileOrigin) -> CreateInstanceRequest {
        CreateInstanceRequest {
            name: "api-main".into(),
            profile: "api".into(),
            profile_origin: origin,
            source: "checkout".into(),
            device: "local".into(),
            parameters: BTreeMap::from([
                ("LOG_LEVEL".into(), "debug".into()),
                ("PORT".into(), "8080".into()),
            ]),
        }
    }

    fn imported_project() -> (TempDir, PathBuf, PathBuf) {
        let project = TempDir::new().unwrap();
        let source = project.path().join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("switchyard-profiles.yaml"),
            r#"version: 1
profiles:
  api:
    parameters:
      LOG_LEVEL: { required: true }
      PORT: { default: "8080" }
    services:
      web:
        execution: { type: container, image: example/api:1 }
"#,
        )
        .unwrap();
        let path = definition(project.path(), "");
        let store = StateStore::open(project.path().join(".switchyard/state.sqlite3"))
            .unwrap()
            .0;
        store
            .register_source(&RegisteredSource {
                name: "profile-source".into(),
                kind: RegisteredSourceKind::Unmanaged,
                path: source.clone(),
                repository_path: None,
                requested_ref: None,
                created_at: 1,
                managed_relative_path: None,
            })
            .unwrap();
        crate::profiles::import_source_profile(project.path(), "profile-source", "api").unwrap();
        (project, path, source)
    }

    #[test]
    fn creates_project_profile_instance_with_parameters_and_device() {
        let project = TempDir::new().unwrap();
        let path = definition(project.path(), PROJECT_BLOCK);
        let created = create_instance(project.path(), &path, &request(ProfileOrigin::Project))
            .expect("project profile should create");
        assert_eq!(created.expanded_services, ["demo--api-main--web"]);
        let updated = fs::read_to_string(path).unwrap();
        assert!(updated.contains("device: local"));
        assert!(updated.contains("LOG_LEVEL: debug"));
        assert!(updated.contains("PORT: '8080'") || updated.contains("PORT: 8080"));
    }

    #[test]
    fn imported_profile_is_materialized_once_and_reused() {
        let (project, path, _) = imported_project();
        let origin = ProfileOrigin::ImportedFromSource {
            source: "profile-source".into(),
            commit: None,
        };
        let first = create_instance(project.path(), &path, &request(origin.clone())).unwrap();
        assert!(first.materialized_profile);
        let mut second_request = request(origin);
        second_request.name = "api-feature".into();
        let second = create_instance(project.path(), &path, &second_request).unwrap();
        assert!(!second.materialized_profile);
        let updated = fs::read_to_string(path).unwrap();
        assert_eq!(updated.matches("    api:\n").count(), 1);
        assert!(updated.contains("name: api-main"));
        assert!(updated.contains("name: api-feature"));
    }

    #[test]
    fn changed_and_not_imported_profiles_are_refused() {
        let (project, path, source) = imported_project();
        fs::write(
            source.join("switchyard-profiles.yaml"),
            "version: 1\nprofiles:\n  api:\n    services:\n      changed:\n        execution: { type: container, image: changed:1 }\n",
        )
        .unwrap();
        let changed = create_instance(
            project.path(),
            &path,
            &request(ProfileOrigin::ImportedFromSource {
                source: "profile-source".into(),
                commit: None,
            }),
        )
        .unwrap_err();
        assert!(changed.to_string().contains("Profiles view"));

        let fresh = TempDir::new().unwrap();
        let source = fresh.path().join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("switchyard-profiles.yaml"),
            "version: 1\nprofiles:\n  api:\n    services:\n      web:\n        execution: { type: container, image: api:1 }\n",
        )
        .unwrap();
        let definition = definition(fresh.path(), "");
        let store = StateStore::open(fresh.path().join(".switchyard/state.sqlite3"))
            .unwrap()
            .0;
        store
            .register_source(&RegisteredSource {
                name: "profile-source".into(),
                kind: RegisteredSourceKind::Unmanaged,
                path: source,
                repository_path: None,
                requested_ref: None,
                created_at: 1,
                managed_relative_path: None,
            })
            .unwrap();
        let error = create_instance(
            fresh.path(),
            &definition,
            &request(ProfileOrigin::DiscoveredInSource {
                source: "profile-source".into(),
                commit: None,
            }),
        )
        .unwrap_err();
        assert!(error.to_string().contains("Profiles view"));
    }

    #[test]
    fn validation_failures_leave_definition_untouched() {
        let project = TempDir::new().unwrap();
        let path = definition(project.path(), PROJECT_BLOCK);
        let before = fs::read_to_string(&path).unwrap();
        let mut missing = request(ProfileOrigin::Project);
        missing.parameters.remove("LOG_LEVEL");
        let error = create_instance(project.path(), &path, &missing).unwrap_err();
        assert!(matches!(error, CreateInstanceError::Validation(_)));
        assert!(error.to_string().contains("parameters.LOG_LEVEL"));
        assert_eq!(fs::read_to_string(&path).unwrap(), before);

        let mut remote = request(ProfileOrigin::Project);
        remote.device = "builder".into();
        let error = create_instance(project.path(), &path, &remote).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("remote placement is not yet supported")
        );
        assert_eq!(fs::read_to_string(path).unwrap(), before);
    }
}
