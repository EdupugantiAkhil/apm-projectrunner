use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    path::Path,
};

use serde_yaml::Value;
use switchyard_planner::{API_VERSION, KIND};

const PREVIOUS_API_VERSION: &str = "switchyard.dev/v1alpha1";
pub const FORMAT_WARNING: &str = "migration uses YAML serialization; comments, anchors, and hand formatting are not preserved. Back up the file or review the diff after migration";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MigrationResult {
    pub changed: bool,
    pub changes: Vec<String>,
}

#[derive(Debug)]
pub struct MigrationError(String);

impl fmt::Display for MigrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for MigrationError {}

pub fn migrate(
    path: &Path,
    before_write: impl FnOnce(),
) -> Result<MigrationResult, MigrationError> {
    let original = fs::read_to_string(path).map_err(|error| {
        MigrationError(format!(
            "could not read deployment `{}`: {error}",
            path.display()
        ))
    })?;
    let mut document: Value = serde_yaml::from_str(&original).map_err(|error| {
        MigrationError(format!(
            "could not parse deployment `{}`: {error}",
            path.display()
        ))
    })?;
    let version = document
        .get("apiVersion")
        .and_then(Value::as_str)
        .ok_or_else(|| MigrationError("deployment has no string apiVersion".into()))?;
    let kind = document
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| MigrationError("deployment has no string kind".into()))?;
    if kind != KIND {
        return Err(MigrationError(format!(
            "cannot migrate kind `{kind}`; expected {KIND}"
        )));
    }
    match version {
        API_VERSION => {
            ensure_no_legacy_group_providers(&document)?;
            return Ok(MigrationResult {
                changed: false,
                changes: Vec::new(),
            });
        }
        PREVIOUS_API_VERSION => {}
        other => {
            return Err(MigrationError(format!(
                "cannot migrate apiVersion `{other}`; supported input versions are {PREVIOUS_API_VERSION} and {API_VERSION}"
            )));
        }
    }

    let legacy_groups = resolve_legacy_group_providers(&document)?;
    let mut changes = vec![format!(
        "apiVersion: {PREVIOUS_API_VERSION} -> {API_VERSION}"
    )];
    migrate_group_instances(&mut document, &mut changes)?;
    set_api_version(&mut document)?;
    let migrated = serde_yaml::to_string(&document).map_err(|error| {
        MigrationError(format!("could not encode migrated deployment: {error}"))
    })?;
    let bundle = switchyard_planner::load_bundle_from_str(&migrated, path).map_err(|error| {
        MigrationError(format!(
            "refusing to write `{}` because the deployment could not be fully migrated: {error}",
            path.display()
        ))
    })?;
    if bundle.api_version != API_VERSION || bundle.kind != KIND {
        return Err(MigrationError(format!(
            "refusing to write `{}` because the migrated document is not {API_VERSION} kind {KIND}",
            path.display()
        )));
    }
    validate_migrated_group_providers(path, &legacy_groups, &bundle)?;
    before_write();
    write_atomic(path, migrated.as_bytes()).map_err(|error| {
        MigrationError(format!(
            "could not write migrated deployment `{}`: {error}",
            path.display()
        ))
    })?;
    Ok(MigrationResult {
        changed: true,
        changes,
    })
}

fn resolve_legacy_group_providers(
    document: &Value,
) -> Result<BTreeMap<String, BTreeMap<String, String>>, MigrationError> {
    #[derive(Clone, Default)]
    struct LegacyGroup {
        extends: Option<String>,
        providers: BTreeMap<String, String>,
    }

    fn resolve_one(
        name: &str,
        groups: &BTreeMap<String, LegacyGroup>,
        visiting: &mut BTreeSet<String>,
        resolved: &mut BTreeMap<String, BTreeMap<String, String>>,
    ) -> Result<BTreeMap<String, String>, MigrationError> {
        if let Some(group) = resolved.get(name) {
            return Ok(group.clone());
        }
        if !visiting.insert(name.to_owned()) {
            return Err(MigrationError(format!(
                "cannot migrate group `{name}`: inheritance contains a cycle"
            )));
        }
        let group = groups.get(name).ok_or_else(|| {
            MigrationError(format!(
                "cannot migrate group `{name}`: inherited group does not exist"
            ))
        })?;
        let mut providers = if let Some(parent) = group.extends.as_deref() {
            resolve_one(parent, groups, visiting, resolved)?
        } else {
            BTreeMap::new()
        };
        providers.extend(group.providers.clone());
        visiting.remove(name);
        resolved.insert(name.to_owned(), providers.clone());
        Ok(providers)
    }

    let Some(groups) = document.get("spec").and_then(|spec| spec.get("groups")) else {
        return Ok(BTreeMap::new());
    };
    let groups = groups
        .as_mapping()
        .ok_or_else(|| MigrationError("cannot migrate: spec.groups must be a mapping".into()))?;
    let mut parsed = BTreeMap::new();
    for (name, group) in groups {
        let name = name
            .as_str()
            .ok_or_else(|| MigrationError("cannot migrate: group names must be strings".into()))?;
        group.as_mapping().ok_or_else(|| {
            MigrationError(format!(
                "cannot migrate group `{name}`: its definition must be a mapping"
            ))
        })?;
        let extends = match group.get("extends") {
            Some(Value::String(parent)) => Some(parent.clone()),
            Some(_) => {
                return Err(MigrationError(format!(
                    "cannot migrate group `{name}`: extends must name a group"
                )));
            }
            None => None,
        };
        let mut providers = BTreeMap::new();
        if let Some(value) = group.get("providers") {
            let values = value.as_mapping().ok_or_else(|| {
                MigrationError(format!(
                    "cannot migrate group `{name}`: providers must be a mapping"
                ))
            })?;
            for (capability, provider) in values {
                let capability = capability.as_str().ok_or_else(|| {
                    MigrationError(format!(
                        "cannot migrate group `{name}`: every providers key must be a capability string"
                    ))
                })?;
                let provider = provider.as_str().ok_or_else(|| {
                    MigrationError(format!(
                        "cannot migrate group `{name}`: every providers value must be an instance reference string"
                    ))
                })?;
                providers.insert(capability.to_owned(), provider.to_owned());
            }
        }
        parsed.insert(name.to_owned(), LegacyGroup { extends, providers });
    }

    let mut resolved = BTreeMap::new();
    for name in parsed.keys() {
        resolve_one(name, &parsed, &mut BTreeSet::new(), &mut resolved)?;
    }
    Ok(resolved)
}

fn validate_migrated_group_providers(
    path: &Path,
    expected: &BTreeMap<String, BTreeMap<String, String>>,
    bundle: &switchyard_planner::Bundle,
) -> Result<(), MigrationError> {
    let actual = switchyard_planner::resolve_service_groups(bundle).map_err(|diagnostics| {
        MigrationError(format!(
            "refusing to write `{}` because the group provider mapping could not be fully migrated: {}",
            path.display(),
            diagnostics
                .into_iter()
                .map(|diagnostic| diagnostic.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        ))
    })?;
    if &actual == expected {
        return Ok(());
    }

    for group in expected
        .keys()
        .chain(actual.keys())
        .collect::<BTreeSet<_>>()
    {
        let before = expected.get(group.as_str()).cloned().unwrap_or_default();
        let after = actual.get(group.as_str()).cloned().unwrap_or_default();
        for capability in before.keys().chain(after.keys()).collect::<BTreeSet<_>>() {
            if before.get(capability.as_str()) != after.get(capability.as_str()) {
                return Err(MigrationError(format!(
                    "refusing to write `{}` because group `{group}` cannot be fully migrated without changing capability `{capability}` from {} to {}",
                    path.display(),
                    before
                        .get(capability.as_str())
                        .map_or("absent", String::as_str),
                    after
                        .get(capability.as_str())
                        .map_or("absent", String::as_str)
                )));
            }
        }
    }
    Ok(())
}

fn migrate_group_instances(
    document: &mut Value,
    changes: &mut Vec<String>,
) -> Result<(), MigrationError> {
    let Some(spec) = document.get_mut("spec") else {
        return Ok(());
    };
    let spec = spec
        .as_mapping_mut()
        .ok_or_else(|| MigrationError("cannot migrate: spec must be a mapping".into()))?;
    let groups_key = Value::String("groups".into());
    let Some(groups) = spec.get_mut(&groups_key) else {
        return Ok(());
    };
    let groups = groups
        .as_mapping_mut()
        .ok_or_else(|| MigrationError("cannot migrate: spec.groups must be a mapping".into()))?;
    for (name, group) in groups {
        let name = name
            .as_str()
            .ok_or_else(|| MigrationError("cannot migrate: group names must be strings".into()))?;
        let group = group.as_mapping_mut().ok_or_else(|| {
            MigrationError(format!(
                "cannot migrate group `{name}`: its definition must be a mapping"
            ))
        })?;
        let providers_key = Value::String("providers".into());
        let instances_key = Value::String("instances".into());
        if group.contains_key(&instances_key) {
            validate_instances(name, group.get(&instances_key).expect("key exists"))?;
        }
        let Some(providers) = group.remove(&providers_key) else {
            continue;
        };
        if group.contains_key(&instances_key) {
            return Err(MigrationError(format!(
                "cannot migrate group `{name}` because it contains both providers and instances"
            )));
        }
        let providers = providers.as_mapping().ok_or_else(|| {
            MigrationError(format!(
                "cannot migrate group `{name}`: providers must be a mapping"
            ))
        })?;
        let capability_count = providers.len();
        let mut seen = BTreeSet::new();
        let mut instances = Vec::new();
        for provider in providers.values() {
            let provider = provider.as_str().ok_or_else(|| {
                MigrationError(format!(
                    "cannot migrate group `{name}`: every providers value must be an instance reference string"
                ))
            })?;
            if seen.insert(provider.to_owned()) {
                instances.push(Value::String(provider.to_owned()));
            }
        }
        let entry_label = if capability_count == 1 {
            "entry"
        } else {
            "entries"
        };
        let member_label = if instances.len() == 1 {
            "member"
        } else {
            "members"
        };
        changes.push(format!(
            "spec.groups.{name}: providers mapping with {capability_count} {entry_label} -> instances list with {} unique {member_label}",
            instances.len()
        ));
        group.insert(instances_key, Value::Sequence(instances));
    }
    Ok(())
}

fn validate_instances(name: &str, instances: &Value) -> Result<(), MigrationError> {
    let instances = instances.as_sequence().ok_or_else(|| {
        MigrationError(format!(
            "cannot migrate group `{name}`: instances must be a list"
        ))
    })?;
    if instances.iter().any(|instance| instance.as_str().is_none()) {
        return Err(MigrationError(format!(
            "cannot migrate group `{name}`: every instances entry must be a string"
        )));
    }
    Ok(())
}

fn ensure_no_legacy_group_providers(document: &Value) -> Result<(), MigrationError> {
    let Some(groups) = document
        .get("spec")
        .and_then(|spec| spec.get("groups"))
        .and_then(Value::as_mapping)
    else {
        return Ok(());
    };
    for (name, group) in groups {
        if group.get("providers").is_some() {
            return Err(MigrationError(format!(
                "deployment already uses {API_VERSION}, but group `{}` still contains providers",
                name.as_str().unwrap_or("<non-string>")
            )));
        }
    }
    Ok(())
}

fn set_api_version(document: &mut Value) -> Result<(), MigrationError> {
    let root = document
        .as_mapping_mut()
        .ok_or_else(|| MigrationError("deployment document must be a mapping".into()))?;
    root.insert(
        Value::String("apiVersion".into()),
        Value::String(API_VERSION.into()),
    );
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let temporary = path.with_extension("migrate.tmp");
    fs::write(&temporary, bytes)?;
    fs::set_permissions(&temporary, fs::metadata(path)?.permissions())?;
    fs::rename(temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_group_providers_and_is_idempotent() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("deployment.yaml");
        fs::write(
            &path,
            r#"apiVersion: switchyard.dev/v1alpha1
kind: Deployment
metadata: { name: demo }
spec:
  sources:
    app: { path: . }
  blocks:
    main-suite:
      services:
        suite:
          execution: { type: container, image: example/main:1 }
          provides:
            search: { protocol: http, port: 8001 }
            reports: { protocol: http, port: 8002 }
    feature-suite:
      services:
        suite:
          execution: { type: container, image: example/feature:1 }
          provides:
            search: { protocol: http, port: 8001 }
            reports: { protocol: http, port: 8002 }
    database:
      services:
        main:
          execution: { type: container, image: example/database:1 }
          provides:
            database: { protocol: tcp, port: 5432 }
  instances:
    - { name: ai-main, block: main-suite, source: app }
    - { name: ai-feature, block: feature-suite, source: app }
    - { name: db-main, block: database, source: app }
  groups:
    base:
      providers:
        search: ai-main/suite
        reports: ai-main/suite
        database: db-main
    feature:
      extends: base
      providers:
        search: ai-feature/suite
        reports: ai-feature/suite
"#,
        )
        .unwrap();

        let result = migrate(&path, || {}).unwrap();
        assert!(result.changed);
        assert_eq!(
            result
                .changes
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            [
                "apiVersion: switchyard.dev/v1alpha1 -> switchyard.dev/v1alpha2",
                "spec.groups.base: providers mapping with 3 entries -> instances list with 2 unique members",
                "spec.groups.feature: providers mapping with 2 entries -> instances list with 1 unique member",
            ]
        );
        let first = fs::read_to_string(&path).unwrap();
        let bundle = switchyard_planner::load_bundle(&path).unwrap();
        assert_eq!(bundle.api_version, API_VERSION);
        assert_eq!(
            bundle.spec.groups["base"].instances,
            ["ai-main/suite", "db-main"]
        );
        assert_eq!(
            bundle.spec.groups["feature"].instances,
            ["ai-feature/suite"]
        );

        let second = migrate(&path, || panic!("an up-to-date file must not be rewritten")).unwrap();
        assert!(!second.changed);
        assert!(second.changes.is_empty());
        assert_eq!(fs::read_to_string(&path).unwrap(), first);
    }

    #[test]
    fn warning_hook_runs_before_destructive_rewrite() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("deployment.yaml");
        let original = r#"apiVersion: switchyard.dev/v1alpha1
kind: Deployment
metadata: { name: demo }
spec:
  sources:
    app: { path: . }
  blocks:
    provider:
      services:
        api:
          execution: { type: container, image: example/provider:1 }
          provides:
            search: { protocol: http, port: 8080 }
  instances:
    - { name: provider-main, block: provider, source: app }
  groups:
    base:
      providers: { search: provider-main/api } # hand-authored comment
"#;
        fs::write(&path, original).unwrap();
        let warning_shown = std::cell::Cell::new(false);

        let result = migrate(&path, || {
            warning_shown.set(true);
            assert_eq!(fs::read_to_string(&path).unwrap(), original);
            assert!(FORMAT_WARNING.contains("comments"));
            assert!(FORMAT_WARNING.contains("anchors"));
            assert!(FORMAT_WARNING.contains("formatting"));
        })
        .unwrap();

        assert!(warning_shown.get());
        assert!(result.changed);
        assert_eq!(result.changes.len(), 2);
        assert!(
            !fs::read_to_string(path)
                .unwrap()
                .contains("hand-authored comment")
        );
    }

    #[test]
    fn refuses_group_migration_that_would_change_capabilities() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("deployment.yaml");
        let original = r#"apiVersion: switchyard.dev/v1alpha1
kind: Deployment
metadata: { name: demo }
spec:
  sources:
    app: { path: . }
  blocks:
    provider:
      services:
        api:
          execution: { type: container, image: example/provider:1 }
          provides:
            search: { protocol: http, port: 8080 }
            reports: { protocol: http, port: 8081 }
  instances:
    - { name: provider-main, block: provider, source: app }
  groups:
    base:
      providers: { search: provider-main/api }
"#;
        fs::write(&path, original).unwrap();

        let error = migrate(&path, || {
            panic!("an unsafe migration must not reach the write hook")
        })
        .unwrap_err()
        .to_string();

        assert!(error.contains("cannot be fully migrated"));
        assert!(error.contains("capability `reports` from absent to provider-main/api"));
        assert_eq!(fs::read_to_string(path).unwrap(), original);
    }

    #[test]
    fn refuses_incomplete_migration_without_writing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("deployment.yaml");
        let original = r#"apiVersion: switchyard.dev/v1alpha1
kind: Deployment
metadata: { name: demo }
spec:
  groups:
    broken:
      providers: [not, a, map]
"#;
        fs::write(&path, original).unwrap();
        let error = migrate(&path, || {}).unwrap_err().to_string();
        assert!(error.contains("providers must be a mapping"));
        assert_eq!(fs::read_to_string(path).unwrap(), original);
    }
}
