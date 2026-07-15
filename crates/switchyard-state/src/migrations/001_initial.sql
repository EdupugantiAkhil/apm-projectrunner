CREATE TABLE schema_versions (
    version INTEGER PRIMARY KEY,
    applied_at INTEGER NOT NULL
) STRICT;

CREATE TABLE deployments (
    id TEXT PRIMARY KEY NOT NULL CHECK(length(id) > 0),
    applied_definition_hash TEXT,
    applied_snapshot_json TEXT CHECK(applied_snapshot_json IS NULL OR json_valid(applied_snapshot_json)),
    applied_at INTEGER,
    last_observed_at INTEGER,
    CHECK((applied_definition_hash IS NULL) = (applied_snapshot_json IS NULL)),
    CHECK((applied_definition_hash IS NULL) = (applied_at IS NULL))
) STRICT;

CREATE TABLE deployment_history (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    deployment_id TEXT NOT NULL REFERENCES deployments(id),
    event TEXT NOT NULL,
    definition_hash TEXT,
    recorded_at INTEGER NOT NULL,
    context_json TEXT CHECK(context_json IS NULL OR json_valid(context_json))
) STRICT;

CREATE TABLE operations (
    id TEXT PRIMARY KEY NOT NULL,
    deployment_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('pending','running','succeeded','failed','cancelled')),
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    error_code TEXT,
    error_context_json TEXT CHECK(error_context_json IS NULL OR json_valid(error_context_json))
) STRICT;

CREATE TABLE resources (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    deployment_id TEXT NOT NULL REFERENCES deployments(id),
    kind TEXT NOT NULL CHECK(kind IN ('container','image','network','volume')),
    runtime_id TEXT NOT NULL,
    name TEXT NOT NULL,
    resource_hash TEXT,
    state TEXT,
    labels_json TEXT NOT NULL CHECK(json_valid(labels_json)),
    observed_at INTEGER NOT NULL,
    active INTEGER NOT NULL CHECK(active IN (0,1))
) STRICT;
CREATE INDEX resources_deployment_active ON resources(deployment_id, active);

CREATE TABLE health_observations (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    deployment_id TEXT NOT NULL,
    subject TEXT NOT NULL,
    health TEXT NOT NULL,
    readiness TEXT NOT NULL,
    observed_at INTEGER NOT NULL,
    context_json TEXT NOT NULL CHECK(json_valid(context_json))
) STRICT;

CREATE TABLE operation_locks (
    deployment_id TEXT PRIMARY KEY NOT NULL,
    owner_instance TEXT NOT NULL,
    owner_pid INTEGER NOT NULL,
    owner_started_at INTEGER NOT NULL,
    token TEXT NOT NULL,
    heartbeat_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL CHECK(expires_at > heartbeat_at)
) STRICT;
