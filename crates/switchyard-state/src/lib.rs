//! Persistent, synchronous SQLite state for the Switchyard control plane.
//!
//! The intended project-local database is `.switchyard/state.sqlite3`, but callers
//! always supply its path. [`StateStore::open`] applies ordered migrations in a single
//! transaction. Before pending migrations touch an existing database, the file is
//! backed up beside it with a `.pre-migration-vN.bak` suffix. A database whose recorded
//! schema is newer than this crate is rejected without modification.
//!
//! Deployment YAML remains the source of desired state. SQLite keeps only the last
//! successfully applied, resolved snapshot and its definition hash. Reconciliation
//! compares that applied record with generated manifests and injected Docker ownership
//! observations. It appends observations and diagnostics but never changes Docker or
//! promotes a manifest to applied desired state. Consequently, a missing database can
//! be rebuilt safely: deployment/resource observations return, while applied snapshots
//! remain absent until a real apply operation records one.
//!
//! Mutations are serialized by expiring per-deployment leases. A lease records a
//! process identity, an unguessable caller-provided owner instance, a token, heartbeat,
//! and expiry. Acquisition atomically replaces an expired lease; heartbeat and release
//! require the original identity and token, preventing an old process from disturbing a
//! recovered lease.
//!
//! Secret values have no persistence API or schema column. Secret-bearing resolved
//! fields must contain a validated [`SecretReference`], and [`AppliedSnapshot`]
//! construction rejects literal values at secret-like keys. This is defense in depth;
//! adapters should still avoid materializing secrets into resolved state at all.
//!
//! # Schema model
//!
//! `deployments` holds the nullable last-applied tuple; nullability is intentional for
//! recovered observations. `deployment_history`, `operations`, `resources`, and
//! `health_observations` are append-oriented audit tables. Resource rows use an
//! `active` marker so each reconciliation preserves earlier observations. `routes` and
//! `route_snapshots` preserve route selection and activation attempts independently;
//! `router_bindings` exposes desired, current, previous, and observed acknowledgement
//! state without rewriting that history. `operation_locks` is the only replace-in-place
//! coordination table. Arbitrary
//! diagnostic JSON is admitted only through [`StructuredContext`]; observed Docker
//! labels are reduced to the three Switchyard ownership fields before storage.
//!
//! Migration SQL lives in numbered, embedded files. Versions are recorded only after
//! their SQL succeeds inside the migration transaction. Backups never overwrite an
//! earlier backup: repeated recovery from the same old version receives a numeric
//! suffix.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs, io,
    path::{Path, PathBuf},
    time::Duration,
};

use rusqlite::{
    Connection, OptionalExtension, Transaction, TransactionBehavior, backup::Backup, params,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The schema version understood by this crate.
pub const SCHEMA_VERSION: i64 = 6;
/// Ownership label used by the existing Docker runtime.
pub const MANAGED_LABEL: &str = "dev.switchyard.managed";
/// Deployment ownership label used by the existing Docker runtime.
pub const DEPLOYMENT_LABEL: &str = "dev.switchyard.deployment";
/// Resource topology hash label used by the existing Docker runtime.
pub const RESOURCE_HASH_LABEL: &str = "dev.switchyard.resource-hash";
pub const DEVICE_LABEL: &str = "dev.switchyard.device";

const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("migrations/001_initial.sql")),
    (2, include_str!("migrations/002_routes.sql")),
    (3, include_str!("migrations/003_live_routes.sql")),
    (4, include_str!("migrations/004_sources.sql")),
    (5, include_str!("migrations/005_devices.sql")),
    (6, include_str!("migrations/006_profiles.sql")),
];

/// A source-local startup profile explicitly reviewed and imported into project state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedProfile {
    pub name: String,
    pub source_name: String,
    pub source_commit: Option<String>,
    pub content_hash: String,
    pub definition_json: String,
    pub imported_at: i64,
}

/// Persisted outcome of the most recent SSH connectivity check.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceCheckStatus {
    Never,
    Ok,
    Unreachable,
    AuthFailed,
}

impl DeviceCheckStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Never => "never",
            Self::Ok => "ok",
            Self::Unreachable => "unreachable",
            Self::AuthFailed => "auth-failed",
        }
    }

    fn parse(value: &str) -> Result<Self, StateError> {
        match value {
            "never" => Ok(Self::Never),
            "ok" => Ok(Self::Ok),
            "unreachable" => Ok(Self::Unreachable),
            "auth-failed" => Ok(Self::AuthFailed),
            _ => Err(invalid(
                "invalid_device_status",
                format!("unknown device status `{value}`"),
            )),
        }
    }
}

impl fmt::Display for DeviceCheckStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A remote machine registered for future SSH-backed execution.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredDevice {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub identity_file: Option<PathBuf>,
    pub created_at: i64,
    pub last_checked_at: Option<i64>,
    pub last_check_status: DeviceCheckStatus,
    pub last_check_detail: Option<String>,
}

/// Durable ownership classification for a registered source.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RegisteredSourceKind {
    /// An existing path recorded without ownership.
    Unmanaged,
    /// A clone or worktree created under Switchyard's managed roots.
    Managed,
}

impl RegisteredSourceKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Unmanaged => "unmanaged",
            Self::Managed => "managed",
        }
    }

    fn parse(value: &str) -> Result<Self, StateError> {
        match value {
            "unmanaged" => Ok(Self::Unmanaged),
            "managed" => Ok(Self::Managed),
            _ => Err(invalid(
                "invalid_source_kind",
                format!("unknown source kind `{value}`"),
            )),
        }
    }
}

/// A registered source record. Git observations are deliberately absent and derived live.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredSource {
    /// Stable user-facing name.
    pub name: String,
    /// Immutable ownership kind.
    pub kind: RegisteredSourceKind,
    /// Absolute selected source path.
    pub path: PathBuf,
    /// Repository path used for managed worktree operations.
    pub repository_path: Option<PathBuf>,
    /// Requested branch, tag, or revision.
    pub requested_ref: Option<String>,
    /// Registration or creation timestamp in Unix milliseconds.
    pub created_at: i64,
    /// Location relative to the appropriate managed root.
    pub managed_relative_path: Option<PathBuf>,
}

/// Stable state-layer failures suitable for API translation.
#[derive(Debug)]
pub enum StateError {
    /// Filesystem access failed.
    Io(io::Error),
    /// SQLite rejected an operation.
    Sqlite(rusqlite::Error),
    /// Structured input was invalid.
    Json(serde_json::Error),
    /// The database belongs to newer software.
    NewerSchema { found: i64, supported: i64 },
    /// A public value violated a state contract.
    InvalidInput { code: &'static str, context: String },
    /// Another live owner holds the deployment mutation lease.
    LockContended {
        deployment: String,
        owner: String,
        expires_at: i64,
    },
    /// A lease token or owner no longer identifies the active lease.
    LockLost { deployment: String },
}

impl StateError {
    /// Returns a stable machine-readable error code.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Io(_) => "state_io",
            Self::Sqlite(_) => "state_sqlite",
            Self::Json(_) => "invalid_json",
            Self::NewerSchema { .. } => "newer_schema",
            Self::InvalidInput { code, .. } => code,
            Self::LockContended { .. } => "operation_lock_contended",
            Self::LockLost { .. } => "operation_lock_lost",
        }
    }
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "state filesystem error: {error}"),
            Self::Sqlite(error) => write!(f, "SQLite state error: {error}"),
            Self::Json(error) => write!(f, "invalid state JSON: {error}"),
            Self::NewerSchema { found, supported } => write!(
                f,
                "database schema {found} is newer than supported schema {supported}"
            ),
            Self::InvalidInput { code, context } => write!(f, "{code}: {context}"),
            Self::LockContended {
                deployment,
                owner,
                expires_at,
            } => write!(
                f,
                "deployment `{deployment}` is locked by `{owner}` until {expires_at}"
            ),
            Self::LockLost { deployment } => {
                write!(f, "operation lock for `{deployment}` is no longer owned")
            }
        }
    }
}

impl std::error::Error for StateError {}

impl From<io::Error> for StateError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}
impl From<rusqlite::Error> for StateError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sqlite(value)
    }
}
impl From<serde_json::Error> for StateError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

/// Result of opening and, when needed, migrating a store.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenReport {
    /// Versions applied in ascending order.
    pub applied_migrations: Vec<i64>,
    /// Backup created before the first pending migration on an existing database.
    pub backup_path: Option<PathBuf>,
}

/// A validated reference to a secret managed outside SQLite.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SecretReference {
    /// Name of an environment variable resolved only at execution time.
    EnvironmentVariable(String),
    /// Path to a file read only at execution time.
    File(PathBuf),
}

impl SecretReference {
    /// Constructs an environment reference, rejecting values and shell expressions.
    pub fn environment(name: impl Into<String>) -> Result<Self, StateError> {
        let name = name.into();
        let valid = !name.is_empty()
            && name.bytes().enumerate().all(|(index, byte)| {
                byte == b'_' || byte.is_ascii_uppercase() || index > 0 && byte.is_ascii_digit()
            });
        if !valid {
            return Err(invalid(
                "invalid_secret_reference",
                "environment references must be uppercase variable names",
            ));
        }
        Ok(Self::EnvironmentVariable(name))
    }

    /// Constructs a non-empty file reference.
    pub fn file(path: impl Into<PathBuf>) -> Result<Self, StateError> {
        let path = path.into();
        if path.as_os_str().is_empty() {
            return Err(invalid(
                "invalid_secret_reference",
                "secret file references cannot be empty",
            ));
        }
        Ok(Self::File(path))
    }
}

/// Canonical, secret-safe last-applied resolved desired state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppliedSnapshot(String);

impl AppliedSnapshot {
    /// Validates JSON and stores its canonical compact representation.
    pub fn from_json(value: Value) -> Result<Self, StateError> {
        validate_no_secret_values(&value, "$")?;
        Ok(Self(serde_json::to_string(&value)?))
    }

    /// Returns the canonical JSON representation.
    pub fn as_json(&self) -> &str {
        &self.0
    }
}

/// Validated structured diagnostic context that cannot contain literal secret fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructuredContext(String);

impl StructuredContext {
    /// Validates and canonicalizes JSON context.
    pub fn new(value: Value) -> Result<Self, StateError> {
        validate_no_secret_values(&value, "$")?;
        Ok(Self(serde_json::to_string(&value)?))
    }

    /// Returns canonical JSON.
    pub fn as_json(&self) -> &str {
        &self.0
    }
}

fn validate_no_secret_values(value: &Value, path: &str) -> Result<(), StateError> {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let child_path = format!("{path}.{key}");
                if secret_like(key) && !valid_secret_reference(child) {
                    return Err(invalid(
                        "secret_value_forbidden",
                        format!(
                            "{child_path} must contain an environmentVariable or file reference"
                        ),
                    ));
                }
                validate_no_secret_values(child, &child_path)?;
            }
        }
        Value::Array(values) => {
            for (index, child) in values.iter().enumerate() {
                validate_no_secret_values(child, &format!("{path}[{index}]"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn secret_like(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace(['-', '_'], "");
    [
        "secret",
        "password",
        "passwd",
        "token",
        "apikey",
        "privatekey",
    ]
    .iter()
    .any(|word| normalized.contains(word))
        && !normalized.ends_with("reference")
        && !normalized.ends_with("ref")
}

fn valid_secret_reference(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.len() == 1
        && object.iter().all(|(kind, value)| {
            let Some(value) = value.as_str() else {
                return false;
            };
            match kind.as_str() {
                "environmentVariable" => SecretReference::environment(value).is_ok(),
                "file" => SecretReference::file(value).is_ok(),
                _ => false,
            }
        })
}

fn invalid(code: &'static str, context: impl Into<String>) -> StateError {
    StateError::InvalidInput {
        code,
        context: context.into(),
    }
}

fn source_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RegisteredSource> {
    let kind = row.get::<_, String>(1)?;
    let kind = RegisteredSourceKind::parse(&kind).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(RegisteredSource {
        name: row.get(0)?,
        kind,
        path: PathBuf::from(row.get::<_, String>(2)?),
        repository_path: row.get::<_, Option<String>>(3)?.map(PathBuf::from),
        requested_ref: row.get(4)?,
        created_at: row.get(5)?,
        managed_relative_path: row.get::<_, Option<String>>(6)?.map(PathBuf::from),
    })
}

fn device_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RegisteredDevice> {
    let status = row.get::<_, String>(7)?;
    let status = DeviceCheckStatus::parse(&status).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(7, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(RegisteredDevice {
        name: row.get(0)?,
        host: row.get(1)?,
        port: row.get(2)?,
        user: row.get(3)?,
        identity_file: row.get::<_, Option<String>>(4)?.map(PathBuf::from),
        created_at: row.get(5)?,
        last_checked_at: row.get(6)?,
        last_check_status: status,
        last_check_detail: row.get(8)?,
    })
}

fn imported_profile_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ImportedProfile> {
    Ok(ImportedProfile {
        name: row.get(0)?,
        source_name: row.get(1)?,
        source_commit: row.get(2)?,
        content_hash: row.get(3)?,
        definition_json: row.get(4)?,
        imported_at: row.get(5)?,
    })
}

/// Synchronous project state store. A future daemon can serialize access around it.
pub struct StateStore {
    connection: Connection,
    path: PathBuf,
}

impl StateStore {
    /// Opens a database and applies all pending migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<(Self, OpenReport), StateError> {
        let path = path.as_ref().to_owned();
        if let Some(parent) = path.parent().filter(|path| !path.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        let existed = path.exists();
        let mut connection = Connection::open(&path)?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let current = current_schema_version(&connection)?;
        if current > SCHEMA_VERSION {
            return Err(StateError::NewerSchema {
                found: current,
                supported: SCHEMA_VERSION,
            });
        }
        let pending = MIGRATIONS
            .iter()
            .filter(|(version, _)| *version > current)
            .collect::<Vec<_>>();
        let backup_path = if existed && !pending.is_empty() {
            let backup = backup_path(&path, current);
            backup_database(&connection, &backup)?;
            Some(backup)
        } else {
            None
        };
        let mut applied_migrations = Vec::new();
        if !pending.is_empty() {
            let transaction = connection.transaction()?;
            for (version, sql) in pending {
                transaction.execute_batch(sql)?;
                transaction.execute(
                    "INSERT INTO schema_versions(version, applied_at) VALUES (?1, unixepoch('subsec') * 1000)",
                    [version],
                )?;
                applied_migrations.push(*version);
            }
            transaction.commit()?;
        }
        Ok((
            Self { connection, path },
            OpenReport {
                applied_migrations,
                backup_path,
            },
        ))
    }

    /// Returns the explicit database path supplied by the caller.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Registers a source without persisting any live Git observations.
    pub fn register_source(&self, source: &RegisteredSource) -> Result<(), StateError> {
        validate_id("source name", &source.name)?;
        if source.path.as_os_str().is_empty() {
            return Err(invalid(
                "invalid_source_path",
                "source path cannot be empty",
            ));
        }
        if source.kind == RegisteredSourceKind::Unmanaged && source.managed_relative_path.is_some()
        {
            return Err(invalid(
                "invalid_source_registration",
                "unmanaged sources cannot have a managed location",
            ));
        }
        if source.kind == RegisteredSourceKind::Managed && source.managed_relative_path.is_none() {
            return Err(invalid(
                "invalid_source_registration",
                "managed sources require a managed location",
            ));
        }
        self.connection.execute(
            "INSERT INTO registered_sources(name,kind,path,repository_path,requested_ref,created_at,managed_relative_path) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![source.name, source.kind.as_str(), source.path.to_string_lossy(), source.repository_path.as_ref().map(|path| path.to_string_lossy()), source.requested_ref, source.created_at, source.managed_relative_path.as_ref().map(|path| path.to_string_lossy())],
        ).map_err(|error| match error {
            rusqlite::Error::SqliteFailure(ref failure, _) if failure.code == rusqlite::ErrorCode::ConstraintViolation => invalid("source_already_registered", format!("source `{}` or path `{}` is already registered", source.name, source.path.display())),
            other => other.into(),
        })?;
        Ok(())
    }

    /// Loads a registered source by name.
    pub fn source(&self, name: &str) -> Result<Option<RegisteredSource>, StateError> {
        self.connection.query_row(
            "SELECT name,kind,path,repository_path,requested_ref,created_at,managed_relative_path FROM registered_sources WHERE name=?1",
            [name],
            source_from_row,
        ).optional().map_err(Into::into)
    }

    /// Lists registered sources in stable name order.
    pub fn sources(&self) -> Result<Vec<RegisteredSource>, StateError> {
        let mut statement = self.connection.prepare(
            "SELECT name,kind,path,repository_path,requested_ref,created_at,managed_relative_path FROM registered_sources ORDER BY name",
        )?;
        let rows = statement.query_map([], source_from_row)?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Forgets a source record without modifying its path.
    pub fn deregister_source(&self, name: &str) -> Result<(), StateError> {
        let changed = self
            .connection
            .execute("DELETE FROM registered_sources WHERE name=?1", [name])?;
        if changed == 0 {
            return Err(invalid(
                "source_not_found",
                format!("source `{name}` is not registered"),
            ));
        }
        Ok(())
    }

    /// Records or replaces an explicitly reviewed source-local startup profile.
    pub fn record_imported_profile(&self, profile: &ImportedProfile) -> Result<(), StateError> {
        validate_id("profile name", &profile.name)?;
        validate_id("profile source name", &profile.source_name)?;
        validate_id("profile content hash", &profile.content_hash)?;
        serde_json::from_str::<Value>(&profile.definition_json)?;
        self.connection.execute(
            "INSERT INTO imported_profiles(name,source_name,source_commit,content_hash,definition_json,imported_at) VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(name) DO UPDATE SET source_name=excluded.source_name,source_commit=excluded.source_commit,content_hash=excluded.content_hash,definition_json=excluded.definition_json,imported_at=excluded.imported_at",
            params![profile.name, profile.source_name, profile.source_commit, profile.content_hash, profile.definition_json, profile.imported_at],
        )?;
        Ok(())
    }

    /// Loads an imported startup profile by name.
    pub fn imported_profile(&self, name: &str) -> Result<Option<ImportedProfile>, StateError> {
        self.connection
            .query_row(
                "SELECT name,source_name,source_commit,content_hash,definition_json,imported_at FROM imported_profiles WHERE name=?1",
                [name],
                imported_profile_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    /// Lists imported startup profiles in stable name order.
    pub fn imported_profiles(&self) -> Result<Vec<ImportedProfile>, StateError> {
        let mut statement = self.connection.prepare(
            "SELECT name,source_name,source_commit,content_hash,definition_json,imported_at FROM imported_profiles ORDER BY name",
        )?;
        let rows = statement.query_map([], imported_profile_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Removes an imported startup profile without touching its source checkout.
    pub fn remove_imported_profile(&self, name: &str) -> Result<(), StateError> {
        let changed = self
            .connection
            .execute("DELETE FROM imported_profiles WHERE name=?1", [name])?;
        if changed == 0 {
            return Err(invalid(
                "profile_not_imported",
                format!("profile `{name}` is not imported"),
            ));
        }
        Ok(())
    }

    /// Registers a remote SSH device. Only an identity path is retained, never key material.
    pub fn register_device(&self, device: &RegisteredDevice) -> Result<(), StateError> {
        validate_device(device)?;
        self.connection.execute(
            "INSERT INTO devices(name,host,port,user,identity_file,created_at,last_checked_at,last_check_status,last_check_detail) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![device.name, device.host, device.port, device.user, device.identity_file.as_ref().map(|path| path.to_string_lossy()), device.created_at, device.last_checked_at, device.last_check_status.as_str(), device.last_check_detail],
        ).map_err(|error| match error {
            rusqlite::Error::SqliteFailure(ref failure, _) if failure.code == rusqlite::ErrorCode::ConstraintViolation => invalid("device_already_registered", format!("device `{}` is already registered", device.name)),
            other => other.into(),
        })?;
        Ok(())
    }

    /// Loads a registered device by name.
    pub fn device(&self, name: &str) -> Result<Option<RegisteredDevice>, StateError> {
        self.connection.query_row(
            "SELECT name,host,port,user,identity_file,created_at,last_checked_at,last_check_status,last_check_detail FROM devices WHERE name=?1",
            [name], device_from_row,
        ).optional().map_err(Into::into)
    }

    /// Lists devices in stable name order.
    pub fn devices(&self) -> Result<Vec<RegisteredDevice>, StateError> {
        let mut statement = self.connection.prepare(
            "SELECT name,host,port,user,identity_file,created_at,last_checked_at,last_check_status,last_check_detail FROM devices ORDER BY name",
        )?;
        let rows = statement.query_map([], device_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Removes a device registration without touching SSH configuration or files.
    pub fn deregister_device(&self, name: &str) -> Result<(), StateError> {
        let changed = self
            .connection
            .execute("DELETE FROM devices WHERE name=?1", [name])?;
        if changed == 0 {
            return Err(invalid(
                "device_not_found",
                format!("device `{name}` is not registered"),
            ));
        }
        Ok(())
    }

    /// Persists a connectivity check and returns the updated record.
    pub fn record_device_check(
        &self,
        name: &str,
        checked_at: i64,
        status: DeviceCheckStatus,
        detail: Option<&str>,
    ) -> Result<RegisteredDevice, StateError> {
        if status == DeviceCheckStatus::Never {
            return Err(invalid(
                "invalid_device_status",
                "a completed check cannot have status `never`",
            ));
        }
        let changed = self.connection.execute(
            "UPDATE devices SET last_checked_at=?2,last_check_status=?3,last_check_detail=?4 WHERE name=?1",
            params![name, checked_at, status.as_str(), detail],
        )?;
        if changed == 0 {
            return Err(invalid(
                "device_not_found",
                format!("device `{name}` is not registered"),
            ));
        }
        self.device(name)?.ok_or_else(|| {
            invalid(
                "device_not_found",
                format!("device `{name}` is not registered"),
            )
        })
    }

    /// Records the last successfully applied resolved snapshot and appends history.
    pub fn record_applied_snapshot(
        &mut self,
        deployment: &str,
        definition_hash: &str,
        snapshot: &AppliedSnapshot,
        applied_at: i64,
    ) -> Result<(), StateError> {
        validate_id("deployment", deployment)?;
        validate_id("definition hash", definition_hash)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO deployments(id, applied_definition_hash, applied_snapshot_json, applied_at, last_observed_at) \
             VALUES (?1, ?2, ?3, ?4, NULL) ON CONFLICT(id) DO UPDATE SET \
             applied_definition_hash=excluded.applied_definition_hash, applied_snapshot_json=excluded.applied_snapshot_json, applied_at=excluded.applied_at",
            params![deployment, definition_hash, snapshot.as_json(), applied_at],
        )?;
        deployment_event(
            &tx,
            deployment,
            "applied",
            Some(definition_hash),
            applied_at,
            None,
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Loads the last applied snapshot, if one has ever been recorded.
    pub fn applied_snapshot(
        &self,
        deployment: &str,
    ) -> Result<Option<AppliedDeployment>, StateError> {
        self.connection.query_row(
            "SELECT applied_definition_hash, applied_snapshot_json, applied_at FROM deployments WHERE id=?1 AND applied_definition_hash IS NOT NULL",
            [deployment],
            |row| Ok(AppliedDeployment { deployment: deployment.to_owned(), definition_hash: row.get(0)?, snapshot: AppliedSnapshot(row.get(1)?), applied_at: row.get(2)? }),
        ).optional().map_err(Into::into)
    }

    /// Lists every deployment known to SQLite with its latest operation, if any.
    pub fn deployments(&self) -> Result<Vec<StoredDeployment>, StateError> {
        let mut statement = self.connection.prepare(
            "SELECT d.id,d.applied_definition_hash,d.applied_snapshot_json,d.applied_at,\
             o.id,o.kind,o.status,o.started_at,o.finished_at,o.error_code,o.error_context_json \
             FROM deployments d LEFT JOIN operations o ON o.id=(\
               SELECT latest.id FROM operations latest WHERE latest.deployment_id=d.id \
               ORDER BY latest.started_at DESC,latest.id DESC LIMIT 1) ORDER BY d.id",
        )?;
        let rows = statement.query_map([], |row| {
            let operation_id = row.get::<_, Option<String>>(4)?;
            Ok(StoredDeployment {
                deployment: row.get(0)?,
                definition_hash: row.get(1)?,
                snapshot_json: row.get(2)?,
                applied_at: row.get(3)?,
                last_operation: operation_id.map(|id| StoredOperation {
                    id,
                    deployment: row.get(0).expect("deployment column is valid"),
                    kind: row.get(5).expect("joined operation kind is present"),
                    status: row.get(6).expect("joined operation status is present"),
                    started_at: row.get(7).expect("joined operation start is present"),
                    finished_at: row.get(8).expect("joined operation finish is valid"),
                    error_code: row.get(9).expect("joined operation error code is valid"),
                    error_context_json: row
                        .get(10)
                        .expect("joined operation error context is valid"),
                }),
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Starts an auditable operation.
    pub fn start_operation(&self, operation: &OperationRecord) -> Result<(), StateError> {
        validate_id("operation id", &operation.id)?;
        validate_id("deployment", &operation.deployment)?;
        self.connection.execute(
            "INSERT INTO operations(id, deployment_id, kind, status, started_at, finished_at, error_code, error_context_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![operation.id, operation.deployment, operation.kind.as_str(), operation.status.as_str(), operation.started_at, operation.finished_at, operation.error.as_ref().map(|e| e.code.as_str()), operation.error.as_ref().map(|e| e.context.as_json())],
        )?;
        Ok(())
    }

    /// Completes or updates an operation without discarding earlier history.
    pub fn update_operation(
        &self,
        id: &str,
        status: OperationStatus,
        finished_at: Option<i64>,
        error: Option<&OperationError>,
    ) -> Result<(), StateError> {
        let changed = self.connection.execute(
            "UPDATE operations SET status=?2, finished_at=?3, error_code=?4, error_context_json=?5 WHERE id=?1",
            params![id, status.as_str(), finished_at, error.map(|e| e.code.as_str()), error.map(|e| e.context.as_json())],
        )?;
        if changed == 0 {
            return Err(invalid(
                "operation_not_found",
                format!("operation `{id}` does not exist"),
            ));
        }
        Ok(())
    }

    /// Loads an operation by its stable identifier.
    pub fn operation(&self, id: &str) -> Result<Option<StoredOperation>, StateError> {
        self.connection
            .query_row(
                "SELECT id, deployment_id, kind, status, started_at, finished_at, error_code, error_context_json FROM operations WHERE id=?1",
                [id],
                |row| {
                    let error_code = row.get::<_, Option<String>>(6)?;
                    let error_context = row.get::<_, Option<String>>(7)?;
                    Ok(StoredOperation {
                        id: row.get(0)?,
                        deployment: row.get(1)?,
                        kind: row.get(2)?,
                        status: row.get(3)?,
                        started_at: row.get(4)?,
                        finished_at: row.get(5)?,
                        error_code,
                        error_context_json: error_context,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// Marks operations left running by an earlier daemon as failed during recovery.
    pub fn recover_abandoned_operations(&self, finished_at: i64) -> Result<usize, StateError> {
        self.connection
            .execute(
                "UPDATE operations SET status='failed', finished_at=?1, error_code='daemon_restarted', error_context_json='{}' WHERE status IN ('pending','running')",
                [finished_at],
            )
            .map_err(Into::into)
    }

    /// Appends a health/readiness observation.
    pub fn record_health(&self, observation: &HealthObservation) -> Result<(), StateError> {
        self.connection.execute(
            "INSERT INTO health_observations(deployment_id, subject, health, readiness, observed_at, context_json) VALUES (?1,?2,?3,?4,?5,?6)",
            params![observation.deployment, observation.subject, observation.health, observation.readiness, observation.observed_at, observation.context.as_json()],
        )?;
        Ok(())
    }

    /// Appends a dynamic route record.
    pub fn record_route(&self, route: &RouteRecord) -> Result<(), StateError> {
        self.connection.execute(
            "INSERT INTO routes(deployment_id, route_key, consumer, provider, protocol, recorded_at) VALUES (?1,?2,?3,?4,?5,?6)",
            params![route.deployment, route.route_key, route.consumer, route.provider, route.protocol, route.recorded_at],
        )?;
        Ok(())
    }

    /// Appends an immutable route-snapshot activation attempt.
    pub fn record_route_snapshot(&self, snapshot: &RouteSnapshotRecord) -> Result<(), StateError> {
        self.connection.execute(
            "INSERT INTO route_snapshots(deployment_id, version, checksum, activation_status, recorded_at, context_json, router_id, binding_id, operation_id) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![snapshot.deployment, snapshot.version, snapshot.checksum, snapshot.activation_status.as_str(), snapshot.recorded_at, snapshot.context.as_json(), snapshot.router, snapshot.binding, snapshot.operation_id],
        )?;
        Ok(())
    }

    /// Records a router acknowledgement or failure and updates version visibility atomically.
    pub fn record_router_apply(&mut self, record: &RouterApplyRecord) -> Result<(), StateError> {
        for (name, value) in [
            ("deployment", record.deployment.as_str()),
            ("router", record.router.as_str()),
            ("binding", record.binding.as_str()),
            ("checksum", record.checksum.as_str()),
        ] {
            validate_id(name, value)?;
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = tx
            .query_row(
                "SELECT current_version,current_checksum FROM router_bindings WHERE deployment_id=?1 AND router_id=?2 AND binding_id=?3",
                params![record.deployment, record.router, record.binding],
                |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()?;
        let (old_version, old_checksum) = existing.unwrap_or((None, None));
        let activates = record.status == RouterApplyStatus::Active
            || record.status == RouterApplyStatus::RolledBack && record.error_code.is_none();
        let current_version = activates
            .then_some(record.version)
            .or(old_version)
            .or(record.observed_version);
        let current_checksum = if activates {
            Some(record.checksum.clone())
        } else {
            old_checksum
                .clone()
                .or_else(|| record.observed_checksum.clone())
        };
        let previous_version = activates
            .then(|| old_version.or(record.observed_version))
            .flatten();
        let previous_checksum = activates
            .then(|| old_checksum.or_else(|| record.observed_checksum.clone()))
            .flatten();
        let observed_version = if activates {
            current_version
        } else {
            record.observed_version.or(current_version)
        };
        let observed_checksum = if activates {
            current_checksum.clone()
        } else {
            record
                .observed_checksum
                .clone()
                .or_else(|| current_checksum.clone())
        };
        tx.execute(
            "INSERT INTO router_bindings(deployment_id,router_id,binding_id,desired_version,desired_checksum,current_version,current_checksum,previous_version,previous_checksum,observed_version,observed_checksum,apply_status,transition_json,last_error_code,updated_at) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15) \
             ON CONFLICT(deployment_id,router_id,binding_id) DO UPDATE SET desired_version=excluded.desired_version,desired_checksum=excluded.desired_checksum,current_version=excluded.current_version,current_checksum=excluded.current_checksum,previous_version=CASE WHEN ?16 THEN excluded.previous_version ELSE router_bindings.previous_version END,previous_checksum=CASE WHEN ?16 THEN excluded.previous_checksum ELSE router_bindings.previous_checksum END,observed_version=excluded.observed_version,observed_checksum=excluded.observed_checksum,apply_status=excluded.apply_status,transition_json=excluded.transition_json,last_error_code=excluded.last_error_code,updated_at=excluded.updated_at",
            params![record.deployment, record.router, record.binding, record.desired_version, record.desired_checksum, current_version, current_checksum, previous_version, previous_checksum, observed_version, observed_checksum, record.status.as_str(), record.transition.as_json(), record.error_code, record.recorded_at, activates],
        )?;
        tx.execute(
            "INSERT INTO route_snapshots(deployment_id,version,checksum,activation_status,recorded_at,context_json,router_id,binding_id,operation_id) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![record.deployment, record.version, record.checksum, record.status.history_status(), record.recorded_at, record.context.as_json(), record.router, record.binding, record.operation_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Returns current version state for every router/binding in a deployment.
    pub fn router_bindings(&self, deployment: &str) -> Result<Vec<RouterBindingState>, StateError> {
        let mut statement = self.connection.prepare(
            "SELECT router_id,binding_id,desired_version,desired_checksum,current_version,current_checksum,previous_version,previous_checksum,observed_version,observed_checksum,apply_status,transition_json,last_error_code,updated_at FROM router_bindings WHERE deployment_id=?1 ORDER BY router_id,binding_id",
        )?;
        statement
            .query_map([deployment], |row| {
                Ok(RouterBindingState {
                    deployment: deployment.to_owned(),
                    router: row.get(0)?,
                    binding: row.get(1)?,
                    desired_version: row.get(2)?,
                    desired_checksum: row.get(3)?,
                    current_version: row.get(4)?,
                    current_checksum: row.get(5)?,
                    previous_version: row.get(6)?,
                    previous_checksum: row.get(7)?,
                    observed_version: row.get(8)?,
                    observed_checksum: row.get(9)?,
                    status: row.get(10)?,
                    transition_json: row.get(11)?,
                    last_error_code: row.get(12)?,
                    updated_at: row.get(13)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Returns append-only route activation history for a deployment.
    pub fn route_history(&self, deployment: &str) -> Result<Vec<StoredRouteSnapshot>, StateError> {
        let mut statement = self.connection.prepare(
            "SELECT sequence,router_id,binding_id,operation_id,version,checksum,activation_status,recorded_at,context_json FROM route_snapshots WHERE deployment_id=?1 ORDER BY sequence",
        )?;
        statement
            .query_map([deployment], |row| {
                Ok(StoredRouteSnapshot {
                    sequence: row.get(0)?,
                    deployment: deployment.to_owned(),
                    router: row.get(1)?,
                    binding: row.get(2)?,
                    operation_id: row.get(3)?,
                    version: row.get(4)?,
                    checksum: row.get(5)?,
                    activation_status: row.get(6)?,
                    recorded_at: row.get(7)?,
                    context_json: row.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Atomically acquires a deployment mutation lease or recovers an expired one.
    pub fn acquire_lock(&mut self, request: &LockRequest<'_>) -> Result<OperationLock, StateError> {
        if request.ttl_millis <= 0 {
            return Err(invalid("invalid_lock_ttl", "lock TTL must be positive"));
        }
        for (name, value) in [
            ("deployment", request.deployment),
            ("owner", request.owner),
            ("token", request.token),
        ] {
            validate_id(name, value)?;
        }
        let expires_at = request
            .now
            .checked_add(request.ttl_millis)
            .ok_or_else(|| invalid("invalid_lock_ttl", "lock expiry overflow"))?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = tx
            .query_row(
                "SELECT owner_instance, expires_at FROM operation_locks WHERE deployment_id=?1",
                [request.deployment],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        if let Some((owner, existing_expiry)) = existing
            .as_ref()
            .filter(|(_, expiry)| *expiry > request.now)
        {
            return Err(StateError::LockContended {
                deployment: request.deployment.into(),
                owner: owner.clone(),
                expires_at: *existing_expiry,
            });
        }
        tx.execute(
            "INSERT INTO operation_locks(deployment_id, owner_instance, owner_pid, owner_started_at, token, heartbeat_at, expires_at) VALUES (?1,?2,?3,?4,?5,?6,?7)\
             ON CONFLICT(deployment_id) DO UPDATE SET owner_instance=excluded.owner_instance, owner_pid=excluded.owner_pid, owner_started_at=excluded.owner_started_at, token=excluded.token, heartbeat_at=excluded.heartbeat_at, expires_at=excluded.expires_at",
            params![request.deployment, request.owner, request.pid, request.process_started_at, request.token, request.now, expires_at],
        )?;
        tx.commit()?;
        Ok(OperationLock {
            deployment: request.deployment.into(),
            owner: request.owner.into(),
            token: request.token.into(),
            expires_at,
            recovered: existing.is_some(),
        })
    }

    /// Extends a lease only if the original owner and token still match and it has not expired.
    pub fn heartbeat_lock(
        &self,
        lock: &mut OperationLock,
        now: i64,
        ttl_millis: i64,
    ) -> Result<(), StateError> {
        if ttl_millis <= 0 {
            return Err(invalid("invalid_lock_ttl", "lock TTL must be positive"));
        }
        let expires_at = now
            .checked_add(ttl_millis)
            .ok_or_else(|| invalid("invalid_lock_ttl", "lock expiry overflow"))?;
        let changed = self.connection.execute(
            "UPDATE operation_locks SET heartbeat_at=?4, expires_at=?5 WHERE deployment_id=?1 AND owner_instance=?2 AND token=?3 AND expires_at>?4",
            params![lock.deployment, lock.owner, lock.token, now, expires_at],
        )?;
        if changed == 0 {
            return Err(StateError::LockLost {
                deployment: lock.deployment.clone(),
            });
        }
        lock.expires_at = expires_at;
        Ok(())
    }

    /// Releases a lease only if the original owner and token still match.
    pub fn release_lock(&self, lock: OperationLock) -> Result<(), StateError> {
        let changed = self.connection.execute(
            "DELETE FROM operation_locks WHERE deployment_id=?1 AND owner_instance=?2 AND token=?3",
            params![lock.deployment, lock.owner, lock.token],
        )?;
        if changed == 0 {
            return Err(StateError::LockLost {
                deployment: lock.deployment,
            });
        }
        Ok(())
    }

    /// Reconciles applied records, generated manifests, and Docker label observations.
    ///
    /// Only observed deployment/resource state is updated; no desired snapshot is invented
    /// and no runtime resource is changed.
    pub fn reconcile(
        &mut self,
        input: &ReconciliationInput,
        observed_at: i64,
    ) -> Result<ReconciliationReport, StateError> {
        let mut deployments = BTreeSet::new();
        deployments.extend(
            input
                .manifests
                .iter()
                .map(|manifest| manifest.deployment.clone()),
        );
        deployments.extend(
            input
                .resources
                .iter()
                .filter_map(|resource| resource.labels.get(DEPLOYMENT_LABEL).cloned()),
        );
        {
            let mut statement = self.connection.prepare("SELECT id FROM deployments")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            for row in rows {
                deployments.insert(row?);
            }
        }
        let mut preserved_unreachable = BTreeMap::<String, Vec<OwnedResourceObservation>>::new();
        if !input.unreachable_devices.is_empty() {
            for deployment in &deployments {
                let resources = self
                    .active_resources(deployment)?
                    .into_iter()
                    .filter(|resource| input.unreachable_devices.contains(&resource.device))
                    .collect::<Vec<_>>();
                if !resources.is_empty() {
                    preserved_unreachable.insert(deployment.clone(), resources);
                }
            }
        }
        let tx = self.connection.transaction()?;
        let mut reports = Vec::new();
        for deployment in deployments {
            let manifest = input
                .manifests
                .iter()
                .find(|manifest| manifest.deployment == deployment);
            let mut resources = input
                .resources
                .iter()
                .filter(|resource| {
                    resource.labels.get(DEPLOYMENT_LABEL).map(String::as_str)
                        == Some(deployment.as_str())
                })
                .cloned()
                .collect::<Vec<_>>();
            resources.extend(
                preserved_unreachable
                    .remove(&deployment)
                    .unwrap_or_default(),
            );
            tx.execute("INSERT INTO deployments(id,last_observed_at) VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET last_observed_at=excluded.last_observed_at", params![deployment, observed_at])?;
            deployment_event(
                &tx,
                &deployment,
                "reconciled",
                manifest.map(|m| m.definition_hash.as_str()),
                observed_at,
                None,
            )?;
            tx.execute(
                "UPDATE resources SET active=0 WHERE deployment_id=?1 AND active=1",
                [&deployment],
            )?;
            for resource in &resources {
                tx.execute(
                    "INSERT INTO resources(deployment_id, kind, runtime_id, name, resource_hash, state, labels_json, observed_at, active) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,1)",
                    params![deployment, resource.kind.as_str(), resource.id, resource.name, resource.labels.get(RESOURCE_HASH_LABEL), resource.state, ownership_labels_json(&resource.labels)?, observed_at],
                )?;
            }
            let applied = tx.query_row(
                "SELECT applied_definition_hash FROM deployments WHERE id=?1",
                [&deployment],
                |row| row.get::<_, Option<String>>(0),
            )?;
            let mut report = reconcile_deployment(
                &deployment,
                applied.as_deref(),
                manifest,
                &resources.iter().collect::<Vec<_>>(),
            );
            if let Some(manifest) = manifest {
                for device in manifest
                    .remote_projects
                    .keys()
                    .filter(|device| input.unreachable_devices.contains(*device))
                {
                    report.diagnostics.push(diagnostic(
                        DriftCode::DeviceUnreachable,
                        format!("observed.devices.{device}"),
                        format!("device `{device}` unreachable; previous resource observations were retained"),
                    ));
                }
            }
            reports.push(report);
        }
        tx.commit()?;
        Ok(ReconciliationReport {
            deployments: reports,
        })
    }

    /// Loads the latest active resource observations in deterministic order.
    pub fn active_resources(
        &self,
        deployment: &str,
    ) -> Result<Vec<OwnedResourceObservation>, StateError> {
        let mut statement = self.connection.prepare(
            "SELECT kind, runtime_id, name, labels_json, state FROM resources \
             WHERE deployment_id=?1 AND active=1 ORDER BY kind, name, runtime_id",
        )?;
        let rows = statement.query_map([deployment], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?;
        rows.map(|row| {
            let (kind, id, name, labels, state) = row?;
            let labels = serde_json::from_str::<BTreeMap<String, String>>(&labels)?;
            Ok(OwnedResourceObservation {
                kind: ResourceKind::parse(&kind)?,
                id,
                name,
                device: labels
                    .get(DEVICE_LABEL)
                    .cloned()
                    .unwrap_or_else(local_device),
                labels,
                state,
            })
        })
        .collect()
    }
}

fn current_schema_version(connection: &Connection) -> Result<i64, StateError> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='schema_versions')",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(0);
    }
    connection
        .query_row(
            "SELECT COALESCE(MAX(version),0) FROM schema_versions",
            [],
            |row| row.get(0),
        )
        .map_err(Into::into)
}

fn backup_path(path: &Path, version: i64) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".pre-migration-v{version}.bak"));
    let candidate = PathBuf::from(name);
    if !candidate.exists() {
        return candidate;
    }
    for suffix in 1_u64.. {
        let mut name = candidate.as_os_str().to_owned();
        name.push(format!(".{suffix}"));
        let next = PathBuf::from(name);
        if !next.exists() {
            return next;
        }
    }
    unreachable!("an available backup suffix exists")
}

fn backup_database(source: &Connection, path: &Path) -> Result<(), StateError> {
    let mut destination = Connection::open(path)?;
    let result = {
        let backup = Backup::new(source, &mut destination)?;
        backup.run_to_completion(64, Duration::from_millis(10), None)
    };
    if let Err(error) = result {
        drop(destination);
        let _ = fs::remove_file(path);
        return Err(error.into());
    }
    Ok(())
}

fn validate_id(name: &str, value: &str) -> Result<(), StateError> {
    if value.trim().is_empty() {
        Err(invalid(
            "invalid_identifier",
            format!("{name} cannot be empty"),
        ))
    } else {
        Ok(())
    }
}

fn validate_device(device: &RegisteredDevice) -> Result<(), StateError> {
    validate_id("device name", &device.name)?;
    if device.host.trim().is_empty()
        || device.host.chars().any(char::is_whitespace)
        || device.host.starts_with('-')
    {
        return Err(invalid(
            "invalid_device_host",
            "device host cannot be empty, contain whitespace, or start with `-`",
        ));
    }
    // The SSH destination is passed as `user@host`; a leading `-` would be
    // parsed by ssh as an option (e.g. -oProxyCommand=...), so reject it.
    if device.user.trim().is_empty()
        || device.user.chars().any(char::is_whitespace)
        || device.user.starts_with('-')
        || device.user.contains('@')
    {
        return Err(invalid(
            "invalid_device_user",
            "device user cannot be empty, contain whitespace or `@`, or start with `-`",
        ));
    }
    if device.port == 0 {
        return Err(invalid(
            "invalid_device_port",
            "device port must be between 1 and 65535",
        ));
    }
    if device
        .identity_file
        .as_ref()
        .is_some_and(|path| path.as_os_str().is_empty())
    {
        return Err(invalid(
            "invalid_identity_file",
            "identity file path cannot be empty",
        ));
    }
    Ok(())
}

fn ownership_labels_json(labels: &BTreeMap<String, String>) -> Result<String, StateError> {
    let retained = labels
        .iter()
        .filter(|(key, _)| {
            matches!(
                key.as_str(),
                MANAGED_LABEL | DEPLOYMENT_LABEL | RESOURCE_HASH_LABEL | DEVICE_LABEL
            )
        })
        .collect::<BTreeMap<_, _>>();
    Ok(serde_json::to_string(&retained)?)
}

fn deployment_event(
    tx: &Transaction<'_>,
    deployment: &str,
    event: &str,
    definition_hash: Option<&str>,
    recorded_at: i64,
    context: Option<&Value>,
) -> Result<(), StateError> {
    tx.execute("INSERT INTO deployment_history(deployment_id,event,definition_hash,recorded_at,context_json) VALUES (?1,?2,?3,?4,?5)", params![deployment,event,definition_hash,recorded_at,context.map(serde_json::to_string).transpose()?])?;
    Ok(())
}

/// Last-applied deployment state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppliedDeployment {
    /// Stable deployment identifier.
    pub deployment: String,
    /// Hash of the human-authored definition that produced the snapshot.
    pub definition_hash: String,
    /// Canonical resolved state recorded by the successful apply.
    pub snapshot: AppliedSnapshot,
    /// Caller-supplied Unix timestamp in milliseconds.
    pub applied_at: i64,
}

/// Deployment list projection used by control-plane clients.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredDeployment {
    pub deployment: String,
    pub definition_hash: Option<String>,
    pub snapshot_json: Option<String>,
    pub applied_at: Option<i64>,
    pub last_operation: Option<StoredOperation>,
}

/// Audited control-plane operation kind.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OperationKind {
    /// Image or artifact build.
    Build,
    /// Deployment start.
    Start,
    /// Deployment stop.
    Stop,
    /// Atomic binding change.
    Bind,
    /// Desired-state apply.
    Apply,
    /// Ownership-aware cleanup.
    Cleanup,
    /// Forward-compatible adapter operation.
    Other(String),
}
impl OperationKind {
    fn as_str(&self) -> &str {
        match self {
            Self::Build => "build",
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Bind => "bind",
            Self::Apply => "apply",
            Self::Cleanup => "cleanup",
            Self::Other(value) => value,
        }
    }
}
/// Audited operation status.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationStatus {
    /// Queued but not executing.
    Pending,
    /// Currently executing.
    Running,
    /// Completed successfully.
    Succeeded,
    /// Completed with structured failure context.
    Failed,
    /// Cancelled by an authorized caller.
    Cancelled,
}
impl OperationStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}
/// Structured operation error, with stable code and JSON context.
#[derive(Clone, Debug, PartialEq)]
pub struct OperationError {
    /// Stable machine-readable error code.
    pub code: String,
    /// Secret-safe structured context.
    pub context: StructuredContext,
}
/// Operation history input.
#[derive(Clone, Debug, PartialEq)]
pub struct OperationRecord {
    /// Stable operation identifier shared with manifests and events.
    pub id: String,
    /// Deployment being observed or mutated.
    pub deployment: String,
    /// Operation category.
    pub kind: OperationKind,
    /// Current operation status.
    pub status: OperationStatus,
    /// Unix start timestamp in milliseconds.
    pub started_at: i64,
    /// Unix completion timestamp in milliseconds, when terminal.
    pub finished_at: Option<i64>,
    /// Structured terminal failure, when applicable.
    pub error: Option<OperationError>,
}

/// Persisted operation fields returned to the control plane without framework types.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredOperation {
    pub id: String,
    pub deployment: String,
    pub kind: String,
    pub status: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub error_code: Option<String>,
    pub error_context_json: Option<String>,
}
/// Health and readiness history input.
#[derive(Clone, Debug, PartialEq)]
pub struct HealthObservation {
    pub deployment: String,
    pub subject: String,
    pub health: String,
    pub readiness: String,
    pub observed_at: i64,
    pub context: StructuredContext,
}
/// Dynamic route history input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteRecord {
    pub deployment: String,
    pub route_key: String,
    pub consumer: String,
    pub provider: String,
    pub protocol: String,
    pub recorded_at: i64,
}
/// Snapshot activation state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivationStatus {
    Pending,
    Active,
    Rejected,
    RolledBack,
}
impl ActivationStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Rejected => "rejected",
            Self::RolledBack => "rolled_back",
        }
    }
}
/// Immutable route snapshot history input.
#[derive(Clone, Debug, PartialEq)]
pub struct RouteSnapshotRecord {
    pub deployment: String,
    pub router: Option<String>,
    pub binding: Option<String>,
    pub operation_id: Option<String>,
    pub version: i64,
    pub checksum: String,
    pub activation_status: ActivationStatus,
    pub recorded_at: i64,
    pub context: StructuredContext,
}

/// Per-router apply status used by the acknowledgement gate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouterApplyStatus {
    Pending,
    Active,
    Failed,
    RolledBack,
}

impl RouterApplyStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Failed => "failed",
            Self::RolledBack => "rolled_back",
        }
    }

    const fn history_status(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Failed => "rejected",
            Self::RolledBack => "rolled_back",
        }
    }
}

/// One desired snapshot attempt and its observed acknowledgement state.
#[derive(Clone, Debug, PartialEq)]
pub struct RouterApplyRecord {
    pub deployment: String,
    pub router: String,
    pub binding: String,
    pub operation_id: String,
    pub desired_version: i64,
    pub desired_checksum: String,
    pub version: i64,
    pub checksum: String,
    pub status: RouterApplyStatus,
    pub observed_version: Option<i64>,
    pub observed_checksum: Option<String>,
    pub transition: StructuredContext,
    pub error_code: Option<String>,
    pub recorded_at: i64,
    pub context: StructuredContext,
}

/// Queryable desired/applied/observed route versions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouterBindingState {
    pub deployment: String,
    pub router: String,
    pub binding: String,
    pub desired_version: Option<i64>,
    pub desired_checksum: Option<String>,
    pub current_version: Option<i64>,
    pub current_checksum: Option<String>,
    pub previous_version: Option<i64>,
    pub previous_checksum: Option<String>,
    pub observed_version: Option<i64>,
    pub observed_checksum: Option<String>,
    pub status: String,
    pub transition_json: String,
    pub last_error_code: Option<String>,
    pub updated_at: i64,
}

/// One append-only route snapshot history row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredRouteSnapshot {
    pub sequence: i64,
    pub deployment: String,
    pub router: Option<String>,
    pub binding: Option<String>,
    pub operation_id: Option<String>,
    pub version: i64,
    pub checksum: String,
    pub activation_status: String,
    pub recorded_at: i64,
    pub context_json: String,
}

/// Mutation lease acquisition request.
#[derive(Clone, Copy, Debug)]
pub struct LockRequest<'a> {
    /// Deployment whose mutations will be serialized.
    pub deployment: &'a str,
    /// Unique process instance identity, not merely an executable name.
    pub owner: &'a str,
    /// Operating-system process identifier for diagnostics.
    pub pid: u32,
    /// Process start timestamp, used to distinguish PID reuse.
    pub process_started_at: i64,
    /// Unguessable lease token generated by the caller.
    pub token: &'a str,
    /// Current Unix timestamp in milliseconds.
    pub now: i64,
    /// Positive lease lifetime in milliseconds.
    pub ttl_millis: i64,
}
/// An acquired lease. Tokens should be random UUIDs supplied by the daemon/CLI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationLock {
    /// Locked deployment.
    pub deployment: String,
    /// Owning process instance.
    pub owner: String,
    token: String,
    /// Current Unix expiry timestamp in milliseconds.
    pub expires_at: i64,
    /// Whether acquisition replaced an expired owner.
    pub recovered: bool,
}

/// Runtime resource kind aligned with CLI Docker discovery.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Container,
    Image,
    Network,
    Volume,
}
impl ResourceKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Container => "container",
            Self::Image => "image",
            Self::Network => "network",
            Self::Volume => "volume",
        }
    }

    fn parse(value: &str) -> Result<Self, StateError> {
        match value {
            "container" => Ok(Self::Container),
            "image" => Ok(Self::Image),
            "network" => Ok(Self::Network),
            "volume" => Ok(Self::Volume),
            _ => Err(invalid(
                "invalid_resource_kind",
                format!("stored resource kind `{value}` is unsupported"),
            )),
        }
    }
}
/// Injected Docker ownership-label observation; reconciliation never invokes Docker.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnedResourceObservation {
    /// Docker resource category.
    pub kind: ResourceKind,
    /// Runtime identifier reported by Docker.
    pub id: String,
    /// Human-readable Docker resource name.
    pub name: String,
    /// Injected labels; only ownership labels are persisted.
    pub labels: BTreeMap<String, String>,
    /// Runtime or health state, if the resource exposes one.
    pub state: Option<String>,
    /// Docker daemon placement (`local` or a registered device name).
    #[serde(default = "local_device")]
    pub device: String,
}

fn local_device() -> String {
    "local".into()
}

/// Relevant, version-stable fields from a generated `manifest.json`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedManifest {
    /// Stable deployment identifier.
    pub deployment: String,
    /// Hash of the resolved human-authored definition.
    pub definition_hash: String,
    /// Hash of topology-affecting runtime resources.
    pub resource_hash: String,
    /// Remote Compose projects persisted for restart-safe discovery and cleanup.
    #[serde(default)]
    pub remote_projects: BTreeMap<String, GeneratedRemoteProject>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedRemoteProject {
    pub device: GeneratedDevice,
    pub compose_project: String,
    pub compose_file: PathBuf,
    #[serde(default)]
    pub services: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedDevice {
    pub user: String,
    pub host: String,
    pub port: u16,
    pub identity_file: Option<PathBuf>,
}

impl GeneratedManifest {
    /// Loads all immediate `<deployment>/manifest.json` files in deterministic order.
    pub fn load_generated(root: &Path) -> Result<Vec<Self>, StateError> {
        let mut paths = match fs::read_dir(root) {
            Ok(entries) => entries
                .filter_map(Result::ok)
                .map(|entry| entry.path().join("manifest.json"))
                .filter(|path| path.is_file())
                .collect::<Vec<_>>(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        paths.sort();
        paths
            .into_iter()
            .map(|path| Ok(serde_json::from_slice(&fs::read(path)?)?))
            .collect()
    }
}

/// The three reconciliation sources: store records are read from `StateStore`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReconciliationInput {
    /// Parsed generated manifests.
    pub manifests: Vec<GeneratedManifest>,
    /// Docker observations supplied by the runtime adapter.
    pub resources: Vec<OwnedResourceObservation>,
    /// Devices whose previous observations remain authoritative while unreachable.
    pub unreachable_devices: BTreeSet<String>,
}

/// Stable desired/applied/observed divergence code.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriftCode {
    AppliedStateMissing,
    GeneratedManifestMissing,
    DesiredAppliedHashMismatch,
    ObservedResourcesMissing,
    OwnershipInvalid,
    ObservedResourceHashMissing,
    ObservedResourceHashMismatch,
    MultipleObservedResourceHashes,
    DeviceUnreachable,
}
/// Machine-readable drift plus human context.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriftDiagnostic {
    /// Stable machine-readable drift category.
    pub code: DriftCode,
    /// Logical desired/applied/observed path.
    pub path: String,
    /// Human-readable evidence and safe next-step context.
    pub message: String,
}
/// One deployment's reconciliation result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentReconciliation {
    /// Stable deployment identifier.
    pub deployment: String,
    /// Deterministically ordered divergence observations.
    pub diagnostics: Vec<DriftDiagnostic>,
}
/// Deterministically ordered reconciliation result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconciliationReport {
    /// Results ordered lexically by deployment identifier.
    pub deployments: Vec<DeploymentReconciliation>,
}

fn diagnostic(
    code: DriftCode,
    path: impl Into<String>,
    message: impl Into<String>,
) -> DriftDiagnostic {
    DriftDiagnostic {
        code,
        path: path.into(),
        message: message.into(),
    }
}

fn reconcile_deployment(
    deployment: &str,
    applied_hash: Option<&str>,
    manifest: Option<&GeneratedManifest>,
    resources: &[&OwnedResourceObservation],
) -> DeploymentReconciliation {
    let mut diagnostics = Vec::new();
    match (manifest, applied_hash) {
        (Some(_), None) => diagnostics.push(diagnostic(DriftCode::AppliedStateMissing, "applied", "generated state exists but no successful apply is recorded; desired state was not invented")),
        (None, Some(_)) => diagnostics.push(diagnostic(DriftCode::GeneratedManifestMissing, "desired.manifest", "applied state exists but the generated manifest is missing")),
        (Some(manifest), Some(applied)) if manifest.definition_hash != applied => diagnostics.push(diagnostic(DriftCode::DesiredAppliedHashMismatch, "desired.definitionHash", format!("generated definition hash {} differs from applied hash {applied}", manifest.definition_hash))),
        _ => {}
    }
    if resources.is_empty() {
        if manifest.is_some() || applied_hash.is_some() {
            diagnostics.push(diagnostic(
                DriftCode::ObservedResourcesMissing,
                "observed.resources",
                "no labeled Docker resources were observed",
            ));
        }
    } else {
        let invalid_resources = resources
            .iter()
            .filter(|resource| {
                resource.labels.get(MANAGED_LABEL).map(String::as_str) != Some("true")
                    || resource.labels.get(DEPLOYMENT_LABEL).map(String::as_str) != Some(deployment)
            })
            .map(|resource| resource.name.as_str())
            .collect::<Vec<_>>();
        if !invalid_resources.is_empty() {
            diagnostics.push(diagnostic(
                DriftCode::OwnershipInvalid,
                "observed.resources",
                format!(
                    "resources have invalid ownership labels: {}",
                    invalid_resources.join(", ")
                ),
            ));
        }
        let topology = resources
            .iter()
            .filter(|resource| resource.kind != ResourceKind::Volume)
            .copied()
            .collect::<Vec<_>>();
        let hash_sources = if topology.is_empty() {
            resources.to_vec()
        } else {
            topology
        };
        let missing = hash_sources
            .iter()
            .filter(|resource| !resource.labels.contains_key(RESOURCE_HASH_LABEL))
            .map(|resource| resource.name.as_str())
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            diagnostics.push(diagnostic(
                DriftCode::ObservedResourceHashMissing,
                "observed.resources",
                format!(
                    "resources lack resource-hash labels: {}",
                    missing.join(", ")
                ),
            ));
        }
        let hashes = hash_sources
            .iter()
            .filter_map(|resource| resource.labels.get(RESOURCE_HASH_LABEL))
            .cloned()
            .collect::<BTreeSet<_>>();
        if hashes.len() > 1 {
            diagnostics.push(diagnostic(
                DriftCode::MultipleObservedResourceHashes,
                "observed.resourceHash",
                format!(
                    "runtime has multiple resource hashes: {}",
                    hashes.iter().cloned().collect::<Vec<_>>().join(", ")
                ),
            ));
        }
        if let Some(expected) = manifest.map(|manifest| manifest.resource_hash.as_str()) {
            if !hashes.is_empty() && !hashes.contains(expected) {
                diagnostics.push(diagnostic(
                    DriftCode::ObservedResourceHashMismatch,
                    "observed.resourceHash",
                    format!(
                        "generated resource hash {expected} differs from observed hash(es): {}",
                        hashes.into_iter().collect::<Vec<_>>().join(", ")
                    ),
                ));
            }
        }
    }
    DeploymentReconciliation {
        deployment: deployment.into(),
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn open_temp(temp: &TempDir) -> (StateStore, OpenReport) {
        StateStore::open(temp.path().join("state.sqlite3")).unwrap()
    }

    fn manifest(definition_hash: &str, resource_hash: &str) -> GeneratedManifest {
        GeneratedManifest {
            deployment: "demo".into(),
            definition_hash: definition_hash.into(),
            resource_hash: resource_hash.into(),
            remote_projects: BTreeMap::new(),
        }
    }

    fn resource(hash: Option<&str>) -> OwnedResourceObservation {
        let mut labels = BTreeMap::from([
            (MANAGED_LABEL.into(), "true".into()),
            (DEPLOYMENT_LABEL.into(), "demo".into()),
        ]);
        if let Some(hash) = hash {
            labels.insert(RESOURCE_HASH_LABEL.into(), hash.into());
        }
        OwnedResourceObservation {
            kind: ResourceKind::Container,
            id: "container-id".into(),
            name: "demo-api".into(),
            labels,
            state: Some("running".into()),
            device: "local".into(),
        }
    }

    fn lock_request<'a>(owner: &'a str, token: &'a str, now: i64) -> LockRequest<'a> {
        LockRequest {
            deployment: "demo",
            owner,
            pid: 42,
            process_started_at: 100,
            token,
            now,
            ttl_millis: 50,
        }
    }

    fn historical_database(path: &Path, version: i64) {
        let connection = Connection::open(path).unwrap();
        for (migration_version, sql) in MIGRATIONS
            .iter()
            .filter(|(migration_version, _)| *migration_version <= version)
        {
            connection.execute_batch(sql).unwrap();
            connection
                .execute(
                    "INSERT INTO schema_versions(version, applied_at) VALUES (?1, ?2)",
                    params![migration_version, migration_version * 100],
                )
                .unwrap();
        }
        connection
            .execute(
                "INSERT INTO deployments(id,applied_definition_hash,applied_snapshot_json,applied_at,last_observed_at) VALUES (?1,?2,?3,?4,?5)",
                params![
                    format!("demo-v{version}"),
                    format!("definition-v{version}"),
                    format!(r#"{{"deployment":"demo-v{version}","schema":{version}}}"#),
                    1_000 + version,
                    2_000 + version
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO deployment_history(deployment_id,event,definition_hash,recorded_at,context_json) VALUES (?1,?2,?3,?4,?5)",
                params![
                    format!("demo-v{version}"),
                    "applied",
                    format!("definition-v{version}"),
                    3_000 + version,
                    format!(r#"{{"history":"v{version}"}}"#)
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO operations(id,deployment_id,kind,status,started_at,finished_at,error_code,error_context_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    format!("operation-v{version}"),
                    format!("demo-v{version}"),
                    "apply",
                    "succeeded",
                    4_000 + version,
                    4_100 + version,
                    Option::<String>::None,
                    Option::<String>::None
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO resources(deployment_id,kind,runtime_id,name,resource_hash,state,labels_json,observed_at,active) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,1)",
                params![
                    format!("demo-v{version}"),
                    "container",
                    format!("container-v{version}"),
                    format!("api-v{version}"),
                    format!("resource-v{version}"),
                    "running",
                    format!(r#"{{"dev.switchyard.deployment":"demo-v{version}","dev.switchyard.managed":"true"}}"#),
                    5_000 + version
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO health_observations(deployment_id,subject,health,readiness,observed_at,context_json) VALUES (?1,?2,?3,?4,?5,?6)",
                params![
                    format!("demo-v{version}"),
                    "api",
                    "healthy",
                    "ready",
                    6_000 + version,
                    format!(r#"{{"health":"v{version}"}}"#)
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO operation_locks(deployment_id,owner_instance,owner_pid,owner_started_at,token,heartbeat_at,expires_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    format!("locked-v{version}"),
                    format!("owner-v{version}"),
                    42,
                    7_000 + version,
                    format!("token-v{version}"),
                    8_000 + version,
                    9_000 + version
                ],
            )
            .unwrap();
        if version >= 2 {
            connection
                .execute(
                    "INSERT INTO routes(deployment_id,route_key,consumer,provider,protocol,recorded_at) VALUES (?1,?2,?3,?4,?5,?6)",
                    params![
                        format!("demo-v{version}"),
                        "api-to-search",
                        "api",
                        "search",
                        "http",
                        10_000 + version
                    ],
                )
                .unwrap();
            if version == 2 {
                connection
                    .execute(
                        "INSERT INTO route_snapshots(deployment_id,version,checksum,activation_status,recorded_at,context_json) VALUES (?1,?2,?3,?4,?5,?6)",
                        params![
                            format!("demo-v{version}"),
                            version,
                            format!("checksum-v{version}"),
                            "active",
                            11_000 + version,
                            format!(r#"{{"snapshot":"v{version}"}}"#)
                        ],
                    )
                    .unwrap();
            }
        }
        if version >= 3 {
            connection
                .execute(
                    "INSERT INTO route_snapshots(deployment_id,version,checksum,activation_status,recorded_at,context_json,router_id,binding_id,operation_id) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                    params![
                        format!("demo-v{version}"),
                        version,
                        format!("checksum-v{version}"),
                        "active",
                        11_000 + version,
                        format!(r#"{{"snapshot":"v{version}"}}"#),
                        "sidecar:api",
                        "api",
                        format!("operation-v{version}")
                    ],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO router_bindings(deployment_id,router_id,binding_id,desired_version,desired_checksum,current_version,current_checksum,previous_version,previous_checksum,observed_version,observed_checksum,apply_status,transition_json,last_error_code,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
                    params![
                        format!("demo-v{version}"),
                        "sidecar:api",
                        "api",
                        version,
                        format!("checksum-v{version}"),
                        version,
                        format!("checksum-v{version}"),
                        version - 1,
                        format!("checksum-v{}", version - 1),
                        version,
                        format!("checksum-v{version}"),
                        "active",
                        r#"{"strategy":"close"}"#,
                        Option::<String>::None,
                        12_000 + version
                    ],
                )
                .unwrap();
        }
    }

    fn scalar<T: rusqlite::types::FromSql>(connection: &Connection, sql: &str) -> T {
        connection.query_row(sql, [], |row| row.get(0)).unwrap()
    }

    fn assert_consistent(connection: &Connection) {
        assert_eq!(scalar::<String>(connection, "PRAGMA integrity_check"), "ok");
        assert_eq!(scalar::<i64>(connection, "PRAGMA foreign_keys"), 1);
        assert_eq!(
            scalar::<i64>(connection, "SELECT COUNT(*) FROM pragma_foreign_key_check"),
            0
        );
    }

    fn assert_historical_values(connection: &Connection, version: i64) {
        assert_eq!(
            scalar::<String>(
                connection,
                "SELECT applied_snapshot_json FROM deployments ORDER BY id"
            ),
            format!(r#"{{"deployment":"demo-v{version}","schema":{version}}}"#)
        );
        assert_eq!(
            scalar::<String>(
                connection,
                "SELECT context_json FROM deployment_history ORDER BY sequence"
            ),
            format!(r#"{{"history":"v{version}"}}"#)
        );
        assert_eq!(
            scalar::<String>(
                connection,
                "SELECT id || ':' || status FROM operations ORDER BY id"
            ),
            format!("operation-v{version}:succeeded")
        );
        assert_eq!(
            scalar::<String>(
                connection,
                "SELECT runtime_id || ':' || name || ':' || resource_hash FROM resources ORDER BY sequence"
            ),
            format!("container-v{version}:api-v{version}:resource-v{version}")
        );
        assert_eq!(
            scalar::<String>(
                connection,
                "SELECT subject || ':' || health || ':' || readiness || ':' || context_json FROM health_observations ORDER BY sequence"
            ),
            format!(r#"api:healthy:ready:{{"health":"v{version}"}}"#)
        );
        assert_eq!(
            scalar::<String>(
                connection,
                "SELECT owner_instance || ':' || token FROM operation_locks ORDER BY deployment_id"
            ),
            format!("owner-v{version}:token-v{version}")
        );
        if version >= 2 {
            assert_eq!(
                scalar::<String>(
                    connection,
                    "SELECT route_key || ':' || consumer || ':' || provider || ':' || protocol FROM routes ORDER BY sequence"
                ),
                "api-to-search:api:search:http"
            );
            assert_eq!(
                scalar::<String>(
                    connection,
                    "SELECT checksum || ':' || activation_status || ':' || context_json FROM route_snapshots ORDER BY sequence"
                ),
                format!(r#"checksum-v{version}:active:{{"snapshot":"v{version}"}}"#)
            );
        }
        if version >= 3 {
            assert_eq!(
                scalar::<String>(
                    connection,
                    "SELECT router_id || ':' || binding_id || ':' || operation_id FROM route_snapshots ORDER BY sequence"
                ),
                format!("sidecar:api:api:operation-v{version}")
            );
            assert_eq!(
                scalar::<String>(
                    connection,
                    "SELECT router_id || ':' || binding_id || ':' || apply_status || ':' || transition_json FROM router_bindings ORDER BY router_id,binding_id"
                ),
                r#"sidecar:api:api:active:{"strategy":"close"}"#
            );
        }
    }

    fn open_with_test_migrations(
        path: &Path,
        migrations: &[(i64, &str)],
    ) -> Result<OpenReport, StateError> {
        let existed = path.exists();
        let mut connection = Connection::open(path)?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let current = current_schema_version(&connection)?;
        let pending = migrations
            .iter()
            .filter(|(version, _)| *version > current)
            .collect::<Vec<_>>();
        let backup_path = if existed && !pending.is_empty() {
            let backup = backup_path(path, current);
            backup_database(&connection, &backup)?;
            Some(backup)
        } else {
            None
        };
        let transaction = connection.transaction()?;
        let mut applied_migrations = Vec::new();
        for (version, sql) in pending {
            transaction.execute_batch(sql)?;
            transaction.execute(
                "INSERT INTO schema_versions(version, applied_at) VALUES (?1, 1)",
                [version],
            )?;
            applied_migrations.push(*version);
        }
        transaction.commit()?;
        Ok(OpenReport {
            applied_migrations,
            backup_path,
        })
    }

    #[test]
    fn device_validation_and_store_round_trip() {
        let temp = TempDir::new().unwrap();
        let (store, _) = open_temp(&temp);
        let mut device = RegisteredDevice {
            name: "build-host".into(),
            host: "dev.example.test".into(),
            port: 2222,
            user: "operator".into(),
            identity_file: Some(PathBuf::from("keys/device_ed25519")),
            created_at: 100,
            last_checked_at: None,
            last_check_status: DeviceCheckStatus::Never,
            last_check_detail: None,
        };
        store.register_device(&device).unwrap();
        assert_eq!(store.devices().unwrap(), vec![device.clone()]);
        assert_eq!(
            store.register_device(&device).unwrap_err().code(),
            "device_already_registered"
        );

        device.last_checked_at = Some(200);
        device.last_check_status = DeviceCheckStatus::Ok;
        device.last_check_detail = Some("SSH connection succeeded".into());
        assert_eq!(
            store
                .record_device_check(
                    "build-host",
                    200,
                    DeviceCheckStatus::Ok,
                    Some("SSH connection succeeded"),
                )
                .unwrap(),
            device
        );
        store.deregister_device("build-host").unwrap();
        assert!(store.devices().unwrap().is_empty());

        let invalid = RegisteredDevice {
            name: "bad".into(),
            host: "has whitespace".into(),
            port: 22,
            user: "operator".into(),
            identity_file: None,
            created_at: 1,
            last_checked_at: None,
            last_check_status: DeviceCheckStatus::Never,
            last_check_detail: None,
        };
        assert_eq!(
            store.register_device(&invalid).unwrap_err().code(),
            "invalid_device_host"
        );
        let mut invalid = invalid;
        invalid.host = "host.test".into();
        invalid.port = 0;
        assert_eq!(
            store.register_device(&invalid).unwrap_err().code(),
            "invalid_device_port"
        );
        invalid.port = 22;
        invalid.name.clear();
        assert_eq!(
            store.register_device(&invalid).unwrap_err().code(),
            "invalid_identifier"
        );

        invalid.name = "bad".into();
        invalid.user = "-oProxyCommand=payload".into();
        assert_eq!(
            store.register_device(&invalid).unwrap_err().code(),
            "invalid_device_user"
        );
        invalid.user = "operator@extra".into();
        assert_eq!(
            store.register_device(&invalid).unwrap_err().code(),
            "invalid_device_user"
        );
        invalid.user = "operator".into();
        invalid.host = "-bad.host".into();
        assert_eq!(
            store.register_device(&invalid).unwrap_err().code(),
            "invalid_device_host"
        );
    }

    #[test]
    fn imported_profile_round_trip_replace_and_remove() {
        let temp = TempDir::new().unwrap();
        let (store, _) = open_temp(&temp);
        let mut profile = ImportedProfile {
            name: "api".into(),
            source_name: "checkout".into(),
            source_commit: Some("abc123".into()),
            content_hash: "first".into(),
            definition_json: r#"{"services":{}}"#.into(),
            imported_at: 10,
        };
        store.record_imported_profile(&profile).unwrap();
        assert_eq!(
            store.imported_profile("api").unwrap(),
            Some(profile.clone())
        );
        assert_eq!(store.imported_profiles().unwrap(), vec![profile.clone()]);

        profile.content_hash = "second".into();
        profile.imported_at = 20;
        store.record_imported_profile(&profile).unwrap();
        assert_eq!(store.imported_profiles().unwrap(), vec![profile]);

        store.remove_imported_profile("api").unwrap();
        assert!(store.imported_profile("api").unwrap().is_none());
        assert_eq!(
            store.remove_imported_profile("api").unwrap_err().code(),
            "profile_not_imported"
        );
    }

    #[test]
    fn version_four_store_upgrades_to_current_schema() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("state-v4.sqlite3");
        historical_database(&path, 4);
        let (store, report) = StateStore::open(&path).unwrap();
        assert_eq!(report.applied_migrations, vec![5, 6]);
        assert!(report.backup_path.unwrap().is_file());
        assert_eq!(
            scalar::<i64>(&store.connection, "SELECT COUNT(*) FROM devices"),
            0
        );
    }

    #[test]
    fn migrations_are_ordered_and_existing_database_is_backed_up() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("state.sqlite3");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(MIGRATIONS[0].1).unwrap();
        connection
            .execute(
                "INSERT INTO schema_versions(version, applied_at) VALUES (1, 1)",
                [],
            )
            .unwrap();
        drop(connection);

        let (store, report) = StateStore::open(&path).unwrap();
        assert_eq!(report.applied_migrations, vec![2, 3, 4, 5, 6]);
        let backup = report.backup_path.unwrap();
        assert!(backup.is_file());
        let versions = store
            .connection
            .prepare("SELECT version FROM schema_versions ORDER BY version")
            .unwrap()
            .query_map([], |row| row.get::<_, i64>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(versions, vec![1, 2, 3, 4, 5, 6]);
        let backup_connection = Connection::open(backup).unwrap();
        assert_eq!(current_schema_version(&backup_connection).unwrap(), 1);
    }

    #[test]
    fn newer_schema_is_refused_without_a_backup() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("state.sqlite3");
        let (store, _) = StateStore::open(&path).unwrap();
        store
            .connection
            .execute(
                "INSERT INTO schema_versions(version, applied_at) VALUES (99, 1)",
                [],
            )
            .unwrap();
        drop(store);

        let error = StateStore::open(&path).err().unwrap();
        assert_eq!(error.code(), "newer_schema");
        assert!(!backup_path(&path, 99).exists());
    }

    #[test]
    fn historical_schema_versions_migrate_with_backups_and_preserve_rows() {
        for version in 1..SCHEMA_VERSION {
            let temp = TempDir::new().unwrap();
            let path = temp.path().join(format!("state-v{version}.sqlite3"));
            historical_database(&path, version);

            let (store, report) = StateStore::open(&path).unwrap();
            assert_eq!(
                report.applied_migrations,
                ((version + 1)..=SCHEMA_VERSION).collect::<Vec<_>>()
            );
            let backup = report
                .backup_path
                .expect("old database should be backed up");
            assert!(backup.is_file());

            assert_consistent(&store.connection);
            assert_eq!(
                scalar::<i64>(
                    &store.connection,
                    "SELECT MAX(version) FROM schema_versions"
                ),
                SCHEMA_VERSION
            );
            assert_historical_values(&store.connection, version);
            assert_eq!(
                scalar::<i64>(&store.connection, "SELECT COUNT(*) FROM registered_sources"),
                0
            );

            let backup_connection = Connection::open(backup).unwrap();
            assert_eq!(current_schema_version(&backup_connection).unwrap(), version);
            assert_historical_values(&backup_connection, version);
        }
    }

    #[test]
    fn failed_migration_backup_can_be_reopened_and_migrated_successfully() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("state.sqlite3");
        historical_database(&path, 2);
        let failing = [
            MIGRATIONS[0],
            MIGRATIONS[1],
            (
                3,
                "CREATE TABLE migration_failure(id INTEGER);\nSELECT missing_function();",
            ),
        ];

        let error = open_with_test_migrations(&path, &failing).unwrap_err();
        assert_eq!(error.code(), "state_sqlite");
        let backup = PathBuf::from(format!("{}.pre-migration-v2.bak", path.display()));
        assert!(backup.is_file());
        let original = Connection::open(&path).unwrap();
        assert_eq!(current_schema_version(&original).unwrap(), 2);
        assert_historical_values(&original, 2);
        drop(original);

        let restored = temp.path().join("restored-from-backup.sqlite3");
        fs::copy(&backup, &restored).unwrap();
        let (store, report) = StateStore::open(&restored).unwrap();
        assert_eq!(report.applied_migrations, vec![3, 4, 5, 6]);
        assert_consistent(&store.connection);
        assert_historical_values(&store.connection, 2);
    }

    #[test]
    fn applied_snapshot_and_hash_round_trip() {
        let temp = TempDir::new().unwrap();
        let (mut store, _) = open_temp(&temp);
        let snapshot = AppliedSnapshot::from_json(json!({
            "deployment": "demo",
            "databasePassword": { "environmentVariable": "DEMO_DB_PASSWORD" }
        }))
        .unwrap();
        store
            .record_applied_snapshot("demo", "definition-a", &snapshot, 123)
            .unwrap();
        assert_eq!(
            store.applied_snapshot("demo").unwrap().unwrap(),
            AppliedDeployment {
                deployment: "demo".into(),
                definition_hash: "definition-a".into(),
                snapshot,
                applied_at: 123,
            }
        );
    }

    #[test]
    fn operation_locks_contend_heartbeat_expire_and_recover_safely() {
        let temp = TempDir::new().unwrap();
        let (mut store, _) = open_temp(&temp);
        let mut first = store
            .acquire_lock(&lock_request("daemon-a", "token-a", 100))
            .unwrap();
        assert!(!first.recovered);
        assert_eq!(
            store
                .acquire_lock(&lock_request("daemon-b", "token-b", 120))
                .unwrap_err()
                .code(),
            "operation_lock_contended"
        );
        store.heartbeat_lock(&mut first, 140, 50).unwrap();
        assert_eq!(first.expires_at, 190);
        assert!(
            store
                .acquire_lock(&lock_request("daemon-b", "token-b", 189))
                .is_err()
        );
        let recovered = store
            .acquire_lock(&lock_request("daemon-b", "token-b", 190))
            .unwrap();
        assert!(recovered.recovered);
        assert_eq!(
            store.release_lock(first).unwrap_err().code(),
            "operation_lock_lost"
        );
        store.release_lock(recovered).unwrap();
    }

    #[test]
    fn secret_values_are_rejected_and_only_references_are_retained() {
        assert_eq!(
            AppliedSnapshot::from_json(json!({ "apiToken": "literal-secret" }))
                .unwrap_err()
                .code(),
            "secret_value_forbidden"
        );
        assert!(
            AppliedSnapshot::from_json(json!({
                "apiToken": { "environmentVariable": "API_TOKEN" },
                "password": { "file": "/run/secrets/password" }
            }))
            .is_ok()
        );
        assert!(SecretReference::environment("API_TOKEN").is_ok());
        assert!(SecretReference::environment("literal-secret").is_err());
        assert_eq!(
            StructuredContext::new(json!({ "password": "oops" }))
                .unwrap_err()
                .code(),
            "secret_value_forbidden"
        );

        let temp = TempDir::new().unwrap();
        let (mut store, _) = open_temp(&temp);
        let mut observed = resource(Some("resource-a"));
        observed
            .labels
            .insert("example.secret".into(), "must-not-persist".into());
        store
            .reconcile(
                &ReconciliationInput {
                    manifests: vec![manifest("definition-a", "resource-a")],
                    resources: vec![observed],
                    unreachable_devices: BTreeSet::new(),
                },
                1,
            )
            .unwrap();
        let labels: String = store
            .connection
            .query_row("SELECT labels_json FROM resources", [], |row| row.get(0))
            .unwrap();
        assert!(!labels.contains("must-not-persist"));
    }

    #[test]
    fn reconciliation_compares_all_three_sources_and_updates_observations() {
        let temp = TempDir::new().unwrap();
        let (mut store, _) = open_temp(&temp);
        let snapshot = AppliedSnapshot::from_json(json!({ "deployment": "demo" })).unwrap();
        store
            .record_applied_snapshot("demo", "applied-hash", &snapshot, 1)
            .unwrap();
        let report = store
            .reconcile(
                &ReconciliationInput {
                    manifests: vec![manifest("desired-hash", "resource-a")],
                    resources: vec![resource(Some("resource-b"))],
                    unreachable_devices: BTreeSet::new(),
                },
                2,
            )
            .unwrap();
        let codes = report.deployments[0]
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect::<BTreeSet<_>>();
        assert!(codes.contains(&DriftCode::DesiredAppliedHashMismatch));
        assert!(codes.contains(&DriftCode::ObservedResourceHashMismatch));
        assert_eq!(store.active_resources("demo").unwrap().len(), 1);
    }

    #[test]
    fn unreachable_device_retains_previous_resources_and_reports_explicitly() {
        let temp = TempDir::new().unwrap();
        let (mut store, _) = open_temp(&temp);
        let mut generated = manifest("definition-a", "resource-a");
        generated.remote_projects.insert(
            "builder".into(),
            GeneratedRemoteProject {
                device: GeneratedDevice {
                    user: "akhil".into(),
                    host: "example-host".into(),
                    port: 22,
                    identity_file: None,
                },
                compose_project: "sy--demo-builder".into(),
                compose_file: "compose.builder.yaml".into(),
                services: vec!["demo--provider--api".into()],
            },
        );
        let mut remote = resource(Some("resource-a"));
        remote.device = "builder".into();
        remote.labels.insert(DEVICE_LABEL.into(), "builder".into());
        store
            .reconcile(
                &ReconciliationInput {
                    manifests: vec![generated.clone()],
                    resources: vec![remote],
                    unreachable_devices: BTreeSet::new(),
                },
                1,
            )
            .unwrap();
        let report = store
            .reconcile(
                &ReconciliationInput {
                    manifests: vec![generated],
                    resources: Vec::new(),
                    unreachable_devices: BTreeSet::from(["builder".into()]),
                },
                2,
            )
            .unwrap();
        let active = store.active_resources("demo").unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].device, "builder");
        assert!(
            report.deployments[0]
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == DriftCode::DeviceUnreachable)
        );
        assert!(
            !report.deployments[0]
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == DriftCode::ObservedResourcesMissing)
        );
    }

    #[test]
    fn deleted_database_rebuilds_observed_state_without_inventing_applied_state() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("state.sqlite3");
        let (mut store, _) = StateStore::open(&path).unwrap();
        let snapshot = AppliedSnapshot::from_json(json!({ "deployment": "demo" })).unwrap();
        store
            .record_applied_snapshot("demo", "old-applied", &snapshot, 1)
            .unwrap();
        drop(store);
        fs::remove_file(&path).unwrap();

        let (mut rebuilt, _) = StateStore::open(&path).unwrap();
        let report = rebuilt
            .reconcile(
                &ReconciliationInput {
                    manifests: vec![manifest("manifest-only", "resource-a")],
                    resources: vec![resource(Some("resource-a"))],
                    unreachable_devices: BTreeSet::new(),
                },
                2,
            )
            .unwrap();
        assert!(rebuilt.applied_snapshot("demo").unwrap().is_none());
        assert_eq!(rebuilt.active_resources("demo").unwrap().len(), 1);
        assert_eq!(
            report.deployments[0].diagnostics[0].code,
            DriftCode::AppliedStateMissing
        );
    }

    #[test]
    fn drift_codes_cover_missing_manifest_resources_hash_and_ownership() {
        let temp = TempDir::new().unwrap();
        let (mut store, _) = open_temp(&temp);
        let snapshot = AppliedSnapshot::from_json(json!({ "deployment": "demo" })).unwrap();
        store
            .record_applied_snapshot("demo", "applied", &snapshot, 1)
            .unwrap();
        let missing = store.reconcile(&ReconciliationInput::default(), 2).unwrap();
        let missing_codes = missing.deployments[0]
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect::<BTreeSet<_>>();
        assert!(missing_codes.contains(&DriftCode::GeneratedManifestMissing));
        assert!(missing_codes.contains(&DriftCode::ObservedResourcesMissing));

        let mut invalid = resource(None);
        invalid.labels.insert(MANAGED_LABEL.into(), "false".into());
        let drifted = store
            .reconcile(
                &ReconciliationInput {
                    manifests: vec![manifest("applied", "resource-a")],
                    resources: vec![invalid],
                    unreachable_devices: BTreeSet::new(),
                },
                3,
            )
            .unwrap();
        let codes = drifted.deployments[0]
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect::<BTreeSet<_>>();
        assert!(codes.contains(&DriftCode::OwnershipInvalid));
        assert!(codes.contains(&DriftCode::ObservedResourceHashMissing));
    }

    #[test]
    fn history_record_apis_accept_structured_records() {
        let temp = TempDir::new().unwrap();
        let (store, _) = open_temp(&temp);
        let context = StructuredContext::new(json!({ "reason": "test" })).unwrap();
        store
            .start_operation(&OperationRecord {
                id: "operation-1".into(),
                deployment: "demo".into(),
                kind: OperationKind::Build,
                status: OperationStatus::Running,
                started_at: 1,
                finished_at: None,
                error: None,
            })
            .unwrap();
        store
            .update_operation("operation-1", OperationStatus::Succeeded, Some(2), None)
            .unwrap();
        store
            .record_health(&HealthObservation {
                deployment: "demo".into(),
                subject: "api".into(),
                health: "healthy".into(),
                readiness: "ready".into(),
                observed_at: 2,
                context: context.clone(),
            })
            .unwrap();
        store
            .record_route(&RouteRecord {
                deployment: "demo".into(),
                route_key: "api-to-db".into(),
                consumer: "api".into(),
                provider: "db".into(),
                protocol: "tcp".into(),
                recorded_at: 2,
            })
            .unwrap();
        store
            .record_route_snapshot(&RouteSnapshotRecord {
                deployment: "demo".into(),
                router: Some("sidecar:api".into()),
                binding: Some("api".into()),
                operation_id: Some("operation-1".into()),
                version: 1,
                checksum: "checksum".into(),
                activation_status: ActivationStatus::Active,
                recorded_at: 2,
                context,
            })
            .unwrap();
        for table in [
            "operations",
            "health_observations",
            "routes",
            "route_snapshots",
        ] {
            let count: i64 = store
                .connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 1, "missing history in {table}");
        }
    }

    #[test]
    fn router_acknowledgement_gates_current_version_and_preserves_history() {
        let temp = TempDir::new().unwrap();
        let (mut store, _) = open_temp(&temp);
        let transition =
            StructuredContext::new(json!({"strategy": "drain", "timeoutMs": 10})).unwrap();
        let context = StructuredContext::new(json!({"reason": "test"})).unwrap();
        let record = |status: RouterApplyStatus,
                      version: i64,
                      checksum: &str,
                      observed: Option<(i64, &str)>| RouterApplyRecord {
            deployment: "demo".into(),
            router: "sidecar:api".into(),
            binding: "api".into(),
            operation_id: format!("op-{version}"),
            desired_version: version,
            desired_checksum: checksum.into(),
            version,
            checksum: checksum.into(),
            status,
            observed_version: observed.map(|(version, _)| version),
            observed_checksum: observed.map(|(_, checksum)| checksum.into()),
            transition: transition.clone(),
            error_code: (status == RouterApplyStatus::Failed).then(|| "timeout".into()),
            recorded_at: version,
            context: context.clone(),
        };
        store
            .record_router_apply(&record(
                RouterApplyStatus::Active,
                2,
                "two",
                Some((1, "one")),
            ))
            .unwrap();
        let active = store.router_bindings("demo").unwrap().pop().unwrap();
        assert_eq!(active.previous_version, Some(1));
        assert_eq!(active.observed_version, Some(2));
        store
            .record_router_apply(&record(
                RouterApplyStatus::Failed,
                3,
                "three",
                Some((2, "two")),
            ))
            .unwrap();
        let state = store.router_bindings("demo").unwrap().pop().unwrap();
        assert_eq!(state.desired_version, Some(3));
        assert_eq!(state.current_version, Some(2));
        assert_eq!(state.observed_version, Some(2));
        assert_eq!(state.status, "failed");
        let rollback = RouterApplyRecord {
            deployment: "demo".into(),
            router: "sidecar:api".into(),
            binding: "api".into(),
            operation_id: "op-rollback".into(),
            desired_version: 3,
            desired_checksum: "three".into(),
            version: 4,
            checksum: "one-rerendered".into(),
            status: RouterApplyStatus::RolledBack,
            observed_version: Some(2),
            observed_checksum: Some("two".into()),
            transition,
            error_code: None,
            recorded_at: 4,
            context,
        };
        store.record_router_apply(&rollback).unwrap();
        let rolled_back = store.router_bindings("demo").unwrap().pop().unwrap();
        assert_eq!(rolled_back.desired_version, Some(3));
        assert_eq!(rolled_back.current_version, Some(4));
        assert_eq!(rolled_back.previous_version, Some(2));
        assert_eq!(rolled_back.status, "rolled_back");
        assert_eq!(store.route_history("demo").unwrap().len(), 3);
    }
}
