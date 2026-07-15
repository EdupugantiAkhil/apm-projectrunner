//! Synchronous, daemon-neutral, non-destructive source and worktree management.

use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::json;
use switchyard_adapter_sdk::{SourceAdapter, SourceIdentity};
use switchyard_adapters::{SourceGitAdapter, SourcePathAdapter};
use switchyard_state::{RegisteredSource, RegisteredSourceKind, StateError, StateStore};

/// A stable source-management failure suitable for API and CLI translation.
#[derive(Debug)]
pub struct SourceError {
    code: &'static str,
    message: String,
}

impl SourceError {
    /// Returns the stable machine-readable failure code.
    pub const fn code(&self) -> &'static str {
        self.code
    }

    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for SourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for SourceError {}

impl From<StateError> for SourceError {
    fn from(error: StateError) -> Self {
        let code = error.code();
        Self::new(code, error.to_string())
    }
}

impl From<io::Error> for SourceError {
    fn from(error: io::Error) -> Self {
        Self::new("source_io", error.to_string())
    }
}

/// Summarized working-tree changes from porcelain status.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirtyState {
    /// Number of paths with staged changes.
    pub staged: usize,
    /// Number of paths with unstaged changes.
    pub unstaged: usize,
    /// Number of untracked paths.
    pub untracked: usize,
}

impl DirtyState {
    /// Whether any staged, unstaged, or untracked change exists.
    pub const fn is_dirty(&self) -> bool {
        self.staged != 0 || self.unstaged != 0 || self.untracked != 0
    }
}

/// Live, read-only inspection of a source path.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceInspection {
    /// Adapter-compatible identity used by resolved deployments.
    pub identity: SourceIdentity,
    /// Whether the selected path is a linked worktree rather than the main worktree.
    pub linked_worktree: Option<bool>,
    /// Current local branch, absent for detached HEAD or unknown state.
    pub branch: Option<String>,
    /// Whether HEAD is detached.
    pub detached: Option<bool>,
    /// Detailed dirty summary, when Git inspection succeeded.
    pub changes: Option<DirtyState>,
    /// Commits ahead of the configured upstream.
    pub ahead: Option<u64>,
    /// Commits behind the configured upstream.
    pub behind: Option<u64>,
    /// Stable degradation code such as `git_unavailable` or `source_not_repository`.
    pub unknown_code: Option<String>,
}

/// One entry returned by `git worktree list --porcelain`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeInspection {
    /// Worktree directory.
    pub path: PathBuf,
    /// Checked-out commit.
    pub commit: Option<String>,
    /// Local branch name, when attached.
    pub branch: Option<String>,
    /// Whether HEAD is detached.
    pub detached: bool,
    /// Whether Git marks the worktree prunable.
    pub prunable: bool,
}

/// A registered record paired with its current live observation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredSourceInspection {
    /// Durable source record.
    pub source: RegisteredSource,
    /// Current Git/path identity.
    pub inspection: SourceInspection,
}

/// The kind of guarded mutation being requested.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mutation {
    /// Create a new owned path.
    Create,
    /// Remove an existing owned path.
    Remove,
}

/// Project-scoped managed source lifecycle.
#[derive(Clone, Debug)]
pub struct SourceManager {
    workspace_root: PathBuf,
    worktree_root: PathBuf,
    clone_root: PathBuf,
}

impl SourceManager {
    /// Uses `.switchyard/worktrees` and `.switchyard/clones` under a project workspace.
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        let workspace_root = workspace_root.into();
        Self {
            worktree_root: workspace_root.join(".switchyard/worktrees"),
            clone_root: workspace_root.join(".switchyard/clones"),
            workspace_root,
        }
    }

    /// Project workspace that owns the registry and managed roots.
    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    /// Inspects a path without mutating it. Expected Git absence/failures degrade to unknown.
    pub fn inspect(&self, path: &Path, requested_ref: Option<&str>) -> SourceInspection {
        inspect_path(path, requested_ref)
    }

    /// Lists live inspection for every registered source.
    pub fn list(&self, store: &StateStore) -> Result<Vec<RegisteredSourceInspection>, SourceError> {
        store
            .sources()?
            .into_iter()
            .map(|source| {
                let inspection = self.inspect(&source.path, source.requested_ref.as_deref());
                Ok(RegisteredSourceInspection { source, inspection })
            })
            .collect()
    }

    /// Registers an existing path as immutable unmanaged ownership.
    pub fn register_unmanaged(
        &self,
        store: &StateStore,
        name: &str,
        path: &Path,
    ) -> Result<RegisteredSource, SourceError> {
        validate_source_name(name)?;
        if !path.exists() {
            return Err(SourceError::new(
                "source_path_not_found",
                format!("source path `{}` does not exist", path.display()),
            ));
        }
        let path = path.canonicalize()?;
        let inspection = self.inspect(&path, None);
        let source = RegisteredSource {
            name: name.into(),
            kind: RegisteredSourceKind::Unmanaged,
            path,
            repository_path: inspection.identity.repository.map(PathBuf::from),
            requested_ref: inspection.identity.r#ref,
            created_at: now_millis()?,
            managed_relative_path: None,
        };
        store.register_source(&source)?;
        Ok(source)
    }

    /// Forgets a record. Managed records must already have had their path removed.
    pub fn deregister(&self, store: &StateStore, name: &str) -> Result<(), SourceError> {
        let source = store.source(name)?.ok_or_else(|| {
            SourceError::new(
                "source_not_found",
                format!("source `{name}` is not registered"),
            )
        })?;
        if source.kind == RegisteredSourceKind::Managed && source.path.exists() {
            return Err(SourceError::new(
                "source_managed_exists",
                format!("managed source `{name}` must be removed before deregistration"),
            ));
        }
        store.deregister_source(name)?;
        Ok(())
    }

    /// Creates a managed linked worktree against a registered repository source.
    pub fn create_worktree(
        &self,
        store: &StateStore,
        repository_name: &str,
        requested_ref: &str,
        name: &str,
        requested_path: Option<&Path>,
    ) -> Result<RegisteredSource, SourceError> {
        validate_source_name(name)?;
        let repository = store.source(repository_name)?.ok_or_else(|| {
            SourceError::new(
                "repository_unregistered",
                format!("repository source `{repository_name}` is not registered"),
            )
        })?;
        let repository_path = repository
            .repository_path
            .as_deref()
            .unwrap_or(&repository.path);
        let target = requested_path
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.worktree_root.join(name));
        self.guard_mutation(None, &target, Mutation::Create, false, &self.worktree_root)?;
        if let Err(error) = git(
            repository_path,
            &[
                "rev-parse",
                "--verify",
                &format!("{requested_ref}^{{commit}}"),
            ],
        ) {
            if error.code() == "git_unavailable" {
                return Err(error);
            }
            return Err(SourceError::new(
                "source_ref_unknown",
                format!("Git ref `{requested_ref}` is unknown"),
            ));
        }
        fs::create_dir_all(target.parent().expect("managed target has parent"))?;
        run_git(
            repository_path,
            &[
                "worktree",
                "add",
                target.to_string_lossy().as_ref(),
                requested_ref,
            ],
            "worktree_create_failed",
        )?;
        let path = target.canonicalize()?;
        let relative = path
            .strip_prefix(self.worktree_root.canonicalize()?)
            .map_err(|_| {
                SourceError::new(
                    "source_outside_managed_root",
                    "created worktree escaped the managed root",
                )
            })?
            .to_owned();
        let source = RegisteredSource {
            name: name.into(),
            kind: RegisteredSourceKind::Managed,
            path,
            repository_path: Some(repository_path.canonicalize()?),
            requested_ref: Some(requested_ref.into()),
            created_at: now_millis()?,
            managed_relative_path: Some(relative),
        };
        if let Err(error) = store.register_source(&source) {
            let _ = run_git(
                repository_path,
                &["worktree", "remove", source.path.to_string_lossy().as_ref()],
                "worktree_remove_failed",
            );
            return Err(error.into());
        }
        Ok(source)
    }

    /// Creates a managed clone under the clone root.
    pub fn create_clone(
        &self,
        store: &StateStore,
        repository_name: &str,
        name: &str,
        requested_ref: Option<&str>,
    ) -> Result<RegisteredSource, SourceError> {
        validate_source_name(name)?;
        let repository = store.source(repository_name)?.ok_or_else(|| {
            SourceError::new(
                "repository_unregistered",
                format!("repository source `{repository_name}` is not registered"),
            )
        })?;
        let repository_path = repository
            .repository_path
            .as_deref()
            .unwrap_or(&repository.path);
        if let Some(reference) = requested_ref {
            if let Err(error) = git(
                repository_path,
                &["rev-parse", "--verify", &format!("{reference}^{{commit}}")],
            ) {
                if error.code() == "git_unavailable" {
                    return Err(error);
                }
                return Err(SourceError::new(
                    "source_ref_unknown",
                    format!("Git ref `{reference}` is unknown"),
                ));
            }
        }
        let target = self.clone_root.join(name);
        self.guard_mutation(None, &target, Mutation::Create, false, &self.clone_root)?;
        fs::create_dir_all(&self.clone_root)?;
        let mut args = vec!["clone"];
        if let Some(reference) = requested_ref {
            args.extend(["--branch", reference]);
        }
        let repository_text = repository_path.to_string_lossy();
        let target_text = target.to_string_lossy();
        args.extend([repository_text.as_ref(), target_text.as_ref()]);
        run_git(&self.workspace_root, &args, "clone_create_failed")?;
        let path = target.canonicalize()?;
        let source = RegisteredSource {
            name: name.into(),
            kind: RegisteredSourceKind::Managed,
            path: path.clone(),
            repository_path: Some(path.clone()),
            requested_ref: requested_ref.map(str::to_owned),
            created_at: now_millis()?,
            managed_relative_path: Some(
                path.strip_prefix(self.clone_root.canonicalize()?)
                    .map_err(|_| {
                        SourceError::new(
                            "source_outside_managed_root",
                            "created clone escaped the managed root",
                        )
                    })?
                    .to_owned(),
            ),
        };
        store.register_source(&source)?;
        Ok(source)
    }

    /// Removes a managed clone or linked worktree after ownership and dirty checks.
    pub fn remove(
        &self,
        store: &StateStore,
        name: &str,
        allow_dirty: bool,
    ) -> Result<DirtyState, SourceError> {
        let source = store.source(name)?.ok_or_else(|| {
            SourceError::new(
                "source_not_found",
                format!("source `{name}` is not registered"),
            )
        })?;
        let root = if source.path.starts_with(&self.worktree_root) {
            &self.worktree_root
        } else {
            &self.clone_root
        };
        let changes = self.guard_mutation(
            Some(&source),
            &source.path,
            Mutation::Remove,
            allow_dirty,
            root,
        )?;
        let linked = self
            .inspect(&source.path, source.requested_ref.as_deref())
            .linked_worktree
            == Some(true);
        if linked && !changes.is_dirty() {
            let repository = source.repository_path.as_deref().ok_or_else(|| {
                SourceError::new(
                    "source_repository_unknown",
                    "managed worktree has no repository path",
                )
            })?;
            run_git(
                repository,
                &["worktree", "remove", source.path.to_string_lossy().as_ref()],
                "worktree_remove_failed",
            )?;
        } else {
            fs::remove_dir_all(&source.path)?;
            if linked {
                let repository = source.repository_path.as_deref().ok_or_else(|| {
                    SourceError::new(
                        "source_repository_unknown",
                        "managed worktree has no repository path",
                    )
                })?;
                run_git(repository, &["worktree", "prune"], "worktree_prune_failed")?;
            }
        }
        Ok(changes)
    }

    /// Validates every mutating operation through a single ownership/containment/dirty gate.
    pub fn guard_mutation(
        &self,
        source: Option<&RegisteredSource>,
        target: &Path,
        mutation: Mutation,
        allow_dirty: bool,
        managed_root: &Path,
    ) -> Result<DirtyState, SourceError> {
        if let Some(source) = source {
            if source.kind != RegisteredSourceKind::Managed {
                return Err(SourceError::new(
                    "source_unmanaged",
                    "unmanaged sources can only be deregistered",
                ));
            }
        }
        validate_containment(target, managed_root, mutation)?;
        if mutation == Mutation::Create {
            if target.exists() {
                return Err(SourceError::new(
                    "source_target_exists",
                    format!("target `{}` already exists", target.display()),
                ));
            }
            return Ok(DirtyState::default());
        }
        let inspection = self.inspect(
            target,
            source.and_then(|value| value.requested_ref.as_deref()),
        );
        let changes = inspection.changes.ok_or_else(|| {
            SourceError::new(
                "source_state_unknown",
                "cannot safely remove a source whose Git state is unknown",
            )
        })?;
        if changes.is_dirty() && !allow_dirty {
            return Err(SourceError::new(
                "source_dirty",
                format!(
                    "source has {} staged, {} unstaged, and {} untracked path(s)",
                    changes.staged, changes.unstaged, changes.untracked
                ),
            ));
        }
        Ok(changes)
    }

    /// Lists all worktrees associated with a repository.
    pub fn worktrees(&self, repository: &Path) -> Result<Vec<WorktreeInspection>, SourceError> {
        let output = git(repository, &["worktree", "list", "--porcelain"])
            .map_err(|error| SourceError::new(error.code, error.message))?;
        Ok(parse_worktrees(&output))
    }
}

fn inspect_path(path: &Path, requested_ref: Option<&str>) -> SourceInspection {
    let path_text = path.to_string_lossy();
    let repository = match git(path, &["rev-parse", "--show-toplevel"]) {
        Ok(value) => value,
        Err(error) => return unknown_inspection(path_text.into_owned(), error.code),
    };
    let reference = requested_ref
        .map(str::to_owned)
        .or_else(|| git(path, &["symbolic-ref", "--quiet", "--short", "HEAD"]).ok());
    let adapter = SourceGitAdapter.inspect(&json!({"path": path_text, "repository": repository, "ref": reference.clone().unwrap_or_else(|| "HEAD".into())})).ok();
    let identity = adapter.unwrap_or(SourceIdentity {
        path: path.to_string_lossy().into_owned(),
        repository: Some(repository.clone()),
        r#ref: reference.clone(),
        commit: git(path, &["rev-parse", "HEAD"]).ok(),
        dirty: None,
    });
    let branch = git(path, &["symbolic-ref", "--quiet", "--short", "HEAD"]).ok();
    let detached = Some(branch.is_none());
    let common = git(path, &["rev-parse", "--git-common-dir"]).ok();
    let git_dir = git(path, &["rev-parse", "--git-dir"]).ok();
    let linked_worktree = common.zip(git_dir).map(|(common, git_dir)| {
        normalize_git_path(path, &common) != normalize_git_path(path, &git_dir)
    });
    let changes = git(path, &["status", "--porcelain=v1", "--untracked-files=all"])
        .ok()
        .map(|value| parse_dirty(&value));
    let upstream = git(
        path,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    )
    .ok();
    let (ahead, behind) = upstream
        .and_then(|_| {
            git(
                path,
                &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
            )
            .ok()
        })
        .and_then(|value| {
            let mut fields = value.split_whitespace();
            Some((fields.next()?.parse().ok()?, fields.next()?.parse().ok()?))
        })
        .map_or((None, None), |(ahead, behind)| (Some(ahead), Some(behind)));
    SourceInspection {
        identity: SourceIdentity {
            dirty: changes.as_ref().map(DirtyState::is_dirty),
            ..identity
        },
        linked_worktree,
        branch,
        detached,
        changes,
        ahead,
        behind,
        unknown_code: None,
    }
}

fn unknown_inspection(path: String, code: &'static str) -> SourceInspection {
    let identity = SourcePathAdapter
        .inspect(&json!({"path": path}))
        .unwrap_or(SourceIdentity {
            path: String::new(),
            repository: None,
            r#ref: None,
            commit: None,
            dirty: None,
        });
    SourceInspection {
        identity,
        linked_worktree: None,
        branch: None,
        detached: None,
        changes: None,
        ahead: None,
        behind: None,
        unknown_code: Some(code.into()),
    }
}

fn git(path: &Path, args: &[&str]) -> Result<String, SourceError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                SourceError::new("git_unavailable", "git binary is unavailable")
            } else {
                SourceError::new("git_inspection_failed", error.to_string())
            }
        })?;
    output_text(output, "source_not_repository")
}

fn run_git(path: &Path, args: &[&str], code: &'static str) -> Result<String, SourceError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .map_err(|error| {
            SourceError::new(
                if error.kind() == io::ErrorKind::NotFound {
                    "git_unavailable"
                } else {
                    code
                },
                error.to_string(),
            )
        })?;
    output_text(output, code)
}

fn output_text(output: Output, code: &'static str) -> Result<String, SourceError> {
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().into())
    } else {
        Err(SourceError::new(
            code,
            String::from_utf8_lossy(&output.stderr).trim(),
        ))
    }
}

fn parse_dirty(status: &str) -> DirtyState {
    let mut result = DirtyState::default();
    for line in status.lines() {
        let bytes = line.as_bytes();
        if bytes.starts_with(b"??") {
            result.untracked += 1;
            continue;
        }
        if bytes.first().is_some_and(|value| *value != b' ') {
            result.staged += 1;
        }
        if bytes.get(1).is_some_and(|value| *value != b' ') {
            result.unstaged += 1;
        }
    }
    result
}

fn parse_worktrees(value: &str) -> Vec<WorktreeInspection> {
    let mut result = Vec::new();
    let mut current: Option<WorktreeInspection> = None;
    for line in value.lines().chain([""]) {
        if line.is_empty() {
            if let Some(entry) = current.take() {
                result.push(entry);
            }
        } else if let Some(path) = line.strip_prefix("worktree ") {
            current = Some(WorktreeInspection {
                path: path.into(),
                commit: None,
                branch: None,
                detached: false,
                prunable: false,
            });
        } else if let Some(entry) = current.as_mut() {
            if let Some(commit) = line.strip_prefix("HEAD ") {
                entry.commit = Some(commit.into());
            } else if let Some(branch) = line.strip_prefix("branch ") {
                entry.branch = Some(branch.strip_prefix("refs/heads/").unwrap_or(branch).into());
            } else if line == "detached" {
                entry.detached = true;
            } else if line.starts_with("prunable") {
                entry.prunable = true;
            }
        }
    }
    result
}

fn normalize_git_path(worktree: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        worktree.join(path)
    }
}

fn validate_containment(target: &Path, root: &Path, mutation: Mutation) -> Result<(), SourceError> {
    let canonical_root = if root.exists() {
        root.canonicalize()?
    } else {
        let parent = root.parent().ok_or_else(|| {
            SourceError::new("source_outside_managed_root", "managed root has no parent")
        })?;
        fs::create_dir_all(parent)?;
        fs::create_dir_all(root)?;
        root.canonicalize()?
    };
    let canonical_target = if target.exists() {
        target.canonicalize()?
    } else {
        let parent = target.parent().ok_or_else(|| {
            SourceError::new("source_outside_managed_root", "target has no parent")
        })?;
        let existing = nearest_existing(parent);
        let canonical_parent = existing.canonicalize()?;
        canonical_parent
            .join(parent.strip_prefix(&existing).unwrap_or(parent))
            .join(target.file_name().ok_or_else(|| {
                SourceError::new("source_outside_managed_root", "target has no file name")
            })?)
    };
    if canonical_target == canonical_root || !canonical_target.starts_with(&canonical_root) {
        return Err(SourceError::new(
            "source_outside_managed_root",
            format!(
                "{} target `{}` is outside `{}`",
                match mutation {
                    Mutation::Create => "creation",
                    Mutation::Remove => "removal",
                },
                target.display(),
                root.display()
            ),
        ));
    }
    Ok(())
}

fn nearest_existing(path: &Path) -> PathBuf {
    let mut current = path;
    while !current.exists() {
        current = current.parent().unwrap_or(Path::new("."));
    }
    current.to_owned()
}

fn now_millis() -> Result<i64, SourceError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| SourceError::new("clock_before_epoch", error.to_string()))?
        .as_millis();
    i64::try_from(millis).map_err(|_| {
        SourceError::new(
            "clock_overflow",
            "current timestamp exceeds SQLite integer range",
        )
    })
}

fn validate_source_name(name: &str) -> Result<(), SourceError> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(SourceError::new(
            "invalid_source_name",
            "source names may contain only ASCII letters, digits, '.', '-', and '_'",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn command(path: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn repository(temp: &TempDir) -> PathBuf {
        let repository = temp.path().join("repository");
        fs::create_dir(&repository).unwrap();
        command(&repository, &["init", "-b", "main"]);
        command(
            &repository,
            &["config", "user.email", "tests@switchyard.invalid"],
        );
        command(&repository, &["config", "user.name", "Switchyard Tests"]);
        fs::write(repository.join("tracked"), "initial\n").unwrap();
        command(&repository, &["add", "tracked"]);
        command(&repository, &["commit", "-m", "initial"]);
        repository
    }

    fn store(temp: &TempDir) -> StateStore {
        StateStore::open(temp.path().join("state.sqlite3"))
            .unwrap()
            .0
    }

    #[test]
    fn inspects_repository_linked_worktree_and_dirty_categories() {
        let temp = TempDir::new().unwrap();
        let repository = repository(&temp);
        let linked = temp.path().join("linked");
        command(
            &repository,
            &[
                "worktree",
                "add",
                "-b",
                "feature",
                linked.to_str().unwrap(),
                "HEAD",
            ],
        );
        fs::write(linked.join("tracked"), "unstaged\n").unwrap();
        fs::write(linked.join("staged"), "staged\n").unwrap();
        command(&linked, &["add", "staged"]);
        fs::write(linked.join("untracked"), "untracked\n").unwrap();
        let inspection = SourceManager::new(temp.path()).inspect(&linked, Some("feature"));
        assert_eq!(inspection.linked_worktree, Some(true));
        assert_eq!(inspection.branch.as_deref(), Some("feature"));
        assert!(inspection.identity.commit.is_some());
        assert_eq!(
            inspection.changes,
            Some(DirtyState {
                staged: 1,
                unstaged: 1,
                untracked: 1
            })
        );
        let worktrees = SourceManager::new(temp.path())
            .worktrees(&repository)
            .unwrap();
        assert_eq!(worktrees.len(), 2);
        assert!(worktrees.iter().any(|entry| entry.path == linked));
    }

    #[test]
    fn unmanaged_registration_and_removal_never_mutate_path() {
        let temp = TempDir::new().unwrap();
        let repository = repository(&temp);
        let store = store(&temp);
        let manager = SourceManager::new(temp.path());
        let source = manager
            .register_unmanaged(&store, "repo", &repository)
            .unwrap();
        assert_eq!(source.kind, RegisteredSourceKind::Unmanaged);
        let error = manager.remove(&store, "repo", true).unwrap_err();
        assert_eq!(error.code(), "source_unmanaged");
        assert!(repository.join("tracked").exists());
        manager.deregister(&store, "repo").unwrap();
        assert!(repository.exists());
    }

    #[test]
    fn managed_worktree_round_trip_and_dirty_override() {
        let temp = TempDir::new().unwrap();
        let repository = repository(&temp);
        let store = store(&temp);
        let manager = SourceManager::new(temp.path());
        manager
            .register_unmanaged(&store, "repo", &repository)
            .unwrap();
        let source = manager
            .create_worktree(&store, "repo", "HEAD", "feature", None)
            .unwrap();
        fs::write(source.path.join("untracked"), "change\n").unwrap();
        let error = manager.remove(&store, "feature", false).unwrap_err();
        assert_eq!(error.code(), "source_dirty");
        assert!(source.path.exists());
        let dirty = manager.remove(&store, "feature", true).unwrap();
        assert_eq!(dirty.untracked, 1);
        assert!(!source.path.exists());
        manager.deregister(&store, "feature").unwrap();
    }

    #[test]
    fn managed_clone_round_trip_stays_under_clone_root() {
        let temp = TempDir::new().unwrap();
        let repository = repository(&temp);
        let store = store(&temp);
        let manager = SourceManager::new(temp.path());
        manager
            .register_unmanaged(&store, "repo", &repository)
            .unwrap();
        let clone = manager
            .create_clone(&store, "repo", "clone", Some("main"))
            .unwrap();
        assert!(
            clone
                .path
                .starts_with(temp.path().join(".switchyard/clones"))
        );
        assert_eq!(
            manager.inspect(&clone.path, Some("main")).identity.dirty,
            Some(false)
        );
        manager.remove(&store, "clone", false).unwrap();
        assert!(!clone.path.exists());
        manager.deregister(&store, "clone").unwrap();
    }

    #[test]
    fn refuses_path_escape_and_existing_target() {
        let temp = TempDir::new().unwrap();
        let manager = SourceManager::new(temp.path());
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let error = manager
            .guard_mutation(
                None,
                &outside,
                Mutation::Create,
                false,
                &manager.worktree_root,
            )
            .unwrap_err();
        assert_eq!(error.code(), "source_outside_managed_root");
        let target = manager.worktree_root.join("existing");
        fs::create_dir_all(&target).unwrap();
        let error = manager
            .guard_mutation(
                None,
                &target,
                Mutation::Create,
                false,
                &manager.worktree_root,
            )
            .unwrap_err();
        assert_eq!(error.code(), "source_target_exists");
    }

    #[test]
    fn plain_path_degrades_to_explicit_unknown() {
        let temp = TempDir::new().unwrap();
        let inspection = SourceManager::new(temp.path()).inspect(temp.path(), None);
        assert_eq!(
            inspection.unknown_code.as_deref(),
            Some("source_not_repository")
        );
        assert_eq!(inspection.identity.path, temp.path().to_string_lossy());
    }
}
