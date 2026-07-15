# Switchyard implementation progress

Updated: 2026-07-15

## Release status

- Routing proof (Phases 0–4): complete.
- Product MVP (Phases 5–6): Phase 5 complete; Phase 6 not started.
- Team release (Phase 7): not started.

`IMPLEMENTATION_PLAN.md` remains the task-level checklist. This file records the
implemented shape and the evidence used to close a phase.

## Phase 4 implementation

- The planned routing-matrix contains three independently sourced UIs, two
  independently sourced backends, two five-service groups, and a shared audit provider.
- UI custom domains and fixed `localhost:10081` browser routing run through the native
  gateway; backend fixed ports `8001`–`8005` run through namespace-sharing sidecars.
- `uiRoutes` cross-checks Origin-to-backend routing, backend bindings, and downstream
  group expectations. Conflicts fail with `BackendGroupInvariant` and duplication
  guidance. `bind` updates all attached UI expectations with the backend group.
- Candidate snapshots are provider-health-gated. An unhealthy candidate returns a
  rollback diagnostic and leaves the active version unchanged.
- Provider DNS is resolved before Pingora peer construction, and health probes are
  task-isolated so an upstream resolution failure cannot take down a router worker.
- Generated long-running Compose services use `restart: unless-stopped`. The host
  runtime detects changed ephemeral Docker publications and refreshes its owned gateway.
- `examples/routing-matrix/smoke.sh` covers live UI/group switching, complete snapshot
  observations, rollback, delayed readiness, provider/router/application/host crashes,
  Docker/Compose recovery, custom domains, fixed addresses, and volume persistence.
- `scripts/phase4-proof.sh` is the clean-checkout release command; CI runs it on Linux
  `x86_64`, and it was run locally on Linux `aarch64`.

## Verification

- `cargo test -p switchyard-cli -p switchyard-planner --all-features`: passed.
- `cargo test -p router-pingora --test http_proxy --all-features`: passed.
- `cargo test --workspace --all-features`: passed, including router health rollback,
  DNS containment, protocol, transition, and shutdown coverage.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: passed.
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps`: passed.
- `./scripts/phase4-proof.sh`: passed as the final one-command release check.
- `examples/routing-matrix/smoke.sh`: passed on Linux `aarch64` with Docker Engine
  29.5.2 and Docker Compose 5.1.4; its cleanup left zero owned containers and volumes.
- Rust formatting was checked with the available Nix-provided Rust 1.95 `rustfmt`; the
  shell's `cargo-fmt` shim could not launch because its dynamic loader is absent.

## Phase 5 implementation

### SQLite state

- `switchyard-state` is a synchronous, daemon-neutral library using bundled SQLite at
  an explicit caller-provided path; `.switchyard/state.sqlite3` is the documented
  per-project convention.
- Two ordered embedded migrations establish applied deployment snapshots, append-only
  deployment/operation/resource/health/route history, immutable route-snapshot
  activation records, and expiring operation leases. Existing databases receive a
  non-overwriting pre-migration file backup, and newer schemas are refused.
- Applied snapshots and structured contexts reject literal values in secret-bearing
  fields. The public secret type represents environment-variable and file references,
  and reconciliation retains only Switchyard ownership labels from Docker observations.
- Reconciliation compares generated manifest definition/resource hashes, nullable
  last-applied state, and injected Docker-label observations. It records observations
  without changing runtime resources or promoting recovered manifests to desired state.
  Stable drift codes cover missing, mismatched, multiply hashed, and invalidly owned
  state. A deleted or older restored database therefore recovers observations without
  inventing a successful apply.
- Focused offline evidence: 9 unit tests passed; isolated crate Clippy passed with
  `-D warnings`; isolated crate rustdoc passed with `RUSTDOCFLAGS=-D warnings`; and
  workspace formatting passed.
- The required repository-level state test, workspace test, workspace Clippy, and
  workspace rustdoc commands were attempted, but Cargo stopped before compilation
  because this shell could not resolve `index.crates.io` while fetching the pre-existing
  `bytes` dependency of `router-pingora`. They must be rerun in a network-enabled or
  fully vendored environment; this is an environment verification gap, not a recorded
  pass.

### Daemon and API

- `switchyard-daemon` provides a standalone binary and the developer-facing
  `switchyard daemon run/status/stop` group. It binds loopback only, runs migrations and
  startup reconciliation, writes an atomic mode-0600 discovery document, and cancels
  and joins active operations before graceful shutdown.
- Axum is the small HTTP routing layer on the existing Tokio runtime. Versioned serde
  contract types remain framework-neutral. Every endpoint is under `/api/v1`, uses
  stable JSON error codes, and requires a random project-local bearer credential.
- The subprocess backend reuses the exact one-shot CLI implementation with an internal
  recursion guard, preserving stdout, stderr, and exit codes. Secure discovery selects
  the daemon when reachable; absent or stale discovery retains the old one-shot path.
- Mutations use heartbeated `switchyard-state` deployment leases; apply work also uses a
  configurable global semaphore. Reads acquire neither. Cancellation, shutdown,
  subprocess completion, durable status updates, and lock release share a terminal path.
- Per-operation SSE publishes operation, build, health, route, and log events with
  monotonic IDs, retains 2,048 records, and replays records after `Last-Event-ID`.
  Status and structured errors survive restart in SQLite; raw command output and event
  buffers remain memory-only to avoid persisting possible application secrets.
- Phase 5 review hardening retains live-bind and rollback attempts across partial
  failures, cancels and joins blocking bind work after lease loss, bounds in-memory
  terminal operations to the most recent 64, waits through SSE with backed-off polling
  fallback, authenticates discovery peers with daemon status, and applies bearer
  authentication exactly once in router middleware.
- Docker-free tests cover auth, versioned-only routing, every SSE category and replay,
  mutation contention, global limiting, mid-operation cancellation, SQLite restart
  recovery, no-daemon fallback, and byte-identical API-backend CLI output. The production
  listener and Docker observation paths remain integration boundaries; this execution
  sandbox rejects socket creation with `EPERM`.
- Verification for this increment: `cargo test -p switchyard-daemon --all-features`
  passed (6 tests plus doc tests); the focused CLI fallback/API parity integration test
  passed; workspace Clippy with `-D warnings` passed; workspace rustdoc with
  `RUSTDOCFLAGS="-D warnings"` passed; and workspace formatting passed. The exact
  workspace test built successfully and passed every test reached before the first
  socket-based Pingora integration test (`grpc_h2c`) failed to bind with sandbox
  `EPERM`. An earlier isolated CLI run reached the same restriction in its pre-existing
  Unix-socket host-runtime test. This is the sole repository-test verification gap.

### Live router control

- Router administration is now a shared typed crate used by both the one-shot CLI and
  daemon. It retains the existing newline-delimited Unix-socket protocol, provides
  configurable timeouts, and decodes snapshot identities and activation
  acknowledgements without exposing credentials in errors.
- The real daemon backend owns binding changes. It plans from the last generated
  resolved state, pushes complete monotonic snapshots to the selected consumer sidecar
  and a running host gateway, and requires matching version, checksum, and `activated`
  status before recording success or replacing generated artifacts.
- Multi-router changes compensate for partial activation by reapplying the prior route
  configuration at a newer version. Timeouts, invalid/stale acknowledgements,
  provider-health rollback, compensation success, and compensation failure are stored
  as secret-safe route history and linked to the durable operation ID.
- SQLite schema version 3 adds per-router/binding desired, current, previous, and
  observed version/checksum state, transition policy, status, and last error code.
  `/api/v1/deployments/:deployment/routes` returns this state and append-only history;
  daemon-backed `status --routes` and `routes` append a compact version summary.
- Bind requests and `switchyard bind` accept additive close, drain (with timeout), and
  pin controls. The selected policy is applied consistently to HTTP, HTTPS, WebSocket,
  gRPC, and TCP fields in the router's existing transition contract.

### Phase 5 exit gate

- Successful daemon applies persist the resolved desired snapshot and definition hash.
  A transport-independent restart test proves custom domains and bindings remain in
  SQLite, while a live-binding test proves failed and rolled-back route history and all
  visible versions survive daemon reconstruction.
- The same recovery test deletes SQLite and verifies startup rediscovers the generated
  routing-matrix manifest with `applied_state_missing` drift instead of inventing an
  apply. State-layer coverage injects owned Docker-label observations and proves the
  same safe recovery path for runtime resources.
- CLI parsing, daemon request generation, no-daemon fallback, byte-compatible command
  output, additive route-version output, and the shared transition policy contract are
  automated. Existing command output remains unchanged before the additive version
  section.

## Phase 5 verification

- `cargo test -p switchyard-daemon --all-features --test api`: passed (8 tests),
  including restart, domain/binding persistence, route failure/rollback persistence,
  lock-loss cancellation with attempt persistence, bounded terminal retention, and
  deleted-database recovery.
- `cargo test -p switchyard-state -p switchyard-router-admin -p switchyard-daemon
  --all-features --no-fail-fast`: passed (state, shared client, daemon, integration, and
  doc tests).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: passed.
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps`: passed.
- `cargo fmt --all -- --check`: passed after formatting the increment.
- `cargo test --workspace --all-features`: compilation succeeded and all tests reached
  passed until the pre-existing `router-pingora` `grpc_h2c` socket test; its listener
  failed with sandbox `EPERM`. The exact workspace command therefore did not pass in
  this environment.
- `./scripts/phase5-proof.sh`: daemon/recovery portion passed; the Docker routing-matrix
  gate was explicitly skipped because access to `/var/run/docker.sock` was denied.
  Docker Compose 5.1.2 is installed, but the Engine is unavailable to this sandbox.
