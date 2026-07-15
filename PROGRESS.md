# Switchyard implementation progress

Updated: 2026-07-15

## Release status

- Routing proof (Phases 0–4): complete.
- Product MVP (Phases 5–6): Phase 5 in progress.
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

## Phase 5 (in progress)

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
