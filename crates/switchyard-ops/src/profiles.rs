use std::{
    collections::BTreeMap,
    fmt, fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use switchyard_planner::{Block, Execution};
use switchyard_sources::{RegisteredSourceInspection, SourceManager};
use switchyard_state::{ImportedProfile, StateError, StateStore};

const MANIFEST_FILE: &str = "switchyard-profiles.yaml";

/// A validated source-local startup-profile manifest.
#[derive(Clone, Debug)]
pub struct SourceProfileManifest {
    pub version: u32,
    pub profiles: BTreeMap<String, Block>,
}

/// Deterministic hashing for a parsed planner block body.
pub trait ProfileContentHash {
    fn content_hash(&self) -> String;
}

impl ProfileContentHash for Block {
    fn content_hash(&self) -> String {
        let canonical = canonical_definition_json(self)
            .expect("serializing a parsed planner block to JSON cannot fail");
        format!("{:x}", Sha256::digest(canonical.as_bytes()))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    version: u32,
    profiles: BTreeMap<String, serde_yaml::Value>,
}

/// Typed profile-domain failure suitable for interactive clients.
#[derive(Debug)]
pub enum ProfileError {
    Io { path: PathBuf, source: io::Error },
    MalformedManifest { path: PathBuf, message: String },
    UnsupportedVersion { path: PathBuf, version: u32 },
    InvalidProfile { profile: String, message: String },
    SourceNotFound { source: String },
    ProfileNotFound { source: String, profile: String },
    Definition(String),
    State(StateError),
    Clock,
}

impl ProfileError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Io { .. } => "profile_io",
            Self::MalformedManifest { .. } => "profile_manifest_malformed",
            Self::UnsupportedVersion { .. } => "profile_manifest_version_unsupported",
            Self::InvalidProfile { .. } => "profile_invalid",
            Self::SourceNotFound { .. } => "source_not_found",
            Self::ProfileNotFound { .. } => "profile_not_found",
            Self::Definition(_) => "profile_definition_invalid",
            Self::State(_) => "profile_state",
            Self::Clock => "profile_clock",
        }
    }
}

impl fmt::Display for ProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(formatter, "could not read {}: {source}", path.display())
            }
            Self::MalformedManifest { path, message } => {
                write!(
                    formatter,
                    "invalid profile manifest {}: {message}",
                    path.display()
                )
            }
            Self::UnsupportedVersion { path, version } => write!(
                formatter,
                "profile manifest {} uses unsupported version {version}",
                path.display()
            ),
            Self::InvalidProfile { profile, message } => {
                write!(formatter, "invalid profile `{profile}`: {message}")
            }
            Self::SourceNotFound { source } => {
                write!(formatter, "source `{source}` is not registered")
            }
            Self::ProfileNotFound { source, profile } => write!(
                formatter,
                "profile `{profile}` was not found in source `{source}`"
            ),
            Self::Definition(message) => formatter.write_str(message),
            Self::State(error) => error.fmt(formatter),
            Self::Clock => formatter.write_str("system time is before the Unix epoch"),
        }
    }
}

impl std::error::Error for ProfileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::State(source) => Some(source),
            _ => None,
        }
    }
}

impl From<StateError> for ProfileError {
    fn from(error: StateError) -> Self {
        Self::State(error)
    }
}

/// Reads only the well-known manifest at a source checkout root.
pub fn discover_source_profiles(
    source_root: &Path,
) -> Result<Option<SourceProfileManifest>, ProfileError> {
    let path = source_root.join(MANIFEST_FILE);
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(ProfileError::Io { path, source }),
    };
    let raw: RawManifest =
        serde_yaml::from_str(&contents).map_err(|error| ProfileError::MalformedManifest {
            path: path.clone(),
            message: error.to_string(),
        })?;
    if raw.version != 1 {
        return Err(ProfileError::UnsupportedVersion {
            path,
            version: raw.version,
        });
    }
    let mut profiles = BTreeMap::new();
    for (name, value) in raw.profiles {
        let block = serde_yaml::from_value::<Block>(value).map_err(|error| {
            ProfileError::InvalidProfile {
                profile: name.clone(),
                message: error.to_string(),
            }
        })?;
        switchyard_planner::validate_block(&name, &block).map_err(|diagnostics| {
            ProfileError::InvalidProfile {
                profile: name.clone(),
                message: diagnostics
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; "),
            }
        })?;
        profiles.insert(name, block);
    }
    Ok(Some(SourceProfileManifest {
        version: raw.version,
        profiles,
    }))
}

fn canonical_definition_json(block: &Block) -> Result<String, ProfileError> {
    let value = serde_json::to_value(block).map_err(|error| {
        ProfileError::Definition(format!("could not serialize profile definition: {error}"))
    })?;
    serde_json::to_string(&sort_json(value)).map_err(|error| {
        ProfileError::Definition(format!("could not serialize profile definition: {error}"))
    })
}

fn sort_json(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(sort_json).collect()),
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, sort_json(value)))
                .collect(),
        ),
        scalar => scalar,
    }
}

#[derive(Clone, Debug)]
pub struct DiscoveredSourceProfiles {
    pub source: String,
    pub commit: Option<String>,
    pub manifest: SourceProfileManifest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProfileOrigin {
    Project,
    ImportedFromSource {
        source: String,
        commit: Option<String>,
    },
    DiscoveredInSource {
        source: String,
        commit: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileTrust {
    Trusted,
    Imported,
    Changed,
    NotImported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileAdapterKind {
    Container,
    Script,
    ProcessCompose,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProfileService {
    pub name: String,
    pub adapter_kind: ProfileAdapterKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProfileRow {
    pub name: String,
    pub origin: ProfileOrigin,
    pub trust: ProfileTrust,
    pub shadowed: bool,
    pub services: Vec<ProfileService>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceManifestError {
    pub source: String,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct ProfileListing {
    pub rows: Vec<ProfileRow>,
    pub source_errors: Vec<SourceManifestError>,
}

/// Builds profile rows in project, imported, then discovered precedence order.
pub fn project_profile_rows(
    project_blocks: &BTreeMap<String, Block>,
    imported: &[ImportedProfile],
    discovered: &[DiscoveredSourceProfiles],
) -> Result<Vec<ProfileRow>, ProfileError> {
    let mut rows = project_blocks
        .iter()
        .map(|(name, block)| {
            profile_row(
                name,
                ProfileOrigin::Project,
                ProfileTrust::Trusted,
                false,
                block,
            )
        })
        .collect::<Vec<_>>();

    for profile in imported {
        let block: Block = serde_json::from_str(&profile.definition_json).map_err(|error| {
            ProfileError::InvalidProfile {
                profile: profile.name.clone(),
                message: format!("stored definition is invalid: {error}"),
            }
        })?;
        let changed = discovered
            .iter()
            .find(|source| source.source == profile.source_name)
            .and_then(|source| source.manifest.profiles.get(&profile.name))
            .is_some_and(|current| current.content_hash() != profile.content_hash);
        let trust = if changed {
            ProfileTrust::Changed
        } else {
            ProfileTrust::Imported
        };
        rows.push(profile_row(
            &profile.name,
            ProfileOrigin::ImportedFromSource {
                source: profile.source_name.clone(),
                commit: profile.source_commit.clone(),
            },
            trust,
            project_blocks.contains_key(&profile.name),
            &block,
        ));
    }

    for source in discovered {
        for (name, block) in &source.manifest.profiles {
            if imported.iter().any(|profile| profile.name == *name) {
                continue;
            }
            rows.push(profile_row(
                name,
                ProfileOrigin::DiscoveredInSource {
                    source: source.source.clone(),
                    commit: source.commit.clone(),
                },
                ProfileTrust::NotImported,
                project_blocks.contains_key(name),
                block,
            ));
        }
    }
    Ok(rows)
}

fn profile_row(
    name: &str,
    origin: ProfileOrigin,
    trust: ProfileTrust,
    shadowed: bool,
    block: &Block,
) -> ProfileRow {
    let services = block
        .services
        .iter()
        .map(|(name, service)| ProfileService {
            name: name.clone(),
            adapter_kind: match service.execution {
                Execution::Container { .. } => ProfileAdapterKind::Container,
                Execution::Script { .. } => ProfileAdapterKind::Script,
                Execution::ProcessCompose { .. } => ProfileAdapterKind::ProcessCompose,
            },
        })
        .collect();
    ProfileRow {
        name: name.into(),
        origin,
        trust,
        shadowed,
        services,
    }
}

/// Loads project, imported, and currently discovered profiles for an interactive client.
pub fn list_profiles(
    project_dir: &Path,
    deployment_definition: &Path,
) -> Result<ProfileListing, ProfileError> {
    let bundle = switchyard_planner::load_bundle(deployment_definition)
        .map_err(|error| ProfileError::Definition(error.to_string()))?;
    let store = StateStore::open(project_dir.join(".switchyard/state.sqlite3"))?.0;
    let imported = store.imported_profiles()?;
    let sources = SourceManager::new(project_dir)
        .list(&store)
        .map_err(|error| ProfileError::Definition(error.to_string()))?;
    let (discovered, source_errors) = discover_registered_sources(&sources);
    let rows = project_profile_rows(&bundle.spec.blocks, &imported, &discovered)?;
    Ok(ProfileListing {
        rows,
        source_errors,
    })
}

fn discover_registered_sources(
    sources: &[RegisteredSourceInspection],
) -> (Vec<DiscoveredSourceProfiles>, Vec<SourceManifestError>) {
    let mut discovered = Vec::new();
    let mut source_errors = Vec::new();
    for source in sources {
        match discover_source_profiles(&source.source.path) {
            Ok(Some(manifest)) => discovered.push(DiscoveredSourceProfiles {
                source: source.source.name.clone(),
                commit: source.inspection.identity.commit.clone(),
                manifest,
            }),
            Ok(None) => {}
            Err(error) => source_errors.push(SourceManifestError {
                source: source.source.name.clone(),
                message: error.to_string(),
            }),
        }
    }
    (discovered, source_errors)
}

/// Re-parses the selected profile through the domain layer for inspection.
pub fn load_profile_block(
    project_dir: &Path,
    deployment_definition: &Path,
    name: &str,
    origin: &ProfileOrigin,
) -> Result<Block, ProfileError> {
    match origin {
        ProfileOrigin::Project => {
            let bundle = switchyard_planner::load_bundle(deployment_definition)
                .map_err(|error| ProfileError::Definition(error.to_string()))?;
            bundle.spec.blocks.get(name).cloned().ok_or_else(|| {
                ProfileError::Definition(format!("project profile `{name}` no longer exists"))
            })
        }
        ProfileOrigin::ImportedFromSource { .. } => {
            let store = StateStore::open(project_dir.join(".switchyard/state.sqlite3"))?.0;
            let profile = store.imported_profile(name)?.ok_or_else(|| {
                ProfileError::Definition(format!("imported profile `{name}` no longer exists"))
            })?;
            serde_json::from_str(&profile.definition_json).map_err(|error| {
                ProfileError::InvalidProfile {
                    profile: name.into(),
                    message: format!("stored definition is invalid: {error}"),
                }
            })
        }
        ProfileOrigin::DiscoveredInSource { source, .. } => {
            load_source_profile_block(project_dir, source, name)
        }
    }
}

/// Re-discovers one source definition for the explicit import review.
pub fn load_source_profile_block(
    project_dir: &Path,
    source_name: &str,
    profile_name: &str,
) -> Result<Block, ProfileError> {
    let store = StateStore::open(project_dir.join(".switchyard/state.sqlite3"))?.0;
    let source = store
        .source(source_name)?
        .ok_or_else(|| ProfileError::SourceNotFound {
            source: source_name.into(),
        })?;
    let manifest =
        discover_source_profiles(&source.path)?.ok_or_else(|| ProfileError::ProfileNotFound {
            source: source_name.into(),
            profile: profile_name.into(),
        })?;
    manifest
        .profiles
        .get(profile_name)
        .cloned()
        .ok_or_else(|| ProfileError::ProfileNotFound {
            source: source_name.into(),
            profile: profile_name.into(),
        })
}

/// Re-discovers, validates, and records one explicitly selected source-local profile.
pub fn import_source_profile(
    project_dir: &Path,
    source_name: &str,
    profile_name: &str,
) -> Result<ImportedProfile, ProfileError> {
    let store = StateStore::open(project_dir.join(".switchyard/state.sqlite3"))?.0;
    let source = store
        .source(source_name)?
        .ok_or_else(|| ProfileError::SourceNotFound {
            source: source_name.into(),
        })?;
    let inspection =
        SourceManager::new(project_dir).inspect(&source.path, source.requested_ref.as_deref());
    let manifest =
        discover_source_profiles(&source.path)?.ok_or_else(|| ProfileError::ProfileNotFound {
            source: source_name.into(),
            profile: profile_name.into(),
        })?;
    let block =
        manifest
            .profiles
            .get(profile_name)
            .ok_or_else(|| ProfileError::ProfileNotFound {
                source: source_name.into(),
                profile: profile_name.into(),
            })?;
    switchyard_planner::validate_block(profile_name, block).map_err(|diagnostics| {
        ProfileError::InvalidProfile {
            profile: profile_name.into(),
            message: diagnostics
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; "),
        }
    })?;
    let imported = ImportedProfile {
        name: profile_name.into(),
        source_name: source_name.into(),
        source_commit: inspection.identity.commit,
        content_hash: block.content_hash(),
        definition_json: canonical_definition_json(block)?,
        imported_at: now_millis()?,
    };
    store.record_imported_profile(&imported)?;
    Ok(imported)
}

/// Removes one imported profile from project state.
pub fn remove_imported_profile(project_dir: &Path, profile_name: &str) -> Result<(), ProfileError> {
    let store = StateStore::open(project_dir.join(".switchyard/state.sqlite3"))?.0;
    store.remove_imported_profile(profile_name)?;
    Ok(())
}

fn now_millis() -> Result<i64, ProfileError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ProfileError::Clock)?;
    i64::try_from(duration.as_millis()).map_err(|_| ProfileError::Clock)
}

#[cfg(test)]
mod tests {
    use super::*;
    use switchyard_state::{RegisteredSource, RegisteredSourceKind};
    use tempfile::TempDir;

    const VALID_MANIFEST: &str = r#"
version: 1
profiles:
  api:
    parameters:
      LOG_LEVEL:
        required: false
    services:
      web:
        execution:
          type: container
          image: busybox:latest
          command: [sleep, infinity]
"#;

    fn write_manifest(root: &Path, contents: &str) {
        fs::create_dir_all(root).unwrap();
        fs::write(root.join(MANIFEST_FILE), contents).unwrap();
    }

    fn block(contents: &str) -> Block {
        serde_yaml::from_str(contents).unwrap()
    }

    fn register_named_source(project: &Path, name: &str, source_root: &Path) {
        let store = StateStore::open(project.join(".switchyard/state.sqlite3"))
            .unwrap()
            .0;
        store
            .register_source(&RegisteredSource {
                name: name.into(),
                kind: RegisteredSourceKind::Unmanaged,
                path: source_root.to_path_buf(),
                repository_path: None,
                requested_ref: None,
                created_at: 1,
                managed_relative_path: None,
            })
            .unwrap();
    }

    fn register_source(project: &Path, source_root: &Path) {
        register_named_source(project, "checkout", source_root);
    }

    #[test]
    fn manifest_discovery_accepts_valid_and_absent_files() {
        let temp = TempDir::new().unwrap();
        assert!(discover_source_profiles(temp.path()).unwrap().is_none());
        write_manifest(temp.path(), VALID_MANIFEST);
        let manifest = discover_source_profiles(temp.path()).unwrap().unwrap();
        assert_eq!(manifest.version, 1);
        assert_eq!(manifest.profiles["api"].services.len(), 1);
    }

    #[test]
    fn manifest_discovery_reports_malformed_version_and_named_invalid_profile() {
        let malformed = TempDir::new().unwrap();
        write_manifest(malformed.path(), "version: [");
        assert!(matches!(
            discover_source_profiles(malformed.path()).unwrap_err(),
            ProfileError::MalformedManifest { .. }
        ));

        let version = TempDir::new().unwrap();
        write_manifest(version.path(), "version: 2\nprofiles: {}\n");
        assert!(matches!(
            discover_source_profiles(version.path()).unwrap_err(),
            ProfileError::UnsupportedVersion { version: 2, .. }
        ));

        let invalid = TempDir::new().unwrap();
        write_manifest(
            invalid.path(),
            "version: 1\nprofiles:\n  broken:\n    services: {}\n",
        );
        assert!(matches!(
            discover_source_profiles(invalid.path()).unwrap_err(),
            ProfileError::InvalidProfile { profile, .. } if profile == "broken"
        ));
    }

    #[test]
    fn content_hash_is_independent_of_yaml_key_order() {
        let left = block(
            r#"
services:
  web:
    execution:
      type: container
      image: busybox:latest
      command: [sleep, infinity]
parameters:
  B: { default: two }
  A: { default: one }
"#,
        );
        let right = block(
            r#"
parameters:
  A: { default: one }
  B: { default: two }
services:
  web:
    execution:
      command: [sleep, infinity]
      image: busybox:latest
      type: container
"#,
        );
        assert_eq!(left.content_hash(), right.content_hash());
    }

    #[test]
    fn projection_orders_precedence_and_marks_project_shadowing() {
        let project_block =
            block("services:\n  project:\n    execution: { type: container, image: busybox }\n");
        let imported_block = block(
            "services:\n  imported:\n    execution: { type: script, image: busybox, command: [run] }\n",
        );
        let discovered_block =
            block("services:\n  found:\n    execution: { type: container, image: busybox }\n");
        let projects = BTreeMap::from([("api".into(), project_block)]);
        let imported = vec![ImportedProfile {
            name: "api".into(),
            source_name: "checkout".into(),
            source_commit: Some("abc".into()),
            content_hash: imported_block.content_hash(),
            definition_json: canonical_definition_json(&imported_block).unwrap(),
            imported_at: 1,
        }];
        let discovered = vec![DiscoveredSourceProfiles {
            source: "other".into(),
            commit: None,
            manifest: SourceProfileManifest {
                version: 1,
                profiles: BTreeMap::from([("extra".into(), discovered_block)]),
            },
        }];
        let rows = project_profile_rows(&projects, &imported, &discovered).unwrap();
        assert_eq!(
            rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
            vec!["api", "api", "extra"]
        );
        assert_eq!(rows[0].origin, ProfileOrigin::Project);
        assert!(!rows[0].shadowed);
        assert_eq!(rows[1].trust, ProfileTrust::Imported);
        assert!(rows[1].shadowed);
        assert_eq!(rows[1].services[0].adapter_kind, ProfileAdapterKind::Script);
        assert_eq!(rows[2].trust, ProfileTrust::NotImported);
    }

    #[test]
    fn projection_detects_changed_manifest_after_edit() {
        let source = TempDir::new().unwrap();
        write_manifest(source.path(), VALID_MANIFEST);
        let original = discover_source_profiles(source.path()).unwrap().unwrap();
        let original_block = &original.profiles["api"];
        let imported = vec![ImportedProfile {
            name: "api".into(),
            source_name: "checkout".into(),
            source_commit: None,
            content_hash: original_block.content_hash(),
            definition_json: canonical_definition_json(original_block).unwrap(),
            imported_at: 1,
        }];
        write_manifest(
            source.path(),
            &VALID_MANIFEST.replace("busybox:latest", "busybox:stable"),
        );
        let current = discover_source_profiles(source.path()).unwrap().unwrap();
        let discovered = vec![DiscoveredSourceProfiles {
            source: "checkout".into(),
            commit: None,
            manifest: current,
        }];
        let rows = project_profile_rows(&BTreeMap::new(), &imported, &discovered).unwrap();
        assert_eq!(rows[0].trust, ProfileTrust::Changed);
    }

    #[test]
    fn import_remove_round_trip_and_unknown_profile_refusal() {
        let project = TempDir::new().unwrap();
        let source = project.path().join("source");
        write_manifest(&source, VALID_MANIFEST);
        register_source(project.path(), &source);

        let imported = import_source_profile(project.path(), "checkout", "api").unwrap();
        let store = StateStore::open(project.path().join(".switchyard/state.sqlite3"))
            .unwrap()
            .0;
        assert_eq!(store.imported_profile("api").unwrap(), Some(imported));
        assert!(matches!(
            import_source_profile(project.path(), "checkout", "missing").unwrap_err(),
            ProfileError::ProfileNotFound { profile, .. } if profile == "missing"
        ));
        remove_imported_profile(project.path(), "api").unwrap();
        assert!(store.imported_profile("api").unwrap().is_none());
    }

    #[test]
    fn listing_keeps_good_profiles_when_another_source_manifest_is_broken() {
        let project = TempDir::new().unwrap();
        fs::write(
            project.path().join("deployment.yaml"),
            "apiVersion: switchyard.dev/v1alpha1\nkind: Deployment\nmetadata:\n  name: demo\nspec: {}\n",
        )
        .unwrap();
        let good = project.path().join("good");
        let broken = project.path().join("broken");
        write_manifest(&good, VALID_MANIFEST);
        write_manifest(&broken, "version: [");
        register_named_source(project.path(), "good", &good);
        register_named_source(project.path(), "broken", &broken);

        let listing = list_profiles(project.path(), &project.path().join("deployment.yaml"))
            .expect("one broken source is a diagnostic, not a listing failure");
        assert!(listing.rows.iter().any(|row| {
            row.name == "api"
                && row.origin
                    == ProfileOrigin::DiscoveredInSource {
                        source: "good".into(),
                        commit: None,
                    }
        }));
        assert_eq!(listing.source_errors.len(), 1);
        assert_eq!(listing.source_errors[0].source, "broken");
        assert!(
            listing.source_errors[0]
                .message
                .contains("invalid profile manifest")
        );
    }
}
