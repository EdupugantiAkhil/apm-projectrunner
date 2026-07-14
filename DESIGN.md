# Switchyard: composable development deployments

Status: proposed design

Working name: **Switchyard**

Audience: developers testing combinations of services from monorepo worktrees and
independent Git repositories.

## 1. Purpose

Switchyard is a local-first deployment orchestrator. It lets a developer define
reusable startup blocks, create multiple instances of those blocks from different source
trees, and explicitly choose which instances communicate.

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
5. **Make the execution boundary visible.** Container and runner-script blocks are the
   safe default. Explicitly trusted host-command blocks are supported for existing Nix,
   Gradle, gcloud, and Process Compose development workflows. The plan and GUI must make
   host execution unmistakable before it starts.
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

Trusted host script:

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

### Reference fixture: JAS mixed-runtime deployment

The parent workspace provides the first real integration fixture. It deliberately mixes
execution mechanisms, but none of its details belong in the product core:

| Block | Current entry point | Execution model |
|---|---|---|
| JAS databases | `/zfs/projects/FR/jasBase/start-local-jas.sh` | Host script controlling Docker Compose resources |
| Java JAS service | `/zfs/projects/FR/jasBase/start-jas-service.sh` | Long-running trusted host script using Nix/Gradle |
| Python AI suite | `process-compose -f ai-services.process-compose.yaml up` in `/zfs/projects/FR/jasBase` | Long-running trusted Process Compose suite |
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
was started by Docker, a runner script, a host script, or Process Compose.

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

### Deployment

The desired combination of sources, instances, parameters, and routes.

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
                         ┌─────────────────────────────┐
                         │ Web GUI                     │
                         │ canvas, logs, sources       │
                         └──────────────┬──────────────┘
                                        │ HTTP + SSE
┌──────────────┐          ┌──────────────▼──────────────┐
│ CLI          ├─────────►│ Switchyard control plane   │
└──────────────┘          │ validation + desired state │
                          └───┬──────────┬──────────┬───┘
                              │          │          │
                         Git/worktrees   │       SQLite
                                         │
                              Compose generator
                                         │
                    ┌────────────────────▼────────────────────┐
                    │ Docker Engine                           │
                    │ generated services, networks, volumes   │
                    └────────────────────┬────────────────────┘
                                         │
                              internal route gateway
                                         │
                    ┌────────────────────▼────────────────────┐
                    │ Portless ingress                        │
                    │ *.localhost, optional *.local LAN mode  │
                    └─────────────────────────────────────────┘
```

### Control plane

A long-running local process owns deployment operations and exposes an API used by both
the CLI and GUI. Only one operation may mutate a deployment at a time. Other deployments
may build or start concurrently within a configurable concurrency limit.

Recommended implementation:

- Node.js 24 and TypeScript.
- Fastify or the Node HTTP API for the control API.
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
- Loopback-only host ports where Portless requires them.
- Read-only or read-write source mounts for script runners, as explicitly declared.
- One-shot dependency conditions for successful `task` scripts.

Host commands are not emitted as Compose services. The control plane supervises them in
parallel with the Compose runtime and includes them in the same dependency graph.

Compose `--scale` must not be used for instances with different sources or parameters.
They must be separate generated services.

Generated output belongs under:

```text
.switchyard/generated/<deployment>/compose.yaml
.switchyard/generated/<deployment>/resolved-deployment.yaml
.switchyard/generated/<deployment>/manifest.json
```

Only human-authored definitions are committed. `.switchyard/generated` is ignored.

### Routing

There are two routing layers:

1. **Ingress routing** gives humans stable addresses such as
   `ui-2.comparison.localhost:1355`. Portless remains suitable for local access.
2. **Service routing** gives containers stable dependency endpoints. A small dynamic
   gateway maps consumer-specific aliases to selected provider instances.

Example:

```text
ui-2 requests java.ui-2.internal
                       │
                       └──► comparison--backend-a--api

ui-2 requests analysis.python.ui-2.internal
                       │
                       └──► comparison--python-feature--analysis
```

Changing a route should update the gateway atomically. Rebuilding or restarting the UI
must not be required unless that UI embeds upstream URLs at compile time. The GUI must
identify those compile-time routes and require a rebuild explicitly.

HTTP routes can usually switch live. Database protocols and applications that read
dependency URLs only at startup may require a controlled restart of the consumer. The
route plan must state `live`, `restart consumer`, or `rebuild consumer` before applying
each change.

### State and ownership

SQLite records observations, not the only copy of desired state. Deployment YAML remains
portable and reviewable. Docker labels allow recovery if the database is deleted.

The control plane records:

- Deployment state and last applied definition hash.
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

Local mode uses `*.localhost` through an unprivileged Portless port.

Optional LAN mode uses `*.local` and mDNS:

- The Docker host advertises instance names through Portless LAN mode.
- TCP port `1355` and mDNS UDP port `5353` must be permitted.
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
blocks/                       reusable block definitions
deployments/                  saved deployment definitions
packages/
  core/                       schemas, planner, naming, validation
  compose-runtime/            Compose generation and execution
  source-manager/             Git and worktree inspection
  router/                     internal dynamic routing
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
  route-http/                 hostname/path HTTP routing
  route-binding/              environment and rendered-config bindings
examples/
  shared-database-demo/       current small integration fixture
  jas-base/                   mixed-runtime parent-workspace fixture
scripts/                      bootstrap and development scripts
.switchyard/                  ignored generated state
```

The existing three-container demonstration becomes the first integration fixture rather
than the permanent application structure.

## 13. Delivery phases

### Phase 1: definitions and deterministic planning

- Versioned schemas for sources, blocks, instances, routes, and deployments.
- Existing local paths and worktrees.
- Validation, naming, planning, and generated Compose.
- Container, containerized-script, and trusted host-command execution modes, including
  one-shot tasks and host resource-conflict detection.
- Golden-file tests for the example deployment.
- Adapter SDK and registry with JSON Schema-driven configuration.
- A core test fixture using invented capability names to prevent accidental assumptions
  about languages or common web application roles.
- Ordered overlay resolution for environment, dotenv files, injected files, parameters,
  and routes, including origin traces and deterministic resolved hashes.

### Phase 2: lifecycle CLI

- Build, up, status, logs, and down.
- Health-aware ordering and operation events.
- Portless local ingress.
- Static routes selected at deployment time.
- Host process-group supervision and the Process Compose adapter.

### Phase 3: GUI foundation

- Deployment list, route canvas, instance inspector, logs, and health.
- Deployment builder using existing blocks and sources.
- Plan/apply workflow with diffs.
- Overlay editor, ordering controls, resolved-value origins, and variation comparison.

### Phase 4: live routing and source management

- Atomic route switching without full redeployment.
- Worktree creation and managed repository clones.
- Dirty-source protections and source comparison.

### Phase 5: LAN and team workflows

- Portless LAN/mDNS preflight and publication.
- Import/exportable deployment bundles without secrets.
- Optional Tailscale or private-DNS adapter.

## 14. MVP acceptance criteria

The first complete version is successful when a developer can:

1. Register a monorepo and at least two existing worktrees.
2. Define database, UI, Java, and five-service Python blocks.
   The fixtures must cover a Dockerfile, a containerized script, a trusted host script,
   and a Process Compose suite.
3. Create one database, five UI instances, two Python suites, and three Java suites.
4. Preview exactly which containers, images, volumes, and routes will be created.
5. Start the deployment and wait for health-based readiness.
6. Open each UI at a stable hostname.
7. See the source path, branch, and commit behind every running instance.
8. Select which Java and Python instances each UI uses.
9. View combined and per-service logs.
10. Stop the deployment without deleting database state.
11. Perform all common operations from both the CLI and GUI.
12. Replace the JAS example with an unrelated generic fixture without changing core,
    API, CLI, or GUI code.
13. Apply two different overlay sets to one base deployment and run both variations
    concurrently without modifying either source worktree.

## 15. Explicit non-goals for the MVP

- Replacing Kubernetes or becoming a production scheduler.
- Running containers across multiple Docker hosts.
- Public internet exposure.
- Automatic resolution of incompatible database migrations.
- Destructive Git operations.
- Host shell scripts that have not received explicit block-level trust approval.
- Multi-user authentication and authorization.
