# Switchyard implementation progress

Updated: 2026-07-15

## Release status

- Routing proof (Phases 0–4): complete.
- Product MVP (Phases 5–6): complete.
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

## Phase 6 implementation

### Adapter SDK (Part 1)

- `switchyard-adapter-sdk` defines the versioned `switchyard.dev/adapter-sdk/v1alpha1`
  contracts for source, execution, supervisor, route, and probe adapters. Configuration
  and recovery handles cross the boundary as serializable JSON; states, events, logs,
  claims, source identity, and route observations use normalized SDK types.
- Every adapter declares id, semantic version, supported SDK contract versions, and
  protocol/live-update/recovery/feature capabilities, and must publish a draft 2020-12
  JSON Schema (schemars generation, offline jsonschema validation). The registry rejects
  malformed ids/versions, duplicates, and incompatible contract declarations with stable
  `RegistryErrorCode`s; listing returns declaration + schema metadata for schema-driven
  forms.
- A public conformance suite checks schema compilation and dialect, valid/invalid
  examples, deterministic validation, capability consistency, compatibility, and
  lossless opaque-handle round trips.
- `switchyard-adapters` implements the seven built-ins (`source-path`, `source-git`,
  `execution-container`, `execution-runner-script`, `supervisor-process-compose`,
  `route-switchyard`, `probe-health`) at planning level; execution remains owned by the
  existing generated-Compose runtime. Trusted host execution is explicitly deferred and
  guarded by a registry test.
- `switchyard-planner` validation resolves sources, executions, probes, provider
  capabilities, and route slots through the built-in registry while keeping the
  deployment YAML, diagnostics style, and deterministic artifact generation unchanged.
  A regression test proves worktree sources still require an existing repository and a
  non-empty ref through the adapter path.
- Documentation: `docs/adapters.md`.

### Phase 6 Part 1 verification

- `cargo fmt --all -- --check`: passed.
- `cargo test --workspace --all-features`: passed on this host (all suites, including
  the socket-based router integration tests unavailable to the implementation sandbox).
- `cargo test -p switchyard-planner --test planner`: 12 passed, including the new
  worktree adapter-path regression test.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: passed.
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps`: passed.

### Source and worktree management (Part 2)

- `switchyard-sources` is a synchronous, daemon-neutral library: read-only Git
  inspection (repository root, linked-worktree detection, branch/detached HEAD, commit,
  staged/unstaged/untracked summary, ahead/behind), managed worktree/clone creation
  under `.switchyard/worktrees` and `.switchyard/clones`, and non-destructive removal.
  Non-repo paths and a missing git binary degrade to explicit unknown codes.
- Every mutating operation passes one `guard_mutation` gate: unmanaged sources are
  never mutated (deregistration only forgets the record), canonicalized paths must stay
  inside the managed roots, dirty working trees refuse removal without an explicit
  `allow_dirty` override, and unknown Git state refuses removal. No git command ever
  uses `--force`.
- SQLite schema version 4 (`registered_sources`) persists name, immutable
  managed/unmanaged kind, path, repository path, requested ref, and managed-relative
  location; live Git observations are always derived, never persisted as truth.
- `/api/v1/sources` (GET/POST/DELETE) and `/api/v1/worktrees` (GET/POST/DELETE) follow
  the existing bearer-auth and stable-error-code conventions. Review hardening moved
  all five handlers onto the Tokio blocking pool so a slow clone or worktree operation
  cannot stall async workers.
- CLI: `source list [--json]`, `source register/deregister`, `worktree create/remove
  [--allow-dirty]` with daemon-first execution and byte-stable one-shot fallback.
- Plans, manifests, and `switchyard status` now carry per-instance live source
  identities (path, repository, ref, commit, dirty) captured at plan time; definition
  and resource hashes still derive only from desired state.
- Documentation: `docs/control-plane-api.md` endpoints and a sources/worktrees section
  in `docs/development.md`.

### Phase 6 Part 2 verification

- `cargo fmt --all -- --check`: passed.
- `cargo test --workspace --all-features`: passed on this host (Codex-side run reached
  the known sandbox socket restriction only).
- Post-review daemon/sources rerun after the blocking-pool hardening: passed
  (daemon 4 unit + 9 API + parity, sources 6).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: passed.
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps`: passed.

### Overlays and variations (Part 3)

- Overlay documents (`kind: Overlay`) support deployment/instance selectors (required
  selectors must match unless `optional: true`), ordered environment (`envFiles`
  strict dotenv, `set`, `unset`), file injection (path or inline content, optional
  restricted templates, `replace: true` conflicts), parameters, and route selection.
  Deployments list overlays in order via `spec.overlays`; instances gained optional
  selector labels.
- Resolution follows the DESIGN.md precedence chain (block defaults < deployment
  overlays in order < instance values < `--set` ephemeral overrides), merges maps by
  key, honors `unset`, and records an origin trace with full shadowing history for
  every resolved environment value, parameter, file, and route.
- Injected files materialize only under
  `.switchyard/generated/<deployment>/overlays/<instance>/<content-hash>/` and are
  bind-mounted read-only; targets reject relative paths and `..` traversal and must
  fall under controlled container mount roots. Templates support only fixed-namespace
  `${...}` lookup (overlay variables, instance/deployment names, parameters) with
  unknown variables rejected — no execution of any kind.
- Secret overlay values are environment-variable or file references; previews, origin
  traces, resolved YAML, manifests, and Compose show only placeholders. Generated
  Compose interpolates `${SWITCHYARD_OVERLAY_SECRET_<hash>:?}` and the runtime injects
  real values solely into the `docker compose` process environment at apply time.
  Secret file injection is explicitly rejected as unsupported.
- `overlay validate` and `overlay diff --with ...` provide stable diagnostics and a
  per-service live/restart/rebuild classification against currently generated
  artifacts. `plan`/`up`/`down`/`status` accept `--with`, `--variation`, and `--set`;
  variations rename the deployment through existing deterministic naming with
  cross-variation listener/publication collision checks. Overlay-less output remains
  byte-stable.
- Documentation: `docs/overlays.md`.

### Phase 6 Part 3 verification

- `cargo fmt --all -- --check`: passed.
- `cargo test --workspace --all-features`: passed on this host (planner 17, CLI 32,
  all router/daemon suites green).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: passed.
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps`: passed.

### Schema-driven GUI foundation (Part 4a)

- Daemon additions: `GET /api/v1/deployments` (+ per-deployment detail with applied
  snapshot, reconciliation summary, resources, and manifest source identities),
  `GET /api/v1/adapters` (registry declarations plus JSON Schemas for schema-driven
  forms), and `/gui/` static serving of `packages/web/dist` (configurable, SPA
  fallback, traversal-safe). Static assets bypass bearer auth; `/api/v1` is unchanged
  except that operation SSE additionally accepts the credential via `access_token`
  query parameter because EventSource cannot set headers (loopback-only rationale
  documented).
- `switchyard gui` prints and best-effort-opens `http://127.0.0.1:<port>/gui/#token=…`
  using daemon discovery; the credential travels only in the URL fragment, which the
  web client captures into memory and strips from the location immediately.
- `packages/web` (Vite + React 19 + TypeScript, committed scaffold with pre-installed
  dependencies): typed API client with structured errors, operation polling and SSE
  subscription; DESIGN.md shell (deployment rail, canvas, inspector, collapsible
  event/log drawer, exact color tokens); deployment list/detail with per-instance
  source identity, live route versions, domains, and bindings; sources view with
  register-unmanaged and worktree create plus a two-step dirty-removal dialog;
  operations timeline with cancel and failure detail; guarded destructive commands
  (typed confirmation for down/cleanup, dirty-worktree acknowledgement before up);
  keyboard navigation, aria-live announcements, reduced-motion support, responsive
  fallbacks.
- Verification: workspace tests passed on this host (daemon API 12); fmt, workspace
  clippy `-D warnings`, and rustdoc `-D warnings` passed; `npm run build` passed and
  `npm test` passed (6 Vitest tests).

### Schema-driven GUI completion (Part 4b)

- Deployment definition API: `GET /api/v1/deployments/{name}/definition`,
  `POST /api/v1/deployments` (validate-first, `validateOnly` dry-run with plan
  preview, atomic hard-link create refusing overwrite), and
  `PUT .../definition` (SHA-256 optimistic concurrency, validate-first, atomic
  rename). All definition and source handlers run on the Tokio blocking pool because
  planner validation shells out to git for source identities.
- Patch bay: typed consumer/provider/group lanes, SVG cables colored by capability
  with direction arrows, node inspector (source, health, resources, active routes),
  keyboard-first switching through compatible-group selects (incompatible groups are
  omitted with an explanatory count), an always-available accessible route-matrix
  table that is also the narrow-viewport rendering, and reduced-motion compliance.
- Atomic switching: selecting a group prepares a pending change set; a preview dialog
  shows the complete replacement route table (old→new provider per slot) and the
  superseded snapshot version, with close/drain(timeout)/pin transition selection;
  apply goes through the existing `bind` command and surfaces
  acknowledgement/rollback results.
- Deployment builder: name validation, block instances with schema-driven adapter
  configuration, source selection from registered sources, parameters, continuous
  validation through the dry-run endpoint, expanded-service/compose preview, save,
  optional follow-up Up.
- `SchemaForm` renders draft 2020-12 object schemas (string/number/integer/boolean/
  enum/nested object/string array, required markers, descriptions) and degrades to a
  validated JSON textarea for unsupported constructs; a read-only block library lists
  registered adapters from `/api/v1/adapters`. No hard-coded per-adapter forms exist.
- Routing panel: custom domains, browser identity routes, and managed profiles are
  edited through the authored definition with a full line diff, validate-first gating,
  and optional plan/up follow-through — the CLI/API equivalent is the definition file
  plus `switchyard validate`.
- Per-instance log access from instance cards passes the existing `target` command
  field (review addition), completing combined and per-service logs in the GUI.

### Phase 6 Part 4 verification

- `cargo fmt --all -- --check`: passed.
- `cargo test --workspace --all-features`: passed on this host.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: passed.
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps`: passed.
- `npm run build`: passed; `npm test`: 12 Vitest tests passed.

### Real-codebase validation (Part 5)

- `examples/jas-base/` is a self-contained generic stand-in for the JAS legacy
  workspace: two image-backed database stand-ins with named volumes and a one-shot
  `lifecycle: task` schema-initialization service (`dependsOn: healthy`, consumers
  gated on `completed_successfully`), a fixed-port legacy shell script in a runner
  image for the Java stand-in, a five-process Process Compose suite per AI instance,
  and Dockerfile-built UIs with custom domains. The DESIGN.md topology is expressed
  verbatim (`ui-a → jas-main + ai-feature`, `ui-b → jas-feature + ai-main`, shared
  `db-main`), with both Java stand-ins consuming identical fixed `127.0.0.1:8001–8005`
  and `9101/9102` slots and both UIs consuming `127.0.0.1:10081`.
- Planner tests (`real_codebase_fixtures.rs`): full expansion assertions for the
  fixture; a fixture-swap test planning jas-base and routing-matrix through the
  identical deterministic path; an overlay/variation disjointness test; and a guard
  test proving no `jas` identifier exists in any production crate source.
- Discovered gap recorded in the plan (Phase 7): declared `LifecycleHooks`
  (`prepare`/`postReady`/`stop`/`cleanup`) are schema-only — nothing generates or
  executes them; the fixture deliberately uses task-lifecycle services instead and
  documents the gap in its README.
- Review fixes: the UI `java` slot originally declared `host: localhost`, which the
  router rejects (`invalid IP address syntax`) because listener binds require IP
  literals — changed to `127.0.0.1`, which serves the unchanged app's
  `localhost:10081` calls identically inside the namespace. The smoke script's
  variation demonstration now skips with a notice when another generated deployment
  legitimately claims `127.0.0.1:10081` (the collision guard working as designed in a
  shared workspace).

### Phase 6 Part 5 verification

- `cargo test -p switchyard-planner --all-features`: passed (21 tests including the
  four new fixture tests).
- `cargo fmt --all -- --check`, workspace clippy `-D warnings`, rustdoc `-D warnings`:
  passed.
- `examples/jas-base/smoke.sh`: PASSED end to end on this host (Docker Engine 29.4.0,
  Compose 5.1.2, Linux aarch64): build, registered unmanaged source + managed
  worktree, typed topology observations for both UIs and both Java stand-ins,
  task-based schema initialization, live AI-group switch without restarting the Java
  stand-in, source identity in status, database volume persistence across down/up,
  and zero owned resources after cleanup with the workspace git status unchanged.

### Phase 6 exit gate (Part 6)

- `docs/mvp-acceptance.md` audits every DESIGN.md §14 criterion (1–21) against named
  Rust tests, Vitest tests, and smoke-script assertions, deliberately distinguishing
  complete automation from partial automation; criteria 1, 3, 7, 14, and 18 carry
  documented manual procedures for their remaining manual portions. The CLI/API/GUI
  parity matrix covers every common operation; the two gaps it found were closed
  during review: `switchyard operation cancel <id>` (daemon-backed arbitrary
  operation cancellation from the CLI) and an instance-card **Open** button for
  managed-profile instances in the GUI.
- `docs/upgrade-recovery.md` documents test-backed upgrade (ordered migrations,
  pre-migration backups, newer-schema refusal, backup-based downgrade) and recovery
  procedures (daemon restart, deleted/restored SQLite, drift review, data-safety
  guarantees), each referencing the proving test by name.
- `scripts/phase6-proof.sh` is the one-command Phase 6 check: `scripts/check.sh`
  (fmt, workspace tests, clippy `-D warnings`, rustdoc `-D warnings`), a clean GUI
  `npm ci`/build/test, and the live `examples/jas-base/smoke.sh`.
- Honest residual limits recorded in the audit: browser routing is live-proven with
  Origin-bearing requests rather than a driven browser; Docker-label recovery by a
  restarted real daemon and Docker Engine restarts remain integration boundaries;
  concurrent variation execution is proven at planning level with a manual live
  procedure; the lifecycle-hooks execution gap is tracked as Phase 7 work.

## Phase 6 verification

- `./scripts/phase6-proof.sh`: PASSED on this host (Linux aarch64, Docker Engine
  29.4.0, Compose 5.1.2, Node 24): workspace formatting, full workspace tests,
  clippy `-D warnings`, rustdoc `-D warnings`, GUI clean install/build and 12 Vitest
  tests, and the complete live jas-base smoke (topology, worktree sources, live group
  switching, task initialization, volume persistence, ownership-scoped cleanup).
- `cargo test -p switchyard-cli --all-features`: passed (35 unit tests including the
  new `parses_operation_cancel`, plus the daemon-parity integration test).
- Earlier per-part verification is recorded in the Part 1–5 sections above; the
  routing proof remains covered by `scripts/phase4-proof.sh`.

## Post-phase-6 full review and re-verification (2026-07-15)

- `./scripts/phase6-proof.sh`: re-run PASSED end to end on this host, including the
  live jas-base smoke with clean ownership-scoped teardown.
- `examples/routing-matrix/smoke.sh`: re-run PASSED (the standing live gate for
  Phases 4 and 5; `phase4-proof.sh`/`phase5-proof.sh` are this plus already-passed
  workspace/daemon tests).
- `./scripts/check.sh audit`: cargo-audit 0.22.1 (0.22.2 needs rustc 1.88; the
  workspace toolchain is 1.85) with the two documented protobuf ignores.
- Manual code review of the highest-risk paths (daemon auth middleware and SSE
  query-token scope, GUI static serving traversal guard, definition create/update
  atomicity and optimistic concurrency, live-bind rollback/compensation, state-store
  lease acquire/heartbeat/release, sources `guard_mutation` containment, overlay file
  injection and secret placeholder/runtime injection, daemon discovery client): no
  major defects found.
- Review fix: `PUT /api/v1/deployments/{name}/definition` now validates the
  deployment name before deriving the definition path (the GET already did),
  closing a percent-encoded traversal-shaped read; covered by a new 404 assertion in
  `definition_absence_and_validation_failures_have_stable_structured_errors`.
- Note for test runs: the daemon test suite's startup reconciliation observes real
  Docker labels, so running it concurrently with a live smoke script can leak that
  smoke's deployments into `deployment_and_adapter_endpoints_are_authenticated_and_
  shape_empty_state`; run them sequentially.
