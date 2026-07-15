# Switchyard implementation plan

This file is the execution checklist for the architecture in [DESIGN.md](DESIGN.md).
It deliberately separates implementation progress from design discussion so completed
work can be marked without rewriting the specification.

## How to use this checklist

- Mark a task `[x]` only after its code, tests, and relevant documentation are merged.
- Keep partially completed tasks unchecked and add indented notes or links beneath them.
- Do not start a phase whose entry gate is incomplete unless the work is an isolated
  experiment that will not constrain the public contracts.
- Treat every exit gate as required. A phase is complete only when all of its tasks and
  its exit gate are checked.
- Add newly discovered required work to the relevant phase instead of silently expanding
  an existing checkbox.

## Release map

- [x] **Routing proof:** Phases 0–4 demonstrate the zero-application-change topology.
- [ ] **Product MVP:** Phase 5 persistent control plane is complete; Phase 6 adapters
      and GUI are not started.
- [ ] **Team release:** Phase 7 adds LAN workflows and production-quality hardening.

## Phase 0 — Repository and contract foundation

Goal: establish stable contracts and test fixtures before implementing network behavior.

### Workspace

- [x] Create the root Rust workspace and pin the supported Rust toolchain.
- [x] Create `router-core`, `router-config`, `router-pingora`, and `router-tcp` crates.
- [x] Add shared formatting, linting, dependency-audit, and test commands.
- [x] Add CI for Rust formatting, Clippy, unit tests, and documentation checks.
- [x] Add a development bootstrap command that checks Rust, Docker Engine, Docker
      Compose, and required host capabilities.
- [x] Document supported host platforms and the initial Linux-first development path.

### Configuration contracts

- [x] Define versioned identifiers for deployments, instances, components, groups,
      bindings, route slots, and route snapshots.
- [x] Define the versioned router configuration schema.
- [x] Represent HTTP, HTTPS, WebSocket, gRPC, and raw TCP listeners explicitly.
- [x] Represent custom domains, legacy localhost destinations, providers, health checks,
      and connection transition policies explicitly.
- [x] Define browser identity precedence: explicit header, Origin, then proxy listener.
- [x] Define validation errors with stable machine-readable codes and human-readable
      context.
- [x] Add serialization round-trip and schema compatibility tests.
- [x] Add invalid-configuration fixtures for duplicate listeners, missing providers,
      incompatible protocols, and incomplete groups.

### Initial fixture contracts

- [x] Specify the `routing-matrix` fixture with three UIs, two backend instances, two
      five-service groups, and at least one shared service.
- [x] Ensure fixture applications use fixed localhost addresses and require no
      Switchyard-specific code.
- [x] Define observable responses that identify the selected backend and every selected
      downstream service.

### Phase 0 exit gate

- [x] A clean checkout can run all contract tests with one documented command.
- [x] The schemas can express the complete routing-matrix topology without runtime code.

## Phase 1 — Rust router engine

Goal: implement one reliable routing engine that can be embedded in host-gateway,
sidecar, and forward-proxy modes.

Entry gate: Phase 0 is complete.

### Route compilation and snapshots

- [x] Parse and validate a complete router configuration.
- [x] Compile configuration into an immutable runtime route snapshot.
- [x] Atomically replace the active snapshot without exposing partial group updates.
- [x] Retain the previous snapshot long enough to apply the configured connection policy.
- [x] Reject stale or out-of-order snapshot versions.
- [x] Return an acknowledgement containing version, checksum, and activation status.
- [x] Add deterministic route matching and precedence tests.
- [x] Add concurrent lookup/reload stress tests.

### HTTP-family data plane

- [x] Integrate Pingora without exposing Pingora types in the public configuration
      contract.
- [x] Implement HTTP/1.1 reverse proxying.
- [x] Implement HTTP/2 and gRPC proxying.
- [x] Implement WebSocket upgrade and long-lived connection proxying.
- [x] Preserve required forwarding metadata while removing internal Switchyard headers
      before provider delivery where configured.
- [x] Implement upstream connection timeouts, request limits, and structured errors.
- [x] Implement provider readiness and health checks.
- [x] Add HTTP, gRPC, WebSocket, and unhealthy-provider integration tests.

### Raw TCP data plane

- [x] Implement Tokio-based TCP listeners and bidirectional forwarding.
- [x] Implement connect, idle, and shutdown timeouts.
- [x] Implement close, drain, and pin behavior for existing connections during a route
      change.
- [x] Add TCP reload and long-lived-connection integration tests.

### Router control and inspection

- [x] Implement a local authenticated administration channel.
- [x] Support configuration validate, apply, current-version, routes, health, and drain
      operations.
- [x] Emit structured access, routing-decision, health, reload, and rejection events.
- [x] Expose counters for requests, connections, errors, and active snapshot version.
- [x] Ensure secrets and sensitive headers are redacted from logs.
- [x] Implement graceful process shutdown.

### Phase 1 exit gate

- [x] One router process can proxy every supported protocol in automated tests.
- [x] Route snapshots switch atomically under concurrent traffic.
- [x] Invalid or ambiguous routing input fails closed with an actionable diagnostic.

## Phase 2 — Docker runtime and container-local routing

Goal: run unchanged application instances in isolated Docker network namespaces and
route their fixed localhost dependencies through router sidecars.

Entry gate: the Phase 1 router engine is stable.

### Planner and generated Compose

- [x] Implement minimal YAML schemas for sources, blocks, instances, service groups,
      bindings, routes, lifecycle hooks, and probes.
- [x] Validate names, paths, required variables, dependency cycles, listener conflicts,
      and missing providers before mutation.
- [x] Resolve a deployment into deterministic container, network, volume, DNS, and route
      names.
- [x] Generate Docker Compose as an internal artifact under `.switchyard/generated`.
- [x] Create one private bridge network per deployment.
- [x] Add stable ownership labels to every generated Docker resource.
- [x] Publish human-facing container ports only on host loopback using dynamically
      allocated ports.
- [x] Support image-backed containers and containerized legacy scripts.
- [x] Support Process Compose inside an isolated runner container.

### Sidecar namespace model

- [x] Generate one router sidecar for each consumer with loopback route slots.
- [x] Join the consumer namespace with `network_mode: service:<consumer>`.
- [x] Bind the consumer's declared fixed addresses such as `127.0.0.1:8001`.
- [x] Resolve provider targets through deterministic private-network DNS aliases.
- [x] Confirm that two consumers can independently bind identical localhost ports.
- [x] Ensure sidecar readiness gates consumer startup when the application would
      otherwise race its dependencies.
- [x] Reload a sidecar route snapshot without restarting its application container.

### One-shot CLI

- [x] Implement `switchyard validate`.
- [x] Implement `switchyard plan` with a complete mutation and route preview.
- [x] Implement `switchyard up` with build and health progress.
- [x] Implement `switchyard bind` with validation and atomic snapshot application.
- [x] Implement `switchyard status` and route inspection.
- [x] Implement combined and per-instance `switchyard logs`.
- [x] Implement `switchyard down` without deleting persistent volumes by default.
- [x] Implement explicit destructive cleanup with confirmation and ownership checks.

### Recovery without SQLite

- [x] Write a resolved deployment manifest for each apply operation.
- [x] Discover running resources from Docker ownership labels.
- [x] Detect manifest/runtime drift and report it without silently mutating resources.
- [x] Prove stop and cleanup work after the original CLI process exits.

### Phase 2 exit gate

- [x] Two unchanged backend containers both call the same localhost ports and reach
      different provider groups.
- [x] Switching one backend's five-service group does not restart either backend.
- [x] Persistent application data survives `switchyard down` and a later `up`.

## Phase 3 — Native host gateway and browser routing

Goal: route custom domains and unchanged browser calls to localhost without application
changes.

Entry gate: Docker instances and sidecar routing work end to end.

### Host gateway

- [x] Add native host-gateway mode to the Switchyard Router binary.
- [x] Bind configured custom-domain and legacy localhost listeners on the host.
- [x] Route custom domains to loopback-only container upstream ports.
- [x] Add configurable local HTTP and HTTPS modes.
- [x] Implement local certificate generation, trust setup guidance, renewal, and cleanup.
- [x] Detect host-port and domain conflicts before applying configuration.
- [x] Preserve WebSocket, gRPC, streaming, and raw TCP behavior through the host gateway.

### Origin routing and browser safety

- [x] Match browser requests by exact configured Origin and destination listener.
- [x] Handle CORS preflight requests in the gateway.
- [x] Add narrowly scoped CORS response headers for configured origins only.
- [x] Handle browser private-network preflight requirements where the target browser
      requires them.
- [x] Reject missing, unknown, conflicting, or spoofed routing identity according to the
      configured trust policy.
- [x] Return an actionable ambiguity page or JSON error with candidate routes.
- [x] Add tests for requests with Origin, without Origin, and with disallowed origins.

### Explicit per-tab identity

- [x] Specify the `X-Switchyard-Route` header format and trust boundary.
- [x] Build a minimal Chromium extension that associates an allowed route with a tab.
- [x] Ensure extension rules cannot target undeclared Switchyard deployments.
- [x] Strip the identity header before forwarding unless a provider explicitly opts in.
- [x] Document extension installation, permissions, and disable/remove behavior.
- [x] Test multiple UI tabs making identical localhost requests concurrently.

### Managed profile fallback

- [x] Implement `switchyard open <ui>`.
- [x] Allocate a dedicated authenticated forward-proxy listener per managed profile.
- [x] Launch Chromium with the required proxy and loopback-bypass arguments.
- [x] Store browser data in deployment-scoped, removable profile directories.
- [x] Detect unsupported browsers and provide an actionable fallback message.
- [x] Test guaranteed routing with no extension and no usable Origin.

### Phase 3 exit gate

- [x] Three unchanged UIs can all call `localhost:10081` while routing to independently
      selected backend instances.
- [x] Header, Origin, and managed-profile modes each pass end-to-end tests.
- [x] An unidentifiable request is rejected rather than sent to an arbitrary backend.

## Phase 4 — Complete routing proof and hard boundary tests

Goal: prove the root problem is solved before investing in the persistent product layer.

Entry gate: Phases 0–3 are complete.

### Routing-matrix fixture

- [x] Implement three UI instances from independently selectable sources.
- [x] Implement two backend instances from independently selectable sources.
- [x] Implement two named groups containing five independently observable services each.
- [x] Add at least one provider shared between both groups.
- [x] Configure `ui-1 → backend-1 → feature-services`.
- [x] Configure `ui-2 → backend-2 → main-services`.
- [x] Configure `ui-3 → backend-1 → feature-services`.
- [x] Verify custom domains for every UI.
- [x] Verify all fixed browser and backend localhost addresses remain unchanged.

### Dynamic switching

- [x] Switch a UI between backend instances without restarting the UI container.
- [x] Switch a backend between complete five-service groups without restarting the
      backend container.
- [x] Verify no request observes a partially switched group.
- [x] Verify existing connections follow their declared close, drain, or pin policy.
- [x] Verify rollback to the previous snapshot after provider health failure.
- [x] Record routing decisions and snapshot versions in test output.

### Architectural boundary

- [x] Demonstrate that UIs sharing one backend also share its downstream group.
- [x] Demonstrate two backend instances from the same source when UIs require different
      downstream groups.
- [x] Document why per-request downstream selection is impossible without application
      context propagation.
- [x] Fail planning when a requested topology violates this invariant.

### Failure and lifecycle tests

- [x] Test provider crash, router crash, application crash, and Docker restart recovery.
- [x] Test a failed route apply and confirm the previous snapshot remains active.
- [x] Test startup with unavailable dependencies and delayed readiness.
- [x] Test clean shutdown with active HTTP, WebSocket, gRPC, and TCP connections.
- [x] Test parallel deployments for naming, port, network, and volume isolation.

### Phase 4 exit gate — routing proof release

- [x] The complete routing-matrix scenario passes from a clean checkout with one
      documented command.
- [x] The test requires no changes or Switchyard libraries inside fixture applications.
- [x] The routing proof is reproducible on every initially supported host platform.
- [x] Known limitations and the backend-group invariant are visible in CLI diagnostics
      and documentation.

## Phase 5 — Persistent control plane and SQLite state

Goal: promote the proven routing behavior into a recoverable local service without
changing the topology model.

Entry gate: the routing proof release is complete.

### Daemon and API

- [x] Implement the long-running Switchyard control-plane daemon.
- [x] Define a versioned HTTP API shared by the CLI and future GUI.
- [x] Add Server-Sent Events for operations, builds, health, routes, and logs.
- [x] Serialize mutations per deployment and enforce a global concurrency limit.
- [x] Add cancellation and resumable observation for long-running operations.
- [x] Move one-shot CLI behavior behind the API without removing script-friendly output.

### SQLite state

- [x] Add versioned SQLite migrations and automatic backup before migration.
- [x] Store the last applied resolved desired-state snapshot and definition hash.
- [x] Store deployment, operation, resource, health, route, and snapshot history.
- [x] Store locks and recover or expire abandoned operations safely.
- [x] Keep secrets out of SQLite and persist references only.
- [x] Reconcile SQLite records, generated manifests, and Docker ownership labels at
      daemon startup.
- [x] Rebuild observed state when SQLite is deleted or restored from backup.
- [x] Detect and report desired/applied/observed drift.

### Live router control

- [x] Push versioned snapshots to host and sidecar routers.
- [x] Require acknowledgement before marking a binding applied.
- [x] Persist apply failure and rollback history.
- [x] Expose current, previous, desired, and observed route versions.
- [x] Add graceful connection-drain policy controls.

### Phase 5 exit gate

- [x] Restarting the daemon preserves deployments, domains, bindings, and route history.
- [x] Deleting SQLite allows safe observed-state recovery from manifests and Docker.
- [x] CLI operations behave consistently through the daemon API.

## Phase 6 — Product MVP: adapters, sources, overlays, and GUI

Goal: make the proven engine usable for existing multi-repository development workflows.

Entry gate: the persistent control plane is complete.

### Adapter SDK

- [ ] Define versioned source, execution, supervisor, route, and probe adapter contracts.
- [ ] Require JSON Schema for every adapter's user configuration.
- [ ] Add capability and compatibility declarations.
- [ ] Add adapter conformance tests and compatibility-version checks.
- [ ] Implement path, Git/worktree, container, runner-script, Process Compose, HTTP/TCP
      route, and health-probe adapters.
- [ ] Defer trusted host execution until its ownership and isolation checks pass the same
      conformance suite.

### Source and worktree management

- [ ] Inspect existing repositories, worktrees, branches, commits, and dirty state.
- [ ] Register unmanaged paths without taking ownership of them.
- [ ] Create and remove managed clones and worktrees non-destructively.
- [ ] Prevent destructive Git operations against unmanaged or dirty worktrees.
- [ ] Surface exact source identity for every running instance.

### Overlays and variations

- [ ] Implement ordered environment, dotenv, file, parameter, and route overlays.
- [ ] Track the origin and shadowing history of every resolved value.
- [ ] Materialize injected files outside source worktrees.
- [ ] Keep secret values out of generated previews, logs, YAML, and SQLite.
- [ ] Preview whether each change requires live reload, restart, or rebuild.
- [ ] Run multiple resolved variations concurrently without resource collisions.

### Schema-driven GUI

- [ ] Implement the deployment list and creation flow.
- [ ] Implement the patch-bay topology view for instances, groups, and bindings.
- [ ] Implement instance inspection with source, health, resources, and active routes.
- [ ] Implement atomic group and backend switching with a complete change preview.
- [ ] Implement custom-domain and browser-routing management.
- [ ] Implement combined and per-service logs and operation progress.
- [ ] Generate basic adapter forms from JSON Schema.
- [ ] Meet keyboard navigation, screen-reader, contrast, and responsive-layout criteria.
- [ ] Ensure every common GUI operation has an equivalent CLI and API operation.

### Real-codebase validation

- [ ] Add the JAS legacy deployment as a generic integration fixture.
- [ ] Run its database, Java, UI, and five-service Python components in declared blocks.
- [ ] Validate multiple source/worktree combinations without modifying the codebase.
- [ ] Replace the JAS fixture with an unrelated fixture without changing core code.

### Phase 6 exit gate — product MVP release

- [ ] All MVP acceptance criteria in `DESIGN.md` are checked against automated or
      documented manual tests.
- [ ] Common deployments can be created, inspected, switched, and stopped from both CLI
      and GUI.
- [ ] Database data persists unless the user explicitly requests destructive cleanup.
- [ ] Upgrade and recovery procedures are documented and tested.

## Phase 7 — LAN, team workflows, and hardening

Goal: extend the local product carefully without turning it into a production scheduler.

Entry gate: the product MVP is stable for local single-developer use.

### LAN and private-network access

- [ ] Implement opt-in LAN gateway binding with an explicit exposure warning.
- [ ] Implement mDNS publication and preflight checks.
- [ ] Detect firewall, subnet, guest-network, VPN, and name-resolution limitations where
      practical.
- [ ] Add an optional Tailscale or private-DNS publication adapter.
- [ ] Keep public-internet exposure outside the supported scope.

### Import, export, and collaboration

- [ ] Export portable deployment bundles without secrets or machine-specific state.
- [ ] Import bundles with compatibility validation and a complete mutation preview.
- [ ] Add conflict detection for names, domains, ports, and external resources.
- [ ] Document safe sharing of block, deployment, group, and overlay definitions.

### Reliability and release engineering

- [ ] Add upgrade tests across every supported configuration and SQLite schema version.
- [ ] Add resource-leak, long-running soak, reload-storm, and high-concurrency tests.
- [ ] Add platform packaging, checksums, signatures, and release notes.
- [ ] Add diagnostics bundle generation with automatic secret redaction.
- [ ] Complete a security review of host listeners, extension permissions, admin
      channels, Docker authority, file mounts, and secret handling.
- [ ] Publish support and deprecation policies for configuration and API versions.

### Phase 7 exit gate — team release

- [ ] LAN/private-network sharing is explicit, inspectable, reversible, and secure by
      default.
- [ ] Deployment bundles round-trip across supported machines without embedding secrets.
- [ ] Release artifacts pass installation, upgrade, recovery, and uninstall tests.

## Deferred ideas

These items are intentionally outside the current phase gates. Move an item into a phase
only after its requirement and acceptance test are understood.

- [ ] Evaluate Podman as a runtime adapter.
- [ ] Evaluate Kubernetes, containerd, or Nomad adapters.
- [ ] Evaluate non-Chromium browser extension support.
- [ ] Evaluate multi-host scheduling.
- [ ] Evaluate multi-user authentication and authorization.
- [ ] Evaluate public plugin distribution and sandboxing.
