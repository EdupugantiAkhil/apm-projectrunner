# Support and deprecation policy

Switchyard is pre-1.0. Its alpha schemas can change in a minor release, but a supported
format or API is never changed silently. This policy applies to released builds; a build
from an untagged development commit is supported only with artifacts produced by that
same commit.

## Supported clients

The CLI and Ratatui TUI are the command-line and primary interactive control planes. The
React GUI remains a supported secondary client for deployment monitoring and operations,
including status, topology, route operations, logs, and lifecycle controls.

Source-local startup-profile authoring, the guided instance-creation wizard, the
connections matrix, and device placement are TUI-only. No GUI delivery schedule or
authoring-parity promise applies to those workflows. Adding that parity requires a new,
separately approved milestone; absence of those workflows from the GUI is not a client
deprecation.

The GUI's existing operational route table, topology visualization, and complete
binding changes remain in scope. They do not imply support for the new Connections
authoring workflow.

## What “not silently” means

Every compatibility-affecting change must include all of the following in the same
reviewed commit:

1. a changelog entry that names the affected version string and user-visible impact;
2. a dedicated **Compatibility and deprecations** section in the release notes, including
   migration or rollback instructions;
3. the deliberate compatibility-golden change, addition, or removal that makes CI prove
   the intended boundary.

Changing a parser or serializer first and updating a golden later is not an acceptable
release process. Emergency security removals may shorten a support window, but still
require all three records plus the security reason and safest available recovery path.

## Configuration and state schemas

The currently published schemas include:

- `switchyard.dev/v1alpha1` for `Deployment` and `Overlay` documents;
- `switchyard.dev/router/v1alpha1` for router snapshots;
- `switchyard.dev/bundle/v1alpha1` for portable bundles;
- local state/artifact formats including `switchyard.dev/host-process/v1alpha1`,
  `switchyard.dev/mdns-publication/v1alpha1`,
  `switchyard.dev/tailscale-publication/v1alpha1`,
  `switchyard.dev/managed-profile/v1alpha1`, and
  `switchyard.dev/diagnostics/v1alpha1`.

`alpha` means the shape is usable and tested but not stable enough for a 1.0 promise.
Fields may be added, renamed, or removed in a minor release. Alpha does **not** mean an
existing version string may be reinterpreted incompatibly.

Within one exact version string, Switchyard promises:

- previously valid documents continue to parse and retain their field meanings;
- omitted optional fields retain compatible defaults;
- serializers remain deterministic where artifacts are defined as deterministic;
- a document is not silently accepted with a materially different meaning; and
- unknown/new required behavior is not backported under the old string.

The deployment goldens in
[`crates/switchyard-planner/tests/compat.rs`](../crates/switchyard-planner/tests/compat.rs)
pin accepted definitions, definition/resource hashes, route counts, and deterministic
generation. Router compatibility is pinned by
[`crates/router-config/tests/contracts.rs`](../crates/router-config/tests/contracts.rs),
including the minimal/defaulted document and LAN host-router golden. Portable bundles
also carry a content hash and reject an unsupported API version with a stable error code.

A breaking schema change requires a new version string (for example `v1alpha2` or
`v1beta1`), a parallel parser during migration, migration notes with before/after
examples, and new goldens. Existing goldens are regenerated only after reviewing both
the source fixture and all derived hash/artifact changes.

When a new configuration or state-file version supersedes an old one, the previous
version remains readable for at least one subsequent minor release and at least 90 days
after the replacement release, whichever is longer. During that window Switchyard may
write only the new version after an explicit migration. Removal must be announced in the
first replacement release and again in the removal release. Portable bundles get the
same parsing window so teams can exchange bundles across a rolling upgrade.

Ephemeral state files are not interchange formats: users should not hand-edit or copy
host process, publication, managed-profile, or diagnostics files between machines.
Nevertheless, a version change follows the same notice and parsing window. If safe
migration is impossible, the old file must be rejected with an actionable cleanup or
regeneration instruction; it must not be misread as the new shape.

## HTTP API

The daemon contract is `/api/v1`, with response `apiVersion: "v1"`, defined in
[`contract.rs`](../crates/switchyard-daemon/src/contract.rs) and documented in
[`control-plane-api.md`](control-plane-api.md).

Within v1, compatible changes may add endpoints, optional request fields, optional
response fields, event data, and error codes. Clients must ignore unknown response fields
and unknown event data. Switchyard will not remove or rename an endpoint/field, add a new
required request field, narrow a previously valid value, change an existing enum value's
meaning, or change success/error semantics incompatibly within v1.

An incompatible change requires `/api/v2`. The release that introduces v2 must:

- keep v1 available for at least one subsequent minor release and at least 90 days,
  whichever is longer;
- mark v1 deprecated in release notes and API documentation, with the planned earliest
  removal release/date;
- provide migration notes and, where practical, response-level deprecation signaling;
- migrate the bundled CLI and GUI before v1 removal; and
- test v1 and v2 independently throughout the overlap.

The supported CLI/daemon pairing is the CLI and daemon from the same Switchyard release.
Because v1 permits additive responses, an older v1 client should tolerate a newer v1
daemon, but cross-minor skew is not a release guarantee until that exact pair appears in
compatibility CI. A discovery/API version mismatch must fail with an actionable error;
it must not fall through to a differently interpreted contract. During a v1-to-v2
transition, the bundled CLI must negotiate/use a version supported by both sides or tell
the user which component to upgrade.

## SQLite schema

SQLite uses forward-only, ordered, embedded migrations. The current implementation and
schema number are in
[`switchyard-state/src/lib.rs`](../crates/switchyard-state/src/lib.rs#L59-L73).

Before pending migrations modify an existing database, Switchyard creates a consistent
side-by-side backup named from the old schema version. Migrations run in order in one
transaction and each version is recorded only after its SQL succeeds. This is proved by
`migrations_are_ordered_and_existing_database_is_backed_up` and the all-historical-
versions test `historical_schema_versions_migrate_with_backups_and_preserve_rows`.

A binary refuses a database whose recorded schema is newer than it supports, without
modifying it or creating a misleading backup. Test:
`newer_schema_is_refused_without_a_backup`.

There are no reverse migrations. Downgrade is supported only by stopping Switchyard and
restoring the appropriate pre-migration backup, as documented in
[`upgrade-recovery.md`](upgrade-recovery.md). The restored database represents the
migration boundary; generated manifests and Docker resources may be newer and must be
reviewed as drift before further mutation.

Each new migration must be appended with the next integer, preserve every supported
historical migration test, add a fixture/assertion for the new version, and document any
irreversible data transformation in release notes. Existing migration files are
immutable after release; a correction is a new migration.

## Deprecation lifecycle

1. **Announce:** identify the surface/version, replacement, migration, earliest removal
   release/date, and security implications in changelog and release notes.
2. **Overlap:** parse/serve both versions for the applicable one-minor/90-day minimum;
   keep compatibility goldens and API tests for both.
3. **Warn:** emit an actionable warning when the old surface is read or used, without
   placing secrets or full documents in the warning.
4. **Remove:** delete the old parser/endpoint only in the announced release, update the
   same-commit goldens, and retain migration/recovery documentation.

Before 1.0, this process favors honesty over indefinite compatibility: alpha users may
need to migrate at a minor release, but they receive a distinct version, an overlap
window, deterministic test evidence, and explicit recovery instructions.
