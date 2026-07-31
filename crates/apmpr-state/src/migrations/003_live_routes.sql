ALTER TABLE route_snapshots ADD COLUMN router_id TEXT;
ALTER TABLE route_snapshots ADD COLUMN binding_id TEXT;
ALTER TABLE route_snapshots ADD COLUMN operation_id TEXT;

CREATE INDEX route_snapshots_deployment_router
ON route_snapshots(deployment_id, router_id, binding_id, sequence);

CREATE TABLE router_bindings (
    deployment_id TEXT NOT NULL,
    router_id TEXT NOT NULL,
    binding_id TEXT NOT NULL,
    desired_version INTEGER,
    desired_checksum TEXT,
    current_version INTEGER,
    current_checksum TEXT,
    previous_version INTEGER,
    previous_checksum TEXT,
    observed_version INTEGER,
    observed_checksum TEXT,
    apply_status TEXT NOT NULL CHECK(apply_status IN ('pending','active','failed','rolled_back')),
    transition_json TEXT NOT NULL CHECK(json_valid(transition_json)),
    last_error_code TEXT,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(deployment_id, router_id, binding_id)
) STRICT;
