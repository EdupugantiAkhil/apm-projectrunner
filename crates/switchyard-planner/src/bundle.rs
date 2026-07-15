use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    Bundle, Execution, Overlay, OverlayFileSource, OverlayValue, SourceType, load_bundle,
    load_overlay, plan_with_overlays,
};

pub const PORTABLE_BUNDLE_API_VERSION: &str = "switchyard.dev/bundle/v1alpha1";
const CREATED_AT_DETERMINISTIC: &str = "1970-01-01T00:00:00Z";
const SOURCE_TOOL_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportBundleOptions {
    pub overlays: Vec<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct ExportBundleResult {
    pub bundle: PortableBundle,
    pub warnings: Vec<BundleWarning>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportBundleOptions {
    pub force: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportBundleResult {
    pub deployment_name: String,
    pub definition_path: PathBuf,
    pub overlay_paths: Vec<PathBuf>,
    pub required_local_inputs: Vec<RequiredLocalInput>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PortableBundle {
    pub api_version: String,
    pub metadata: PortableBundleMetadata,
    pub deployment: Bundle,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub overlays: BTreeMap<String, Overlay>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_local_inputs: Vec<RequiredLocalInput>,
    pub content_hash: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PortableBundleMetadata {
    pub deployment_name: String,
    pub created_at: String,
    pub source_tool_version: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Ord, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequiredLocalInput {
    pub name: String,
    pub kind: RequiredLocalInputKind,
    pub expected_shape: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scaffold_paths: Vec<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Ord, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RequiredLocalInputKind {
    SourceDirectory,
    File,
    DotenvFile,
    EnvironmentValue,
    ParameterValue,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundleWarning {
    pub code: BundleWarningCode,
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BundleWarningCode {
    CredentialLikeKey,
    LocalPathReplaced,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundleError {
    pub code: BundleErrorCode,
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BundleErrorCode {
    BundleReadFailed,
    BundleInvalidJson,
    BundleUnsupportedApiVersion,
    BundleHashMismatch,
    BundleWriteConflict,
    BundleWriteFailed,
    BundleValidationFailed,
}

impl fmt::Display for BundleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} at {}: {}",
            self.code.as_str(),
            self.path,
            self.message
        )
    }
}

impl std::error::Error for BundleError {}

impl BundleErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BundleReadFailed => "bundle_read_failed",
            Self::BundleInvalidJson => "bundle_invalid_json",
            Self::BundleUnsupportedApiVersion => "bundle_unsupported_api_version",
            Self::BundleHashMismatch => "bundle_hash_mismatch",
            Self::BundleWriteConflict => "bundle_write_conflict",
            Self::BundleWriteFailed => "bundle_write_failed",
            Self::BundleValidationFailed => "bundle_validation_failed",
        }
    }
}

impl fmt::Display for BundleErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HashPayload<'a> {
    api_version: &'a str,
    metadata: &'a PortableBundleMetadata,
    deployment: &'a Bundle,
    overlays: &'a BTreeMap<String, Overlay>,
    required_local_inputs: &'a [RequiredLocalInput],
}

pub fn export_portable_bundle(
    deployment_path: &Path,
    options: &ExportBundleOptions,
) -> Result<ExportBundleResult, Box<dyn std::error::Error>> {
    let mut deployment = load_bundle(deployment_path)?;
    let mut context = Sanitizer::new();
    context.sanitize_deployment(&mut deployment);

    let mut overlay_paths = deployment
        .spec
        .overlays
        .iter()
        .cloned()
        .map(|path| (path, true))
        .collect::<Vec<_>>();
    overlay_paths.extend(options.overlays.iter().cloned().map(|path| (path, false)));

    let mut overlays = BTreeMap::new();
    let mut imported_overlay_paths = Vec::new();
    for (path, definition_relative) in overlay_paths {
        let resolved = if path.is_absolute() || !definition_relative && path.exists() {
            path
        } else {
            deployment.definition_dir.join(path)
        };
        let mut overlay = load_overlay(&resolved)?;
        context.sanitize_overlay(&mut overlay);
        let file_name = format!("overlays/{}.yaml", overlay.metadata.name);
        if !imported_overlay_paths.contains(&PathBuf::from(&file_name)) {
            imported_overlay_paths.push(PathBuf::from(&file_name));
        }
        overlays.insert(overlay.metadata.name.clone(), overlay);
    }
    deployment.spec.overlays = imported_overlay_paths;

    let metadata = PortableBundleMetadata {
        deployment_name: deployment.metadata.name.clone(),
        created_at: CREATED_AT_DETERMINISTIC.into(),
        source_tool_version: SOURCE_TOOL_VERSION.into(),
    };
    let mut required_local_inputs = context.required.into_values().collect::<Vec<_>>();
    required_local_inputs.sort();
    for input in &mut required_local_inputs {
        input.scaffold_paths.sort();
        input.scaffold_paths.dedup();
    }
    let mut bundle = PortableBundle {
        api_version: PORTABLE_BUNDLE_API_VERSION.into(),
        metadata,
        deployment,
        overlays,
        required_local_inputs,
        content_hash: String::new(),
    };
    bundle.content_hash = content_hash(&bundle)?;
    Ok(ExportBundleResult {
        bundle,
        warnings: context.warnings,
    })
}

pub fn write_portable_bundle(path: &Path, bundle: &PortableBundle) -> Result<(), BundleError> {
    let bytes = serde_json::to_vec_pretty(bundle).map_err(|error| BundleError {
        code: BundleErrorCode::BundleInvalidJson,
        path: "$".into(),
        message: error.to_string(),
    })?;
    fs::write(path, bytes).map_err(|error| BundleError {
        code: BundleErrorCode::BundleWriteFailed,
        path: path.display().to_string(),
        message: error.to_string(),
    })
}

pub fn read_portable_bundle(path: &Path) -> Result<PortableBundle, BundleError> {
    let input = fs::read_to_string(path).map_err(|error| BundleError {
        code: BundleErrorCode::BundleReadFailed,
        path: path.display().to_string(),
        message: error.to_string(),
    })?;
    parse_portable_bundle(&input)
}

pub fn parse_portable_bundle(input: &str) -> Result<PortableBundle, BundleError> {
    let bundle: PortableBundle = serde_json::from_str(input).map_err(|error| BundleError {
        code: BundleErrorCode::BundleInvalidJson,
        path: "$".into(),
        message: error.to_string(),
    })?;
    verify_portable_bundle(bundle)
}

pub fn verify_portable_bundle(bundle: PortableBundle) -> Result<PortableBundle, BundleError> {
    if bundle.api_version != PORTABLE_BUNDLE_API_VERSION {
        return Err(BundleError {
            code: BundleErrorCode::BundleUnsupportedApiVersion,
            path: "apiVersion".into(),
            message: format!("expected {PORTABLE_BUNDLE_API_VERSION}"),
        });
    }
    let expected = content_hash(&bundle)?;
    if expected != bundle.content_hash {
        return Err(BundleError {
            code: BundleErrorCode::BundleHashMismatch,
            path: "contentHash".into(),
            message: format!("expected {expected}, found {}", bundle.content_hash),
        });
    }
    reject_machine_state(&bundle)?;
    Ok(bundle)
}

pub fn import_portable_bundle(
    bundle_path: &Path,
    into: &Path,
    options: &ImportBundleOptions,
) -> Result<ImportBundleResult, BundleError> {
    let bundle = read_portable_bundle(bundle_path)?;
    write_imported_bundle(&bundle, into, options)
}

pub fn write_imported_bundle(
    bundle: &PortableBundle,
    into: &Path,
    options: &ImportBundleOptions,
) -> Result<ImportBundleResult, BundleError> {
    let deployment_name = bundle.deployment.metadata.name.clone();
    let definition_path = into.join(format!("{deployment_name}.yaml"));
    let overlays_dir = into.join("overlays");
    let mut overlay_paths = Vec::new();

    create_dir_all(into)?;
    // Check every destination before writing anything so a conflict cannot
    // leave a partially imported bundle behind.
    if !options.force {
        if definition_path.exists() {
            return Err(write_conflict(&definition_path));
        }
        for name in bundle.overlays.keys() {
            let path = overlays_dir.join(format!("{name}.yaml"));
            if path.exists() {
                return Err(write_conflict(&path));
            }
        }
    }
    let deployment_yaml =
        serde_yaml::to_string(&bundle.deployment).map_err(|error| BundleError {
            code: BundleErrorCode::BundleWriteFailed,
            path: definition_path.display().to_string(),
            message: error.to_string(),
        })?;
    fs::write(&definition_path, deployment_yaml).map_err(|error| BundleError {
        code: BundleErrorCode::BundleWriteFailed,
        path: definition_path.display().to_string(),
        message: error.to_string(),
    })?;

    if !bundle.overlays.is_empty() {
        create_dir_all(&overlays_dir)?;
    }
    for (name, overlay) in &bundle.overlays {
        let path = overlays_dir.join(format!("{name}.yaml"));
        if !options.force && path.exists() {
            return Err(write_conflict(&path));
        }
        let yaml = serde_yaml::to_string(overlay).map_err(|error| BundleError {
            code: BundleErrorCode::BundleWriteFailed,
            path: path.display().to_string(),
            message: error.to_string(),
        })?;
        fs::write(&path, yaml).map_err(|error| BundleError {
            code: BundleErrorCode::BundleWriteFailed,
            path: path.display().to_string(),
            message: error.to_string(),
        })?;
        overlay_paths.push(path);
    }

    scaffold_required_inputs(into, &bundle.required_local_inputs)?;
    let loaded = load_bundle(&definition_path).map_err(|error| BundleError {
        code: BundleErrorCode::BundleValidationFailed,
        path: definition_path.display().to_string(),
        message: error.to_string(),
    })?;
    plan_with_overlays(&loaded, &crate::OverlayOptions::default()).map_err(|diagnostics| {
        BundleError {
            code: BundleErrorCode::BundleValidationFailed,
            path: definition_path.display().to_string(),
            message: serde_json::to_string(&diagnostics).unwrap_or_else(|_| "invalid".into()),
        }
    })?;

    Ok(ImportBundleResult {
        deployment_name,
        definition_path,
        overlay_paths,
        required_local_inputs: bundle.required_local_inputs.clone(),
    })
}

fn content_hash(bundle: &PortableBundle) -> Result<String, BundleError> {
    let payload = HashPayload {
        api_version: &bundle.api_version,
        metadata: &bundle.metadata,
        deployment: &bundle.deployment,
        overlays: &bundle.overlays,
        required_local_inputs: &bundle.required_local_inputs,
    };
    let bytes = serde_json::to_vec(&payload).map_err(|error| BundleError {
        code: BundleErrorCode::BundleInvalidJson,
        path: "$".into(),
        message: error.to_string(),
    })?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn reject_machine_state(bundle: &PortableBundle) -> Result<(), BundleError> {
    for (name, source) in &bundle.deployment.spec.sources {
        if path_is_machine_state(&source.path) {
            return Err(machine_state_error(format!(
                "deployment.spec.sources.{name}.path"
            )));
        }
        if source
            .repository
            .as_ref()
            .is_some_and(|path| path_is_machine_state(path))
        {
            return Err(machine_state_error(format!(
                "deployment.spec.sources.{name}.repository"
            )));
        }
    }
    for (name, overlay) in &bundle.overlays {
        for (index, path) in overlay.spec.environment.env_files.iter().enumerate() {
            if path_is_machine_state(path) {
                return Err(machine_state_error(format!(
                    "overlays.{name}.spec.environment.envFiles[{index}]"
                )));
            }
        }
        for (index, file) in overlay.spec.files.iter().enumerate() {
            if let Some(OverlayFileSource::Path(path)) = &file.source {
                if path_is_machine_state(path) {
                    return Err(machine_state_error(format!(
                        "overlays.{name}.spec.files[{index}].source"
                    )));
                }
            }
            if path_has_switchyard_segment(&file.target) {
                return Err(machine_state_error(format!(
                    "overlays.{name}.spec.files[{index}].target"
                )));
            }
        }
    }
    Ok(())
}

fn machine_state_error(path: String) -> BundleError {
    BundleError {
        code: BundleErrorCode::BundleValidationFailed,
        path,
        message: "portable bundles must not contain absolute host paths or .switchyard state"
            .into(),
    }
}

fn path_is_machine_state(path: &Path) -> bool {
    path.is_absolute() || path_has_switchyard_segment(path)
}

fn path_has_switchyard_segment(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == ".switchyard")
}

fn create_dir_all(path: &Path) -> Result<(), BundleError> {
    fs::create_dir_all(path).map_err(|error| BundleError {
        code: BundleErrorCode::BundleWriteFailed,
        path: path.display().to_string(),
        message: error.to_string(),
    })
}

fn write_conflict(path: &Path) -> BundleError {
    BundleError {
        code: BundleErrorCode::BundleWriteConflict,
        path: path.display().to_string(),
        message: "refusing to overwrite existing file without --force".into(),
    }
}

fn scaffold_required_inputs(into: &Path, inputs: &[RequiredLocalInput]) -> Result<(), BundleError> {
    for input in inputs {
        let root = into.join("required-local-inputs").join(&input.name);
        match input.kind {
            RequiredLocalInputKind::SourceDirectory => create_dir_all(&root)?,
            RequiredLocalInputKind::File | RequiredLocalInputKind::DotenvFile => {
                if let Some(parent) = root.parent() {
                    create_dir_all(parent)?;
                }
                if !root.exists() {
                    fs::write(&root, b"").map_err(|error| BundleError {
                        code: BundleErrorCode::BundleWriteFailed,
                        path: root.display().to_string(),
                        message: error.to_string(),
                    })?;
                }
            }
            RequiredLocalInputKind::EnvironmentValue | RequiredLocalInputKind::ParameterValue => {}
        }
        for relative in &input.scaffold_paths {
            let path = root.join(relative);
            if is_file_like(relative) {
                if let Some(parent) = path.parent() {
                    create_dir_all(parent)?;
                }
                if !path.exists() {
                    fs::write(&path, b"").map_err(|error| BundleError {
                        code: BundleErrorCode::BundleWriteFailed,
                        path: path.display().to_string(),
                        message: error.to_string(),
                    })?;
                }
            } else {
                create_dir_all(&path)?;
            }
        }
    }
    Ok(())
}

fn is_file_like(path: &Path) -> bool {
    path.extension().is_some()
}

struct Sanitizer {
    required: BTreeMap<String, RequiredLocalInput>,
    warnings: Vec<BundleWarning>,
}

impl Sanitizer {
    fn new() -> Self {
        Self {
            required: BTreeMap::new(),
            warnings: Vec::new(),
        }
    }

    fn sanitize_deployment(&mut self, deployment: &mut Bundle) {
        let mut source_blocks = BTreeMap::<String, BTreeSet<String>>::new();
        for instance in &deployment.spec.instances {
            source_blocks
                .entry(instance.source.clone())
                .or_default()
                .insert(instance.block.clone());
        }
        for (name, source) in &mut deployment.spec.sources {
            let input_name = format!("source-{name}");
            let mut input = RequiredLocalInput {
                name: input_name.clone(),
                kind: RequiredLocalInputKind::SourceDirectory,
                expected_shape:
                    "directory containing the source tree expected by referenced blocks".into(),
                description: format!("Local source directory for deployment source `{name}`"),
                scaffold_paths: Vec::new(),
            };
            if let Some(blocks) = source_blocks.get(name) {
                for block_name in blocks {
                    if let Some(block) = deployment.spec.blocks.get(block_name) {
                        for service in block.services.values() {
                            match &service.execution {
                                Execution::Container {
                                    build: Some(build), ..
                                } => input.scaffold_paths.push(build.context.clone()),
                                Execution::ProcessCompose { file, .. } => {
                                    input.scaffold_paths.push(file.clone())
                                }
                                Execution::Container { .. } | Execution::Script { .. } => {}
                            }
                        }
                    }
                }
            }
            self.require(input);
            if matches!(source.r#type, SourceType::Worktree) {
                source.repository = None;
            }
            source.path = PathBuf::from("required-local-inputs").join(&input_name);
            self.warnings.push(BundleWarning {
                code: BundleWarningCode::LocalPathReplaced,
                path: format!("spec.sources.{name}.path"),
                message: format!("source path was replaced by required local input `{input_name}`"),
            });
        }
        for (block_name, block) in &mut deployment.spec.blocks {
            for (service_name, service) in &mut block.services {
                self.sanitize_execution_environment(
                    &mut service.execution,
                    &format!(
                        "spec.blocks.{block_name}.services.{service_name}.execution.environment"
                    ),
                );
            }
        }
        for (index, instance) in deployment.spec.instances.iter_mut().enumerate() {
            sanitize_string_map(
                &mut instance.environment,
                &mut self.required,
                &mut self.warnings,
                &format!("spec.instances[{index}].environment"),
                RequiredLocalInputKind::EnvironmentValue,
            );
            sanitize_string_map(
                &mut instance.parameters,
                &mut self.required,
                &mut self.warnings,
                &format!("spec.instances[{index}].parameters"),
                RequiredLocalInputKind::ParameterValue,
            );
        }
    }

    fn sanitize_overlay(&mut self, overlay: &mut Overlay) {
        for path in &mut overlay.spec.environment.env_files {
            let name = format!("dotenv-{}", slug(path.to_string_lossy().as_ref()));
            self.require(RequiredLocalInput {
                name: name.clone(),
                kind: RequiredLocalInputKind::DotenvFile,
                expected_shape: "dotenv file with KEY=VALUE lines".into(),
                description: format!("Local dotenv file for overlay `{}`", overlay.metadata.name),
                scaffold_paths: Vec::new(),
            });
            *path = PathBuf::from("../required-local-inputs").join(name);
        }
        for (key, value) in &mut overlay.spec.environment.set {
            if credential_like_key(key) {
                match value {
                    OverlayValue::Literal(_) => {
                        let name = format!("env-{}", slug(key));
                        self.require(RequiredLocalInput {
                            name: name.clone(),
                            kind: RequiredLocalInputKind::EnvironmentValue,
                            expected_shape: "non-empty local environment value".into(),
                            description: format!(
                                "Local value for credential-looking overlay key `{key}`"
                            ),
                            scaffold_paths: Vec::new(),
                        });
                        *value = OverlayValue::Literal(format!("${{{name}}}"));
                    }
                    OverlayValue::Secret(_) => {}
                }
                self.warnings.push(BundleWarning {
                    code: BundleWarningCode::CredentialLikeKey,
                    path: format!(
                        "overlay.{}.spec.environment.set.{key}",
                        overlay.metadata.name
                    ),
                    message: format!("credential-looking key `{key}` needs receiver review"),
                });
            }
        }
        sanitize_string_map(
            &mut overlay.spec.parameters,
            &mut self.required,
            &mut self.warnings,
            &format!("overlay.{}.spec.parameters", overlay.metadata.name),
            RequiredLocalInputKind::ParameterValue,
        );
        for (index, file) in overlay.spec.files.iter_mut().enumerate() {
            if let Some(OverlayFileSource::Path(path)) = &mut file.source {
                let name = format!("file-{}", slug(path.to_string_lossy().as_ref()));
                self.require(RequiredLocalInput {
                    name: name.clone(),
                    kind: RequiredLocalInputKind::File,
                    expected_shape: "regular file readable by the overlay file injector".into(),
                    description: format!(
                        "Local file source for overlay `{}` file #{index}",
                        overlay.metadata.name
                    ),
                    scaffold_paths: Vec::new(),
                });
                *path = PathBuf::from("../required-local-inputs").join(name);
            }
        }
    }

    fn sanitize_execution_environment(&mut self, execution: &mut Execution, path: &str) {
        let environment = match execution {
            Execution::Container { environment, .. }
            | Execution::Script { environment, .. }
            | Execution::ProcessCompose { environment, .. } => environment,
        };
        sanitize_string_map(
            environment,
            &mut self.required,
            &mut self.warnings,
            path,
            RequiredLocalInputKind::EnvironmentValue,
        );
    }

    fn require(&mut self, input: RequiredLocalInput) {
        self.required
            .entry(input.name.clone())
            .and_modify(|existing| existing.scaffold_paths.extend(input.scaffold_paths.clone()))
            .or_insert(input);
    }
}

fn sanitize_string_map(
    values: &mut BTreeMap<String, String>,
    required: &mut BTreeMap<String, RequiredLocalInput>,
    warnings: &mut Vec<BundleWarning>,
    path: &str,
    kind: RequiredLocalInputKind,
) {
    for (key, value) in values {
        if credential_like_key(key) {
            let name = format!("{}-{}", input_prefix(&kind), slug(key));
            required.entry(name.clone()).or_insert(RequiredLocalInput {
                name: name.clone(),
                kind: kind.clone(),
                expected_shape: "non-empty local value".into(),
                description: format!("Local value for credential-looking key `{key}`"),
                scaffold_paths: Vec::new(),
            });
            *value = format!("${{{name}}}");
            warnings.push(BundleWarning {
                code: BundleWarningCode::CredentialLikeKey,
                path: format!("{path}.{key}"),
                message: format!(
                    "credential-looking key `{key}` was replaced by required local input `{name}`"
                ),
            });
        }
    }
}

fn input_prefix(kind: &RequiredLocalInputKind) -> &'static str {
    match kind {
        RequiredLocalInputKind::SourceDirectory => "source",
        RequiredLocalInputKind::File => "file",
        RequiredLocalInputKind::DotenvFile => "dotenv",
        RequiredLocalInputKind::EnvironmentValue => "env",
        RequiredLocalInputKind::ParameterValue => "parameter",
    }
}

fn credential_like_key(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    upper.contains("PASSWORD")
        || upper.contains("PASSWD")
        || upper.contains("SECRET")
        || upper.contains("TOKEN")
        || upper.contains("CREDENTIAL")
        || upper.contains("API_KEY")
        || upper.ends_with("_KEY")
        || upper.contains("PRIVATE_KEY")
        || upper.contains("AUTH")
}

fn slug(value: &str) -> String {
    let slug = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        "input".into()
    } else if slug.len() > 48 {
        let digest = format!("{:x}", Sha256::digest(value.as_bytes()));
        format!("{}-{}", &slug[..39], &digest[..8])
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root")
            .to_owned()
    }

    #[test]
    fn example_deployment_exports_imports_and_validates() {
        let root = repo_root();
        let export = export_portable_bundle(
            &root.join("examples/routing-matrix/deployment.yaml"),
            &ExportBundleOptions {
                overlays: Vec::new(),
            },
        )
        .unwrap();
        let temp = tempdir().unwrap();
        let bundle_path = temp.path().join("routing.switchyard-bundle.json");
        write_portable_bundle(&bundle_path, &export.bundle).unwrap();

        let imported = import_portable_bundle(
            &bundle_path,
            &temp.path().join("imported"),
            &ImportBundleOptions { force: false },
        )
        .unwrap();
        let deployment = load_bundle(&imported.definition_path).unwrap();
        plan_with_overlays(&deployment, &crate::OverlayOptions::default()).unwrap();
        assert!(!imported.required_local_inputs.is_empty());
    }

    #[test]
    fn tampered_bundle_is_rejected_with_stable_code() {
        let root = repo_root();
        let mut export = export_portable_bundle(
            &root.join("examples/routing-matrix/deployment.yaml"),
            &ExportBundleOptions {
                overlays: Vec::new(),
            },
        )
        .unwrap()
        .bundle;
        export.metadata.deployment_name = "changed".into();
        let input = serde_json::to_string(&export).unwrap();
        let error = parse_portable_bundle(&input).unwrap_err();
        assert_eq!(error.code, BundleErrorCode::BundleHashMismatch);
    }

    #[test]
    fn unsupported_bundle_api_version_is_rejected_with_stable_code() {
        let root = repo_root();
        let mut export = export_portable_bundle(
            &root.join("examples/routing-matrix/deployment.yaml"),
            &ExportBundleOptions {
                overlays: Vec::new(),
            },
        )
        .unwrap()
        .bundle;
        export.api_version = "switchyard.dev/bundle/v9".into();
        export.content_hash = content_hash(&export).unwrap();
        let input = serde_json::to_string(&export).unwrap();
        let error = parse_portable_bundle(&input).unwrap_err();
        assert_eq!(error.code, BundleErrorCode::BundleUnsupportedApiVersion);
    }
}
