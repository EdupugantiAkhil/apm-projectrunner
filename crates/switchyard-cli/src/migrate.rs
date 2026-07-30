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
    let previous_version = match version {
        API_VERSION => {
            ensure_no_legacy_group_providers(&document)?;
            false
        }
        PREVIOUS_API_VERSION => true,
        other => {
            return Err(MigrationError(format!(
                "cannot migrate apiVersion `{other}`; supported input versions are {PREVIOUS_API_VERSION} and {API_VERSION}"
            )));
        }
    };

    let legacy_groups = if previous_version {
        resolve_legacy_group_providers(&document)?
    } else {
        BTreeMap::new()
    };
    let mut changes = Vec::new();
    if previous_version {
        changes.push(format!(
            "apiVersion: {PREVIOUS_API_VERSION} -> {API_VERSION}"
        ));
    }
    migrate_group_instances(&mut document, &mut changes)?;
    if previous_version {
        let mut group_document = document.clone();
        if let Some(spec) = group_document
            .get_mut("spec")
            .and_then(Value::as_mapping_mut)
        {
            spec.remove(Value::String("uiRoutes".into()));
        }
        set_api_version(&mut group_document)?;
        let group_yaml = serde_yaml::to_string(&group_document).map_err(|error| {
            MigrationError(format!("could not encode group migration check: {error}"))
        })?;
        let group_bundle = switchyard_planner::load_bundle_from_str(&group_yaml, path).map_err(
            |error| {
                MigrationError(format!(
                    "refusing to write `{}` because the group provider mapping could not be fully migrated: {error}",
                    path.display()
                ))
            },
        )?;
        validate_migrated_group_providers(path, &legacy_groups, &group_bundle)?;
    }
    migrate_ui_routes(&mut document, &mut changes)?;
    if changes.is_empty() {
        return Ok(MigrationResult {
            changed: false,
            changes,
        });
    }
    if previous_version {
        set_api_version(&mut document)?;
    }
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

#[derive(Clone)]
struct LegacyUiRoute {
    ui: String,
    origin: String,
    domain: String,
    backend: String,
    group: String,
}

fn parse_ui_routes(value: Value) -> Result<Vec<LegacyUiRoute>, MigrationError> {
    let routes = value
        .as_mapping()
        .ok_or_else(|| MigrationError("cannot migrate: spec.uiRoutes must be a mapping".into()))?;
    let mut parsed = Vec::new();
    let mut groups = BTreeSet::new();
    for (ui, route) in routes {
        let ui = ui.as_str().ok_or_else(|| {
            MigrationError("cannot migrate: every uiRoutes key must name a UI instance".into())
        })?;
        let route = route.as_mapping().ok_or_else(|| {
            MigrationError(format!(
                "cannot migrate UI route `{ui}`: its definition must be a mapping"
            ))
        })?;
        let string_field = |name: &str| {
            route.get(name).and_then(Value::as_str).ok_or_else(|| {
                MigrationError(format!(
                    "cannot migrate UI route `{ui}`: {name} must be a string"
                ))
            })
        };
        let origin = string_field("origin")?;
        let backend = string_field("backend")?;
        let group = string_field("downstreamGroup")?;
        if !groups.insert(group.to_owned()) {
            return Err(MigrationError(format!(
                "cannot migrate UI route `{ui}`: group `{group}` has more than one UI route, but an object may declare only one address"
            )));
        }
        let uri = origin.parse::<http::Uri>().map_err(|error| {
            MigrationError(format!(
                "cannot migrate UI route `{ui}` origin `{origin}`: {error}"
            ))
        })?;
        if !matches!(uri.scheme_str(), Some("http" | "https"))
            || uri.authority().is_none()
            || uri
                .path_and_query()
                .is_some_and(|path| path.as_str() != "/")
        {
            return Err(MigrationError(format!(
                "cannot migrate UI route `{ui}` origin `{origin}`: expected an HTTP(S) origin without a path, query, fragment, or credentials"
            )));
        }
        let authority = uri.authority().expect("checked above");
        if authority.as_str().contains('@') {
            return Err(MigrationError(format!(
                "cannot migrate UI route `{ui}` origin `{origin}`: credentials are not valid in an origin"
            )));
        }
        parsed.push(LegacyUiRoute {
            ui: ui.to_owned(),
            origin: origin.to_owned(),
            domain: authority.host().to_owned(),
            backend: backend.to_owned(),
            group: group.to_owned(),
        });
    }
    Ok(parsed)
}

fn migrate_ui_routes(
    document: &mut Value,
    changes: &mut Vec<String>,
) -> Result<(), MigrationError> {
    let Some(spec) = document.get_mut("spec") else {
        return Ok(());
    };
    let spec = spec
        .as_mapping_mut()
        .ok_or_else(|| MigrationError("cannot migrate: spec must be a mapping".into()))?;
    let routes_key = Value::String("uiRoutes".into());
    let Some(routes) = spec.remove(&routes_key) else {
        return Ok(());
    };
    let routes = parse_ui_routes(routes)?;
    let groups = spec
        .get_mut("groups")
        .and_then(Value::as_mapping_mut)
        .ok_or_else(|| {
            MigrationError("cannot migrate uiRoutes: spec.groups must be a mapping".into())
        })?;
    for route in &routes {
        let group = groups
            .get_mut(route.group.as_str())
            .and_then(Value::as_mapping_mut)
            .ok_or_else(|| {
                MigrationError(format!(
                    "cannot migrate UI route `{}`: downstream group `{}` does not exist",
                    route.ui, route.group
                ))
            })?;
        match group.get("address") {
            Some(Value::String(address)) if address == &route.domain => {}
            Some(Value::String(address)) => {
                return Err(MigrationError(format!(
                    "cannot migrate UI route `{}`: group `{}` already has address `{address}` instead of `{}`",
                    route.ui, route.group, route.domain
                )));
            }
            Some(_) => {
                return Err(MigrationError(format!(
                    "cannot migrate UI route `{}`: group `{}` address must be a string",
                    route.ui, route.group
                )));
            }
            None => {
                group.insert(
                    Value::String("address".into()),
                    Value::String(route.domain.clone()),
                );
            }
        }
        let instances_key = Value::String("instances".into());
        if !group.contains_key(&instances_key) {
            group.insert(instances_key.clone(), Value::Sequence(Vec::new()));
        }
        let instances = group
            .get_mut(&instances_key)
            .and_then(Value::as_sequence_mut)
            .ok_or_else(|| {
                MigrationError(format!(
                    "cannot migrate UI route `{}`: group `{}` instances must be a list",
                    route.ui, route.group
                ))
            })?;
        let mut added = Vec::new();
        for member in [&route.backend, &route.ui] {
            if !instances
                .iter()
                .any(|candidate| candidate.as_str() == Some(member))
            {
                instances.push(Value::String(member.clone()));
                added.push(member.as_str());
            }
        }
        changes.push(format!(
            "spec.groups.{}.address: {} (from spec.uiRoutes.{}.origin)",
            route.group, route.domain, route.ui
        ));
        if !added.is_empty() {
            changes.push(format!(
                "spec.groups.{}.instances: added {}",
                route.group,
                added.join(", ")
            ));
        }
    }

    let mut ui_providers = BTreeMap::<String, BTreeSet<String>>::new();
    if let Some(upstreams) = spec.get("hostUpstreams").and_then(Value::as_mapping) {
        for (provider, upstream) in upstreams {
            let (Some(provider), Some(instance)) = (
                provider.as_str(),
                upstream.get("instance").and_then(Value::as_str),
            ) else {
                continue;
            };
            ui_providers
                .entry(instance.to_owned())
                .or_default()
                .insert(provider.to_owned());
        }
    }

    let mut removed_destinations = 0;
    let mut removed_browser_routes = 0;
    if let Some(router_spec) = spec
        .get_mut("hostRouter")
        .and_then(Value::as_mapping_mut)
        .and_then(|router| router.get_mut("spec"))
        .and_then(Value::as_mapping_mut)
    {
        let route_targets = router_spec
            .get("routes")
            .and_then(Value::as_sequence)
            .into_iter()
            .flatten()
            .filter_map(|route| {
                let route = route.as_mapping()?;
                Some((
                    route
                        .get("consumer")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    route.get("slot")?.as_str()?.to_owned(),
                    route.get("provider")?.as_str()?.to_owned(),
                ))
            })
            .collect::<Vec<_>>();
        if let Some(listeners) = router_spec
            .get_mut("listeners")
            .and_then(Value::as_sequence_mut)
        {
            for listener in listeners {
                let Some(listener) = listener.as_mapping_mut() else {
                    continue;
                };
                let consumer = listener
                    .get("consumer")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let Some(destinations) = listener
                    .get_mut("destinations")
                    .and_then(Value::as_sequence_mut)
                else {
                    continue;
                };
                let before = destinations.len();
                destinations.retain(|destination| {
                    let Some(destination) = destination.as_mapping() else {
                        return true;
                    };
                    if destination.get("kind").and_then(Value::as_str) != Some("custom_domain") {
                        return true;
                    }
                    let (Some(domain), Some(slot)) = (
                        destination.get("domain").and_then(Value::as_str),
                        destination.get("slot").and_then(Value::as_str),
                    ) else {
                        return true;
                    };
                    !routes.iter().any(|legacy| {
                        legacy
                            .domain
                            .trim_end_matches('.')
                            .eq_ignore_ascii_case(domain.trim_end_matches('.'))
                            && ui_providers.get(&legacy.ui).is_some_and(|providers| {
                                route_targets.iter().any(
                                    |(route_consumer, route_slot, provider)| {
                                        route_consumer == &consumer
                                            && route_slot == slot
                                            && providers.contains(provider)
                                    },
                                )
                            })
                    })
                });
                removed_destinations += before - destinations.len();
            }
        }
        if let Some(browser_routes) = router_spec
            .get_mut("browserRoutes")
            .and_then(Value::as_sequence_mut)
        {
            let explicit_templates = browser_routes
                .iter()
                .filter_map(|browser_route| {
                    let browser_route = browser_route.as_mapping()?;
                    let identity = browser_route.get("identity")?.as_mapping()?;
                    if identity.get("source")?.as_str()? != "explicit_header" {
                        return None;
                    }
                    Some((
                        identity.get("value")?.as_str()?.to_owned(),
                        (
                            browser_route.get("destination")?.as_str()?.to_owned(),
                            browser_route.get("provider")?.as_str()?.to_owned(),
                        ),
                    ))
                })
                .fold(
                    BTreeMap::<String, BTreeSet<(String, String)>>::new(),
                    |mut templates, (ui, route)| {
                        templates.entry(ui).or_default().insert(route);
                        templates
                    },
                );
            let before = browser_routes.len();
            browser_routes.retain(|browser_route| {
                let Some(browser_route) = browser_route.as_mapping() else {
                    return true;
                };
                let Some(identity) = browser_route.get("identity").and_then(Value::as_mapping)
                else {
                    return true;
                };
                if identity.get("source").and_then(Value::as_str) != Some("origin") {
                    return true;
                }
                let (Some(origin), Some(destination), Some(provider)) = (
                    identity.get("origin").and_then(Value::as_str),
                    browser_route.get("destination").and_then(Value::as_str),
                    browser_route.get("provider").and_then(Value::as_str),
                ) else {
                    return true;
                };
                !routes.iter().any(|legacy| {
                    legacy.origin == origin
                        && explicit_templates.get(&legacy.ui).is_some_and(|templates| {
                            templates.contains(&(destination.to_owned(), provider.to_owned()))
                        })
                })
            });
            removed_browser_routes = before - browser_routes.len();
        }
    }
    changes.push(format!(
        "spec.uiRoutes: removed {} {}",
        routes.len(),
        if routes.len() == 1 {
            "entry"
        } else {
            "entries"
        }
    ));
    if removed_destinations > 0 {
        changes.push(format!(
            "spec.hostRouter: removed {removed_destinations} generated custom-domain {}",
            if removed_destinations == 1 {
                "destination"
            } else {
                "destinations"
            }
        ));
    }
    if removed_browser_routes > 0 {
        changes.push(format!(
            "spec.hostRouter: removed {removed_browser_routes} generated origin browser {}",
            if removed_browser_routes == 1 {
                "route"
            } else {
                "routes"
            }
        ));
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
    fn migrates_ui_routes_to_group_addresses_and_is_idempotent() {
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
    ui:
      services:
        app:
          execution: { type: container, image: example/ui:1 }
          provides: { ui: { protocol: http, port: 8080 } }
          publish: [8080]
    backend:
      services:
        app:
          execution: { type: container, image: example/backend:1 }
          consumes:
            search: { protocol: http, address: { host: 127.0.0.1, port: 8001 } }
          provides: { backend: { protocol: http, port: 8081 } }
          publish: [8081]
    search:
      services:
        app:
          execution: { type: container, image: example/search:1 }
          provides: { search: { protocol: http, port: 8001 } }
  instances:
    - { name: ui-a, block: ui, source: app }
    - { name: backend-a, block: backend, source: app }
    - { name: search-a, block: search, source: app }
  groups:
    feature:
      providers: { search: search-a/app }
  bindings:
    backend-a: feature
  uiRoutes:
    ui-a:
      origin: http://ui-a.demo.localhost:18080
      backend: backend-a
      downstreamGroup: feature
  hostRouter:
    apiVersion: switchyard.dev/router/v1alpha1
    kind: RouterConfiguration
    metadata: { deployment: demo }
    spec:
      snapshot:
        id: demo-host
        version: 1
        transitions:
          http: { strategy: close }
          https: { strategy: close }
          websocket: { strategy: close }
          grpc: { strategy: close }
          tcp: { strategy: close }
      listeners:
        - consumer: gateway
          bind: { host: 127.0.0.1, port: 18080 }
          protocol: http
          destinations:
            - { kind: custom_domain, slot: ui-a-domain, domain: ui-a.demo.localhost }
        - bind: { host: 127.0.0.1, port: 10081 }
          protocol: http
          destinations:
            - { kind: legacy_localhost, slot: browser-backend, host: localhost }
            - { kind: legacy_localhost, slot: browser-metrics, host: metrics.localhost }
      providers:
        - { id: ui-a, endpoint: { protocol: http, host: 127.0.0.1, port: 0 } }
        - { id: backend-a, endpoint: { protocol: http, host: 127.0.0.1, port: 0 } }
      groups: []
      bindings: []
      routes:
        - { consumer: gateway, slot: ui-a-domain, provider: ui-a }
      browserRoutes:
        - { identity: { source: origin, origin: "http://ui-a.demo.localhost:18080" }, destination: browser-backend, provider: backend-a }
        - { identity: { source: origin, origin: "http://ui-a.demo.localhost:18080" }, destination: browser-metrics, provider: backend-a }
        - { identity: { source: explicit_header, value: ui-a }, destination: browser-backend, provider: backend-a }
      identity: { explicitHeader: X-Switchyard-Route, stripBeforeForwarding: true }
  hostUpstreams:
    ui-a: { instance: ui-a, service: app, port: 8080 }
    backend-a: { instance: backend-a, service: app, port: 8081 }
"#,
        )
        .unwrap();

        let result = migrate(&path, || {}).unwrap();
        assert!(result.changed);
        assert!(result.changes.iter().any(|change| {
            change == "spec.groups.feature.address: ui-a.demo.localhost (from spec.uiRoutes.ui-a.origin)"
        }));
        let first = fs::read_to_string(&path).unwrap();
        let document: Value = serde_yaml::from_str(&first).unwrap();
        assert!(document["spec"].get("uiRoutes").is_none());
        assert_eq!(
            document["spec"]["groups"]["feature"]["address"].as_str(),
            Some("ui-a.demo.localhost")
        );
        assert_eq!(
            document["spec"]["groups"]["feature"]["instances"]
                .as_sequence()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>(),
            ["search-a/app", "backend-a", "ui-a"]
        );
        let remaining_origins = document["spec"]["hostRouter"]["spec"]["browserRoutes"]
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(Value::as_mapping)
            .filter(|route| {
                route
                    .get("identity")
                    .and_then(Value::as_mapping)
                    .and_then(|identity| identity.get("source"))
                    .and_then(Value::as_str)
                    == Some("origin")
            })
            .collect::<Vec<_>>();
        assert_eq!(remaining_origins.len(), 1);
        assert_eq!(
            remaining_origins[0]
                .get("destination")
                .and_then(Value::as_str),
            Some("browser-metrics")
        );
        let bundle = switchyard_planner::load_bundle(&path).unwrap();
        let plan = switchyard_planner::plan(&bundle).unwrap();
        let router: router_config::RouterConfig =
            serde_json::from_str(plan.host_router_config.as_ref().unwrap()).unwrap();
        assert!(router.spec.listeners.iter().any(|listener| {
            listener.destinations.iter().any(|destination| {
                matches!(
                    destination,
                    router_config::ListenerDestination::CustomDomain { domain, .. }
                        if domain == "ui-a.demo.localhost"
                )
            })
        }));
        assert!(router.spec.browser_routes.iter().any(|route| matches!(
            &route.identity,
            router_config::BrowserIdentity::Origin { origin }
                if origin == "http://ui-a.demo.localhost:18080"
        )));

        let second = migrate(&path, || panic!("an up-to-date file must not be rewritten")).unwrap();
        assert!(!second.changed);
        assert!(second.changes.is_empty());
        assert_eq!(fs::read_to_string(&path).unwrap(), first);
    }

    #[test]
    fn refuses_multiple_ui_routes_for_one_group_without_writing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("deployment.yaml");
        let original = r#"apiVersion: switchyard.dev/v1alpha2
kind: Deployment
metadata: { name: demo }
spec:
  uiRoutes:
    ui-a:
      origin: http://ui-a.demo.localhost:18080
      backend: backend-a
      downstreamGroup: shared
    ui-b:
      origin: http://ui-b.demo.localhost:18080
      backend: backend-a
      downstreamGroup: shared
"#;
        fs::write(&path, original).unwrap();

        let error = migrate(&path, || {
            panic!("a refused migration must not prepare to write")
        })
        .expect_err("singular group addresses cannot represent two legacy routes");
        assert!(error.to_string().contains(
            "group `shared` has more than one UI route, but an object may declare only one address"
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
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
