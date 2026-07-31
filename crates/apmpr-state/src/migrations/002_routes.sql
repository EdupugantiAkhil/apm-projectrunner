CREATE TABLE routes (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    deployment_id TEXT NOT NULL,
    route_key TEXT NOT NULL,
    consumer TEXT NOT NULL,
    provider TEXT NOT NULL,
    protocol TEXT NOT NULL,
    recorded_at INTEGER NOT NULL
) STRICT;

CREATE TABLE route_snapshots (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    deployment_id TEXT NOT NULL,
    version INTEGER NOT NULL,
    checksum TEXT NOT NULL,
    activation_status TEXT NOT NULL CHECK(activation_status IN ('pending','active','rejected','rolled_back')),
    recorded_at INTEGER NOT NULL,
    context_json TEXT NOT NULL CHECK(json_valid(context_json)),
    UNIQUE(deployment_id, version, checksum, activation_status, recorded_at)
) STRICT;
