# Upgrade and recovery

Switchyard's durable project state lives at `.switchyard/state.sqlite3`. Generated
recovery manifests live below `.switchyard/generated/`, while runtime resources carry
Switchyard ownership and resource-hash labels. Stop the daemon before copying or
replacing the database.

## Upgrade binaries and schema

The current SQLite state schema is v7. Schema v6 added reviewed source-profile import
state, and schema v7 added remote-device eligibility observations. Existing databases
migrate forward through both versions in order with the same pre-migration backup
behavior described below.

1. Stop active control-plane work, then run `switchyard daemon stop`. Confirm
   `switchyard daemon status` reports no reachable daemon. Running application
   containers may remain up.
2. Replace the `switchyard`, `switchyard-daemon`, and `switchyard-router` binaries as one
   compatible set.
3. Start with `switchyard daemon run`. Opening the state store applies every pending
   embedded SQLite migration in order inside one transaction.
4. If the database already exists and has pending migrations, Switchyard first makes a
   consistent SQLite backup beside it. Its first name is
   `.switchyard/state.sqlite3.pre-migration-vN.bak`, where `N` is the old schema
   version. Existing backups are never overwritten; a numeric suffix is selected.
5. Inspect `switchyard daemon status`, deployment detail/status, route versions, and
   reconciliation diagnostics before applying changes.

`crates/switchyard-state/src/lib.rs::migrations_are_ordered_and_existing_database_is_backed_up`
proves ordered migration, backup creation, current-schema advancement, and that the
backup still has the old schema. A brand-new database needs no pre-migration backup.

If an older binary opens a database whose recorded schema is newer than it supports,
startup fails with stable code `newer_schema`. It does not modify the database and does
not create a misleading backup; this is proved by
`crates/switchyard-state/src/lib.rs::newer_schema_is_refused_without_a_backup`.

### Downgrade by restoring the backup

There is no reverse migration. Use the backup created immediately before the upgrade:

1. Stop the daemon and keep the newer binaries from restarting automatically.
2. Preserve the rejected/current file for diagnosis:
   `mv .switchyard/state.sqlite3 .switchyard/state.sqlite3.after-upgrade`.
3. Copy, do not move, the appropriate backup into place:
   `cp .switchyard/state.sqlite3.pre-migration-vN.bak .switchyard/state.sqlite3`.
4. Install the binary set that understands schema `N`, start the daemon, and inspect
   reconciliation before mutation. Do not restore only selected tables or edit
   `schema_versions` by hand.

The restored database represents state at the migration boundary. Containers and
generated manifests may be newer, so drift is expected and must be reviewed. Keep the
post-upgrade database and original backup until recovery is accepted.

Remote-device deployments need the same review on every registered device. Their
containers carry Switchyard ownership, deployment, resource-hash, and device labels,
and remain discoverable by querying the corresponding remote Docker daemon. If a
device is unreachable during upgrade or recovery, retain its ownership records, restore
access, and rerun status/reconciliation; do not assume its resources are absent or
recreate them elsewhere.

## Recovery scenarios

### Daemon restart

Restart the daemon normally. Applied desired snapshots, terminal operation status,
custom domains, bindings, route versions, and route activation history remain in
SQLite. In-flight operations abandoned by a crashed daemon are recovered as failed
with `daemon_restarted`; raw stdout/stderr and in-memory SSE buffers are intentionally
not durable.

Evidence:

- `crates/switchyard-daemon/tests/api.rs::restart_keeps_final_operation_state_in_sqlite`
- `crates/switchyard-daemon/tests/api.rs::applied_domains_bindings_and_deleted_database_recovery_survive_daemon_restart`
- `crates/switchyard-daemon/tests/api.rs::failed_live_binding_versions_and_rollback_history_survive_restart`

After restart, run `switchyard status <deployment> --routes` and verify the applied
hash, domains/bindings, current/desired route versions, and diagnostics before applying.

### SQLite deleted or an older copy restored

Stop the daemon, restore the chosen file (or leave it absent), then restart. Startup
loads generated manifests and best-effort Docker ownership-label observations. It
records observed resources and reports drift; it never promotes a manifest to applied
desired state and never invents a successful apply. A deleted database should report
`applied_state_missing` for generated deployments.

Evidence:

- daemon-level manifest recovery:
  `applied_domains_bindings_and_deleted_database_recovery_survive_daemon_restart`
- state-level manifest plus injected Docker-label recovery:
  `crates/switchyard-state/src/lib.rs::deleted_database_rebuilds_observed_state_without_inventing_applied_state`
- three-source comparison:
  `crates/switchyard-state/src/lib.rs::reconciliation_compares_all_three_sources_and_updates_observations`

Do not run `up` merely to silence the diagnostic. First compare the authored definition,
generated `manifest.json`, Docker resources/labels, and any retained database copy;
then deliberately apply only if the generated desired state is still correct.

### Manifest/runtime drift

Run `switchyard status <deployment> --routes` and inspect the GUI deployment detail.
Reconciliation has stable diagnostics for a missing generated manifest, missing
runtime resources, desired/applied hash mismatch, observed resource hash missing or
mismatched, and invalid ownership. The coverage is
`crates/switchyard-state/src/lib.rs::drift_codes_cover_missing_manifest_resources_hash_and_ownership`
and `reconciliation_compares_all_three_sources_and_updates_observations`;
`crates/switchyard-daemon/tests/api.rs::deployment_list_and_detail_include_applied_manifest_and_reconciliation`
proves API exposure.

Treat ownership errors as a stop condition. Do not relabel or delete an ambiguous
resource. For definition/runtime drift, regenerate a plan, compare it with the last
applied snapshot and manifest, and use an explicit `up` only after accepting the
mutation preview.

### Router crash and restart

For a consumer sidecar, Docker's restart policy restarts the router while the
application container and route configuration remain owned by the deployment. For the
native host gateway, rerun `switchyard up <deployment>`; host lifecycle recovery detects
the dead owned process and recreates it from generated configuration and current Docker
publications. Then check the stable UI hostname and `switchyard status --routes`.

`examples/routing-matrix/smoke.sh` kills the backend-1 sidecar PID 1, waits for routing
to recover, kills the native host-gateway PID, reruns `up`, and verifies browser
routing. It also checks provider/application crash containment and health-gated route
rollback.

### Docker or Compose restart

After Docker returns, run `docker info` and `docker compose version`, then run
`switchyard status <deployment> --routes`. If containers have not returned, use
`switchyard up <deployment>`; readiness waits for health and the host gateway refreshes
changed ephemeral publications. Do not use cleanup as a recovery step.

`examples/routing-matrix/smoke.sh` runs `docker compose restart`, then Switchyard
down/up, and verifies the same service selection plus a persisted request counter.
This is a Compose-wide runtime restart proof. It does not reboot the Docker daemon
process itself; a literal Engine/service restart remains a host-level manual check.

## Data safety: down versus cleanup

`switchyard down <deployment>` performs ownership checks and Compose down without
`--volumes`; named database volumes remain. `switchyard cleanup <deployment> --yes` is
the only normal deployment command that passes `--volumes`, and it refuses to run
without explicit confirmation and ownership verification. The GUI separately requires
typing the deployment name before either action and sends `confirmed: true` only for
cleanup.

Evidence:

- `crates/switchyard-cli/src/runtime.rs::down_does_not_delete_volumes`
- `crates/switchyard-cli/src/cli.rs::rejects_volume_deletion_through_down`
- `crates/switchyard-cli/src/runtime.rs::destructive_cleanup_requires_confirmation`
- `crates/switchyard-cli/src/runtime.rs::ownership_rejects_a_resource_without_managed_label`
- `examples/jas-base/smoke.sh`: down/up preserves initialized database contents, then
  explicit cleanup and assertions show zero owned containers, volumes, and networks
- `examples/routing-matrix/smoke.sh`: down/up retains its volume-backed request counter

For routine stopping, use only `down`. Before destructive cleanup, back up application
data by application-specific means, review the plan and ownership labels, and type/run
the explicit confirmation only when permanent volume deletion is intended.
