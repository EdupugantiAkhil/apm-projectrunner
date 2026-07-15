# Switchyard implementation progress

Updated: 2026-07-16

## Release status

- Routing proof (Phases 0–4): complete.
- Product MVP (Phases 5–6): complete.
- Team release (Phase 7): in progress.

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

## Phase 7 import/export and collaboration — Part 4

- `switchyard-planner` owns the portable bundle contract in `bundle.rs`, because it is
  the crate that already owns strict deployment/overlay parsing and validation. The CLI
  keeps only local-machine conflict checks and presentation.
- `switchyard bundle export <deployment.yaml> [--with <overlay.yaml>]... [--output
  <file>]` writes one deterministic, reviewable
  `switchyard.dev/bundle/v1alpha1` JSON file with a SHA-256 content hash over the
  canonical payload. Export embeds deployment and overlay definitions, replaces local
  source/file/dotenv inputs with `requiredLocalInputs`, preserves secret references, and
  warns/replaces credential-looking literal keys.
- `switchyard bundle import <bundle-file> --into <directory> [--force]` verifies
  apiVersion and content hash, rejects machine-state paths in typed host-path fields,
  writes the deployment and overlay YAML without overwriting unless forced, scaffolds
  placeholder local inputs, validates through the existing planner path, prints the
  normal mutation preview, and starts no runtime resources.
- Import conflict reporting is CLI-only and read-only: generated manifests, live daemon
  deployment summaries, live bind checks, and Docker `inspect` probes detect
  `name_conflict`, `domain_conflict`, `port_conflict`, `live_port_conflict`,
  `external_resource_conflict`, and `docker_unavailable`.
- Docker conflict probing degrades to `docker_unavailable` in sandboxes without Docker.
  No new daemon endpoint was added; a future daemon-aware import workflow remains a
  follow-up.
- `docs/bundles.md` documents bundle contents, omitted machine state, secret/local-input
  handling, conflict codes, and safe sharing of block, deployment, group, and overlay
  definitions. `docs/development.md` links it from the documentation index.

### Phase 7 Part 4 verification

- `cargo fmt --all --check`: passed.
- `cargo test -p switchyard-planner`: passed, including export/import validation,
  tampered-hash rejection, and unsupported-apiVersion rejection.
- `cargo test -p switchyard-planner -p switchyard-cli`: compiled and passed all planner
  tests and the new CLI parser test, then hit the pre-existing
  `host_runtime::tests::failed_startup_cleanup_allows_a_clean_retry` Unix-socket bind
  sandbox failure (`Operation not permitted`). This is the same class of socket
  restriction recorded earlier and not a bundle regression.
- `cargo test -p switchyard-cli cli::tests::parses_bundle_commands`: passed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- CLI smoke: `switchyard bundle export examples/routing-matrix/deployment.yaml` to
  `/tmp`, followed by `switchyard bundle import ... --into /tmp/... --force`, passed.
  Import produced placeholder local inputs, a create-artifacts mutation preview, and
  read-only conflict diagnostics; Docker probing degraded to `docker_unavailable` in
  this sandbox.

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
- Follow-up review fixes: `api_for_tests` now prepares the daemon with empty runtime
  observations (production `start_with_backend` still observes Docker), so the daemon
  test suite is hermetic against Switchyard-labeled resources on the host — proven by
  rerunning the empty-state test with a live decoy-labeled container. `check.sh audit`
  now names the toolchain-compatible install (`cargo install cargo-audit --locked
  --version 0.22.1`; 0.22.2+ needs rustc 1.88, the workspace pins 1.85).

## Phase 7 LAN and private-network access — Part 1

- Added the versioned `spec.exposure` host-router contract. Omission remains
  loopback-only; LAN binding requires both `mode: lan` and
  `acknowledgeLanExposureRisk: true`. Stable validation codes cover non-loopback binds
  without opt-in, missing acknowledgement, and non-loopback providers in LAN mode.
- Host mode now accepts acknowledged non-loopback listener binds while keeping provider
  upstreams loopback-only. Wildcard binds expand to concrete local interface addresses,
  emitted in a structured `lan_exposure_warning` startup event and retained in the
  shared exposure summary.
- CLI apply/status output and daemon deployment list/detail inspection surface the
  effective mode and addresses. A changed owned host-router definition is stopped and
  replaced during normal re-apply, so reverting to loopback closes LAN listeners before
  the replacement starts.
- Contract round-trip and invalid-fixture tests cover the secure default and all three
  LAN validation failures; host-gateway tests cover concrete wildcard expansion. Final
  verification: `cargo fmt --all --check` and workspace/all-target clippy with
  `-D warnings` passed; router-config passed all 8 tests; daemon passed all 18 tests;
  CLI passed all 34 non-socket unit tests plus daemon parity; router passed all 10
  non-socket unit tests plus its tokenless host-command test. The sandbox refused the
  existing TCP/Unix-listener tests with `Operation not permitted`; those tests and the
  requested second-machine LAN reachability check remain for reviewer execution on a
  socket-capable host.

### Part 1 reviewer verification (2026-07-15)

- Reviewer fix: `explicit_identity_is_rejected_on_non_loopback_listener`
  (router-pingora, socket-dependent, outside the Codex sandbox's reach) now opts in
  to acknowledged LAN exposure so validation passes, and proves the explicit
  identity header stays untrusted on non-loopback listeners even in LAN mode.
- `./scripts/check.sh`: PASSED end to end (fmt, full workspace tests, clippy
  `-D warnings`, rustdoc `-D warnings`).
- Live LAN proof on this host (192.168.1.10) against a second machine
  (poco-f1-nixos, 192.168.1.167): LAN-mode host router on `0.0.0.0:18980` emitted
  the structured `lan_exposure_warning` listing every concrete interface address;
  a remote curl through the custom domain returned 200 with proxied backend
  content; the same config without the exposure opt-in was refused with
  `LanExposureNotEnabled`; reverting the bind to loopback made the remote curl
  unreachable again while local traffic kept working.

## Phase 7 LAN and private-network access — Part 2

- Added CLI-owned `.local` mDNS publication for acknowledged LAN host gateways. The CLI
  derives only custom domains ending in `.local`, expands them across concrete exposed
  non-loopback addresses, and launches one `avahi-publish-address` process per pair only
  after gateway readiness. Owner-only state records deployment/definition ownership,
  PID start ticks, executable, exact name/address arguments, and the check report.
- Gateway stop, replacement, `down`, `cleanup`, and re-apply to loopback now terminate
  identity-verified publishers and remove their state. Missing `avahi-utils`, an
  unreachable Avahi daemon, or an immediately exiting publisher fails apply with an
  actionable diagnostic and cleans partial publication state.
- Added structured preflight results for Avahi tools/daemon reachability, usable LAN
  interfaces, VPN-style names and `/32`/`/128` host routes, best-effort firewalld/ufw/
  nftables visibility, the always-on link-boundary limitation, and post-publication
  local name resolution. CLI `up` and `status` show checks plus per-name/address
  published/failed state.
- Daemon deployment list/detail now expose optional `mdnsPublication`, derived from the
  CLI's owner-only state; the daemon does not manage Avahi processes. Router docs cover
  setup, check meanings, detection limits, same-link/guest/VPN/firewall/NSS constraints,
  reversal, and the unsupported public-internet boundary.
- Hermetic tests cover `.local` selection, loopback exclusion, state JSON/permissions,
  preflight report shaping, firewall result shaping through command injection,
  VPN/host-route classification, and daemon list/detail projection. Verification run:
  `cargo fmt --all --check`, all 18 daemon tests, 39 CLI unit tests plus daemon parity,
  and workspace/all-target clippy with `-D warnings` passed. The exact requested
  combined package test reached 39/40 CLI tests; only the pre-existing Unix-listener
  startup-cleanup test was blocked by the sandbox's `Operation not permitted`, so the
  CLI suite was re-run successfully with that one socket test filtered out.
- Live verification remains required on a Linux host with `avahi-utils`, Avahi and
  sockets available: confirm publication and local resolution, resolve/connect from a
  second same-LAN machine, observe firewall and VPN warnings on representative hosts,
  verify publisher cleanup on down/re-apply, and exercise the immediate-exit diagnostic
  with Avahi stopped.

### Part 2 reviewer verification (2026-07-16)

- Reviewer fixes after live testing (details in AGENTMISTAKES.md): spawn publishers
  with `-a -R` (argv[0] dispatch and reverse-PTR collision), advertise only
  non-VPN/non-bridge interface addresses while preflight warns on the rest, and
  include the publisher log tail in immediate-exit errors.
- `./scripts/check.sh`: PASSED end to end after the fixes.
- Live proof (radxa 192.168.1.10 publishing, poco-f1-nixos 192.168.1.167
  observing): `switchyard up` on the LAN-enabled routing-matrix fixture published
  `ui-1.routing-matrix.local -> 192.168.1.10` with the full check report (pass:
  avahi binary, avahi-daemon, lan-interface; warn: vpn-interface for tailscale0,
  firewall indeterminate under nftables, network-boundaries, name-resolution
  without nss-mdns). A unicast mDNS query from the second machine returned the
  correct A record and a curl through the published name returned 200 via the
  gateway. `switchyard down` stopped the owned publisher, removed the state file,
  and the name stopped answering.
- Environmental limitation observed and documented: this Wi-Fi network does not
  propagate the radxa host's outbound multicast (its own hostname `.local` record
  also never reaches other devices), so passive discovery from the second machine
  fails while unicast queries and TCP connects succeed — exactly the failure mode
  the preflight's `network-boundaries`/`firewall-udp-5353` warnings describe.

## Phase 7 LAN and private-network access — Part 3

- Added the explicit `GatewayExposure.publishTailscale` opt-in, omitted by default and
  valid only with acknowledged LAN exposure. Router validation exposes a stable error
  code and fixture for invalid combinations, with serialization round-trip coverage.
- Extended the adapter SDK with the `Publication` kind, `PublicationAdapter` contract,
  and structured private-network reachability/check records. The built-in
  `publication-tailscale` adapter validates its JSON Schema configuration, runs only
  `tailscale status --json` behind a command seam, requires a running backend and a
  gateway-exposed Tailscale IP, and derives the ts.net name, Tailscale IPs, and ports.
- CLI `up` now performs the advisory check after gateway readiness and atomically
  persists an owner-only deployment/version-bound record. `status` re-derives current
  tailnet reachability and reports stale/missing state without mutation; gateway stop,
  down, cleanup, and disabling the opt-in remove the record because no process or
  tailnet resource is owned.
- Daemon deployment list/detail project the guarded state as optional
  `tailscalePublication`. Router documentation covers checks, custom-domain resolution
  through MagicDNS split DNS or client-side resolution, and the strict boundary that
  Switchyard never runs Tailscale mutation commands or Funnel/public exposure.
- Hermetic adapter tests cover running, stopped, and missing-binary status through the
  command seam. `cargo fmt --all --check`, workspace/all-target clippy with
  `-D warnings`, and the requested package tests pass except for the pre-existing
  socket-bound CLI startup-cleanup test blocked by the sandbox (`Operation not
  permitted`); rerunning with only that test skipped passes 40 CLI tests plus all
  config, SDK, adapter, daemon, parity, and doc tests. Live two-machine tailnet
  verification remains with the reviewer.

### Part 3 reviewer verification (2026-07-16)

- `./scripts/check.sh`: PASSED end to end.
- Live tailnet proof (radxa publishing, poco-f1-nixos on the same tailnet):
  `switchyard up` with `publishTailscale: true` reported
  `radxa-dragon-q6a.warg-firefighter.ts.net via 100.106.209.100, fd7a:...` with all
  four checks passing. From the second machine over the tailnet, a request to the
  raw ts.net name failed closed with structured `route_not_found` (custom domains
  are not tailnet-resolvable by default, as documented), and a Host-resolved
  request to the custom domain through the tailscale address returned 200.
  `switchyard down` removed the owner-only publication state file.

### Part 4 reviewer verification (2026-07-16)

- Reviewer fix: import now pre-checks every destination path before writing any
  file, so a `bundle_write_conflict` can no longer leave a partially imported
  bundle behind.
- `./scripts/check.sh`: PASSED end to end.
- Live CLI proof: `bundle export` of routing-matrix produced a deterministic
  envelope with 8 source paths replaced by required local inputs and
  `local_path_replaced` warnings; `bundle import` into a clean directory
  reported compatibility ok, scaffolded the inputs, validated, and printed the
  full mutation preview with `Conflicts: none`. Importing into this repository
  (where fixtures already exist) reported `name_conflict` for the existing
  generated routing-matrix and a genuine `port_conflict`: jas-base also claims
  `127.0.0.1:10081`. A tampered bundle was rejected with `bundle_hash_mismatch`
  naming both hashes.

## Phase 7 reliability — Part 5: lifecycle hooks resolved by removal

- The reserved per-service `hooks` field (`prepare`, `postReady`, `stop`,
  `cleanup`) was removed from the planner schema instead of gaining an executor:
  it was never read by any runtime path, no fixture used it, and the real
  initialization mechanism (`execution: script` with `lifecycle: task`, gated via
  `dependsOn: completed_successfully`) already carries logs, status, ownership,
  and recovery like any service. Declaring `hooks` now fails closed with an
  unknown-field error naming the field
  (`declared_lifecycle_hooks_are_rejected_not_silently_ignored`); the supported
  pattern and the removal rationale are documented in `docs/adapters.md`.
- Reviewer verification (2026-07-16): `./scripts/check.sh` PASSED end to end,
  and the live `examples/jas-base/smoke.sh` PASSED, proving task-lifecycle
  database initialization, live group switching, persistence, and
  ownership-scoped cleanup all still work after the removal.

## Phase 7 reliability — Part 6: upgrade and heavy reliability tests

- Added fast SQLite upgrade-matrix tests for schema versions 1, 2, and 3 in
  `switchyard-state`. The fixtures are built through the actual historical DDL
  embedded in `src/migrations` rather than committed binary databases; this keeps
  the rows readable in review, avoids SQLite-file portability churn, and still
  exercises the production migration and backup path. Each version inserts
  representative values into every table that existed at that version, verifies
  current-schema migration to version 4, asserts row values, checks the
  pre-migration backup, and runs `PRAGMA integrity_check` plus foreign-key checks.
- Added a failed-migration recovery test that uses a test-only migration list to
  create the same pre-migration backup production would create, leaves the
  original version-2 database intact after a transaction failure, restores the
  backup to a new path, and verifies the normal current migration succeeds.
- Added schema compatibility goldens: router-config pins a Phase-7 host-router
  JSON fixture with `exposure` LAN/Tailscale fields; switchyard-planner pins
  copied compat deployments for `examples/routing-matrix` and `examples/jas-base`
  with expected definition/resource hashes and deterministic generated router
  configs.
- Added ignored heavy reliability tests and `scripts/reliability.sh`. The suite
  covers router-core reload storms, TCP and Pingora HTTP reload storms under
  concurrent clients, Linux fd/RSS leak sampling, an HTTP soak with health-check
  flapping, and in-process daemon API concurrency with global heavy-operation
  limiting plus per-deployment lock contention. Socket-bound tests are compiled
  here but must be executed by the reviewer on a host that permits loopback
  binding.

### Part 6 reviewer verification (2026-07-16)

- Reviewer fixes, all in the new tests (no product defects found; details in
  AGENTMISTAKES.md): the router-core storm's version-monotonicity check is now
  per-observer-thread (the global fetch_max compare raced benignly across
  threads); the TCP storm flips targets under `Pin`, where every client exchange
  must complete intact (asserting zero incomplete exchanges under `Close` denies
  the policy's defined behavior; `Close` stays covered by its dedicated test and
  the pre-storm sequence, and the pre-storm sequence now asserts that a pinned
  connection survives a later `Close` reload, matching
  `pin_policy_survives_later_route_changes`); the HTTP test upstream stub handles
  connections concurrently on blocking sockets and tolerates dirty disconnects
  (single-threaded serial handling with inherited nonblocking sockets collapsed
  under storm load); the storm providers declare no health checks and the soak
  uses a generous 2s health timeout (50ms timeouts manufactured fail-closed 503s
  under load); soak flap correlation uses timestamped windows with recovery slack
  instead of a boolean read after the response; fd-leak assertions compare
  growth (`end <= warmup`) instead of exact equality.
- `./scripts/reliability.sh` (defaults): PASSED — router-core storm 30s,
  router-tcp storm+leak 30s, HTTP storm+leak 31s, HTTP soak+flap 30s, daemon
  high-concurrency 2s.
- 120-second HTTP soak: PASSED with zero unexpected errors, all
  provider_unhealthy rejections inside flap windows, and no fd/RSS growth.
- `./scripts/check.sh`: PASSED end to end (fast suite runtime unchanged; all
  heavy tests are `#[ignore]`).

## Phase 7 reliability — Part 7: release packaging and diagnostics

- Added native host release assembly in `scripts/release.sh`: Rust release builds for
  `switchyard`, `switchyard-daemon`, and `switchyard-router`; a clean Node.js 24 GUI
  build; a version derived from the workspace version plus `git describe`; a platform
  tarball; generated release notes; mandatory SHA-256 checksums; and optional SSH
  signatures in the fixed `switchyard-release` namespace. No cross-compilation or
  host-dependent GPG tooling is used.
- The archive contains ownership-aware prefix installation and uninstallation. Upgrade
  replacement and deletion require the prior installed-files manifest plus matching
  per-file hashes, non-Switchyard paths are never overwritten, the default prefix is
  user-writable `~/.local`, and the daemon discovers the GUI installed below that
  prefix. `scripts/release-smoke.sh` provides the fast no-Docker artifact checksum,
  extraction, install, executable, uninstall, and clean-prefix proof.
- Added `switchyard diagnostics <deployment.yaml> [--output <path>]`. Its one-file JSON
  report gathers host/tool/runtime versions, planner validation and definition identity,
  daemon detail or deployment-scoped generated/runtime state, host-gateway logs, live
  router events when authenticated locally, and best-effort read-only Docker ownership
  observations. Missing external/runtime services remain structured unavailable data.
- Redaction is recursive and happens before the owner-only file write. Diagnostics and
  daemon event capture now share the planner's line convention; diagnostics also reuse
  the portable-bundle credential-key heuristic, replace process environment and known
  router/daemon token values, and never resolve overlay secret references. Unit tests
  plant credential fields, embedded environment values, router/daemon tokens, and an
  authorization log line and assert none survive while redaction markers do.
- `docs/release.md` documents build, checksum/signature verification, install,
  ownership-checked upgrade/uninstall, the authoritative upgrade/recovery pointer, and
  diagnostics contents and guarantees. Full release and GUI builds require reviewer
  execution where Cargo/npm network or caches are available; verification status is
  recorded below.

### Part 7 sandbox verification (2026-07-16)

- `cargo fmt --all --check`: PASSED.
- `cargo clippy --workspace --all-targets -- -D warnings`: PASSED.
- `bash -n scripts/release.sh scripts/release-smoke.sh`: PASSED; the packaged install
  and uninstall assets also pass `bash -n`.
- `cargo test -p switchyard-cli -p switchyard-planner`: the new diagnostics/parser
  tests and all planner tests pass. The unfiltered command reaches the pre-existing
  `host_runtime::tests::failed_startup_cleanup_allows_a_clean_retry` sandbox failure
  (`Operation not permitted` while exercising process signaling); rerunning with that
  one host-permission test skipped passes 43 CLI unit tests, daemon parity, all 26
  planner unit/integration tests, and planner doc tests.
- A real `target/debug/switchyard diagnostics` run against `routing-matrix` wrote an
  owner-only (`0600`) JSON report, captured generated/runtime/log state, and represented
  unavailable Docker access as best-effort structured data. A synthetic package using
  the built executables passed fresh install, manifest-owned upgrade with obsolete GUI
  removal, executable placement, hash-checked uninstall, and clean-prefix assertions.
- `scripts/release.sh`, signed/unsigned artifact generation, and the full
  `scripts/release-smoke.sh` remain for reviewer execution because the requested clean
  `npm ci`/release build may require network access unavailable in this sandbox.

### Part 7 reviewer verification (2026-07-16)

- Reviewer fix: the diagnostics redactor now scrubs only the values of
  credential-looking process environment variable names (shared planner
  heuristic) plus the daemon discovery and router tokens, instead of every
  process environment value — replacing benign values like `$HOME` erased every
  absolute path from the report (proven on a live bundle), and a variable
  holding a common short word would have mangled arbitrary text.
  `docs/release.md` states the scoped guarantee.
- `./scripts/check.sh`: PASSED end to end.
- `./scripts/release.sh`: PASSED unsigned and signed (throwaway ed25519 key);
  `ssh-keygen -Y verify` accepted `SHA256SUMS.sig` and `sha256sum -c` passed.
- `./scripts/release-smoke.sh`: PASSED (checksum verification, temp-prefix
  install, installed binaries invoke, ownership-checked uninstall, clean
  prefix).
- Live `switchyard diagnostics` against the running routing-matrix deployment
  with a planted `SWITCHYARD_ROUTER_TOKEN`: token absent from the report,
  output mode 0600, all sections present, paths still readable after the
  scoped-redaction fix.
- `/dist/` added to `.gitignore` so release artifacts cannot be committed.

## Phase 7 security and support policies — Part 8

- Audited host listeners, browser-extension permissions, router and daemon
  administration channels, host/mDNS/Tailscale state, Docker ownership and cleanup,
  overlay/script/bundle/diagnostics file paths, secret references and redaction, and
  release archive inputs against DESIGN.md section 8.
- Published `docs/security-review.md` with concrete implementation/test evidence,
  adversarial checks, and nine stable findings. Severity count: critical 0, high 4,
  medium 4, low 0, informational 1. No product code was changed; remediation remains for
  reviewer triage.
- Published `docs/support-policy.md` covering alpha configuration and state schemas,
  deliberate compatibility goldens, the one-minor/90-day parsing and API overlap window,
  additive `/api/v1` evolution, same-release CLI/daemon support, ordered forward-only
  SQLite migration/backups, newer-schema refusal, and backup-only downgrade.
- Linked both policies from `docs/development.md` and the repository README. The Phase 7
  implementation-plan checkboxes remain untouched for reviewer verification.
- Part 8 verification: `cargo fmt --all --check` passed; every new relative Markdown
  link target was inspected and exists; `git diff --check` passed.

### Part 8 reviewer verification and Phase 7 exit gate (2026-07-16)

- Security review (`docs/security-review.md`): the reviewer independently
  verified the four high findings against the code. SR-2 (unowned Compose-project
  orphans deletable via `up --remove-orphans` without the ownership proof that
  `down`/`cleanup` already required) was confirmed and fixed during sign-off:
  `DockerRuntime::up` now runs the same `discover_compose_project` +
  `verify_ownership` preflight, proven by
  `up_refuses_when_the_compose_project_contains_an_unowned_container`. SR-3, SR-4,
  and SR-7 (high) and the four mediums are accurate and recorded as an explicit
  unchecked remediation item in Phase 7 — their fixes need deliberate design
  decisions, not rushed patches. Support/deprecation policies published in
  `docs/support-policy.md`.
- Exit gate evidence:
  - LAN sharing explicit/inspectable/reversible/secure-by-default: Parts 1–3
    live proofs (opt-in + acknowledgement, exposure warnings and status/API
    surfacing, remote reachability and revert-to-loopback closure verified from
    a second machine, mDNS withdrawal on down, advisory-only tailnet
    publication).
  - Bundle round-trip across supported machines: routing-matrix exported here,
    imported and validated with the *installed release binary* on a second
    aarch64 Linux machine (poco-f1-nixos, NixOS): checksum verified,
    `Compatibility: ok`, required-local-inputs scaffolded, definition validates;
    sanitization tests prove no secrets/absolute paths embedded.
  - Release artifacts: `release-smoke.sh` locally plus on the second machine a
    full checksum-verify → install → run → reinstall (upgrade) → uninstall
    sequence ending with zero files in the prefix; an accidental default-prefix
    install was also fully removed by the manifest-driven uninstall, a
    real-world ownership-cleanup proof. Recovery procedures remain covered by
    the tested `docs/upgrade-recovery.md` paths (pre-migration backups,
    newer-schema refusal, SQLite delete/restore rebuild).
- Phase 7 remains open only on the tracked security-remediation item; every
  other Phase 7 task and the exit gate are complete.
