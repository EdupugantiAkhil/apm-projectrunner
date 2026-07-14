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

- [ ] **Routing proof:** Phases 0–4 demonstrate the zero-application-change topology.
- [ ] **Product MVP:** Phases 5–6 add persistent state, APIs, adapters, and the GUI.
- [ ] **Team release:** Phase 7 adds LAN workflows and production-quality hardening.

## Phase 0 — Repository and contract foundation

Goal: establish stable contracts and test fixtures before implementing network behavior.

### Workspace

- [ ] Create the root Rust workspace and pin the supported Rust toolchain.
- [ ] Create `router-core`, `router-config`, `router-pingora`, and `router-tcp` crates.
- [ ] Add shared formatting, linting, dependency-audit, and test commands.
- [ ] Add CI for Rust formatting, Clippy, unit tests, and documentation checks.
- [ ] Add a development bootstrap command that checks Rust, Docker Engine, Docker
      Compose, and required host capabilities.
- [ ] Document supported host platforms and the initial Linux-first development path.

### Configuration contracts

- [ ] Define versioned identifiers for deployments, instances, components, groups,
      bindings, route slots, and route snapshots.
- [ ] Define the versioned router configuration schema.
- [ ] Represent HTTP, HTTPS, WebSocket, gRPC, and raw TCP listeners explicitly.
- [ ] Represent custom domains, legacy localhost destinations, providers, health checks,
      and connection transition policies explicitly.
- [ ] Define browser identity precedence: explicit header, Origin, then proxy listener.
- [ ] Define validation errors with stable machine-readable codes and human-readable
      context.
- [ ] Add serialization round-trip and schema compatibility tests.
- [ ] Add invalid-configuration fixtures for duplicate listeners, missing providers,
      incompatible protocols, and incomplete groups.

### Initial fixture contracts

- [ ] Specify the `routing-matrix` fixture with three UIs, two backend instances, two
      five-service groups, and at least one shared service.
- [ ] Ensure fixture applications use fixed localhost addresses and require no
      Switchyard-specific code.
- [ ] Define observable responses that identify the selected backend and every selected
      downstream service.

### Phase 0 exit gate

- [ ] A clean checkout can run all contract tests with one documented command.
- [ ] The schemas can express the complete routing-matrix topology without runtime code.

## Phase 1 — Rust router engine

Goal: implement one reliable routing engine that can be embedded in host-gateway,
sidecar, and forward-proxy modes.

Entry gate: Phase 0 is complete.

### Route compilation and snapshots

- [ ] Parse and validate a complete router configuration.
- [ ] Compile configuration into an immutable runtime route snapshot.
- [ ] Atomically replace the active snapshot without exposing partial group updates.
- [ ] Retain the previous snapshot long enough to apply the configured connection policy.
- [ ] Reject stale or out-of-order snapshot versions.
- [ ] Return an acknowledgement containing version, checksum, and activation status.
- [ ] Add deterministic route matching and precedence tests.
- [ ] Add concurrent lookup/reload stress tests.

### HTTP-family data plane

- [ ] Integrate Pingora without exposing Pingora types in the public configuration
      contract.
- [ ] Implement HTTP/1.1 reverse proxying.
- [ ] Implement HTTP/2 and gRPC proxying.
- [ ] Implement WebSocket upgrade and long-lived connection proxying.
- [ ] Preserve required forwarding metadata while removing internal Switchyard headers
      before provider delivery where configured.
- [ ] Implement upstream connection timeouts, request limits, and structured errors.
- [ ] Implement provider readiness and health checks.
- [ ] Add HTTP, gRPC, WebSocket, and unhealthy-provider integration tests.

### Raw TCP data plane

- [ ] Implement Tokio-based TCP listeners and bidirectional forwarding.
- [ ] Implement connect, idle, and shutdown timeouts.
- [ ] Implement close, drain, and pin behavior for existing connections during a route
      change.
- [ ] Add TCP reload and long-lived-connection integration tests.

### Router control and inspection

- [ ] Implement a local authenticated administration channel.
- [ ] Support configuration validate, apply, current-version, routes, health, and drain
      operations.
- [ ] Emit structured access, routing-decision, health, reload, and rejection events.
- [ ] Expose counters for requests, connections, errors, and active snapshot version.
- [ ] Ensure secrets and sensitive headers are redacted from logs.
- [ ] Implement graceful process shutdown.

### Phase 1 exit gate

- [ ] One router process can proxy every supported protocol in automated tests.
- [ ] Route snapshots switch atomically under concurrent traffic.
- [ ] Invalid or ambiguous routing input fails closed with an actionable diagnostic.

## Phase 2 — Docker runtime and container-local routing

Goal: run unchanged application instances in isolated Docker network namespaces and
route their fixed localhost dependencies through router sidecars.

Entry gate: the Phase 1 router engine is stable.

### Planner and generated Compose

- [ ] Implement minimal YAML schemas for sources, blocks, instances, service groups,
      bindings, routes, lifecycle hooks, and probes.
- [ ] Validate names, paths, required variables, dependency cycles, listener conflicts,
      and missing providers before mutation.
- [ ] Resolve a deployment into deterministic container, network, volume, DNS, and route
      names.
- [ ] Generate Docker Compose as an internal artifact under `.switchyard/generated`.
- [ ] Create one private bridge network per deployment.
- [ ] Add stable ownership labels to every generated Docker resource.
- [ ] Publish human-facing container ports only on host loopback using dynamically
      allocated ports.
- [ ] Support image-backed containers and containerized legacy scripts.
- [ ] Support Process Compose inside an isolated runner container.

### Sidecar namespace model

- [ ] Generate one router sidecar for each consumer with loopback route slots.
- [ ] Join the consumer namespace with `network_mode: service:<consumer>`.
- [ ] Bind the consumer's declared fixed addresses such as `127.0.0.1:8001`.
- [ ] Resolve provider targets through deterministic private-network DNS aliases.
- [ ] Confirm that two consumers can independently bind identical localhost ports.
- [ ] Ensure sidecar readiness gates consumer startup when the application would
      otherwise race its dependencies.
- [ ] Reload a sidecar route snapshot without restarting its application container.

### One-shot CLI

- [ ] Implement `switchyard validate`.
- [ ] Implement `switchyard plan` with a complete mutation and route preview.
- [ ] Implement `switchyard up` with build and health progress.
- [ ] Implement `switchyard bind` with validation and atomic snapshot application.
- [ ] Implement `switchyard status` and route inspection.
- [ ] Implement combined and per-instance `switchyard logs`.
- [ ] Implement `switchyard down` without deleting persistent volumes by default.
- [ ] Implement explicit destructive cleanup with confirmation and ownership checks.

### Recovery without SQLite

- [ ] Write a resolved deployment manifest for each apply operation.
- [ ] Discover running resources from Docker ownership labels.
- [ ] Detect manifest/runtime drift and report it without silently mutating resources.
- [ ] Prove stop and cleanup work after the original CLI process exits.

### Phase 2 exit gate

- [ ] Two unchanged backend containers both call the same localhost ports and reach
      different provider groups.
- [ ] Switching one backend's five-service group does not restart either backend.
- [ ] Persistent application data survives `switchyard down` and a later `up`.

## Phase 3 — Native host gateway and browser routing

Goal: route custom domains and unchanged browser calls to localhost without application
changes.

Entry gate: Docker instances and sidecar routing work end to end.

### Host gateway

- [ ] Add native host-gateway mode to the Switchyard Router binary.
- [ ] Bind configured custom-domain and legacy localhost listeners on the host.
- [ ] Route custom domains to loopback-only container upstream ports.
- [ ] Add configurable local HTTP and HTTPS modes.
- [ ] Implement local certificate generation, trust setup guidance, renewal, and cleanup.
- [ ] Detect host-port and domain conflicts before applying configuration.
- [ ] Preserve WebSocket, gRPC, streaming, and raw TCP behavior through the host gateway.

### Origin routing and browser safety

- [ ] Match browser requests by exact configured Origin and destination listener.
- [ ] Handle CORS preflight requests in the gateway.
- [ ] Add narrowly scoped CORS response headers for configured origins only.
- [ ] Handle browser private-network preflight requirements where the target browser
      requires them.
- [ ] Reject missing, unknown, conflicting, or spoofed routing identity according to the
      configured trust policy.
- [ ] Return an actionable ambiguity page or JSON error with candidate routes.
- [ ] Add tests for requests with Origin, without Origin, and with disallowed origins.

### Explicit per-tab identity

- [ ] Specify the `X-Switchyard-Route` header format and trust boundary.
- [ ] Build a minimal Chromium extension that associates an allowed route with a tab.
- [ ] Ensure extension rules cannot target undeclared Switchyard deployments.
- [ ] Strip the identity header before forwarding unless a provider explicitly opts in.
- [ ] Document extension installation, permissions, and disable/remove behavior.
- [ ] Test multiple UI tabs making identical localhost requests concurrently.

### Managed profile fallback

- [ ] Implement `switchyard open <ui>`.
- [ ] Allocate a dedicated authenticated forward-proxy listener per managed profile.
- [ ] Launch Chromium with the required proxy and loopback-bypass arguments.
- [ ] Store browser data in deployment-scoped, removable profile directories.
- [ ] Detect unsupported browsers and provide an actionable fallback message.
- [ ] Test guaranteed routing with no extension and no usable Origin.

### Phase 3 exit gate

- [ ] Three unchanged UIs can all call `localhost:10081` while routing to independently
      selected backend instances.
- [ ] Header, Origin, and managed-profile modes each pass end-to-end tests.
- [ ] An unidentifiable request is rejected rather than sent to an arbitrary backend.

## Phase 4 — Complete routing proof and hard boundary tests

Goal: prove the root problem is solved before investing in the persistent product layer.

Entry gate: Phases 0–3 are complete.

### Routing-matrix fixture

- [ ] Implement three UI instances from independently selectable sources.
- [ ] Implement two backend instances from independently selectable sources.
- [ ] Implement two named groups containing five independently observable services each.
- [ ] Add at least one provider shared between both groups.
- [ ] Configure `ui-1 → backend-1 → feature-services`.
- [ ] Configure `ui-2 → backend-2 → main-services`.
- [ ] Configure `ui-3 → backend-1 → feature-services`.
- [ ] Verify custom domains for every UI.
- [ ] Verify all fixed browser and backend localhost addresses remain unchanged.

### Dynamic switching

- [ ] Switch a UI between backend instances without restarting the UI container.
- [ ] Switch a backend between complete five-service groups without restarting the
      backend container.
- [ ] Verify no request observes a partially switched group.
- [ ] Verify existing connections follow their declared close, drain, or pin policy.
- [ ] Verify rollback to the previous snapshot after provider health failure.
- [ ] Record routing decisions and snapshot versions in test output.

### Architectural boundary

- [ ] Demonstrate that UIs sharing one backend also share its downstream group.
- [ ] Demonstrate two backend instances from the same source when UIs require different
      downstream groups.
- [ ] Document why per-request downstream selection is impossible without application
      context propagation.
- [ ] Fail planning when a requested topology violates this invariant.

### Failure and lifecycle tests

- [ ] Test provider crash, router crash, application crash, and Docker restart recovery.
- [ ] Test a failed route apply and confirm the previous snapshot remains active.
- [ ] Test startup with unavailable dependencies and delayed readiness.
- [ ] Test clean shutdown with active HTTP, WebSocket, gRPC, and TCP connections.
- [ ] Test parallel deployments for naming, port, network, and volume isolation.

### Phase 4 exit gate — routing proof release

- [ ] The complete routing-matrix scenario passes from a clean checkout with one
      documented command.
- [ ] The test requires no changes or Switchyard libraries inside fixture applications.
- [ ] The routing proof is reproducible on every initially supported host platform.
- [ ] Known limitations and the backend-group invariant are visible in CLI diagnostics
      and documentation.

## Phase 5 — Persistent control plane and SQLite state

Goal: promote the proven routing behavior into a recoverable local service without
changing the topology model.

Entry gate: the routing proof release is complete.

### Daemon and API

- [ ] Implement the long-running Switchyard control-plane daemon.
- [ ] Define a versioned HTTP API shared by the CLI and future GUI.
- [ ] Add Server-Sent Events for operations, builds, health, routes, and logs.
- [ ] Serialize mutations per deployment and enforce a global concurrency limit.
- [ ] Add cancellation and resumable observation for long-running operations.
- [ ] Move one-shot CLI behavior behind the API without removing script-friendly output.

### SQLite state

- [ ] Add versioned SQLite migrations and automatic backup before migration.
- [ ] Store the last applied resolved desired-state snapshot and definition hash.
- [ ] Store deployment, operation, resource, health, route, and snapshot history.
- [ ] Store locks and recover or expire abandoned operations safely.
- [ ] Keep secrets out of SQLite and persist references only.
- [ ] Reconcile SQLite records, generated manifests, and Docker ownership labels at
      daemon startup.
- [ ] Rebuild observed state when SQLite is deleted or restored from backup.
- [ ] Detect and report desired/applied/observed drift.

### Live router control

- [ ] Push versioned snapshots to host and sidecar routers.
- [ ] Require acknowledgement before marking a binding applied.
- [ ] Persist apply failure and rollback history.
- [ ] Expose current, previous, desired, and observed route versions.
- [ ] Add graceful connection-drain policy controls.

### Phase 5 exit gate

- [ ] Restarting the daemon preserves deployments, domains, bindings, and route history.
- [ ] Deleting SQLite allows safe observed-state recovery from manifests and Docker.
- [ ] CLI operations behave consistently through the daemon API.

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
