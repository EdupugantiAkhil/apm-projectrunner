# Switchyard: composable development deployments

Status: proposed design

Working name: **Switchyard**

Audience: developers testing combinations of services from monorepo worktrees and
independent Git repositories.

## 1. Purpose

Switchyard is a local-first deployment and topology orchestrator. It lets a developer
define reusable startup blocks, create multiple instances from different source trees,
combine providers into named service groups, and choose which group each consumer uses.

Existing application code must not require modification. A containerized consumer may
continue calling fixed dependency addresses such as `localhost:8001`; Switchyard routes
those calls to the selected provider group inside the consumer's isolated network
namespace.

Switchyard's core is solution-agnostic. Java, Python, JAS, UI, and database terminology
in this document describes the first reference fixture only. The runtime must work with
any executable, container image, repository layout, language, framework, protocol, or
service grouping that can satisfy the generic contracts below.

Example deployment:

- One shared database block.
- Five UI instances.
- Two Python suites, each containing five services (ten Python containers total).
- Three Java backend suites.
- A selectable route from each UI to one Java suite, one Python suite, and one database.

The system must support sources from:

- Worktrees in the same monorepo.
- Normal directories in the same monorepo.
- Existing checkouts of unrelated Git repositories.
- Optionally, repositories and worktrees created by Switchyard in a managed workspace.

## 2. Design principles

1. **Declare combinations, do not hand-edit Compose.** Human-authored block and
   deployment files are the source of truth. Generated Compose files are disposable.
2. **Treat a suite as a unit.** A Python suite can expand into five related services;
   duplicating the suite duplicates all five with consistent naming and configuration.
3. **Make routing explicit.** A UI never silently connects to whichever backend happens
   to be available. Its selected dependencies are visible and inspectable.
4. **Keep source separate from runtime.** A source identifies code; a block describes
   how to run it; an instance combines them for a deployment.
5. **Use containers as the first isolation boundary.** Phase 1 wraps every long-running
   instance in a container-backed network namespace. This makes repeated fixed ports
   safe and enables transparent loopback routing. Other execution adapters remain part
   of the product model but are deferred until this path is proven.
6. **Fail before mutation.** Validate paths, names, ports, route targets, dependency
   cycles, Dockerfiles, and required variables before starting a deployment.
7. **Local first, LAN optional.** `.localhost` is the safe default. mDNS LAN exposure is
   an explicit opt-in with visible security warnings.
8. **Examples are not product concepts.** No JAS path, service name, language, port, or
   environment variable may be hard-coded in the planner, runtime, API, CLI, or GUI.

## 2.1 Genericity boundary

The core understands only these concepts:

- A **source** supplies files.
- A **block** expands into components.
- An **execution adapter** starts and stops a component.
- A **lifecycle** describes preparation, readiness, and cleanup.
- A **capability** is something a component provides, identified by a user-defined name.
- A **slot** consumes a compatible capability.
- A **route adapter** connects a slot to a provider.
- A **service group** is a named, reusable selection of providers.
- A **binding** selects one service group for a consumer instance.
- A **probe** observes readiness or health.
- A **deployment** selects instances and connections.

Names such as `java`, `python`, `database`, and `ui` are ordinary user-defined
capabilities. Another solution could instead define `payments-api`, `message-broker`,
`firmware-simulator`, `model-server`, or `sap-gateway` without changing Switchyard.

Generic definitions are extended through versioned adapter interfaces rather than
conditionals in the core:

```text
SourceAdapter     path | git | worktree | future plugin
ExecutionAdapter  container | runner-script | host | future plugin
SupervisorAdapter process | process-compose | future plugin
RouteAdapter      http | tcp | environment | rendered-config | future plugin
ProbeAdapter      http | tcp | command | log-pattern | process | future plugin
```

Adapters publish JSON Schema for configuration and capabilities. The CLI validates that
schema, and the GUI renders controls from it. Adding an adapter must not require a custom
GUI screen for basic operation.

### Delivery staging

The final architecture includes the daemon/API, SQLite state and recovery, adapter SDK,
live route control, schema-driven GUI, and managed Git/worktrees. They are core product
capabilities, but they are not prerequisites for validating the routing model.

Phase 1 is a vertical routing proof built as a one-shot CLI over generated Compose,
Docker network namespaces, and the Switchyard Router in native-host and per-consumer
sidecar modes. The router, rather than Portless, is the authoritative routing layer.
Phase 2 promotes that proven behavior into the persistent control plane and GUI without
changing the human-authored topology model.

## 3. Domain model

### Source

A directory containing code to build or run.

```yaml
sources:
  monorepo-main:
    type: path
    path: /code/product

  backend-feature-a:
    type: worktree
    repository: /code/product
    path: /code/worktrees/backend-feature-a
    ref: feature/backend-a

  experimental-python:
    type: repository
    url: git@github.com:example/experimental-python.git
    ref: main
    managedPath: ~/.switchyard/sources/experimental-python
```

Initial releases should consume existing paths and worktrees. Managed clone, fetch, and
worktree creation should be added only after the execution model is stable.

### Block

A reusable startup definition. A block may contain one service or a coordinated suite.
Each service chooses one of three execution modes:

Phase 1 implements `container` and `script`. The `host` mode below is part of the Phase
2 adapter model and is not required for the routing proof.

- `container`: build a Dockerfile or run an existing image as a normal service.
- `script`: mount the selected source into a runner image and execute a declared command
  inside that container. This supports repositories that already have startup scripts
  without requiring a separate Dockerfile for every process.
- `host`: run a trusted command directly on the Docker host with an explicit working
  directory, environment, claimed ports, lifecycle, and shutdown behavior. This is
  required for existing scripts that depend on host Nix environments, credentials,
  worktrees, virtual environments, or Process Compose.

Container-backed block:

```yaml
apiVersion: switchyard.dev/v1alpha1
kind: Block
metadata:
  name: java-backend
spec:
  services:
    api:
      execution:
        type: container
        build:
          context: services/api
          dockerfile: Dockerfile
      healthcheck: /actuator/health
```

Script-backed block:

```yaml
apiVersion: switchyard.dev/v1alpha1
kind: Block
metadata:
  name: ui-dev-server
spec:
  services:
    ui:
      execution:
        type: script
        image: node:24-alpine
        workingDirectory: /workspace/ui
        command: ["npm", "run", "dev", "--", "--host", "0.0.0.0"]
        sourceMount: /workspace
        lifecycle: service
      healthcheck: /health
```

`command` is an argument array by default and does not invoke a shell. A block may opt
into a shell only when pipes, redirects, or other shell behavior are required. Script
lifecycles are:

- `service`: a long-running process that participates in health checks and routing.
- `task`: a one-shot command, such as compilation or migration, that must exit
  successfully before dependent services start.

A block may mix execution modes. For example, a Python suite may build two production-style
containers while starting three development services through scripts in Python runner
containers.

Phase 2 trusted host script:

```yaml
apiVersion: switchyard.dev/v1alpha1
kind: Block
metadata:
  name: jas-service
spec:
  trust: host-command
  services:
    jas:
      execution:
        type: host
        workingDirectory: /zfs/projects/FR/jasBase
        command: ["/zfs/projects/FR/jasBase/start-jas-service.sh"]
        environment:
          AUTONOMUS_IAM_ROOT: "${source.path}"
          JAS_RUNTIME_DIR: "${deployment.runtimeDir}/jas"
        lifecycle: service
        stopSignal: SIGTERM
        stopTimeout: 30s
        claimedPorts: [10081]
      healthcheck:
        http:
          url: http://127.0.0.1:10081/actuator/health
```

The command is an argument array and is spawned without an implicit shell. Absolute
paths are allowed. `${source.path}` resolves to the selected repository or worktree, so
the same block definition can start JAS from different worktrees without editing the
script.

Process Compose suite:

```yaml
apiVersion: switchyard.dev/v1alpha1
kind: Block
metadata:
  name: ai-services
spec:
  trust: host-command
  services:
    suite:
      execution:
        type: host
        adapter: process-compose
        workingDirectory: /zfs/projects/FR/jasBase
        command:
          - process-compose
          - --ordered-shutdown
          - --no-server
          - -t=false
          - -f
          - ai-services.process-compose.yaml
          - up
        environment:
          AUTONOMUS_IAM_ROOT: "${source.path}"
          AI_SERVICES_ROOT: "${source.path}/helix/ai-services"
        lifecycle: service
        stopSignal: SIGTERM
        stopTimeout: 45s
        claimedPorts: [8001, 8002, 8003, 8004, 8006]
```

The `process-compose` adapter treats the command as one block instance while importing
its child-process names, dependency states, readiness probes, and logs into Switchyard.
Process Compose remains responsible for its internal startup and ordered shutdown.

Host commands run in a new process group. Switchyard sends the declared stop signal to
the group, waits for the timeout, and only then escalates. It records the PID, executable,
working directory, definition hash, start time, and child processes so it never stops an
unrelated process that happens to reuse a port.

```yaml
apiVersion: switchyard.dev/v1alpha1
kind: Block
metadata:
  name: python-suite
spec:
  parameters:
    DATABASE_URL:
      required: true
  services:
    ingest:
      execution:
        type: container
        build:
          context: services/ingest
          dockerfile: Dockerfile
      healthcheck: /health
    analysis:
      execution:
        type: script
        image: python:3.13-slim
        workingDirectory: /workspace/services/analysis
        command: ["./start-dev.sh"]
        sourceMount: /workspace
        lifecycle: service
      healthcheck: /health
    reports:
      context: services/reports
      dockerfile: Dockerfile
      healthcheck: /health
    scheduler:
      context: services/scheduler
      dockerfile: Dockerfile
      healthcheck: /health
    worker:
      context: services/worker
      dockerfile: Dockerfile
      healthcheck: /health
```

Example block categories in the reference fixture:

- `database`: PostgreSQL or another stateful dependency with named volumes.
- `java-backend`: one or more Java services built from a selected source.
- `python-suite`: a coordinated group of Python services.
- `ui`: a browser application with selectable upstream routes.
- `generic`: any other component or coordinated suite.

These categories are tags and templates, not a closed enum. Users can create any block
name and any number of components. Switchyard does not branch on these values.

Execution mode is independent of block type: Java, Python, UI, and generic blocks may
use containers, containerized scripts, or explicitly trusted host commands.

### Host resource claims

Host commands do not receive Docker network isolation. Their definitions must declare
ports, writable directories, and exclusive resources. Planning fails when two instances
claim the same resource.

The current JAS and AI Process Compose scripts use fixed ports. Consequently, multiple
copies cannot run on the same host unchanged. Before Switchyard starts two copies, one of
the following must be true:

- The scripts and Process Compose file accept per-instance port parameters.
- Switchyard renders a per-instance Process Compose file with unique ports and matching
  dependency URLs.
- Each copy moves into its own container or network namespace.

Switchyard must never silently offset ports because service-to-service URLs may be
embedded in scripts, environment files, or application configuration.

### Adapter contracts

Every execution adapter implements the same control contract:

```text
validate(context) → diagnostics
plan(context)     → resources + commands + claims
prepare(context)  → operation events
start(context)    → runtime handle
inspect(handle)   → observed state
logs(handle)      → stream
stop(handle)      → operation events
cleanup(handle)   → operation events
recover(labels)   → runtime handle or diagnostic
```

Runtime handles are opaque to the core and serializable for recovery. Adapters must emit
normalized state and events so a host process, Compose suite, container, or future
runtime looks consistent to the GUI.

Route adapters implement:

```text
validate(consumer, slot, provider) → diagnostics
plan(connection)                   → live | restart | rebuild
apply(connection)                  → route handle
remove(route handle)
inspect(route handle)              → observed target
```

Capabilities and slots carry protocol metadata without assuming HTTP:

```yaml
provides:
  query-api:
    protocol: http
    endpoint: "http://${runtime.host}:${runtime.port}"
  event-stream:
    protocol: kafka
    endpoint: "${runtime.bootstrapServers}"
consumes:
  primary-query:
    accepts: [query-api]
    routeAdapter: environment
    binding:
      variable: QUERY_API_URL
```

### Reference fixture: JAS legacy deployment

The parent workspace provides the first real integration fixture. It deliberately mixes
execution mechanisms, but none of its details belong in the product core:

| Block | Current entry point | Phase 1 treatment |
|---|---|---|
| JAS databases | decomposed commands from `/zfs/projects/FR/jasBase/start-local-jas.sh` | database containers plus runner-container tasks |
| Java JAS service | `/zfs/projects/FR/jasBase/start-jas-service.sh` | runner image containing the required Nix/Gradle tooling |
| Python AI suite | `process-compose -f ai-services.process-compose.yaml up` in `/zfs/projects/FR/jasBase` | Process Compose inside an AI runner container |
| UI | Worktree-specific Dockerfile or runner script | Container or containerized script |

The desired topology is expressed through typed slots rather than being embedded in the
startup mechanism:

```text
ui-a ──java────► jas-main ──database──► db-main
  │                 │
  └──python─────────┴───────────────► ai-feature

ui-b ──java────► jas-feature ─database► db-main
  │                 │
  └──python─────────┴───────────────► ai-main
```

The routing layer owns the selections. A block does not need to know whether its target
uses an image-backed container, runner script, or supervised Process Compose suite.

All files for this integration belong under `examples/jas-base/` as ordinary source,
block, adapter-configuration, and deployment definitions. Automated tests must prove
that the generic planner produces the expected result without importing a JAS-specific
module.

Example deployment intent:

```yaml
instances:
  - name: db-main
    block: jas-databases
    source: monorepo-main
  - name: jas-main
    block: jas-service
    source: monorepo-main
  - name: jas-feature
    block: jas-service
    source: jas-feature-worktree
  - name: ai-main
    block: ai-services
    source: monorepo-main
  - name: ai-feature
    block: ai-services
    source: ai-feature-worktree
  - name: ui-a
    block: ui
    source: ui-feature-a
  - name: ui-b
    block: ui
    source: ui-feature-b

routes:
  ui-a: { java: jas-main, python: ai-feature }
  ui-b: { java: jas-feature, python: ai-main }
  jas-main: { database: db-main, python: ai-feature }
  jas-feature: { database: db-main, python: ai-main }
```

#### Required decomposition of `start-local-jas.sh`

The current script performs several lifecycle phases in one command:

1. Stops and removes globally named `jas-cassandra`, `jas-mongo`, and `jas-elastic`
   containers and the `autonomous` network.
2. Starts database containers and OpenSearch through Docker Compose.
3. Initializes Cassandra and MongoDB schemas.
4. Waits for JAS itself on port `10081`.
5. Creates and reads the `autoid` tenant.

It therefore cannot be treated as a reusable database-only start command unchanged: it
waits for a downstream Java service and mutates globally named Docker resources. The
integration should expose these as explicit hooks:

```yaml
services:
  databases:
    execution:
      type: host
      lifecycle: external-resources
      startCommand: ["./start-jas-databases.sh"]
      readyWhen:
        - tcp: { host: 127.0.0.1, port: "${ports.cassandra}" }
        - http: { url: "http://127.0.0.1:${ports.opensearch}" }
      cleanupCommand: ["./stop-jas-databases.sh"]
      ownedResources:
        dockerProject: "switchyard-${deployment.name}-${instance.name}"
  initialize-tenant:
    execution:
      type: host
      lifecycle: task
      command: ["./initialize-jas-tenant.sh"]
    dependsOn:
      databases: ready
      "${routes.java}": ready
```

The first implementation may wrap the legacy script for a single instance, with a clear
warning. Supporting multiple database instances requires parameterized Compose project
names, container names, networks, volumes, and ports. Supporting multiple Java and Python
instances likewise requires their fixed ports and internal URLs to become instance
parameters.

### Lifecycle hooks and external resources

Blocks may need commands at more than one phase:

- `prepare`: validate credentials, build virtual environments, or render configuration.
- `start`: launch a long-running process or create Docker resources.
- `ready`: probe the resources or process.
- `postReady`: initialize schemas or seed data after dependencies are ready.
- `stop`: terminate the supervised process gracefully.
- `cleanup`: remove resources explicitly owned by the instance.

An `external-resources` lifecycle covers a host command that exits after starting Docker
containers or other background resources. It must declare how readiness is detected,
how ownership is verified, and how cleanup occurs. A successful start-command exit alone
does not mean the block is ready.

### Instance

A concrete copy of a block in a deployment. Each instance selects a source and supplies
parameters.

```yaml
instances:
  - name: python-main
    block: python-suite
    source: monorepo-main
    parameters:
      LOG_LEVEL: info

  - name: python-feature
    block: python-suite
    source: experimental-python
    parameters:
      LOG_LEVEL: debug
```

Every expanded service name is namespaced:

```text
<deployment>--<instance>--<service>
comparison--python-feature--analysis
```

### Route

A typed connection from a consumer slot to a provider instance or service.

```yaml
routes:
  ui-main:
    java: backend-main
    python: python-main
    database: shared-db

  ui-feature:
    java: backend-feature-a
    python: python-feature
    database: shared-db
```

Blocks declare the slots they consume and capabilities they provide. Validation rejects
invalid connections, such as routing a `database` slot to a UI.

Slots may also declare the address already used by an unchanged application:

```yaml
consumes:
  ai-ingest:
    accepts: [ai-ingest]
    routeAdapter: loopback-proxy
    address: { host: 127.0.0.1, port: 8001 }
  ai-analysis:
    accepts: [ai-analysis]
    routeAdapter: loopback-proxy
    address: { host: 127.0.0.1, port: 8002 }
```

The address is the consumer contract, not the provider's runtime address. A proxy
sidecar sharing the consumer's container network namespace binds the declared loopback
ports and forwards them to the selected providers.

### Service group and binding

A service group selects providers as one reusable unit. It can inherit a shared baseline
and replace only selected services.

```yaml
groups:
  ai-main:
    providers:
      ai-ingest: ai-main/ingest
      ai-analysis: ai-main/analysis
      ai-reports: ai-main/reports
      ai-scheduler: ai-main/scheduler
      ai-worker: ai-main/worker

  ai-feature:
    extends: ai-main
    providers:
      ai-analysis: ai-feature/analysis
      ai-reports: ai-feature/reports
```

A binding selects the group used by one consumer:

```yaml
bindings:
  jas-main: ai-main
  jas-feature: ai-feature
```

The resolved topology still consists of ordinary typed routes. Group switching replaces
the consumer's complete route table as one operation; partial group application is
invalid.

### Deployment

The desired combination of sources, instances, parameters, groups, bindings, and any
direct route overrides.

```yaml
apiVersion: switchyard.dev/v1alpha1
kind: Deployment
metadata:
  name: comparison
spec:
  overlays:
    - overlays/development.yaml
    - overlays/mongodb.yaml
  instances:
    - { name: shared-db, block: postgres, source: infrastructure }
    - { name: backend-main, block: java-backend, source: monorepo-main }
    - { name: backend-a, block: java-backend, source: backend-feature-a }
    - { name: backend-b, block: java-backend, source: backend-feature-b }
    - { name: python-main, block: python-suite, source: monorepo-main }
    - { name: python-feature, block: python-suite, source: experimental-python }
    - { name: ui-1, block: ui, source: monorepo-main }
    - { name: ui-2, block: ui, source: ui-feature-a }
    - { name: ui-3, block: ui, source: ui-feature-b }
    - { name: ui-4, block: ui, source: ui-feature-c }
    - { name: ui-5, block: ui, source: ui-feature-d }
  routes:
    ui-1: { java: backend-main, python: python-main, database: shared-db }
    ui-2: { java: backend-a, python: python-feature, database: shared-db }
```

### Overlay

An overlay creates a product variation without copying its block or deployment
definition. It may inject environment values and files, override declared parameters,
and select routes for matching deployment instances.

```yaml
apiVersion: switchyard.dev/v1alpha1
kind: Overlay
metadata:
  name: mongodb-development
spec:
  selectors:
    instances:
      matchLabels:
        product: identity
  environment:
    envFiles:
      - ./env/common.env
      - ./env/mongodb.env
    set:
      EPS_DB_SOURCE: mongodb
      LOG_LEVEL: DEBUG
    unset:
      - LEGACY_DATABASE_URL
  files:
    - source: ./config/application-mongodb.yml
      target: /runtime/config/application.yml
      mode: "0644"
    - content: |
        featureFlags:
          newSearch: ${overlay.variables.enableNewSearch}
      target: /runtime/config/features.yml
      template: true
      mode: "0644"
  parameters:
    migrationPolicy: isolated-database
  routes:
    database: mongodb-main
```

Overlay selectors may target deployment labels, block instances, expanded components,
or capability consumers. A selector must match at least one target unless explicitly
marked optional; misspelled selectors must not silently do nothing.

#### Composition and precedence

Overlays are applied in listed order. Later layers win for scalar values:

```text
adapter defaults
  < block defaults
  < deployment overlays, in order
  < deployment instance values
  < explicitly named ephemeral CLI overrides
```

Maps merge by key. `unset` removes an inherited environment key. File targets and route
slots must be unique after resolution unless a later overlay explicitly declares
`replace: true`. Lists do not merge implicitly; each schema declares whether a list is
replace-only, appendable, or keyed.

Switchyard must render and display the fully resolved deployment and an origin trace for
every value:

```text
LOG_LEVEL=DEBUG  ← overlays/mongodb.yaml
DATABASE_URL=…   ← deployment instance ui-a
PORT=8001        ← block default ai-services
```

#### File injection

Injected files never modify a source repository or worktree by default. Switchyard
materializes them under:

```text
.switchyard/generated/<deployment>/overlays/<instance>/<content-hash>/
```

Execution adapters decide how the materialized file is presented:

- Container and runner-script adapters bind-mount it at the declared target.
- Host-command adapters receive the generated path through a declared environment or
  command argument binding.
- A `materialized-workspace` adapter may create a disposable copy-on-write workspace
  when a legacy tool requires configuration at a source-relative path.

Direct writes into unmanaged worktrees require a separate unsafe mode and are outside
the MVP. File content participates in the plan hash, so changes reliably cause the
adapter-declared action: live reload, restart, or rebuild.

File sources may be:

- A repository-relative or deployment-relative path.
- Inline text for small non-secret configuration.
- A template using a restricted, non-executable expression language.
- A secret reference resolved at apply time.

The template engine must not execute shell commands or arbitrary JavaScript.

#### Overlay portability

Committed overlays contain portable configuration and secret references. Machine-local
values belong in ignored overlays such as `overlays/local.user.yaml`. Absolute paths are
allowed only for explicitly host-bound overlays and are reported by portability checks.

The same base deployment can therefore produce named variations:

```text
comparison + development + mongodb
comparison + development + cassandra
comparison + auth-enabled + feature-a
comparison + performance + production-like-data
```

Variations receive distinct resolved hashes and may run concurrently when their resource
claims do not collide.

## 4. System architecture

```text
 Browser / CLI / GUI
          │
          ▼
 ┌─────────────────────────────────────────────────────────┐
 │ Native Switchyard Router                               │
 │ custom domains + TLS + legacy localhost listeners      │
 │ Origin/header/profile identity + CORS/preflight         │
 └──────────────────────────┬──────────────────────────────┘
                            │ loopback-only published ports
                            ▼
 ┌─────────────────────────────────────────────────────────┐
 │ Docker Engine: one private bridge network per deployment│
 │ UI instances     backend instances       service groups │
 │                         │                               │
 │                         ▼                               │
 │              Switchyard Router sidecar                  │
 │              shared consumer network namespace          │
 │              owns localhost:8001, ...                   │
 └──────────────────────────┬──────────────────────────────┘
                            │
                            ▼
             selected providers / shared services

 CLI / Web GUI ──HTTP+SSE──► Switchyard control plane
                              ├── planner + Compose generator
                              ├── router configuration
                              ├── Git/worktrees
                              └── SQLite
```

### Runtime and isolation

Docker Engine is the Phase 1 container runtime. Switchyard generates Docker Compose as
an internal lifecycle artifact; users do not have to author Compose and the domain model
does not depend on it. Every deployment receives a private Docker bridge network.
Provider instances receive deterministic internal DNS aliases, while host exposure is
loopback-only.

Every application instance runs in a container and therefore has its own Linux network
namespace. A router sidecar uses `network_mode: service:<consumer>` to join the exact
same namespace as its consumer. The sidecar can consequently bind
`127.0.0.1:8001` for one backend while another sidecar independently binds the same
address for another backend. No application-code or address changes are required.

The host router runs as a native process. Browser `localhost` refers to the developer
host, and native execution gives consistent access to host listeners and Docker's
loopback-published ports on Linux, macOS, and Windows. A Linux-only host-network
container may be offered later, but is not the portable default.

The initial stack is therefore:

| Concern | Phase 1 choice |
| --- | --- |
| Container lifecycle | Docker Engine through generated Docker Compose |
| Container isolation | Docker-provided Linux network namespaces |
| Internal fixed-port routing | Switchyard Router sidecars |
| Browser, custom-domain, and TLS routing | Native Switchyard Router |
| Desired state | Versioned YAML |
| Observed/control state | Generated manifests and Docker labels; SQLite in Phase 2 |
| Application data | Docker named volumes or explicitly declared external services |

Runtime adapters for Podman, Kubernetes, containerd, or Nomad may be added later. They
must preserve the same isolation and routing contracts and are not required by the core
model.

### Control plane

A long-running local process owns deployment operations and exposes an API used by both
the CLI and GUI. Only one operation may mutate a deployment at a time. Other deployments
may build or start concurrently within a configurable concurrency limit.

This is the Phase 2 product shape. Phase 1 uses the same planner in one-shot CLI mode,
writes generated manifests, and derives observed state from Docker labels. This avoids
building persistence and concurrency machinery before the routing approach is proven.

Recommended implementation:

- A Rust workspace for the router and its shared route/configuration types.
- [Cloudflare Pingora](https://github.com/cloudflare/pingora) for programmable HTTP/1,
  HTTP/2, TLS, gRPC, WebSocket proxying, and graceful reload behavior.
- Tokio listeners for raw TCP forwarding where HTTP semantics are unavailable.
- The control API may initially remain a separate TypeScript service; it communicates
  with routers only through the versioned configuration contract.
- SQLite for runtime metadata, locks, operation history, and GUI preferences.
- Server-Sent Events for build output, logs, health changes, and operation progress.
- Docker Compose CLI as the first runtime adapter; Docker Engine API can follow later.

### Compose generator

The generator expands every block instance into concrete Compose services. It assigns:

- Deterministic image, service, network, and volume names.
- Unique internal DNS names.
- Health checks and dependency conditions.
- Source-specific build contexts.
- Deployment and instance labels for discovery and cleanup.
- Ephemeral loopback-only host ports used as native-router upstreams.
- Read-only or read-write source mounts for script runners, as explicitly declared.
- One-shot dependency conditions for successful `task` scripts.
- One Switchyard Router sidecar for each consumer with loopback-proxy slots. It uses
  `network_mode: service:<consumer>` so it shares that consumer's isolated localhost.

Host commands are not emitted as Compose services. The control plane supervises them in
parallel with the Compose runtime and includes them in the same dependency graph.

Compose `--scale` must not be used for instances with different sources or parameters.
They must be separate generated services.

Generated output belongs under:

```text
.switchyard/generated/<deployment>/compose.yaml
.switchyard/generated/<deployment>/resolved-deployment.yaml
.switchyard/generated/<deployment>/manifest.json
.switchyard/generated/<deployment>/routes/<consumer>.cfg
```

Only human-authored definitions are committed. `.switchyard/generated` is ignored.

### Switchyard Router

One Rust codebase provides three modes with the same configuration and route-table
semantics:

1. **Host gateway** owns custom local domains, TLS, browser-facing legacy localhost
   ports, CORS/preflight handling, and routes to loopback-published container ports.
2. **Container sidecar** shares a consumer's network namespace, binds fixed addresses
   such as `127.0.0.1:8001`, and forwards to providers on the deployment network.
3. **Forward proxy** gives a managed browser profile an explicit routing identity when
   neither a request header nor `Origin` is sufficient.

Route tables are validated as complete immutable snapshots and swapped atomically.
Updates never expose a partially changed five-service group. HTTP connections can drain
under the previous snapshot; new connections use the new snapshot. Raw TCP routes have
an explicit close, drain, or pin policy. The router also owns health checks, structured
access logs, and route inspection. It must never silently choose a target for an
ambiguous request.

Portless was useful for the original hostname proof-of-concept but is not part of the
authoritative runtime. It cannot provide consumer-specific browser identity and
container-local fixed-port routing under one configuration contract.

Ingress names are desired state, not transient CLI output:

```yaml
ingress:
  ui-a:
    instance: ui-a
    domain: ui-a.comparison.localhost
  ui-b:
    instance: ui-b
    domain: feature-b.product.test
```

Phase 1 persists these declarations in deployment YAML and generated manifests. Phase 2
also records their applied and observed state in SQLite for recovery and the GUI.

Example:

```text
jas-main requests 127.0.0.1:8001
                           │
                           └──► comparison--ai-feature--ingest

jas-feature requests 127.0.0.1:8001
                              │
                              └──► comparison--ai-main--ingest
```

In Phase 1, changing a binding renders and validates a complete replacement router
configuration, then atomically reloads only the router sidecar. The application
container is not restarted; applying slots one at a time is forbidden.

Phase 2 adds versioned live route snapshots, acknowledgements, history in SQLite, and
graceful connection policies. The route plan states whether existing connections drain,
close, or remain pinned while new connections use the new group.

### Browser routing identity

Browser JavaScript calling `localhost:<port>` connects to the native host router, not
the UI container. The router selects a backend using this precedence:

1. `X-Switchyard-Route`, injected per tab by the optional Switchyard browser extension.
2. The request's `Origin`, mapped from the UI's custom domain.
3. The identity of a dedicated forward-proxy listener used by a managed browser profile.

For example, all three unchanged UIs may call `http://localhost:10081` while the router
uses their origins to select a backend:

```text
Origin: https://ui-1.comparison.localhost ──► backend-1
Origin: https://ui-2.comparison.localhost ──► backend-2
Origin: https://ui-3.comparison.localhost ──► backend-1
```

```yaml
browserRoutes:
  - origin: https://ui-1.comparison.localhost
    destination: http://localhost:10081
    provider: backend-1
  - origin: https://ui-2.comparison.localhost
    destination: http://localhost:10081
    provider: backend-2
  - origin: https://ui-3.comparison.localhost
    destination: http://localhost:10081
    provider: backend-1
```

The host gateway answers preflight requests and adds narrowly scoped CORS response
headers for configured UI origins. The [Fetch standard](https://fetch.spec.whatwg.org/)
defines the `Origin` behavior used by this mode. Requests that lack usable identity are
rejected with a diagnostic response instead of being routed arbitrarily.

The extension can associate routing rules with tabs and attach the explicit header
without application changes; see Chrome's
[declarative request API](https://developer.chrome.com/docs/extensions/reference/api/declarativeNetRequest).
For an extension-free guaranteed mode, `switchyard open <ui>` launches an isolated
browser profile with `--proxy-server=<listener>` and
`--proxy-bypass-list=<-loopback>`, as supported by
[Chromium's proxy configuration](https://chromium.googlesource.com/chromium/src/+/HEAD/net/docs/proxy.md).

### Downstream group invariant

A backend instance has exactly one selected downstream group at a time. Two UIs can
share a backend only when they also share that backend's downstream group. If two UIs
need the same backend source with different groups, Switchyard runs two backend
instances:

```text
ui-1 ──► backend-1a ──► group-a
ui-3 ──► backend-1b ──► group-b
```

Without application-level context propagation, a single backend cannot associate an
outbound `localhost:8001` connection with the inbound UI request that caused it. This is
a fundamental boundary, not a router implementation limitation.

### State and ownership

Deployment YAML remains the portable, reviewable source of desired state. SQLite stores
the last applied resolved snapshot as well as observations, but is never the only copy
of user intent. Docker labels allow runtime recovery if the database is deleted.

SQLite is introduced in Phase 2. Phase 1 preserves the same stable deployment and
operation identifiers in generated manifests and Docker labels so the database can be
added without changing resource identity.

The control plane records:

- Deployment state and last applied definition hash.
- Last applied resolved desired-state snapshot.
- Container, image, network, and volume identifiers.
- Current dynamic route table.
- Build/start/stop operation history.
- Source and worktree observations.
- Health and readiness history.

## 5. GUI design

### Product subject and primary job

The GUI is a developer's deployment switchyard. Its primary job is to answer and change:
**“Which exact source-backed instances are connected right now?”**

It is an operational tool, not a generic admin dashboard. The main view should resemble
a disciplined physical patch bay: service instances sit in typed lanes and visible
cables connect consumers to providers.

### Visual direction

The interface takes cues from lab equipment and rack labels without imitating a terminal.
It should feel precise, inspectable, and calm under high information density.

Color tokens:

| Token | Value | Use |
|---|---:|---|
| Bench | `#E8E7E1` | Main work surface |
| Panel | `#F7F6F1` | Cards and inspectors |
| Ink | `#182027` | Primary text and structure |
| Cobalt | `#2457D6` | Java routes and primary actions |
| Violet | `#7651C9` | Python suite routes |
| Copper | `#B25C32` | Database routes and destructive warnings |
| Signal | `#15805D` | Healthy and ready states |

Typography:

- Display and navigation: **Space Grotesk**, compact and technical without looking like
  a code editor.
- Body and controls: **IBM Plex Sans**, optimized for dense operational interfaces.
- Identifiers, refs, ports, and logs: **IBM Plex Mono**.

Layout:

- A narrow deployment rail on the left.
- A large route canvas in the center.
- A contextual inspector on the right.
- A collapsible event and log drawer along the bottom.

Signature element: the **live patch bay**. Route cables are the only visually bold
gesture. They use capability colors, clear direction, and restrained motion when a route
changes. Everything else remains flat, quiet, and compact.

This avoids a generic grid of statistic cards: counts matter less than topology and
source identity in this product.

### Main deployment view

```text
┌──────────────┬────────────────────────────────────┬───────────────────┐
│ DEPLOYMENTS  │ comparison                 Running │ INSTANCE INSPECTOR│
│              │                                    │                   │
│ ● comparison │ UI CONSUMERS       PROVIDERS       │ ui-feature        │
│ ○ regression │                                    │ feature/ui-redesign│
│ ○ clean-main │ ┌────────────┐   ┌──────────────┐  │ /worktrees/ui-a   │
│              │ │ ui-main    ├──►│ backend-main │  │                   │
│ + Deployment │ │ main@9ca21 │╲  └──────────────┘  │ Routes            │
│              │ └────────────┘ ╲ ┌──────────────┐  │ Java   backend-a  │
│ SOURCES      │                 ├►│ python-main  │  │ Python python-a   │
│ 8 clean      │ ┌────────────┐  │ └──────────────┘  │ DB      shared-db  │
│ 2 modified   │ │ ui-feature ├──┼► backend-a       │                   │
│ 1 missing    │ │ feat@35ad2 │  ├► python-a        │ [Apply routes]    │
│              │ └────────────┘  └► shared-db       │ [Open] [Logs]     │
├──────────────┴────────────────────────────────────┴───────────────────┤
│ EVENTS  Build completed: python-a/analysis                     ▴     │
└──────────────────────────────────────────────────────────────────────┘
```

Interaction rules:

- Selecting an instance opens its source, commit, health, environment, routes, and logs.
- Dragging a cable to another compatible socket prepares a route change; it does not
  apply until the user confirms the route diff.
- Incompatible sockets do not accept the cable and explain the capability mismatch.
- Modified worktrees are visible before build and require acknowledgment.
- Each route displays both the friendly instance name and abbreviated commit.
- Keyboard users can change routes through select controls in the inspector; dragging is
  never the only interaction.
- Reduced-motion mode replaces cable animation with an immediate color/state change.

### Additional screens

#### Deployment builder

- Choose a saved template or start empty.
- Add block instances from a searchable library.
- Select a source and worktree for each instance.
- Set parameters using block-provided field definitions.
- Connect required slots and validate the graph continuously.
- Preview the expanded service and resource count before starting.

#### Overlay editor and variation comparison

- Add, remove, and reorder overlays on a deployment.
- Edit schema-approved environment values, dotenv inputs, file injections, parameters,
  and route selections.
- Show the origin of every resolved value and a warning when a later overlay shadows it.
- Compare two variations side by side across source refs, environment keys, injected
  file hashes, routes, images, ports, and resource claims.
- Preview injected text files with secrets redacted and binary files as metadata only.
- Show whether each change applies live or requires a restart or rebuild before apply.

#### Sources and worktrees

- Repository, worktree path, branch, commit, dirty state, ahead/behind state.
- Actions: inspect, refresh, open directory, create worktree, remove managed worktree.
- Destructive Git actions are excluded. Switchyard never resets or discards changes.

#### Block library

- Block description, capabilities, consumed slots, expanded services, parameters, and
  health contract.
- Validate a block against a selected source without starting it.
- Show the generated service preview.
- Identify whether each service is Dockerfile/image-backed or a containerized script,
  including its runner image, command, mounts, and lifecycle.
- Mark host-command blocks with a persistent `Runs on host` label and show the exact
  command, working directory, environment names, resource claims, and trust status.
- For Process Compose blocks, show imported child processes and dependency/readiness
  relationships rather than presenting the suite as an opaque command.
- Render adapter-specific fields from the adapter's JSON Schema. The web application
  must not contain forms hard-coded for JAS, Java, Python, or a fixed set of block types.

#### Operations and logs

- One timeline for validation, build, start, readiness, route changes, and stop.
- Filter logs by deployment, block instance, or expanded service.
- Preserve ANSI colors where accessible and provide plain-text copying.
- Errors state the failed command, affected service, exit code, and suggested recovery.

### Responsive behavior

The full patch bay targets desktop widths of 1280 px and above. At smaller widths:

- The deployment rail becomes a drawer.
- The inspector becomes a full-height sheet.
- The canvas switches to a route matrix rather than squeezing cables into a narrow view.
- Mobile supports observation, logs, start/stop, and simple route changes; complex
  deployment construction remains a desktop task.

### Accessibility

- Meet WCAG 2.2 AA contrast and focus visibility.
- Never encode service type or health by color alone; use labels and icons.
- Provide a table representation of every route graph.
- Announce build and health changes through a restrained live region.
- Preserve complete keyboard operation and logical focus after route changes.

## 6. CLI and API

Proposed CLI:

```text
switchyard validate <deployment>
switchyard plan <deployment>
switchyard overlay validate <overlay>
switchyard overlay diff <deployment> --with <overlay...>
switchyard build <deployment> [--instance <name>]
switchyard up <deployment>
switchyard status [deployment]
switchyard group list <deployment>
switchyard bind <consumer> <group>
switchyard route set <consumer> <slot> <provider>
switchyard logs <deployment> [instance[/service]]
switchyard open <instance>
switchyard down <deployment> [--volumes]
switchyard source list
switchyard worktree create <repository> <ref> <path>
switchyard gui
```

The CLI calls the same API as the GUI. It must also support a one-shot mode for CI and
recovery when the daemon is not running.

Initial API groups:

```text
/api/deployments
/api/deployments/:name/plan
/api/deployments/:name/operations
/api/deployments/:name/groups
/api/deployments/:name/bindings
/api/deployments/:name/routes
/api/deployments/:name/events
/api/blocks
/api/sources
/api/worktrees
/api/runtime
```

Mutating requests use operation IDs and idempotency keys. Long operations return
immediately and stream progress separately.

## 7. Lifecycle

### Plan

1. Parse block, source, and deployment definitions.
2. Resolve paths and Git identities.
3. Validate required capabilities and route slots.
4. Calculate expanded services, images, networks, volumes, and hostnames.
5. Detect conflicts and show a deterministic diff against the active deployment.

### Apply

1. Acquire the deployment mutation lock.
2. Write generated artifacts atomically.
3. Build changed images with bounded concurrency.
4. Start stateful dependencies and wait for readiness.
5. Start provider suites and wait for readiness.
6. Apply the internal route table.
7. Start consumers and register ingress hostnames.
8. Stream the final state and release the lock.

### Stop and cleanup

Stopping preserves named volumes by default. Deleting volumes, images, managed
worktrees, or clones requires separate explicit actions. Cleanup operates only on
resources carrying matching Switchyard ownership labels.

## 8. Security and safety

- Bind the control API and local ingress to loopback by default.
- LAN mode requires an explicit deployment setting and displays the exposed interfaces.
- Do not store secrets in deployment YAML or SQLite; reference environment files or a
  pluggable secret provider.
- Redact declared secrets from plans, logs, errors, and generated manifests.
- Do not copy secret overlay values into resolved manifests or content-addressed overlay
  directories. Materialize them at apply time with restrictive permissions and remove
  them during cleanup.
- Ignore machine-local overlays by default and warn before committing a file that appears
  to contain credentials.
- Restrict file targets to adapter-declared mount roots and reject traversal through
  `..`, symlinks, or absolute targets outside those roots.
- Treat Docker access as host-level authority and state this during setup.
- Show every host command before first execution and record it in operation history.
- Never infer host execution from a script path. It requires `execution.type: host` and
  `spec.trust: host-command` in the block definition.
- Require per-block trust approval before the first host execution and again whenever
  the command, working directory, or source-controlled script content hash changes.
- Host-command environment allowlists must prevent accidental inheritance of unrelated
  credentials. Secret values are referenced, not copied into generated definitions.
- Containerized script blocks continue to run in runner containers with declared mounts,
  environment, user, and resource limits.
- Default script source mounts to read-only. Require an explicit writable mount for
  compilers or development servers that create artifacts.
- Run script containers as a non-root container user unless the block explicitly
  declares and justifies another user.
- Never reset, clean, checkout, delete, or modify an unmanaged worktree.
- Validate bind mounts so a block cannot accidentally mount broad host paths.
- Require confirmation before exposing databases or internal providers to the LAN.
- Refuse to stop or remove externally created Docker resources unless their labels and
  recorded identifiers prove that the current block instance owns them.

## 9. Database compatibility

Sharing a database between branches is risky when schemas differ. Each database block
must declare a migration policy:

- `none`: Switchyard never runs migrations.
- `owner`: exactly one selected instance owns migrations.
- `isolated-schema`: each consumer set receives a separate schema in one server.
- `isolated-database`: each consumer set receives a separate database.

The plan must warn when multiple instances claim migration ownership or when branches
with different migration fingerprints share a schema.

## 10. Remote access

Local mode uses custom `*.localhost` names through the native Switchyard Router. The
router may use an unprivileged HTTP port by default or a locally trusted certificate and
platform-specific privileged-port setup for HTTPS. Domain and certificate ownership is
explicit desired state and can be inspected from the CLI and GUI.

Optional LAN mode uses `*.local` and mDNS:

- A future publication adapter advertises Switchyard gateway instance names.
- The configured gateway TCP port and mDNS UDP port `5353` must be permitted.
- Linux requires `avahi-publish-address` from `avahi-utils`.
- The GUI shows whether mDNS publication and remote reachability checks pass.
- mDNS is not assumed to cross subnets, VLANs, VPNs, or guest Wi-Fi isolation.

Cross-network access is a later adapter using normal DNS, Tailscale, or another private
network. It must not silently expose deployments to the public internet.

## 11. Observability

Every expanded service reports one of:

```text
unbuilt → building → starting → ready
                     └────────→ unhealthy
          └───────────────────→ failed
ready → stopping → stopped
```

The GUI and CLI expose:

- Build progress and cache use.
- Container health and restart count.
- Source commit and dirty state.
- Active routes and route history.
- CPU and memory where Docker provides them.
- Structured operation events and raw container logs.

## 12. Repository layout

Proposed layout:

```text
Cargo.toml                    Rust workspace for routing components
blocks/                       reusable block definitions
deployments/                  saved deployment definitions
crates/
  router-core/                route identity, matching, snapshots, and policy
  router-pingora/             HTTP/TLS/gRPC/WebSocket gateway implementation
  router-tcp/                 Tokio raw TCP forwarding
  router-config/              versioned router configuration protocol
packages/
  core/                       schemas, planner, naming, validation
  compose-runtime/            Compose generation and execution
  source-manager/             Git and worktree inspection
  server/                     API, state, operations, event streams
  cli/                        command-line client
  web/                        React GUI
  adapter-sdk/                public adapter contracts and schema helpers
adapters/
  source-path/                local directory sources
  source-git/                 repositories and worktrees
  execution-container/        image and Dockerfile components
  execution-runner-script/    scripts isolated in runner containers
  execution-host/             explicitly trusted host commands
  supervisor-process-compose/ Process Compose inspection and lifecycle
  route-switchyard/           native gateway and sidecar lifecycle
  route-binding/              environment and rendered-config bindings
examples/
  routing-matrix/             3 UIs, 2 backends, and switchable service groups
  jas-base/                   containerized legacy parent-workspace fixture
old/
  shared-database-portless-demo/ archived hostname/database proof-of-concept
scripts/                      bootstrap and development scripts
.switchyard/                  ignored generated state
```

The existing three-container Portless demonstration is archived under `old/`. It remains
runnable for historical comparison but is not a template for the new implementation.

## 13. Delivery phases

### Phase 1: routing proof

- Minimal schemas for existing sources, blocks, instances, groups, bindings, and routes.
- Container and containerized-script execution only, including Process Compose inside a
  runner container.
- Deterministic planning and generated Compose as an internal runtime implementation.
- Rust Switchyard Router in native host-gateway and per-consumer sidecar modes.
- Pingora HTTP/TLS/gRPC/WebSocket proxying plus Tokio raw TCP forwarding.
- Custom local domains and browser legacy-localhost routing by explicit header, Origin,
  or managed-profile proxy identity.
- Explicit rejection and diagnostics when browser routing identity is ambiguous.
- Per-consumer sidecars sharing Docker network namespaces for fixed
  `localhost:<port>` slots.
- One-shot CLI commands: validate, plan, up, bind, status, logs, and down.
- Generated manifests and Docker ownership labels; no daemon or SQLite dependency.
- Golden tests plus a real fixture with three UIs, two backends, and two five-service
  groups. All unchanged consumers use the same localhost ports while reaching their
  selected providers.
- Group switching through validated, complete, atomic router snapshot replacement.

Phase 1 is a technical proof, not the complete product MVP.

### Phase 2: product MVP

- Long-running control-plane daemon and HTTP/SSE API shared by CLI and GUI.
- SQLite state, operation locking, history, recovery metadata, route history, and GUI
  preferences. Desired state remains in portable YAML.
- Versioned live route snapshots with acknowledgement and connection-drain policies.
- Adapter SDK and registry with JSON Schema validation.
- Schema-driven GUI with the deployment builder, patch-bay topology, instance inspector,
  logs, health, group switching, and custom-domain management.
- First-class source inspection plus managed Git clones and worktree creation, while
  preserving non-destructive behavior for unmanaged worktrees.
- Ordered overlays, resolved-value origins, secret-safe file injection, and variation
  comparison.
- Additional execution adapters, including explicitly trusted host execution, only
  after they meet the same ownership and isolation contracts.

### Phase 3: LAN and team workflows

- Switchyard gateway LAN/mDNS preflight and publication.
- Import/exportable deployment bundles without secrets.
- Optional Tailscale or private-DNS adapter.

## 14. MVP acceptance criteria

The first complete version is successful when a developer can:

1. Register a monorepo and at least two existing worktrees.
2. Define database, UI, Java, and five-service Python blocks.
   The fixtures must cover a Dockerfile, a containerized legacy script, and a Process
   Compose suite inside a runner container.
3. Create one database, five UI instances, two Python suites, and three Java suites.
4. Preview exactly which containers, images, volumes, and routes will be created.
5. Start the deployment and wait for health-based readiness.
6. Open each UI at a stable hostname.
7. See the source path, branch, and commit behind every running instance.
8. Select which Java and Python instances each UI uses.
9. Define named five-service groups assembled from one or several source variants.
10. Run two consumers that both call the same `localhost:8001` while reaching different
    provider groups.
11. Switch a consumer's complete group without restarting the application container.
12. Assign and persist custom domains for human-facing instances through the native
    Switchyard Router.
13. Recover observed deployment and route state through SQLite and Docker labels after a
    control-plane restart.
14. View combined and per-service logs.
15. Stop the deployment without deleting database state.
16. Perform all common operations from both the CLI and schema-driven GUI.
17. Replace the JAS example with an unrelated generic fixture without changing core,
    API, CLI, or GUI code.
18. Apply two different overlay sets to one base deployment and run both variations
    concurrently without modifying either source worktree.
19. Route three unchanged browser UIs that all call `localhost:10081` to independently
    selected backend instances using Origin, an extension header, or managed profiles.
20. Reject an ambiguous browser request with an actionable diagnostic.
21. Run duplicate backend instances when two UIs require the same backend source but
    different downstream service groups.

## 15. Explicit non-goals for the MVP

- Replacing Kubernetes or becoming a production scheduler.
- Running containers across multiple Docker hosts.
- Public internet exposure.
- Automatic resolution of incompatible database migrations.
- Destructive Git operations.
- Host shell scripts that have not received explicit block-level trust approval.
- Multi-user authentication and authorization.
