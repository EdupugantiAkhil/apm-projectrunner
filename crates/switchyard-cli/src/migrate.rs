use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    path::{Path, PathBuf},
    process::Command,
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

    let mut changes = Vec::new();
    if previous_version {
        changes.push(format!(
            "apiVersion: {PREVIOUS_API_VERSION} -> {API_VERSION}"
        ));
    }
    migrate_group_instances(&mut document, &mut changes)?;
    materialize_group_instances(&mut document, &mut changes)?;
    remove_identity_routing_metadata(&mut document, &mut changes)?;
    migrate_ui_routes(&mut document, &mut changes)?;
    migrate_connections(&mut document, &mut changes)?;
    migrate_sources(&mut document, path, &mut changes)?;
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
    if let Err(diagnostics) = switchyard_planner::plan(&bundle) {
        return Err(MigrationError(format!(
            "refusing to write `{}` because the migrated deployment is invalid: {}",
            path.display(),
            serde_json::to_string(&diagnostics).unwrap_or_else(|_| "invalid deployment".into())
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

fn migrate_connections(
    document: &mut Value,
    changes: &mut Vec<String>,
) -> Result<(), MigrationError> {
    let spec = document
        .get_mut("spec")
        .and_then(Value::as_mapping_mut)
        .ok_or_else(|| MigrationError("cannot migrate: spec must be a mapping".into()))?;

    let mut occurrences = BTreeMap::<String, Vec<(String, usize)>>::new();
    for (group_name, group) in spec
        .get("groups")
        .and_then(Value::as_mapping)
        .into_iter()
        .flatten()
    {
        let group_name = group_name.as_str().ok_or_else(|| {
            MigrationError("cannot migrate: every group name must be a string".into())
        })?;
        for (index, member) in group
            .get("instances")
            .and_then(Value::as_sequence)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let member = member.as_str().ok_or_else(|| {
                MigrationError(format!(
                    "cannot migrate: spec.groups.{group_name}.instances[{index}] must be a string"
                ))
            })?;
            let instance = member.split('/').next().unwrap_or(member);
            occurrences
                .entry(instance.to_owned())
                .or_default()
                .push((group_name.to_owned(), index));
        }
    }

    let duplicates = occurrences
        .iter()
        .filter(|(_, entries)| entries.len() > 1)
        .map(|(instance, entries)| {
            format!(
                "`{instance}` at {}",
                entries
                    .iter()
                    .map(|(group, index)| format!("spec.groups.{group}.instances[{index}]"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .collect::<Vec<_>>();
    if !duplicates.is_empty() {
        return Err(MigrationError(format!(
            "cannot migrate instances listed in several groups: {}; create a separate instance for each group",
            duplicates.join("; ")
        )));
    }

    if let Some(bindings) = spec.get("bindings") {
        let bindings = if bindings.is_null() {
            None
        } else {
            Some(bindings.as_mapping().ok_or_else(|| {
                MigrationError("cannot migrate: spec.bindings must be a mapping".into())
            })?)
        };
        let mut conflicts = Vec::new();
        if let Some(bindings) = bindings {
            for (instance, selected_group) in bindings {
                let instance = instance.as_str().ok_or_else(|| {
                    MigrationError(
                        "cannot migrate: every spec.bindings key must be a string".into(),
                    )
                })?;
                let selected_group = selected_group.as_str().ok_or_else(|| {
                    MigrationError(format!(
                        "cannot migrate: spec.bindings.{instance} must name a group"
                    ))
                })?;
                match occurrences.get(instance).and_then(|entries| entries.first()) {
                    Some((member_group, _)) if member_group == selected_group => {}
                    Some((member_group, index)) => conflicts.push(format!(
                        "spec.bindings.{instance} selects `{selected_group}` but \
                         spec.groups.{member_group}.instances[{index}] places it in `{member_group}`"
                    )),
                    None => conflicts.push(format!(
                        "spec.bindings.{instance} selects `{selected_group}` but the instance is not a member of any group"
                    )),
                }
            }
        }
        if !conflicts.is_empty() {
            return Err(MigrationError(format!(
                "cannot migrate disagreeing connection declarations: {}",
                conflicts.join("; ")
            )));
        }
        let count = bindings.map_or(0, serde_yaml::Mapping::len);
        spec.remove("bindings");
        changes.push(format!(
            "spec.bindings: removed {count} redundant membership {}",
            if count == 1 {
                "selection"
            } else {
                "selections"
            }
        ));
    }

    if let Some(routes) = spec.get("routes") {
        let empty = routes.is_null()
            || routes
                .as_mapping()
                .is_some_and(serde_yaml::Mapping::is_empty)
            || routes.as_sequence().is_some_and(Vec::is_empty);
        if !empty {
            return Err(MigrationError(
                "cannot migrate non-empty spec.routes: direct routes bypass group membership; express the connection through one group's instances list"
                    .into(),
            ));
        }
        spec.remove("routes");
        changes.push("spec.routes: removed empty compatibility section".into());
    }
    Ok(())
}

fn migrate_sources(
    document: &mut Value,
    deployment: &Path,
    changes: &mut Vec<String>,
) -> Result<(), MigrationError> {
    let Some(spec) = document.get_mut("spec").and_then(Value::as_mapping_mut) else {
        return Ok(());
    };
    let sources_key = Value::String("sources".into());
    let Some(mut sources_value) = spec.remove(&sources_key) else {
        return Ok(());
    };
    let sources = sources_value
        .as_mapping_mut()
        .ok_or_else(|| MigrationError("cannot migrate: spec.sources must be a mapping".into()))?;
    let mut repositories = spec
        .remove(Value::String("repositories".into()))
        .unwrap_or_else(|| Value::Mapping(Default::default()))
        .as_mapping()
        .cloned()
        .ok_or_else(|| {
            MigrationError("cannot migrate: spec.repositories must be a mapping".into())
        })?;
    let mut adopted = BTreeMap::<String, String>::new();
    for (name, repository) in &repositories {
        if let (Some(name), Some(clone)) = (
            name.as_str(),
            repository.get("clone").and_then(Value::as_str),
        ) {
            adopted.insert(clone.into(), name.into());
        }
    }
    let definition_dir = deployment
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));

    for (source_name, source) in sources {
        let name = source_name.as_str().ok_or_else(|| {
            MigrationError("cannot migrate: every source name must be a string".into())
        })?;
        let mapping = source.as_mapping_mut().ok_or_else(|| {
            MigrationError(format!(
                "cannot migrate source `{name}`: definition must be a mapping"
            ))
        })?;
        let path = mapping
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                MigrationError(format!(
                    "cannot migrate source `{name}`: path must be a string"
                ))
            })?
            .to_owned();
        let already_current = mapping.get("repository").and_then(Value::as_str).is_some()
            && mapping.get("ref").and_then(Value::as_str).is_some()
            && mapping.get("type").is_none();
        if already_current {
            continue;
        }

        let source_path = resolve_migration_path(definition_dir, Path::new(&path));
        let legacy_repository = mapping
            .get("repository")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let repository_path = match legacy_repository {
            Some(repository) => repository,
            None => discover_separate_repository(name, &source_path, definition_dir)?,
        };
        let reference = match mapping.get("ref").and_then(Value::as_str) {
            Some(reference) if !reference.trim().is_empty() => reference.to_owned(),
            _ => discover_ref(name, &source_path)?,
        };
        let repository_name = if let Some(existing) = adopted.get(&repository_path) {
            existing.clone()
        } else {
            let base = migration_repository_name(&repository_path);
            let mut candidate = base.clone();
            let mut suffix = 2;
            while repositories.contains_key(Value::String(candidate.clone())) {
                candidate = format!("{base}-{suffix}");
                suffix += 1;
            }
            repositories.insert(
                Value::String(candidate.clone()),
                serde_yaml::to_value(BTreeMap::from([("clone", repository_path.clone())]))
                    .expect("string repository definition serializes"),
            );
            adopted.insert(repository_path.clone(), candidate.clone());
            candidate
        };
        mapping.remove("type");
        mapping.insert(
            Value::String("repository".into()),
            Value::String(repository_name),
        );
        mapping.insert(Value::String("ref".into()), Value::String(reference));
        changes.push(format!(
            "source `{name}` now references a declared adopted repository and ref"
        ));
    }
    spec.insert(
        Value::String("repositories".into()),
        Value::Mapping(repositories),
    );
    spec.insert(sources_key, sources_value);
    Ok(())
}

fn resolve_migration_path(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        base.join(path)
    }
}

fn discover_separate_repository(
    name: &str,
    source: &Path,
    definition_dir: &Path,
) -> Result<String, MigrationError> {
    let worktrees = git_output(source, &["worktree", "list", "--porcelain"]).map_err(|error| {
        MigrationError(format!(
            "cannot migrate plain-path source `{name}`: {error}; create a separate Git worktree and author its repository and ref"
        ))
    })?;
    let repository = worktrees
        .lines()
        .find_map(|line| line.strip_prefix("worktree "))
        .map(PathBuf::from)
        .ok_or_else(|| {
            MigrationError(format!(
                "cannot migrate plain-path source `{name}`: Git did not report a repository worktree"
            ))
        })?;
    let source = source.canonicalize().map_err(|error| {
        MigrationError(format!(
            "cannot migrate plain-path source `{name}` at `{}`: {error}",
            source.display()
        ))
    })?;
    let repository = repository.canonicalize().map_err(|error| {
        MigrationError(format!(
            "cannot migrate repository for source `{name}` at `{}`: {error}",
            repository.display()
        ))
    })?;
    if source == repository || source.starts_with(&repository) || repository.starts_with(&source) {
        return Err(MigrationError(format!(
            "cannot migrate plain-path source `{name}` because its checkout overlaps the repository clone; create a separate worktree, then update the source path"
        )));
    }
    Ok(repository
        .strip_prefix(definition_dir)
        .map(Path::to_path_buf)
        .unwrap_or(repository)
        .to_string_lossy()
        .into_owned())
}

fn discover_ref(name: &str, source: &Path) -> Result<String, MigrationError> {
    let branch = git_output(source, &["branch", "--show-current"]).map_err(|error| {
        MigrationError(format!("cannot determine ref for source `{name}`: {error}"))
    })?;
    if !branch.is_empty() {
        Ok(branch)
    } else {
        git_output(source, &["rev-parse", "HEAD"]).map_err(|error| {
            MigrationError(format!("cannot determine ref for source `{name}`: {error}"))
        })
    }
}

fn git_output(directory: &Path, arguments: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .output()
        .map_err(|error| format!("could not run Git: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().into())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().into())
    }
}

fn migration_repository_name(path: &str) -> String {
    let candidate = Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("repository");
    let mut name = candidate
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    while name.contains("--") {
        name = name.replace("--", "-");
    }
    let name = name.trim_matches('-');
    if name.is_empty() {
        "repository".into()
    } else if name.starts_with(|character: char| character.is_ascii_lowercase()) {
        name.chars()
            .take(63)
            .collect::<String>()
            .trim_end_matches('-')
            .into()
    } else {
        format!("repository-{name}")
            .chars()
            .take(63)
            .collect::<String>()
            .trim_end_matches('-')
            .into()
    }
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

fn materialize_group_instances(
    document: &mut Value,
    changes: &mut Vec<String>,
) -> Result<(), MigrationError> {
    fn member_capabilities(document: &Value, member: &str) -> BTreeSet<String> {
        let (instance_name, requested_service) = member
            .split_once('/')
            .map_or((member, None), |(instance, service)| {
                (instance, Some(service))
            });
        let block_name = document
            .get("spec")
            .and_then(|spec| spec.get("instances"))
            .and_then(Value::as_sequence)
            .into_iter()
            .flatten()
            .find(|instance| instance.get("name").and_then(Value::as_str) == Some(instance_name))
            .and_then(|instance| instance.get("block"))
            .and_then(Value::as_str);
        document
            .get("spec")
            .and_then(|spec| spec.get("blocks"))
            .and_then(|blocks| block_name.and_then(|block| blocks.get(block)))
            .and_then(|block| block.get("services"))
            .and_then(Value::as_mapping)
            .into_iter()
            .flatten()
            .filter(|(service, _)| {
                requested_service.is_none_or(|requested| service.as_str() == Some(requested))
            })
            .flat_map(|(_, service)| {
                service
                    .get("provides")
                    .and_then(Value::as_mapping)
                    .into_iter()
                    .flatten()
                    .filter_map(|(name, _)| name.as_str().map(str::to_owned))
            })
            .collect()
    }

    fn resolve(
        name: &str,
        document: &Value,
        visiting: &mut BTreeSet<String>,
        resolved: &mut BTreeMap<String, Vec<String>>,
    ) -> Result<Vec<String>, MigrationError> {
        if let Some(members) = resolved.get(name) {
            return Ok(members.clone());
        }
        if !visiting.insert(name.to_owned()) {
            return Err(MigrationError(format!(
                "cannot migrate group `{name}`: inheritance contains a cycle"
            )));
        }
        let group = document
            .get("spec")
            .and_then(|spec| spec.get("groups"))
            .and_then(|groups| groups.get(name))
            .ok_or_else(|| {
                MigrationError(format!(
                    "cannot migrate group `{name}`: group does not exist"
                ))
            })?;
        let mut members = if let Some(parent) = group.get("extends") {
            let parent = parent.as_str().ok_or_else(|| {
                MigrationError(format!(
                    "cannot migrate group `{name}`: extends must name a group"
                ))
            })?;
            resolve(parent, document, visiting, resolved)?
        } else {
            Vec::new()
        };
        for member in group
            .get("instances")
            .and_then(Value::as_sequence)
            .into_iter()
            .flatten()
        {
            let member = member.as_str().ok_or_else(|| {
                MigrationError(format!(
                    "cannot migrate group `{name}`: every member must be a string"
                ))
            })?;
            let capabilities = member_capabilities(document, member);
            if !capabilities.is_empty() {
                members.retain(|inherited| {
                    member_capabilities(document, inherited).is_disjoint(&capabilities)
                });
            }
            if !members.iter().any(|existing| existing == member) {
                members.push(member.to_owned());
            }
        }
        visiting.remove(name);
        resolved.insert(name.to_owned(), members.clone());
        Ok(members)
    }

    let group_names = document
        .get("spec")
        .and_then(|spec| spec.get("groups"))
        .and_then(Value::as_mapping)
        .into_iter()
        .flatten()
        .filter_map(|(name, _)| name.as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    let mut resolved = BTreeMap::new();
    for name in &group_names {
        resolve(name, document, &mut BTreeSet::new(), &mut resolved)?;
    }
    let mut groups = document
        .get_mut("spec")
        .and_then(|spec| spec.get_mut("groups"))
        .and_then(Value::as_mapping_mut);
    for (name, members) in resolved {
        let group = groups
            .as_deref_mut()
            .and_then(|groups| groups.get_mut(name.as_str()))
            .and_then(Value::as_mapping_mut)
            .ok_or_else(|| MigrationError(format!("cannot migrate group `{name}`")))?;
        if group.remove("extends").is_some() {
            group.insert(
                Value::String("instances".into()),
                Value::Sequence(members.iter().cloned().map(Value::String).collect()),
            );
            changes.push(format!(
                "spec.groups.{name}: materialized complete ordered instances list and removed extends"
            ));
        }
    }
    Ok(())
}

fn remove_identity_routing_metadata(
    document: &mut Value,
    changes: &mut Vec<String>,
) -> Result<(), MigrationError> {
    let spec = document
        .get_mut("spec")
        .and_then(Value::as_mapping_mut)
        .ok_or_else(|| MigrationError("cannot migrate: spec must be a mapping".into()))?;
    let provider_ports = spec
        .get("blocks")
        .and_then(Value::as_mapping)
        .into_iter()
        .flatten()
        .flat_map(|(_, block)| {
            block
                .get("services")
                .and_then(Value::as_mapping)
                .into_iter()
                .flatten()
        })
        .flat_map(|(_, service)| {
            service
                .get("provides")
                .and_then(Value::as_mapping)
                .into_iter()
                .flatten()
                .filter_map(|(slot, provider)| {
                    Some((slot.as_str()?.to_owned(), provider.get("port")?.as_u64()?))
                })
        })
        .fold(
            BTreeMap::<String, BTreeSet<u64>>::new(),
            |mut ports, (slot, port)| {
                ports.entry(slot).or_default().insert(port);
                ports
            },
        );
    let Some(blocks) = spec.get_mut("blocks").and_then(Value::as_mapping_mut) else {
        return Ok(());
    };
    let mut removed_provides = 0;
    let mut removed_consumes = 0;
    for (block_name, block) in blocks {
        let block_name = block_name.as_str().unwrap_or("<non-string>");
        let Some(services) = block.get_mut("services").and_then(Value::as_mapping_mut) else {
            continue;
        };
        for (service_name, service) in services {
            let service_name = service_name.as_str().unwrap_or("<non-string>");
            let Some(service) = service.as_mapping_mut() else {
                continue;
            };
            if let Some(consumes) = service.get("consumes").and_then(Value::as_mapping) {
                for (slot, definition) in consumes {
                    let slot = slot.as_str().unwrap_or("<non-string>");
                    let host = definition
                        .get("address")
                        .and_then(|address| address.get("host"))
                        .and_then(Value::as_str)
                        .unwrap_or("127.0.0.1");
                    if host != "localhost"
                        && host
                            .parse::<std::net::IpAddr>()
                            .map_or(true, |ip| !ip.is_loopback())
                    {
                        return Err(MigrationError(format!(
                            "cannot remove spec.blocks.{block_name}.services.{service_name}.consumes.{slot}: host `{host}` is a real remap; replace it with identity loopback routing or keep the deployment on the old version"
                        )));
                    }
                    let port = definition
                        .get("address")
                        .and_then(|address| address.get("port"))
                        .and_then(Value::as_u64)
                        .ok_or_else(|| MigrationError(format!(
                            "cannot remove spec.blocks.{block_name}.services.{service_name}.consumes.{slot}: address.port is required to prove identity routing"
                        )))?;
                    let matching_ports = provider_ports.get(slot).cloned().unwrap_or_default();
                    if matching_ports != BTreeSet::from([port]) {
                        return Err(MigrationError(format!(
                            "cannot remove spec.blocks.{block_name}.services.{service_name}.consumes.{slot}: consumer port {port} does not map port-for-port to exactly one provider port (found {})",
                            matching_ports
                                .iter()
                                .map(u64::to_string)
                                .collect::<Vec<_>>()
                                .join(", ")
                        )));
                    }
                }
            }
            if service.remove("provides").is_some() {
                removed_provides += 1;
            }
            if service.remove("consumes").is_some() {
                removed_consumes += 1;
            }
        }
    }
    if removed_provides > 0 || removed_consumes > 0 {
        changes.push(format!(
            "spec.blocks: removed provides from {removed_provides} services and consumes from {removed_consumes} services after verifying identity loopback routing"
        ));
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
    for route in &routes {
        let group = spec
            .get_mut("groups")
            .and_then(Value::as_mapping_mut)
            .ok_or_else(|| {
                MigrationError("cannot migrate uiRoutes: spec.groups must be a mapping".into())
            })?
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
        let instance_address = format!("{}.{}", route.ui, route.domain);
        let instance = spec
            .get_mut("instances")
            .and_then(Value::as_sequence_mut)
            .into_iter()
            .flatten()
            .find(|instance| instance.get("name").and_then(Value::as_str) == Some(&route.ui))
            .and_then(Value::as_mapping_mut)
            .ok_or_else(|| {
                MigrationError(format!(
                    "cannot migrate UI route `{}`: UI instance does not exist",
                    route.ui
                ))
            })?;
        match instance.get("address") {
            Some(Value::String(address)) if address == &instance_address => {}
            Some(Value::String(address)) => {
                return Err(MigrationError(format!(
                    "cannot migrate UI route `{}`: instance already has address `{address}` instead of `{instance_address}`",
                    route.ui
                )));
            }
            Some(_) => {
                return Err(MigrationError(format!(
                    "cannot migrate UI route `{}`: instance address must be a string",
                    route.ui
                )));
            }
            None => {
                instance.insert(
                    Value::String("address".into()),
                    Value::String(instance_address.clone()),
                );
                changes.push(format!(
                    "spec.instances.{}.address: {instance_address}",
                    route.ui
                ));
            }
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

    fn current_membership_fixture() -> Value {
        serde_yaml::from_str(include_str!(
            "../../switchyard-planner/tests/fixtures/deployment.yaml"
        ))
        .unwrap()
    }

    #[test]
    fn removes_redundant_bindings_and_empty_routes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("deployment.yaml");
        let mut document = current_membership_fixture();
        document["spec"]["bindings"] =
            serde_yaml::from_str("consumer-a: base\nconsumer-b: feature\n").unwrap();
        document["spec"]["routes"] = Value::Mapping(Default::default());
        fs::write(&path, serde_yaml::to_string(&document).unwrap()).unwrap();

        let result = migrate(&path, || {}).unwrap();
        assert!(
            result
                .changes
                .iter()
                .any(|change| change.starts_with("spec.bindings: removed 2"))
        );
        let migrated: Value = serde_yaml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(migrated["spec"].get("bindings").is_none());
        assert!(migrated["spec"].get("routes").is_none());
    }

    #[test]
    fn refuses_disagreeing_binding_without_writing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("deployment.yaml");
        let mut document = current_membership_fixture();
        document["spec"]["bindings"] = serde_yaml::from_str("consumer-a: feature\n").unwrap();
        let original = serde_yaml::to_string(&document).unwrap();
        fs::write(&path, &original).unwrap();

        let error = migrate(&path, || panic!("refused migration must not write"))
            .expect_err("disagreeing connection declarations must fail");
        assert!(error.to_string().contains("consumer-a"));
        assert!(error.to_string().contains("places it in `base`"));
        assert!(error.to_string().contains("selects `feature`"));
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn refuses_multi_group_membership_and_names_every_occurrence() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("deployment.yaml");
        let mut document = current_membership_fixture();
        document["spec"]["groups"]["feature"]["instances"]
            .as_sequence_mut()
            .unwrap()
            .push(Value::String("consumer-a/api".into()));
        let original = serde_yaml::to_string(&document).unwrap();
        fs::write(&path, &original).unwrap();

        let error = migrate(&path, || panic!("refused migration must not write"))
            .expect_err("multi-group membership must fail");
        let message = error.to_string();
        assert!(message.contains("spec.groups.base.instances[1]"));
        assert!(message.contains("spec.groups.feature.instances[2]"));
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }

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
  repositories:
    fixture: { url: https://example.invalid/repository.git }
  sources:
    app: { repository: fixture, ref: main, path: worktrees/app }
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
    - { name: db-feature, block: database, source: app }
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
        database: db-feature
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
                "spec.groups.feature: providers mapping with 3 entries -> instances list with 2 unique members",
                "spec.groups.feature: materialized complete ordered instances list and removed extends",
                "spec.blocks: removed provides from 3 services and consumes from 0 services after verifying identity loopback routing",
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
            ["ai-feature/suite", "db-feature"]
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
  repositories:
    fixture: { url: https://example.invalid/repository.git }
  sources:
    app: { repository: fixture, ref: main, path: worktrees/app }
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
  repositories:
    fixture: { url: https://example.invalid/repository.git }
  sources:
    app: { repository: fixture, ref: main, path: worktrees/app }
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
        assert_eq!(result.changes.len(), 3);
        assert!(
            !fs::read_to_string(path)
                .unwrap()
                .contains("hand-authored comment")
        );
    }

    #[test]
    fn refuses_a_real_port_remap_without_writing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("deployment.yaml");
        let original = r#"apiVersion: switchyard.dev/v1alpha1
kind: Deployment
metadata: { name: demo }
spec:
  repositories:
    fixture: { url: https://example.invalid/repository.git }
  sources:
    app: { repository: fixture, ref: main, path: worktrees/app }
  blocks:
    provider:
      services:
        api:
          execution: { type: container, image: example/provider:1 }
          provides:
            search: { protocol: http, port: 8080 }
    consumer:
      services:
        api:
          execution: { type: container, image: example/consumer:1 }
          consumes:
            search: { protocol: http, address: { host: 127.0.0.1, port: 9000 } }
  instances:
    - { name: provider-main, block: provider, source: app }
    - { name: consumer-main, block: consumer, source: app }
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

        assert!(error.contains("consumer port 9000"));
        assert!(error.contains("provider port (found 8080)"));
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

    #[test]
    fn migrates_legacy_worktrees_to_one_adopted_repository() {
        let directory = tempfile::tempdir().unwrap();
        let repository = directory.path().join("repository");
        let project = directory.path().join("project");
        fs::create_dir(&repository).unwrap();
        fs::create_dir(&project).unwrap();
        for arguments in [
            vec!["init", "-b", "main"],
            vec!["config", "user.email", "tests@switchyard.invalid"],
            vec!["config", "user.name", "Switchyard Tests"],
        ] {
            let output = Command::new("git")
                .arg("-C")
                .arg(&repository)
                .args(arguments)
                .output()
                .unwrap();
            assert!(output.status.success());
        }
        fs::write(repository.join("tracked"), "initial\n").unwrap();
        for arguments in [vec!["add", "tracked"], vec!["commit", "-m", "initial"]] {
            let output = Command::new("git")
                .arg("-C")
                .arg(&repository)
                .args(arguments)
                .output()
                .unwrap();
            assert!(output.status.success());
        }
        for source in ["main", "other"] {
            let target = project.join("sources").join(source);
            let output = Command::new("git")
                .arg("-C")
                .arg(&repository)
                .args([
                    "worktree",
                    "add",
                    "--detach",
                    target.to_str().unwrap(),
                    "main",
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let path = project.join("deployment.yaml");
        let original = r#"apiVersion: switchyard.dev/v1alpha2
kind: Deployment
metadata: { name: demo }
spec:
  sources:
    main: { type: worktree, repository: ../repository, ref: main, path: sources/main }
    other: { type: worktree, repository: ../repository, ref: main, path: sources/other }
"#;
        fs::write(&path, original).unwrap();

        let result = migrate(&path, || {}).unwrap();
        assert!(result.changed);
        let migrated: Value = serde_yaml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            migrated["spec"]["repositories"].as_mapping().unwrap().len(),
            1
        );
        let repository_name = migrated["spec"]["sources"]["main"]["repository"]
            .as_str()
            .unwrap();
        assert_eq!(
            migrated["spec"]["sources"]["other"]["repository"].as_str(),
            Some(repository_name)
        );
        assert_eq!(
            migrated["spec"]["repositories"][repository_name]["clone"].as_str(),
            Some("../repository")
        );
        assert!(migrated["spec"]["sources"]["main"].get("type").is_none());
    }

    #[test]
    fn refuses_to_migrate_a_repository_checkout_as_a_source() {
        let directory = tempfile::tempdir().unwrap();
        let output = Command::new("git")
            .arg("-C")
            .arg(directory.path())
            .args(["init", "-b", "main"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let path = directory.path().join("deployment.yaml");
        let original = r#"apiVersion: switchyard.dev/v1alpha2
kind: Deployment
metadata: { name: demo }
spec:
  sources:
    app: { path: . }
"#;
        fs::write(&path, original).unwrap();

        let error = migrate(&path, || {}).unwrap_err().to_string();
        assert!(error.contains("checkout overlaps the repository"));
        assert!(error.contains("create a separate worktree"));
        assert_eq!(fs::read_to_string(path).unwrap(), original);
    }
}
