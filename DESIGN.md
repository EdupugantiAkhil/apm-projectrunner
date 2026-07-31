# APM ProjectRunner: composable development deployments

Status: authoritative implementation architecture. Where this document and
[`docs/vision`](docs/vision) differ, the vision controls the intended product and
[`docs/v2-roadmap.md`](docs/v2-roadmap.md) records the work to close the gap.

Product name: **APM ProjectRunner** (`apmpr`). V2 Part 7 renamed the tree from the
development-era working name `Switchyard`.

Audience: developers testing combinations of services from monorepo worktrees and
independent Git repositories.

## 1. Purpose

APM ProjectRunner is a local-first deployment and topology orchestrator. It lets a developer
define reusable startup blocks, create multiple instances from different source trees,
and combine instances into named service groups. Group membership is the connection; the
authored topology has no separate capability, slot, binding, or direct-route layer.

Existing application code must not require modification. A containerized consumer may
continue calling fixed dependency addresses such as `localhost:8001`; APM ProjectRunner routes
those calls through the instance's group inside its isolated network
namespace.

APM ProjectRunner's core is solution-agnostic. Java, Python, JAS, UI, and database are example
instance or service names in the first reference fixture, not measurable roles. The runtime must work with
any executable, container image, repository layout, language, framework, protocol, or
service grouping that can satisfy the generic contracts below.

Example deployment:

- One database block reused by separate database instances.
- Five UI instances.
- Two Python suites, each containing five services (ten Python containers total).
- Three Java backend suites.
- A selectable route from each UI to one Java suite, one Python suite, and one database.

The system must support sources from:

- Worktrees in the same monorepo.
- Normal directories in the same monorepo.
- Existing checkouts of unrelated Git repositories.
- Optionally, repositories and worktrees created by APM ProjectRunner in a managed workspace.

## 2. Design principles

1. **Declare combinations, do not hand-edit Compose.** Human-authored block and
   deployment files are the source of truth. Generated Compose files are disposable.
2. **Treat a suite as a unit.** A Python suite can expand into five related services;
   duplicating the suite duplicates all five with consistent naming and configuration.
3. **Make membership explicit.** Every instance's complete ordered group is visible and
   inspectable; routing does not infer roles from names such as UI or backend.
4. **Keep source separate from runtime.** A source identifies code; a block describes
   how to run it; an instance combines them for a deployment.
5. **Use containers as the first isolation boundary.** Phase 1 wraps every long-running
   instance in a container-backed network namespace. This makes repeated fixed ports
   safe and enables transparent loopback routing. Other execution adapters remain part
   of the product model but are deferred until this path is proven.
6. **Fail before mutation.** Validate paths, names, addresses, membership,
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
- A **service group** is a named, ordered set of instances that share one localhost.
- A **route adapter** implements transparent transport internally; it is not an authored
  capability-to-slot connection.
- A **probe** observes readiness or health.
- A **deployment** selects instances and their group membership.

Names such as `java`, `python`, `database`, and `ui` may be useful instance or block names,
but they have no routing semantics. Routing follows the destination loopback port and the
authored order of the selected group.

Generic definitions are extended through versioned adapter interfaces rather than
conditionals in the core:

```text
SourceAdapter     git-worktree | future repository providers
ExecutionAdapter  container | runner-script | host | future plugin
SupervisorAdapter process | process-compose | future plugin
RouteAdapter      http | tcp | environment | rendered-config | future plugin
ProbeAdapter      http | tcp | command | log-pattern | process | future plugin
```

Adapters publish JSON Schema for their configuration, plus a declaration of which SDK
operations they implement. The CLI validates that schema, and the GUI renders controls
from it. Adding an adapter must not require a custom GUI screen for basic operation.
These adapter declarations describe an implementation's own features; they are not the
removed authored `provides:`/`consumes:` topology and take no part in routing.

### Delivery staging

The final architecture includes the daemon/API, SQLite state and recovery, adapter SDK,
live route control, schema-driven GUI, and managed Git/worktrees. They are core product
capabilities, but they are not prerequisites for validating the routing model.

Phase 1 is a vertical routing proof built as a one-shot CLI over generated Compose,
Docker network namespaces, and the APM ProjectRunner Router in native-host and per-consumer
sidecar modes. The router, rather than Portless, is the authoritative routing layer.
Phase 2 promotes that proven behavior into the persistent control plane and GUI without
changing the human-authored topology model.

## 3. Domain model

### Repository and source

A repository names Git object and linked-worktree metadata storage once. Exactly one of
`url:` (a bare repository created and managed by APM ProjectRunner) or `clone:` (an adopted bare
repository or ordinary clone) is required. APM ProjectRunner never runs code from a repository
checkout. A source is always an editable and runnable worktree backed by that repository,
with an authored ref and project-relative path.

```yaml
repositories:
  monorepo:
    url: git@github.com:example/product.git
  legacy:
    clone: /code/legacy

sources:
  monorepo-main:
    repository: monorepo
    ref: main
    path: ./sources/monorepo-main

  backend-feature-a:
    repository: monorepo
    ref: feature/backend-a
    path: ./sources/backend-feature-a
```

Managed clones live under `.apmpr/clones/<repository>`. `up` creates a missing
managed clone and any missing source worktree. Existing paths are validated against the
named repository and ref. There is no plain-path source kind, and repository clones and
source worktrees may not overlap or contain one another.

### Device

A device is a project-scoped execution host. Registered devices are records in the
project's `.apmpr/state.sqlite3` database. Each record contains a name, SSH user,
host, port, an optional identity-file path, and the status, detail, and time of the last
connectivity check. APM ProjectRunner stores neither passwords nor private keys; SSH uses the
credentials and agent available to the APM ProjectRunner process.

`local` is an implicit, always-available device and is not stored as a database row.
Connectivity checks for registered SSH devices are explicit background operations.
Their results are persisted for inspection and do not by themselves start an instance.

Every instance has a device. The current runtime honors only `local`; selecting a
non-local device is a validation error until the limited remote execution cut is
implemented. A client or planner must never accept and then ignore a placement field.

User-level device configuration is deliberately outside the current scope. A future
design may add `~/.config/apmpr/devices.yaml`, with project records taking
precedence over same-named global records and every client displaying the effective
record's origin. No client should imply that such global configuration exists today.

### Block

A reusable startup definition. A block may contain one service or a coordinated suite.
Control-plane UIs should present a block as a **startup profile** when a developer creates
an instance: the developer chooses an instance name, a repository checkout/worktree, and
one reusable startup profile. A block is deliberately distinct from an operator run
script: blocks describe the long-running services inside instances, while run scripts
invoke project-level plan, lifecycle, or smoke-test actions.
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
apiVersion: apmpr.dev/v1alpha1
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
apiVersion: apmpr.dev/v1alpha1
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
apiVersion: apmpr.dev/v1alpha1
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
paths are allowed. `${source.path}` resolves to the selected source worktree, so
the same block definition can start JAS from different worktrees without editing the
script.

Process Compose suite:

```yaml
apiVersion: apmpr.dev/v1alpha1
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
its child-process names, dependency states, readiness probes, and logs into APM ProjectRunner.
Process Compose remains responsible for its internal startup and ordered shutdown.

Host commands run in a new process group. APM ProjectRunner sends the declared stop signal to
the group, waits for the timeout, and only then escalates. It records the PID, executable,
working directory, definition hash, start time, and child processes so it never stops an
unrelated process that happens to reuse a port.

```yaml
apiVersion: apmpr.dev/v1alpha1
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
name and any number of components. APM ProjectRunner does not branch on these values.

Execution mode is independent of block type: Java, Python, UI, and generic blocks may
use containers, containerized scripts, or explicitly trusted host commands.

### Source-local startup profiles

A registered source checkout may declare startup profiles in exactly one well-known
file at its root: `apmpr-profiles.yaml`. Discovery reads only that file. It does
not search for likely scripts, inspect other filenames, or execute repository content.
An absent file is not an error.

The manifest has this versioned shape:

```yaml
version: 1
profiles:
  python-suite:
    parameters:
      LOG_LEVEL:
        required: false
    services:
      api:
        execution:
          type: container
          build:
            context: services/api
            dockerfile: Dockerfile
        publish: [8001]
        healthcheck: /health
```

Each value in `profiles` is a block `spec` body using the existing block schema, with
the profile's map key taking the place of `metadata.name`. It may
declare the execution adapter, command, working directory, mounts, published ports,
probes, parameters, and lifecycle. The manifest does not
introduce another execution format, and a source-local profile has the same validation,
planning, isolation, ownership, health, and cleanup contracts as a project block.
Project run actions remain separate operations declared in
`.apmpr/run-scripts.yaml`; they are not profiles and do not own instance services.

Discovery does not make a profile executable. Import is explicit and records the source
name, source commit, and a deterministic content hash of the selected profile definition
in project state. Before import, the client shows the fully expanded definition for
review. If the discovered definition's content hash later differs from the imported
hash, the profile is `changed` and cannot run until it is reviewed and imported again.

A project-declared profile shadows a source-local profile with the same name. Clients
always display whether the effective profile originates in the project or a registered
source, including the source name and imported commit where applicable.

### Host resource claims

Host commands do not receive Docker network isolation. Their definitions must declare
ports, writable directories, and exclusive resources. Planning fails when two instances
claim the same resource.

The current JAS and AI Process Compose scripts use fixed ports. Consequently, multiple
copies cannot run on the same host unchanged. Before APM ProjectRunner starts two copies, one of
the following must be true:

- The scripts and Process Compose file accept per-instance port parameters.
- APM ProjectRunner renders a per-instance Process Compose file with unique ports and matching
  dependency URLs.
- Each copy moves into its own container or network namespace.

APM ProjectRunner must never silently offset ports because service-to-service URLs may be
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

Transparent route adapters implement:

```text
validate(group membership) → diagnostics
plan(group snapshot)       → live | restart | rebuild
apply(group snapshot)      → route handle
remove(route handle)
inspect(route handle)      → observed membership and listeners
```

The authored schema supplies ordered group membership, not typed endpoints. The route
adapter observes listeners and preserves the original TCP destination port. Transport
metadata needed by the browser-facing host router is derived from addresses and listener
configuration rather than capabilities or slots.

### Reference fixture: JAS legacy deployment

The parent workspace provides the first real integration fixture. It deliberately mixes
execution mechanisms, but none of its details belong in the product core:

| Block | Current entry point | Phase 1 treatment |
|---|---|---|
| JAS databases | decomposed commands from `/zfs/projects/FR/jasBase/start-local-jas.sh` | database containers plus runner-container tasks |
| Java JAS service | `/zfs/projects/FR/jasBase/start-jas-service.sh` | runner image containing the required Nix/Gradle tooling |
| Python AI suite | `process-compose -f ai-services.process-compose.yaml up` in `/zfs/projects/FR/jasBase` | Process Compose inside an AI runner container |
| UI | Worktree-specific Dockerfile or runner script | Container or containerized script |

The desired topology is expressed through group membership rather than being embedded in
the startup mechanism:

```text
group-a: [ui-a, jas-main, ai-feature, db-main]
group-b: [ui-b, jas-feature, ai-main, db-main]
```

The routing layer owns the ordered membership. A block does not need to know whether its target
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

groups:
  feature-a:
    instances: [ui-a, jas-main, ai-feature, db-main]
  feature-b:
    instances: [ui-b, jas-feature, ai-main, db-main]
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
        dockerProject: "apmpr-${deployment.name}-${instance.name}"
  initialize-tenant:
    execution:
      type: host
      lifecycle: task
      command: ["./initialize-jas-tenant.sh"]
    dependsOn:
      databases: healthy
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

An instance is either a concrete copy of a block or an external endpoint. A started
instance selects a source, supplies parameters, and selects an execution device; `local`
is the default and the only placement honored before the limited remote execution cut.
An external instance instead declares one upstream host and the ports that group members
may reach. It has no source, block, device, Compose service, source identity, or lifecycle.

```yaml
instances:
  - name: python-main
    block: python-suite
    source: monorepo-main
    device: local
    parameters:
      LOG_LEVEL: info

  - name: python-feature
    block: python-suite
    source: experimental-python
    device: local
    address: python-feature.comparison.localhost
    parameters:
      LOG_LEVEL: debug

  - name: staging-es
    external: search.staging.internal
    ports: [9200, 9300]
    probe: { type: tcp, port: 9200 }
```

#### Addresses

One rule covers both addressable objects: **anything addressable carries `address:`,
declared on the thing it names.** A group's address names the whole combination; an
instance's address names that one instance. Both are optional, both are singular, and
there is no separate `ingress:` or `uiRoutes:` section holding a reference to something
that can be deleted out from under it.

An instance `address:` is not a role marker. The planner resolves it from that instance's
own browser-reachable service, and an instance with no address is simply not reachable by
name, which is the default.

Every expanded service name is namespaced:

```text
<deployment>--<instance>--<service>
comparison--python-feature--analysis
```

### Service group and transparent routing

A service group is the complete ordered list of instances that share one localhost.
There are no authored `provides:`, `consumes:`, `routes:`, or `bindings:` sections.
`extends:` is also absent: without capability keys, replacing an inherited member would
be ambiguous. Each group states its complete membership.

```yaml
groups:
  feature-test:
    address: feature-test.comparison.localhost
    instances: [ui-1, backend-1, backend-canary, db-feature-test]
    disabled: [backend-canary]
  regression:
    address: regression.comparison.localhost
    instances: [ui-2, backend-2, db-regression]
```

`disabled:` names members of that same group which stay running and keep their position
in `instances:` but do not participate in that group's routing, address resolution, or
collision warnings. Removing a name restores it at its authored priority without a
restart. It is a per-group exclusion, not an instance-level stop.

An outbound IPv4 or IPv6 loopback TCP connection is intercepted in the sender's namespace.
The router preserves the destination port and tries active group members on that same port
in authored order. If several members listen, the first listed wins and a warning names
the collision. Started-member ports are observed at runtime; an external member has no
listener registry, so its expanded `ports:` allowlist supplies the same per-port candidacy.
`publish`, started-service probes, and image `EXPOSE` remain lifecycle or ingress metadata,
not routing declarations.

For example, a call to `localhost:9200` from any started member of a group containing
`staging-es` is forwarded to `search.staging.internal:9200`. The external address is used
as authored and must resolve and be reachable from the sidecar routing environment. An
optional external HTTP, HTTPS, TCP, or command probe runs during `up`; failure is reported
as external reachability, separately from managed-instance startup failure.

Each instance has its own namespace so alternative members can remain alive and a group
can switch between them without rebuilding or restarting applications. Reordering members
or temporarily disabling the current winner changes which already-running instance
receives a shared-port call. Same-port coexistence follows from that per-instance
isolation; it is not the primary product reason for it.

An instance may appear in at most one group's membership list. It sees that group's
ordered member view. Reusing the same source or block in another group requires a separate
instance. Validation rejects multi-group membership and names both groups before any
runtime mutation.

`apmpr move <deployment> <instance> <group>` and the control-plane
`commands/membership` endpoint update this authored relationship. A live-compatible move
replaces every affected sidecar snapshot across the source and destination groups and
rolls back already-applied snapshots if a later router fails. Moving a previously
ungrouped instance changes the sidecar resource set, so it is saved as desired membership
and applied through `up` rather than treated as a route-only live change.

For a group address, an explicit instance subdomain or browser route identity selects the
member. A bare group address works only when exactly one active member also carries an
instance `address:`; zero or several such members is an error rather than a guess.

The planner keeps this selection in generated router data rather than adding an authored
topology layer. When no advanced `hostRouter` configuration is supplied, it derives one
HTTP/HTTPS provider from each service whose HTTP probe port is also published, generates
the loopback gateway on a deterministic unprivileged port, and derives browser localhost listeners
from group membership. The bare custom domain has a direct route to the default member.
Gateway allocation probes deterministically within its reserved range, skipping ports already
claimed by generated browser listeners; planning fails if the range is exhausted.
Each active browser-reachable member gets an `<instance>.<group-address>` custom domain,
and the bare destination gets an explicit-header route for that member. Pingora evaluates
that override per request on the loopback gateway, fails closed for unknown identity, and
strips the identity before forwarding. Generated Origin routes retain unambiguous group
context for subsequent browser calls. TCP-only members remain available through the
group's transparent shared localhost.

### Deployment

The desired combination of sources, instances, parameters, and groups.

```yaml
apiVersion: apmpr.dev/v1alpha2
kind: Deployment
metadata:
  name: comparison
spec:
  overlays:
    - overlays/development.yaml
    - overlays/mongodb.yaml
  instances:
    - { name: main-db, block: postgres, source: infrastructure }
    - { name: feature-db, block: postgres, source: infrastructure }
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
  groups:
    main:
      instances: [ui-1, backend-main, python-main, main-db]
    feature:
      instances: [ui-2, backend-a, python-feature, feature-db]
```

### Overlay

An overlay creates a product variation without copying its block or deployment
definition. It may inject environment values and files and override declared parameters.
Overlays do not author topology: group membership, addresses, and external instances are
stated once in the deployment. This keeps the connection model single-sourced, matching
the removal of `bindings:` and `routes:` from the deployment schema.

```yaml
apiVersion: apmpr.dev/v1alpha1
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
  variables:
    enableNewSearch: "true"
```

Overlay selectors may target deployment labels, block instances, or expanded components.
A selector must match at least one target unless explicitly
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

Maps merge by key. `unset` removes an inherited environment key. File targets must be
unique after resolution unless a later overlay explicitly declares `replace: true`. Lists
do not merge implicitly unless their schema explicitly declares append or keyed semantics.

APM ProjectRunner must render and display the fully resolved deployment and an origin trace for
every value:

```text
LOG_LEVEL=DEBUG  ← overlays/mongodb.yaml
DATABASE_URL=…   ← deployment instance ui-a
PORT=8001        ← block default ai-services
```

#### File injection

Injected files never modify a source repository or worktree by default. APM ProjectRunner
materializes them under:

```text
.apmpr/generated/<deployment>/overlays/<instance>/<content-hash>/
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
 Browser / TUI / CLI / GUI
          │
          ▼
 ┌─────────────────────────────────────────────────────────┐
 │ Native APM ProjectRunner Router                         │
 │ custom domains + TLS + legacy localhost listeners       │
 │ Origin/header/profile identity + CORS/preflight         │
 └──────────────────────────┬──────────────────────────────┘
                            │ loopback-only published ports
                            ▼
 ┌─────────────────────────────────────────────────────────┐
 │ Docker Engine: one private bridge network per deployment│
 │ UI instances     backend instances       service groups │
 │                         │                               │
 │                         ▼                               │
 │           APM ProjectRunner Router sidecar              │
 │              shared consumer network namespace          │
 │              owns localhost:8001, ...                   │
 └──────────────────────────┬──────────────────────────────┘
                            │
                            ▼
             selected group members / shared services

 TUI / CLI / Web GUI ──HTTP+SSE──► APM ProjectRunner control plane
                              ├── planner + Compose generator
                              ├── router configuration
                              ├── Git/worktrees
                              └── SQLite
```

### Runtime and isolation

Docker Engine is the Phase 1 container runtime. APM ProjectRunner generates Docker Compose as
an internal lifecycle artifact; users do not have to author Compose and the domain model
does not depend on it. Every deployment receives a private Docker bridge network.
Provider instances receive deterministic internal DNS aliases, while host exposure is
loopback-only.

Every application instance receives one Linux network namespace. All services expanded
from that instance join it, and a router sidecar uses
`network_mode: service:<instance-namespace>` to join the same namespace. Different
instances can consequently bind the same application ports without collision.

The sidecar installs namespace-local IPv4 and IPv6 interception with only the
`NET_ADMIN` capability. Outbound connections to any `127.0.0.0/8` or `::1` TCP port are
redirected to a reserved internal proxy port. A second reserved port reports the
namespace's listening TCP ports without probing application endpoints. The router
recovers the original destination port and tries the active group members in authored
order using that same port. No listener, `publish`, probe, image `EXPOSE`, capability,
or slot declaration is required.

Inbound deployment-network connections are redirected through the receiver's sidecar
too. The sidecar then connects to that member's own loopback, so applications may bind
only `127.0.0.1` or `::1`; they do not have to widen their bind address to
`0.0.0.0`. Marked router-owned sockets bypass interception without recursion. Locally
originated calls still follow authored group priority, including when the calling
instance itself listens on the requested port.

The authored schema has no `provides` or `consumes` override. Routing is port-for-port.
If a future use case requires port remapping, it must receive a separate explicit schema
and migration rather than reviving capability/slot topology.

The host router runs as a native process. Browser `localhost` refers to the developer
host, and native execution gives consistent access to host listeners and Docker's
loopback-published ports on Linux, macOS, and Windows. A Linux-only host-network
container may be offered later, but is not the portable default.

Router administration is transport-independent. The native host router uses an
owner-only Unix socket. A container sidecar keeps its owner-only Unix socket inside the
sidecar filesystem; the control plane reaches it by executing the router's bounded
admin client inside the exact ownership-verified container and passing the authenticated
request over stdin. This avoids depending on host/VM shared filesystems for Unix socket
semantics and does not publish an administration listener on the host or deployment
network.

The initial stack is therefore:

| Concern | Phase 1 choice |
| --- | --- |
| Container lifecycle | Docker Engine through generated Docker Compose |
| Container isolation | Docker-provided Linux network namespaces |
| Internal fixed-port routing | APM ProjectRunner Router sidecars |
| Browser, custom-domain, and TLS routing | Native APM ProjectRunner Router |
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

### Interactive clients and shared operations

The React dashboard is the default local interactive control plane. An existing folder
can be adopted non-destructively with `apmpr project register`, after which
`apmpr daemon install [project]` installs and starts its project-scoped per-user
service, and `apmpr gui [project]` only opens the authenticated dashboard exposed by
that running service. The installer emits a launchd LaunchAgent on macOS or systemd user
unit on Linux, both keyed by canonical project path and configured to append output to
`.apmpr/daemon.log`. The Ratatui TUI remains supported as an optional headless and
SSH-friendly client, including terminal handoff for Git and ephemeral SSH credential
prompts. `apmpr tui [project]` retains its command name.

Application behavior shared by interactive and command-line clients belongs in a new
`apmpr-ops` crate. It owns operations that validate, plan, apply, and mutate
sources, devices, profiles, instances, and group membership. It also owns read-model projections
such as the rows, summaries, validation diagnostics, and operation states rendered by a
view. It does not render widgets or parse command-line arguments and must not depend on
`ratatui` or `clap`.

The dashboard, TUI, CLI, and daemon converge on this layer incrementally. Each new workflow
extracts the operations and projections it touches from client code; existing behavior
is not moved in a big-bang rewrite.

Control-plane clients use these final user-facing labels without renaming persisted
fields:

| UI label | Architecture term | Meaning |
| --- | --- | --- |
| APM ProjectRunner project | Workspace/project directory | Authored deployment, overlays, and project state |
| Code | Sources | Code made available through repository-backed worktrees |
| Repository | Repository | The Git repository and its relationship to linked worktrees |
| Checkout | Source path/worktree | The exact code tree selected for an instance |
| Startup profile | Block | A reusable definition that expands into one service or a coordinated suite |
| Instance | Instance | One checkout run through one startup profile with its own parameters |
| Service group | Service group | A complete ordered set of instances sharing one localhost |
| Connection | Group membership | The group whose localhost an instance uses |
| Run action | Project run script | A project-level Up, Down, Plan, Status, or smoke-test operation |
| Device | Registered device | A known execution host; `local` plus registered SSH hosts |

The handwritten alternative "project / project instance" is rejected because
"APM ProjectRunner project" already means the workspace. Reusing `project` for source code
would make project state, code checkouts, and running instances ambiguous. Persisted
`source`, `block`, `instance`, and `group` field names remain unchanged.

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
- One namespace anchor and APM ProjectRunner Router sidecar for each local group member or
  explicitly routed instance. Every service of the instance joins the anchor namespace.
  Transparent sidecars receive `NET_ADMIN`, drop every other capability, and run with
  `no-new-privileges`.

Host commands are not emitted as Compose services. The control plane supervises them in
parallel with the Compose runtime and includes them in the same dependency graph.

Compose `--scale` must not be used for instances with different sources or parameters.
They must be separate generated services.

Generated output belongs under:

```text
.apmpr/generated/<deployment>/compose.yaml
.apmpr/generated/<deployment>/resolved-deployment.yaml
.apmpr/generated/<deployment>/manifest.json
.apmpr/generated/<deployment>/routes/<instance>.cfg
```

Only human-authored definitions are committed. `.apmpr/generated` is ignored.

### APM ProjectRunner Router

One Rust codebase provides three modes with the same configuration and route-table
semantics:

1. **Host gateway** owns custom local domains, TLS, browser-facing legacy localhost
   ports, CORS/preflight handling, and routes to loopback-published container ports.
2. **Container sidecar** shares an instance's network namespace, transparently captures
   arbitrary loopback TCP destinations, and forwards the original port to the first
   active group member listening there.
3. **Forward proxy** gives a managed browser profile an explicit routing identity when
   neither a request header nor `Origin` is sufficient.

The router's own configuration contract (`apmpr.dev/router/v1alpha1`) still names
listener destinations with a `slot` key. That is an identifier inside a generated
artifact, not authored topology: the planner derives it, and no deployment schema, client
form, or diagnostic asks a developer for one. It survives the removal of authored
capabilities and slots because it labels a listener destination rather than declaring
what an instance provides or consumes.

Route tables are validated as complete immutable snapshots and swapped atomically.
Updates never expose a partially changed five-service group. HTTP connections can drain
under the previous snapshot; new connections use the new snapshot. Raw TCP routes have
an explicit close, drain, or pin policy. The router also owns health checks, structured
access logs, and route inspection. It must never silently choose a target for an
ambiguous request.

Portless was useful for the original hostname proof-of-concept but is not part of the
authoritative runtime. It cannot provide consumer-specific browser identity and
container-local fixed-port routing under one configuration contract.

Example:

```text
jas-main requests 127.0.0.1:8001
                           │
                           └──► comparison--ai-feature--ingest

jas-feature requests 127.0.0.1:8001
                              │
                              └──► comparison--ai-main--ingest
```

Changing group membership renders and validates a complete replacement router
configuration, then atomically reloads only the router sidecar. The application
container is not restarted; a partially applied group is forbidden.

Phase 2 adds versioned live route snapshots, acknowledgements, history in SQLite, and
graceful connection policies. The route plan states whether existing connections drain,
close, or remain pinned while new connections use the new group.

### Browser routing identity

Browser JavaScript calling `localhost:<port>` connects to the native host router, not
the UI container. The router selects a backend using this precedence:

1. `X-Apmpr-Route`, injected per tab by the optional APM ProjectRunner browser extension.
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
For an extension-free guaranteed mode, `apmpr open <deployment> <instance>` launches an isolated
browser profile with `--proxy-server=<listener>` and
`--proxy-bypass-list=<-loopback>`, as supported by
[Chromium's proxy configuration](https://chromium.googlesource.com/chromium/src/+/HEAD/net/docs/proxy.md).

### Unique group membership

An instance may appear in at most one group's `instances:` list. Validation rejects
multi-group membership and names both groups before any mutation. When two combinations
need the same code or startup definition, each combination declares its own instance;
the instances may reuse the same source worktree and block.

This is a structural topology rule. It does not infer UI, backend, database, sender, or
receiver roles from names or runtime traffic.

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

The React GUI is the default local interactive client for project setup, deployment
authoring, monitoring, and operations.
Its primary job is to answer and change:
**“Which exact source-backed instances are connected right now?”**

It is an operational tool, not a generic admin dashboard. The main view should resemble
a disciplined physical patch bay: ordered groups contain visible instance members, and
each instance visibly belongs to at most one group.

The dashboard must grow guided startup-profile authoring, instance creation, group-membership
editing, and device placement through shared operations and schema-driven forms. Raw
YAML remains an inspectable escape hatch, not the intended final interaction. The TUI
may retain equivalent workflows for headless use, but new local authoring work should
not require it.

Running HTTP and HTTPS custom domains are ordinary links in the deployment inspector.
Their URLs are projected from the applied host-router listener, including its actual
unprivileged port, and open through normal browser navigation. This path is separate from
the instance card's **Managed profile** action, which runs the `open` operation and may
require managed Chromium only when proxy identity is needed for fixed localhost calls.

### Visual direction

The interface takes cues from lab equipment and rack labels without imitating a terminal.
It should feel precise, inspectable, and calm under high information density.

Color tokens:

| Token | Value | Use |
|---|---:|---|
| Bench | `#E8E7E1` | Main work surface |
| Panel | `#F7F6F1` | Cards and inspectors |
| Ink | `#182027` | Primary text and structure |
| Cobalt | `#2457D6` | Primary actions and selection |
| Violet | `#7651C9` | Secondary emphasis |
| Copper | `#B25C32` | Destructive warnings |
| Signal | `#15805D` | Healthy and ready states |

Colors carry state and emphasis, never an instance's supposed kind. There is no
schema-visible service type to key a palette to, so a client that colored members by
guessed role would be inventing one.

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

Signature element: the **live patch bay**. Group membership is the only visually bold
gesture. Ordered member rows show routing priority and restrained motion when an instance
moves between groups. Everything else remains flat, quiet, and compact.

This avoids a generic grid of statistic cards: counts matter less than topology and
source identity in this product.

### Main deployment view

```text
┌──────────────┬────────────────────────────────────┬───────────────────┐
│ DEPLOYMENTS  │ comparison                 Running │ INSTANCE INSPECTOR│
│              │                                    │                   │
│ ● comparison │ GROUPS AND ORDERED MEMBERS          │ ui-feature        │
│ ○ regression │                                    │ feature/ui-redesign│
│ ○ clean-main │ ┌────────────┐   ┌──────────────┐  │ /worktrees/ui-a   │
│              │ feature-test: ui-main, backend-main │                   │
│ + Deployment │               python-main, main-db  │ Group             │
│              │                                    │ feature-test      │
│ SOURCES      │ regression:   ui-feature, backend-a │ Position 1        │
│ 8 clean      │               python-a, feature-db  │                   │
│ 2 modified   │                                    │ [Apply membership]│
│ 1 missing    │ collision: none                     │ [Open] [Logs]     │
├──────────────┴────────────────────────────────────┴───────────────────┤
│ EVENTS  Build completed: python-a/analysis                     ▴     │
└──────────────────────────────────────────────────────────────────────┘
```

Interaction rules:

- Selecting an instance opens its source, commit, health, environment, group, and logs.
- Moving an instance between groups prepares a membership change; it does not apply until
  the user confirms the complete group diff.
- Adding an instance already assigned to another group produces a validation error naming
  both groups and directs the user to create a separate instance.
- Modified worktrees are visible before build and require acknowledgment.
- Each member displays both the friendly instance name and abbreviated commit.
- Keyboard users can change membership through select controls in the inspector; dragging is
  never the only interaction.
- Reduced-motion mode replaces cable animation with an immediate color/state change.

### Additional screens

#### Deployment builder

- Choose a saved template or start empty.
- Add block instances from a searchable library.
- Select a source and worktree for each instance.
- Set parameters using block-provided field definitions.
- Build ordered groups and validate membership continuously.
- Preview the expanded service and resource count before starting.

#### Overlay editor and variation comparison

- Add, remove, and reorder overlays on a deployment.
- Edit schema-approved environment values, dotenv inputs, file injections, parameters,
  and group membership.
- Show the origin of every resolved value and a warning when a later overlay shadows it.
- Compare two variations side by side across source refs, environment keys, injected
  file hashes, group membership, images, ports, and resource claims.
- Preview injected text files with secrets redacted and binary files as metadata only.
- Show whether each change applies live or requires a restart or rebuild before apply.

#### Sources and worktrees

- Repository, worktree path, branch, commit, dirty state, ahead/behind state.
- Present repositories as parents and linked worktrees as selectable checkouts, rather
  than flattening both into an unexplained list of sources.
- Actions: inspect, refresh, open directory, create worktree, remove managed worktree.
- Destructive Git actions are excluded. APM ProjectRunner never resets or discards changes.

#### Block library

- Block description, expanded services, parameters, published ports, and health contract.
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

The topology-facing CLI commands. `crates/apmpr-cli/src/cli.rs` holds the complete
surface, including `init`, `bundle export`/`import`, `diagnostics`, `operation cancel`,
and `worktree remove`:

```text
apmpr validate <deployment>
apmpr plan <deployment>
apmpr migrate <deployment>
apmpr overlay validate <overlay>
apmpr overlay diff <deployment> --with <overlay...>
apmpr up <deployment>
apmpr status <deployment> [--routes]
apmpr routes <deployment>
apmpr move <deployment> <instance> <group> [--transition close|drain|pin]
apmpr logs <deployment> [instance[/service]]
apmpr open <deployment> <instance>
apmpr down <deployment>
apmpr cleanup <deployment> --yes
apmpr source list | register | deregister
apmpr worktree create <repository-source> <ref> [--path <path>] [--name <name>]
apmpr device list | add | remove | check
apmpr project register [<directory>] [--name <project-name>]
apmpr gui [<project-directory>]
apmpr tui [<project-directory>]
apmpr daemon install [<project-directory>]
apmpr daemon run | status | stop
```

Group membership is changed with `move`, which names one instance and its destination
group; there is no separate `group` command, because a group's complete ordered
`instances:` list is authored in the deployment and edited there or through a client.

The CLI calls the same API as the GUI. It must also support a one-shot mode for CI and
recovery when the daemon is not running.

The control-plane contract is `/api/v1`, defined in
[`contract.rs`](crates/apmpr-daemon/src/contract.rs) and documented in
[`docs/control-plane-api.md`](docs/control-plane-api.md). Its groups are deployments,
commands, operations, events, sources, repositories, devices, profiles, run actions,
adapters, and project metadata.

Mutating requests use operation IDs and idempotency keys. Long operations return
immediately and stream progress separately.

## 7. Lifecycle

### Plan

1. Parse block, source, and deployment definitions.
2. Resolve paths and Git identities.
3. Validate group membership, addresses, and listener conflicts known before startup.
4. Calculate expanded services, images, networks, volumes, and hostnames.
5. Detect conflicts and show a deterministic diff against the active deployment.

### Apply

1. Acquire the deployment mutation lock.
2. Write generated artifacts atomically.
3. Build changed images with bounded concurrency.
4. Start stateful dependencies and wait for readiness.
5. Start provider suites and wait for readiness.
6. Apply the complete group-membership route snapshot.
7. Start instances and register ingress hostnames.
8. Stream the final state and release the lock.

### Stop and cleanup

Stopping preserves named volumes by default. Deleting volumes, images, managed
worktrees, or clones requires separate explicit actions. Cleanup operates only on
resources carrying matching APM ProjectRunner ownership labels.

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

- `none`: APM ProjectRunner never runs migrations.
- `owner`: exactly one selected instance owns migrations.
- `isolated-schema`: each consumer set receives a separate schema in one server.
- `isolated-database`: each consumer set receives a separate database.

The plan must warn when multiple instances claim migration ownership or when branches
with different migration fingerprints share a schema.

## 10. Remote access

Local mode uses custom `*.localhost` names through the native APM ProjectRunner Router. The
router may use an unprivileged HTTP port by default or a locally trusted certificate and
platform-specific privileged-port setup for HTTPS. Domain and certificate ownership is
explicit desired state and can be inspected from the CLI and GUI.

Optional LAN mode uses `*.local` and mDNS:

- A future publication adapter advertises APM ProjectRunner gateway instance names.
- The configured gateway TCP port and mDNS UDP port `5353` must be permitted.
- Linux requires `avahi-publish-address` from `avahi-utils`.
- The GUI shows whether mDNS publication and remote reachability checks pass.
- mDNS is not assumed to cross subnets, VLANs, VPNs, or guest Wi-Fi isolation.

Cross-network access is a later adapter using normal DNS, Tailscale, or another private
network. It must not silently expose deployments to the public internet.

### Limited remote container execution

The first remote execution cut supports only container-adapter instances on a
registered SSH device. APM ProjectRunner invokes the Docker client against
`ssh://user@host`, using Docker's native SSH transport. Image builds send the selected
local checkout's build context to that daemon over the same transport, so the cut does
not require an image registry or a managed remote checkout.

The APM ProjectRunner Router remains local. A remotely placed service must declare explicit
published ports, and routes address it as `device-host:published-port`. Remote routers,
cross-device sidecar routing, process-adapter instances on remote devices, and
drift-tracked remote checkouts are unsupported by this cut. Validation reports these
boundaries rather than attempting a partial placement.

Remote containers receive the same APM ProjectRunner ownership labels as local containers.
Inspection, reconciliation, and cleanup enumerate resources with those labels on the
selected remote Docker daemon.

Before start, APM ProjectRunner verifies SSH reachability, runs `docker version` through the
SSH transport, and records the reported platform information. A device is eligible only
when both checks succeed and the requested container workload satisfies the cut.
Validation fails with the concrete reason when the host is unreachable, Docker is
absent, access is denied, or another eligibility check fails.

If a device is unreachable during stop or cleanup, the operation fails with the reason
and guidance to restore access and retry. APM ProjectRunner does not report success or silently
orphan the workload. Its ownership records remain intact, and the labeled remote
resources remain discoverable for later reconciliation and cleanup.

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
  apmpr-docker-ssh/      process-scoped explicit-identity Docker SSH transport
packages/
  core/                       schemas, planner, naming, validation
  compose-runtime/            Compose generation and execution
  source-manager/             Git and worktree inspection
  server/                     API, state, operations, event streams
  cli/                        command-line client
  web/                        React GUI
  adapter-sdk/                public adapter contracts and schema helpers
adapters/
  source-git/                 repositories and worktrees
  execution-container/        image and Dockerfile components
  execution-runner-script/    scripts isolated in runner containers
  execution-host/             explicitly trusted host commands
  supervisor-process-compose/ Process Compose inspection and lifecycle
  route-apmpr/           native gateway and sidecar lifecycle
examples/
  routing-matrix/             3 UIs, 2 backends, and switchable service groups
  jas-base/                   containerized legacy parent-workspace fixture
old/
  shared-database-portless-demo/ archived hostname/database proof-of-concept
scripts/                      bootstrap and development scripts
.apmpr/                  ignored generated state
```

The existing three-container Portless demonstration is archived under `old/`. It remains
runnable for historical comparison but is not a template for the new implementation.

## 13. Delivery phases

Task-level progress and phase exit gates are tracked in
[IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md). This section defines product scope;
the implementation plan is the markable execution checklist.

### Phase 1: routing proof

- Minimal schemas for sources, blocks, instances, and ordered groups.
- Container and containerized-script execution only, including Process Compose inside a
  runner container.
- Deterministic planning and generated Compose as an internal runtime implementation.
- Rust APM ProjectRunner Router in native host-gateway and per-consumer sidecar modes.
- Pingora HTTP/TLS/gRPC/WebSocket proxying plus Tokio raw TCP forwarding.
- Custom local domains and browser legacy-localhost routing by explicit header, Origin,
  or managed-profile proxy identity.
- Explicit rejection and diagnostics when browser routing identity is ambiguous.
- Per-instance sidecars sharing Docker network namespaces and transparently intercepting
  fixed `localhost:<port>` calls.
- One-shot CLI commands: validate, plan, up, status, logs, and down.
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

- APM ProjectRunner gateway LAN/mDNS preflight and publication.
- Import/exportable deployment bundles without secrets.
- Optional Tailscale or private-DNS adapter.

## 14. MVP acceptance criteria

These criteria are written against the JAS reference fixture, so they name its example
blocks and instances — database, UI, Java, Python. Those are that fixture's names, not
product roles: criterion 17 requires the same criteria to hold after the fixture is
replaced. Nothing in the planner, API, CLI, or GUI may branch on them.

The first complete version is successful when a developer can:

1. Register a monorepo and at least two existing worktrees.
2. Define database, UI, Java, and five-service Python blocks.
   The fixtures must cover a Dockerfile, a containerized legacy script, and a Process
   Compose suite inside a runner container.
3. Create one database, five UI instances, two Python suites, and three Java suites.
4. Preview exactly which containers, images, volumes, groups, and router resources will be created.
5. Start the deployment and wait for health-based readiness.
6. Open each UI at a stable hostname.
7. See the source path, branch, and commit behind every running instance.
8. Select which instances share a group, by editing that group's ordered membership.
9. Define named five-service groups assembled from one or several source variants.
10. Run two consumers that both call the same `localhost:8001` while reaching different
    provider groups.
11. Move a sender between complete groups without restarting the application container.
12. Assign and persist custom domains for human-facing instances through the native
    APM ProjectRunner Router.
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
