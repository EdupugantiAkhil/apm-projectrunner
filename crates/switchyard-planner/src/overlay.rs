use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{
    API_VERSION, Bundle, Diagnostic, DiagnosticCode, Execution, PlannerError, generate, validate,
};

/// Overlay document kind.
pub const OVERLAY_KIND: &str = "Overlay";

/// A versioned overlay document.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Overlay {
    pub api_version: String,
    pub kind: String,
    pub metadata: OverlayMetadata,
    pub spec: OverlaySpec,
    #[serde(skip)]
    pub(crate) source_path: PathBuf,
}

/// Overlay identity.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OverlayMetadata {
    pub name: String,
}

/// Selectors and mutations declared by an overlay.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OverlaySpec {
    #[serde(default)]
    pub selectors: OverlaySelectors,
    #[serde(default)]
    pub environment: OverlayEnvironment,
    #[serde(default)]
    pub files: Vec<OverlayFile>,
    #[serde(default)]
    pub parameters: BTreeMap<String, String>,
    #[serde(default)]
    pub routes: BTreeMap<String, OverlayRoute>,
    #[serde(default)]
    pub variables: BTreeMap<String, String>,
    /// Allows keyed entries to replace an earlier overlay entry.
    #[serde(default)]
    pub replace: bool,
}

/// Deployment and instance matching rules.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OverlaySelectors {
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub deployment: LabelSelector,
    #[serde(default)]
    pub instances: InstanceSelector,
}

/// Exact label matching.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LabelSelector {
    #[serde(default)]
    pub match_labels: BTreeMap<String, String>,
}

/// Exact instance-name and label matching.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstanceSelector {
    #[serde(default)]
    pub match_labels: BTreeMap<String, String>,
    #[serde(default)]
    pub names: Vec<String>,
}

/// Environment mutations.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OverlayEnvironment {
    #[serde(default)]
    pub env_files: Vec<PathBuf>,
    #[serde(default)]
    pub set: BTreeMap<String, OverlayValue>,
    #[serde(default)]
    pub unset: Vec<String>,
}

/// A literal value or apply-time secret reference.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum OverlayValue {
    Literal(String),
    Secret(OverlaySecretReference),
}

/// Secret-reference shape mirrored from `switchyard-state` without coupling planner to SQLite.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OverlaySecretReference {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_variable: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<PathBuf>,
}

/// One injected file.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OverlayFile {
    #[serde(default)]
    pub source: Option<OverlayFileSource>,
    #[serde(default)]
    pub content: Option<String>,
    pub target: PathBuf,
    #[serde(default = "default_file_mode")]
    pub mode: String,
    #[serde(default)]
    pub template: bool,
    #[serde(default)]
    pub replace: bool,
}

fn default_file_mode() -> String {
    "0644".into()
}

/// Path or secret source for an injected file.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum OverlayFileSource {
    Path(PathBuf),
    Secret(OverlaySecretReference),
}

/// Route provider selection, optionally replacing an earlier overlay selection.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum OverlayRoute {
    Provider(String),
    Detailed {
        provider: String,
        #[serde(default)]
        replace: bool,
    },
}

impl OverlayRoute {
    fn parts(&self) -> (&str, bool) {
        match self {
            Self::Provider(provider) => (provider, false),
            Self::Detailed { provider, replace } => (provider, *replace),
        }
    }
}

/// A previous layer shadowed during resolution.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShadowedOrigin {
    pub value: String,
    pub layer: String,
}

/// Secret-safe final value and complete precedence history.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OriginTrace {
    pub instance: String,
    pub category: String,
    pub key: String,
    pub value: String,
    pub layer: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shadowed: Vec<ShadowedOrigin>,
}

/// Generated injected-file payload. Content is never serialized into plan previews.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InjectedFilePlan {
    pub instance: String,
    pub target: PathBuf,
    pub mode: u32,
    pub content_hash: String,
    pub relative_path: PathBuf,
    pub origin: String,
    #[serde(skip)]
    pub(crate) content: Vec<u8>,
}

/// Explicit planner options used by overlays and named variations.
#[derive(Clone, Debug, Default)]
pub struct OverlayOptions {
    pub overlays: Vec<PathBuf>,
    pub variation: Option<String>,
    pub set: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct OverlayResolution {
    pub origins: Vec<OriginTrace>,
    pub files: Vec<InjectedFilePlan>,
    pub secret_environment: BTreeMap<(String, String), OverlaySecretReference>,
}

/// Runtime action required by one service change.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeImpact {
    Live,
    Restart,
    Rebuild,
}

/// Deterministic per-service change preview.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceChange {
    pub service: String,
    pub impact: ChangeImpact,
}

/// Apply-time secret environment binding; never serialized into generated artifacts.
#[derive(Clone, Debug)]
pub struct RuntimeSecretPlan {
    pub variable: String,
    pub reference: OverlaySecretReference,
}

/// Loads one overlay document.
pub fn load_overlay(path: &Path) -> Result<Overlay, PlannerError> {
    let input = fs::read_to_string(path).map_err(PlannerError::OverlayIo)?;
    let mut overlay: Overlay = serde_yaml::from_str(&input).map_err(PlannerError::OverlayYaml)?;
    overlay.source_path = path.to_owned();
    Ok(overlay)
}

/// Parses a strict dotenv file without shell evaluation or expansion.
pub fn parse_dotenv(input: &str) -> Result<BTreeMap<String, String>, String> {
    let mut values = BTreeMap::new();
    for (index, raw) in input.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("line {} must be KEY=VALUE", index + 1));
        };
        if !valid_environment_name(key) {
            return Err(format!(
                "line {} has invalid environment key `{key}`",
                index + 1
            ));
        }
        if values.insert(key.into(), value.into()).is_some() {
            return Err(format!(
                "line {} repeats environment key `{key}`",
                index + 1
            ));
        }
    }
    Ok(values)
}

fn valid_environment_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_alphabetic() || index > 0 && byte.is_ascii_digit()
        })
}

/// Validates the standalone overlay schema and context-free safety rules.
pub fn validate_overlay(overlay: &Overlay) -> Result<(), Vec<Diagnostic>> {
    let mut errors = Vec::new();
    if overlay.api_version != API_VERSION || overlay.kind != OVERLAY_KIND {
        errors.push(Diagnostic::new(
            DiagnosticCode::UnsupportedSchema,
            "apiVersion",
            format!("expected {API_VERSION} kind {OVERLAY_KIND}"),
        ));
    }
    if overlay.metadata.name.is_empty() {
        errors.push(Diagnostic::new(
            DiagnosticCode::InvalidName,
            "metadata.name",
            "overlay name cannot be empty",
        ));
    }
    for (key, value) in &overlay.spec.environment.set {
        if !valid_environment_name(key) {
            errors.push(Diagnostic::new(
                DiagnosticCode::InvalidOverlay,
                format!("spec.environment.set.{key}"),
                "environment keys must contain only letters, digits, and underscores and cannot start with a digit",
            ));
        }
        if let OverlayValue::Secret(secret) = value {
            validate_secret(secret, &format!("spec.environment.set.{key}"), &mut errors);
        }
    }
    let mut unset = BTreeSet::new();
    for key in &overlay.spec.environment.unset {
        if !valid_environment_name(key) || !unset.insert(key) {
            errors.push(Diagnostic::new(
                DiagnosticCode::InvalidOverlay,
                "spec.environment.unset",
                format!("invalid or repeated environment key `{key}`"),
            ));
        }
    }
    for (index, file) in overlay.spec.files.iter().enumerate() {
        let path = format!("spec.files[{index}]");
        if file.source.is_some() == file.content.is_some() {
            errors.push(Diagnostic::new(
                DiagnosticCode::InvalidOverlay,
                &path,
                "exactly one of source or content is required",
            ));
        }
        if let Some(OverlayFileSource::Secret(_)) = &file.source {
            errors.push(Diagnostic::new(
                DiagnosticCode::UnsupportedSecret,
                format!("{path}.source"),
                "secret file injection is not yet supported",
            ));
        }
        validate_target(&file.target, &path, &mut errors);
        if parse_mode(&file.mode).is_none() {
            errors.push(Diagnostic::new(
                DiagnosticCode::InvalidOverlay,
                format!("{path}.mode"),
                "mode must be a four-digit octal value no more permissive than 0777",
            ));
        }
        if file.template {
            if let Some(content) = &file.content {
                validate_template_namespace(content, overlay, &path, &mut errors);
            }
        }
        if let Some(OverlayFileSource::Path(source)) = &file.source {
            let base = overlay
                .source_path
                .parent()
                .unwrap_or_else(|| Path::new("."));
            let source = if source.is_absolute() {
                source.clone()
            } else {
                base.join(source)
            };
            match fs::read_to_string(&source) {
                Ok(content) if file.template => {
                    validate_template_namespace(&content, overlay, &path, &mut errors);
                }
                Ok(_) => {}
                Err(error) => errors.push(Diagnostic::new(
                    DiagnosticCode::InvalidPath,
                    format!("{path}.source"),
                    format!("could not read injected file source: {error}"),
                )),
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn validate_secret(secret: &OverlaySecretReference, path: &str, errors: &mut Vec<Diagnostic>) {
    match (&secret.environment_variable, &secret.file) {
        (Some(name), None)
            if !name.is_empty()
                && name.bytes().enumerate().all(|(index, byte)| {
                    byte == b'_' || byte.is_ascii_uppercase() || index > 0 && byte.is_ascii_digit()
                }) => {}
        (None, Some(path_value)) if !path_value.as_os_str().is_empty() => {}
        _ => errors.push(Diagnostic::new(
            DiagnosticCode::InvalidOverlay,
            path,
            "secret reference must contain exactly one valid environmentVariable or file",
        )),
    }
}

fn parse_mode(mode: &str) -> Option<u32> {
    (mode.len() == 4 && mode.starts_with('0'))
        .then(|| u32::from_str_radix(mode, 8).ok())
        .flatten()
        .filter(|mode| *mode <= 0o777)
}

fn validate_target(target: &Path, path: &str, errors: &mut Vec<Diagnostic>) {
    if !target.is_absolute()
        || target
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        errors.push(Diagnostic::new(
            DiagnosticCode::InvalidPath,
            format!("{path}.target"),
            "file target must be an absolute normalized container path without `..` traversal",
        ));
    }
}

fn validate_template_namespace(
    input: &str,
    overlay: &Overlay,
    path: &str,
    errors: &mut Vec<Diagnostic>,
) {
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            errors.push(Diagnostic::new(
                DiagnosticCode::InvalidOverlay,
                path,
                "unterminated template variable",
            ));
            return;
        };
        let variable = &after[..end];
        let valid = variable == "instance.name"
            || variable == "deployment.name"
            || variable
                .strip_prefix("overlay.variables.")
                .is_some_and(|name| overlay.spec.variables.contains_key(name))
            || variable
                .strip_prefix("parameters.")
                .is_some_and(|name| !name.is_empty());
        if !valid {
            errors.push(Diagnostic::new(
                DiagnosticCode::MissingVariable,
                path,
                format!("unknown template variable `{variable}`"),
            ));
        }
        rest = &after[end + 1..];
    }
}

/// Resolves deployment-listed and explicitly supplied overlays, then produces a plan.
pub fn plan_with_overlays(
    bundle: &Bundle,
    options: &OverlayOptions,
) -> Result<crate::Plan, Vec<Diagnostic>> {
    plan_with_overlays_and_devices(bundle, options, &BTreeMap::new())
}

pub fn plan_with_overlays_and_devices(
    bundle: &Bundle,
    options: &OverlayOptions,
    devices: &BTreeMap<String, crate::PlanningDevice>,
) -> Result<crate::Plan, Vec<Diagnostic>> {
    let mut paths = bundle
        .spec
        .overlays
        .iter()
        .cloned()
        .map(|path| (path, true))
        .collect::<Vec<_>>();
    paths.extend(options.overlays.iter().cloned().map(|path| (path, false)));
    if paths.is_empty() && options.variation.is_none() && options.set.is_empty() {
        return crate::plan_with_devices(bundle, devices);
    }
    let mut overlays = Vec::new();
    let mut load_errors = Vec::new();
    for (path, definition_relative) in paths {
        let resolved = if path.is_absolute() || !definition_relative && path.exists() {
            path
        } else {
            bundle.definition_dir.join(path)
        };
        match load_overlay(&resolved) {
            Ok(overlay) => overlays.push(overlay),
            Err(error) => load_errors.push(Diagnostic::new(
                DiagnosticCode::InvalidOverlay,
                resolved.display().to_string(),
                error.to_string(),
            )),
        }
    }
    if !load_errors.is_empty() {
        return Err(load_errors);
    }
    let (resolved, mut resolution) = resolve(bundle, &overlays, options)?;
    let groups = validate(&resolved, devices)?;
    for instance in &resolved.spec.instances {
        for (slot, provider) in crate::selected_routes(&resolved, &groups, &instance.name) {
            if !resolution.origins.iter().any(|origin| {
                origin.instance == instance.name && origin.category == "route" && origin.key == slot
            }) {
                resolution.origins.push(OriginTrace {
                    instance: instance.name.clone(),
                    category: "route".into(),
                    key: slot,
                    value: provider,
                    layer: format!("deployment binding {}", instance.name),
                    shadowed: Vec::new(),
                });
            }
        }
    }
    resolution.origins.sort_by(|left, right| {
        (&left.instance, &left.category, &left.key).cmp(&(
            &right.instance,
            &right.category,
            &right.key,
        ))
    });
    let plan = generate(&resolved, &groups, Some(&resolution), devices).map_err(|error| {
        vec![Diagnostic::new(
            DiagnosticCode::InvalidPath,
            "$",
            error.to_string(),
        )]
    })?;
    if options.variation.is_some() {
        let conflicts = existing_variation_conflicts(&bundle.workspace_root, &plan);
        if !conflicts.is_empty() {
            return Err(conflicts);
        }
    }
    Ok(plan)
}

fn resolve(
    bundle: &Bundle,
    overlays: &[Overlay],
    options: &OverlayOptions,
) -> Result<(Bundle, OverlayResolution), Vec<Diagnostic>> {
    let mut resolved = bundle.clone();
    resolved.spec.overlays.clear();
    resolved.spec.resolved_overlay_files.clear();
    let original_parameters = bundle
        .spec
        .instances
        .iter()
        .map(|instance| (instance.name.clone(), instance.parameters.clone()))
        .collect::<BTreeMap<_, _>>();
    let original_environment = bundle
        .spec
        .instances
        .iter()
        .map(|instance| (instance.name.clone(), instance.environment.clone()))
        .collect::<BTreeMap<_, _>>();
    let original_routes = bundle.spec.routes.clone();
    resolved.spec.routes.clear();
    for instance in &mut resolved.spec.instances {
        instance.parameters.clear();
        instance.environment.clear();
        instance.environment_unset.clear();
    }
    let mut errors = Vec::new();
    let mut resolution = OverlayResolution::default();
    let mut traces: BTreeMap<(String, String, String), OriginTrace> = BTreeMap::new();
    let mut overlay_route_keys = BTreeSet::new();
    let mut file_indexes: BTreeMap<(String, PathBuf), usize> = BTreeMap::new();

    for instance in &bundle.spec.instances {
        if let Some(block) = bundle.spec.blocks.get(&instance.block) {
            for (key, parameter) in &block.parameters {
                if let Some(value) = &parameter.default {
                    set_trace(
                        &mut traces,
                        &instance.name,
                        "parameter",
                        key,
                        value,
                        &format!("block default {}", instance.block),
                    );
                }
            }
            for (service_name, service) in &block.services {
                for (key, value) in execution_environment(&service.execution) {
                    set_trace(
                        &mut traces,
                        &instance.name,
                        "environment",
                        key,
                        value,
                        &format!("block default {}/{}", instance.block, service_name),
                    );
                }
            }
        }
    }

    for overlay in overlays {
        if let Err(mut overlay_errors) = validate_overlay(overlay) {
            errors.append(&mut overlay_errors);
        }
        let layer = overlay_layer(overlay);
        let targets = matching_instances(bundle, overlay);
        if targets.is_empty() && !overlay.spec.selectors.optional {
            errors.push(Diagnostic::new(
                DiagnosticCode::SelectorNoMatch,
                "spec.selectors",
                format!("required selector in `{layer}` matched no deployment instances"),
            ));
            continue;
        }
        for target in targets {
            let Some(instance_index) = resolved
                .spec
                .instances
                .iter()
                .position(|instance| instance.name == target)
            else {
                continue;
            };
            let env_file_values = load_env_files(overlay, &mut errors);
            let instance = &mut resolved.spec.instances[instance_index];
            for (key, value) in env_file_values {
                set_trace(&mut traces, &target, "environment", &key, &value, &layer);
                instance
                    .environment_unset
                    .retain(|candidate| candidate != &key);
                resolution
                    .secret_environment
                    .remove(&(target.clone(), key.clone()));
                instance.environment.insert(key, value);
            }
            for key in &overlay.spec.environment.unset {
                if !instance.environment_unset.contains(key) {
                    instance.environment_unset.push(key.clone());
                }
                resolution
                    .secret_environment
                    .remove(&(target.clone(), key.clone()));
                if let Some(previous) = instance.environment.remove(key) {
                    set_trace(&mut traces, &target, "environment", key, "«unset»", &layer);
                    if let Some(trace) =
                        traces.get_mut(&(target.clone(), "environment".into(), key.clone()))
                    {
                        if trace.shadowed.is_empty() {
                            trace.shadowed.push(ShadowedOrigin {
                                value: previous,
                                layer: "earlier overlay".into(),
                            });
                        }
                    }
                }
            }
            for (key, value) in &overlay.spec.environment.set {
                let rendered = overlay_value(value);
                set_trace(&mut traces, &target, "environment", key, &rendered, &layer);
                instance
                    .environment_unset
                    .retain(|candidate| candidate != key);
                instance.environment.insert(key.clone(), rendered);
                match value {
                    OverlayValue::Secret(secret) => {
                        resolution
                            .secret_environment
                            .insert((target.clone(), key.clone()), secret.clone());
                    }
                    OverlayValue::Literal(_) => {
                        resolution
                            .secret_environment
                            .remove(&(target.clone(), key.clone()));
                    }
                }
            }
            for (key, value) in &overlay.spec.parameters {
                set_trace(&mut traces, &target, "parameter", key, value, &layer);
                instance.parameters.insert(key.clone(), value.clone());
            }
            for (slot, route) in &overlay.spec.routes {
                let (provider, route_replace) = route.parts();
                let replace = route_replace || overlay.spec.replace;
                let conflict_key = (target.clone(), slot.clone());
                if overlay_route_keys.contains(&conflict_key) && !replace {
                    errors.push(Diagnostic::new(
                        DiagnosticCode::OverlayConflict,
                        format!("spec.routes.{slot}"),
                        format!("route slot `{slot}` for `{target}` requires replace: true"),
                    ));
                    continue;
                }
                overlay_route_keys.insert(conflict_key);
                set_trace(&mut traces, &target, "route", slot, provider, &layer);
                resolved
                    .spec
                    .routes
                    .entry(target.clone())
                    .or_default()
                    .insert(slot.clone(), provider.into());
            }
            resolve_files(
                bundle,
                overlay,
                instance,
                &layer,
                &mut resolution.files,
                &mut file_indexes,
                &mut traces,
                &mut errors,
            );
        }
    }

    for instance in &mut resolved.spec.instances {
        let layer = format!("deployment instance {}", instance.name);
        if let Some(values) = original_parameters.get(&instance.name) {
            for (key, value) in values {
                set_trace(&mut traces, &instance.name, "parameter", key, value, &layer);
                instance.parameters.insert(key.clone(), value.clone());
            }
        }
        if let Some(values) = original_environment.get(&instance.name) {
            for (key, value) in values {
                set_trace(
                    &mut traces,
                    &instance.name,
                    "environment",
                    key,
                    value,
                    &layer,
                );
                instance.environment.insert(key.clone(), value.clone());
                instance
                    .environment_unset
                    .retain(|candidate| candidate != key);
                resolution
                    .secret_environment
                    .remove(&(instance.name.clone(), key.clone()));
            }
        }
        if let Some(values) = original_routes.get(&instance.name) {
            for (slot, provider) in values {
                set_trace(&mut traces, &instance.name, "route", slot, provider, &layer);
                resolved
                    .spec
                    .routes
                    .entry(instance.name.clone())
                    .or_default()
                    .insert(slot.clone(), provider.clone());
            }
        }
        for (key, value) in &options.set {
            if !valid_environment_name(key) {
                errors.push(Diagnostic::new(
                    DiagnosticCode::InvalidOverlay,
                    "--set",
                    format!("invalid environment key `{key}`"),
                ));
                continue;
            }
            set_trace(
                &mut traces,
                &instance.name,
                "environment",
                key,
                value,
                "ephemeral CLI --set",
            );
            instance.environment.insert(key.clone(), value.clone());
            instance
                .environment_unset
                .retain(|candidate| candidate != key);
            resolution
                .secret_environment
                .remove(&(instance.name.clone(), key.clone()));
        }
    }
    if let Some(variation) = &options.variation {
        if variation.is_empty()
            || !variation
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            errors.push(Diagnostic::new(
                DiagnosticCode::InvalidName,
                "--variation",
                "variation must be a lowercase DNS label",
            ));
        } else {
            let original_name = resolved.metadata.name.clone();
            resolved.metadata.name = format!("{original_name}--{variation}");
            if let Some(router) = &mut resolved.spec.host_router {
                router.metadata.deployment = resolved.metadata.name.as_str().into();
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    resolution.origins = traces
        .into_values()
        .filter(|trace| trace.value != "«unset»")
        .collect();
    for file in &resolution.files {
        resolved
            .spec
            .resolved_overlay_files
            .entry(file.instance.clone())
            .or_default()
            .push(crate::ResolvedOverlayFile {
                target: file.target.clone(),
                content_hash: file.content_hash.clone(),
                mode: format!("0{:03o}", file.mode),
                origin: file.origin.clone(),
            });
    }
    Ok((resolved, resolution))
}

fn execution_environment(execution: &Execution) -> &BTreeMap<String, String> {
    match execution {
        Execution::Container { environment, .. }
        | Execution::Script { environment, .. }
        | Execution::ProcessCompose { environment, .. } => environment,
    }
}

fn matching_instances(bundle: &Bundle, overlay: &Overlay) -> Vec<String> {
    let selectors = &overlay.spec.selectors;
    if !selectors
        .deployment
        .match_labels
        .iter()
        .all(|(key, value)| bundle.metadata.labels.get(key) == Some(value))
    {
        return Vec::new();
    }
    bundle
        .spec
        .instances
        .iter()
        .filter(|instance| {
            (selectors.instances.names.is_empty()
                || selectors.instances.names.contains(&instance.name))
                && selectors
                    .instances
                    .match_labels
                    .iter()
                    .all(|(key, value)| instance.labels.get(key) == Some(value))
        })
        .map(|instance| instance.name.clone())
        .collect()
}

fn overlay_layer(overlay: &Overlay) -> String {
    if overlay.source_path.as_os_str().is_empty() {
        format!("overlay {}", overlay.metadata.name)
    } else {
        overlay.source_path.display().to_string()
    }
}

fn load_env_files(overlay: &Overlay, errors: &mut Vec<Diagnostic>) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    let base = overlay
        .source_path
        .parent()
        .unwrap_or_else(|| Path::new("."));
    for path in &overlay.spec.environment.env_files {
        let path = if path.is_absolute() {
            path.clone()
        } else {
            base.join(path)
        };
        match fs::read_to_string(&path) {
            Ok(input) => match parse_dotenv(&input) {
                Ok(values) => result.extend(values),
                Err(message) => errors.push(Diagnostic::new(
                    DiagnosticCode::InvalidOverlay,
                    path.display().to_string(),
                    message,
                )),
            },
            Err(error) => errors.push(Diagnostic::new(
                DiagnosticCode::InvalidPath,
                path.display().to_string(),
                error.to_string(),
            )),
        }
    }
    result
}

fn overlay_value(value: &OverlayValue) -> String {
    match value {
        OverlayValue::Literal(value) => value.clone(),
        OverlayValue::Secret(secret) => match (&secret.environment_variable, &secret.file) {
            (Some(name), None) => format!("«secret: {name}»"),
            (None, Some(path)) => format!("«secret: {}»", path.display()),
            _ => "«secret: invalid reference»".into(),
        },
    }
}

fn set_trace(
    traces: &mut BTreeMap<(String, String, String), OriginTrace>,
    instance: &str,
    category: &str,
    key: &str,
    value: &str,
    layer: &str,
) {
    let id = (instance.into(), category.into(), key.into());
    if let Some(trace) = traces.get_mut(&id) {
        trace.shadowed.push(ShadowedOrigin {
            value: trace.value.clone(),
            layer: trace.layer.clone(),
        });
        trace.value = value.into();
        trace.layer = layer.into();
    } else {
        traces.insert(
            id,
            OriginTrace {
                instance: instance.into(),
                category: category.into(),
                key: key.into(),
                value: value.into(),
                layer: layer.into(),
                shadowed: Vec::new(),
            },
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_files(
    bundle: &Bundle,
    overlay: &Overlay,
    instance: &crate::Instance,
    layer: &str,
    files: &mut Vec<InjectedFilePlan>,
    indexes: &mut BTreeMap<(String, PathBuf), usize>,
    traces: &mut BTreeMap<(String, String, String), OriginTrace>,
    errors: &mut Vec<Diagnostic>,
) {
    let base = overlay
        .source_path
        .parent()
        .unwrap_or_else(|| Path::new("."));
    for (file_index, file) in overlay.spec.files.iter().enumerate() {
        let key = (instance.name.clone(), file.target.clone());
        if indexes.contains_key(&key) && !(file.replace || overlay.spec.replace) {
            errors.push(Diagnostic::new(
                DiagnosticCode::OverlayConflict,
                format!("spec.files[{file_index}].target"),
                format!(
                    "file target `{}` for `{}` requires replace: true",
                    file.target.display(),
                    instance.name
                ),
            ));
            continue;
        }
        if !target_is_controlled(bundle, instance, &file.target) {
            errors.push(Diagnostic::new(
                DiagnosticCode::InvalidPath,
                format!("spec.files[{file_index}].target"),
                format!(
                    "target `{}` is outside controlled container mount roots",
                    file.target.display()
                ),
            ));
            continue;
        }
        let raw = match (&file.source, &file.content) {
            (Some(OverlayFileSource::Path(path)), None) => {
                let path = if path.is_absolute() {
                    path.clone()
                } else {
                    base.join(path)
                };
                match fs::read_to_string(&path) {
                    Ok(value) => value,
                    Err(error) => {
                        errors.push(Diagnostic::new(
                            DiagnosticCode::InvalidPath,
                            path.display().to_string(),
                            error.to_string(),
                        ));
                        continue;
                    }
                }
            }
            (None, Some(content)) => content.clone(),
            _ => continue,
        };
        let parameters = effective_parameters(bundle, instance);
        let content = if file.template {
            match render_template(&raw, overlay, instance, &bundle.metadata.name, &parameters) {
                Ok(content) => content,
                Err(variable) => {
                    errors.push(Diagnostic::new(
                        DiagnosticCode::MissingVariable,
                        format!("spec.files[{file_index}]"),
                        format!("unknown template variable `{variable}`"),
                    ));
                    continue;
                }
            }
        } else {
            raw
        };
        let hash = format!("{:x}", Sha256::digest(content.as_bytes()));
        let filename = file
            .target
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("injected"));
        let relative_path = PathBuf::from("overlays")
            .join(&instance.name)
            .join(&hash)
            .join(filename);
        let planned = InjectedFilePlan {
            instance: instance.name.clone(),
            target: file.target.clone(),
            mode: parse_mode(&file.mode).unwrap_or(0o644),
            content_hash: hash.clone(),
            relative_path,
            origin: layer.into(),
            content: content.into_bytes(),
        };
        set_trace(
            traces,
            &instance.name,
            "file",
            &file.target.display().to_string(),
            &hash,
            layer,
        );
        if let Some(index) = indexes.get(&key).copied() {
            files[index] = planned;
        } else {
            indexes.insert(key, files.len());
            files.push(planned);
        }
    }
}

fn effective_parameters(bundle: &Bundle, instance: &crate::Instance) -> BTreeMap<String, String> {
    let mut values = bundle
        .spec
        .blocks
        .get(&instance.block)
        .into_iter()
        .flat_map(|block| &block.parameters)
        .filter_map(|(key, parameter)| {
            parameter
                .default
                .as_ref()
                .map(|value| (key.clone(), value.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    values.extend(instance.parameters.clone());
    if let Some(authored) = bundle
        .spec
        .instances
        .iter()
        .find(|candidate| candidate.name == instance.name)
    {
        values.extend(authored.parameters.clone());
    }
    values
}

fn target_is_controlled(bundle: &Bundle, instance: &crate::Instance, target: &Path) -> bool {
    if !target.is_absolute()
        || target
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return false;
    }
    let mut roots = vec![PathBuf::from("/runtime")];
    if let Some(block) = bundle.spec.blocks.get(&instance.block) {
        for service in block.services.values() {
            roots.extend(service.volumes.iter().map(|mount| mount.target.clone()));
            match &service.execution {
                Execution::Script { source_mount, .. }
                | Execution::ProcessCompose { source_mount, .. } => {
                    roots.push(source_mount.clone())
                }
                Execution::Container { .. } => {}
            }
        }
    }
    roots
        .iter()
        .any(|root| target.starts_with(root) && target != root)
}

fn render_template(
    input: &str,
    overlay: &Overlay,
    instance: &crate::Instance,
    deployment: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<String, String> {
    let mut output = String::new();
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        output.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            return Err(after.into());
        };
        let variable = &after[..end];
        let value = if variable == "instance.name" {
            Some(instance.name.as_str())
        } else if variable == "deployment.name" {
            Some(deployment)
        } else if let Some(name) = variable.strip_prefix("overlay.variables.") {
            overlay.spec.variables.get(name).map(String::as_str)
        } else if let Some(name) = variable.strip_prefix("parameters.") {
            parameters.get(name).map(String::as_str)
        } else {
            None
        };
        output.push_str(value.ok_or_else(|| variable.to_owned())?);
        rest = &after[end + 1..];
    }
    output.push_str(rest);
    Ok(output)
}

pub(crate) fn materialize_injected_files(
    artifact_dir: &Path,
    files: &[InjectedFilePlan],
) -> io::Result<()> {
    for file in files {
        let path = artifact_dir.join(&file.relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        super::write_atomic(&path, &file.content)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(file.mode))?;
        }
    }
    Ok(())
}

/// Compares a plan with the currently generated Compose artifact.
pub fn classify_changes(
    workspace_root: &Path,
    plan: &crate::Plan,
) -> io::Result<Vec<ServiceChange>> {
    let compose_path = workspace_root.join(&plan.artifact_dir).join("compose.yaml");
    let manifest_path = workspace_root
        .join(&plan.artifact_dir)
        .join("manifest.json");
    let current_compose = match fs::read_to_string(compose_path) {
        Ok(value) => Some(serde_yaml::from_str::<Value>(&value).map_err(io::Error::other)?),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let current_hashes = fs::read_to_string(manifest_path)
        .ok()
        .and_then(|value| serde_json::from_str::<Value>(&value).ok())
        .map(|value| {
            (
                value
                    .get("definitionHash")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                value
                    .get("resourceHash")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            )
        });
    if current_hashes
        .as_ref()
        .and_then(|hashes| hashes.0.as_deref())
        == Some(&plan.definition_hash)
    {
        return Ok(Vec::new());
    }
    let next: Value = serde_yaml::from_str(&plan.compose_yaml).map_err(io::Error::other)?;
    let next_services = next
        .get("services")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let current_services = current_compose
        .as_ref()
        .and_then(|value| value.get("services"))
        .and_then(Value::as_object);
    let mut changes = Vec::new();
    for (name, next_service) in next_services {
        let impact = match current_services.and_then(|services| services.get(&name)) {
            None => ChangeImpact::Rebuild,
            Some(_)
                if current_hashes
                    .as_ref()
                    .and_then(|hashes| hashes.1.as_deref())
                    == Some(&plan.resource_hash) =>
            {
                ChangeImpact::Live
            }
            Some(current)
                if normalized_service(current.clone())
                    == normalized_service(next_service.clone()) =>
            {
                ChangeImpact::Live
            }
            Some(current) if build_identity(current) != build_identity(&next_service) => {
                ChangeImpact::Rebuild
            }
            Some(_) => ChangeImpact::Restart,
        };
        changes.push(ServiceChange {
            service: name,
            impact,
        });
    }
    Ok(changes)
}

/// Rejects fixed host-listener claims shared by concurrently planned variations.
pub fn validate_variation_collisions(plans: &[crate::Plan]) -> Result<(), Vec<Diagnostic>> {
    let mut claims: BTreeMap<(String, u16), String> = BTreeMap::new();
    let mut errors = Vec::new();
    for plan in plans {
        let Some(config) = &plan.host_router_config else {
            continue;
        };
        let Ok(config) = serde_json::from_str::<router_config::RouterConfig>(config) else {
            continue;
        };
        for listener in config.spec.listeners {
            let claim = (listener.bind.host.to_string(), listener.bind.port);
            if let Some(owner) = claims.insert(claim.clone(), plan.deployment.clone()) {
                if owner != plan.deployment {
                    errors.push(Diagnostic::new(
                        DiagnosticCode::ListenerConflict,
                        "variation.hostListeners",
                        format!(
                            "variations `{owner}` and `{}` both claim {}:{}",
                            plan.deployment, claim.0, claim.1
                        ),
                    ));
                }
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn existing_variation_conflicts(workspace_root: &Path, plan: &crate::Plan) -> Vec<Diagnostic> {
    let Some(planned) = plan
        .host_router_config
        .as_ref()
        .and_then(|config| serde_json::from_str::<router_config::RouterConfig>(config).ok())
    else {
        return Vec::new();
    };
    let planned_claims = planned
        .spec
        .listeners
        .iter()
        .map(|listener| (listener.bind.host, listener.bind.port))
        .collect::<BTreeSet<_>>();
    let generated = workspace_root.join(".switchyard/generated");
    let Ok(entries) = fs::read_dir(generated) else {
        return Vec::new();
    };
    let mut errors = Vec::new();
    for entry in entries.flatten() {
        if entry.file_name() == plan.deployment.as_str() {
            continue;
        }
        let path = entry.path().join("host-router.json");
        let Ok(input) = fs::read_to_string(path) else {
            continue;
        };
        let Ok(config) = serde_json::from_str::<router_config::RouterConfig>(&input) else {
            continue;
        };
        for listener in config.spec.listeners {
            if planned_claims.contains(&(listener.bind.host, listener.bind.port)) {
                errors.push(Diagnostic::new(
                    DiagnosticCode::ListenerConflict,
                    "variation.hostListeners",
                    format!(
                        "variation `{}` conflicts with generated deployment `{}` at {}:{}",
                        plan.deployment,
                        entry.file_name().to_string_lossy(),
                        listener.bind.host,
                        listener.bind.port
                    ),
                ));
            }
        }
    }
    errors
}

fn build_identity(service: &Value) -> (Option<&Value>, Option<&Value>) {
    (service.get("image"), service.get("build"))
}

fn normalized_service(mut service: Value) -> Value {
    if let Some(object) = service.as_object_mut() {
        if let Some(labels) = object.get_mut("labels").and_then(Value::as_object_mut) {
            labels.remove("dev.switchyard.resource-hash");
        }
    }
    service
}
