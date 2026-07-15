# Generic legacy-workspace fixture

This directory is a self-contained stand-in for the deployment shape historically
represented by the JAS workspace. It contains no real application code or host-specific
paths. Its purpose is to prove that Switchyard models a mixed legacy topology entirely
through generic source, execution, supervision, routing, and lifecycle contracts.

| Block | Generic mechanism exercised |
|---|---|
| `jas-databases` | Two image-backed HTTP store stand-ins, named volumes, and a one-shot schema initialization task |
| `jas-service` | An unchanged fixed-port legacy shell script mounted into a runner image (`execution.type: script`) |
| `ai-services` | A runner-container Process Compose definition supervising five individually observable fixed-port processes |
| `ui` | A Dockerfile-built HTTP container reached through a custom local domain |

The fixture image is built from the same Rust and Debian base images used by
`examples/routing-matrix`. `fixture.rs` supplies every application role. The small
fixture-local `process-compose` entry point keeps the build offline and starts exactly
the processes declared by `process-compose.yaml`; the deployment still exercises the
generic `processCompose` supervisor adapter and its generated command/mount contract.
`contract.yaml` records the fixed observable topology independently of generated
Compose and router artifacts.

## Topology

```text
ui-a --java--> jas-main ----database----> db-main/{kv,document}
  |                |
  +--python--------+--------------------> ai-feature/{8001..8005}

ui-b --java--> jas-feature -database---> db-main/{kv,document}
  |                |
  +--python--------+--------------------> ai-main/{8001..8005}
```

Both UIs call the unchanged address `localhost:10081`. Both Java stand-ins call the
same `127.0.0.1:8001` through `:8005` addresses, but their sidecars select different
five-service groups. The Java stand-ins also use fixed database slots on ports 9101 and
9102. Every identity response includes the deployment, instance, service, selected
providers, and source path.

`ai-feature` extends `ai-main` and replaces all five providers. The checked-in
`sources/main` and `sources/feature` paths make ordinary offline planning deterministic.
The smoke proof creates an isolated Git repository from the untouched main fixture
tree, registers it, creates the feature source with `switchyard worktree create`, and
generates a smoke-only deployment pointing the feature instances at that worktree. It
compares the repository status before and after the run.

## Run the proof

From the repository root:

```sh
./examples/jas-base/smoke.sh
```

The script requires Docker, Compose, Cargo, Git, curl, and Python 3. It builds and
starts the fixture, verifies both UI selections and all fixed slots, confirms worktree
source identities in `switchyard status`, switches `jas-main` from `ai-feature` to
`ai-main` without restarting its application container, and performs a down/up cycle.
The schema task increments state in both named volumes, so the second initialization
proves the prior state survived `down`. Final cleanup verifies that no owned containers
or volumes remain.

Two overlay/variation plans can be previewed without touching either source tree:

```sh
switchyard plan examples/jas-base/deployment.yaml \
  --with examples/jas-base/overlays/main.yaml --variation main
switchyard plan examples/jas-base/deployment.yaml \
  --with examples/jas-base/overlays/feature.yaml --variation feature
```

The variation names produce disjoint deployment, Compose-project, artifact, container,
network, and volume names. Because this fixture deliberately claims stable host ports
for its custom domains, attempting to *run* both at once is correctly rejected as a
host-listener collision; concurrent planning remains useful and deterministic.

## MVP evidence

The bundle covers the mechanisms behind MVP criteria 1–3, 9–11, 17, and 18:

- path and managed-worktree source identities are observable without source edits;
- database, Dockerfile UI, runner-script Java, and five-process Python suite blocks are
  declared together;
- inherited five-service groups route identical localhost slots per consumer and switch
  atomically without application restart;
- ordered overlays create disjoint named variation plans and materialize files outside
  the source trees; and
- the planner swap test plans this fixture and `examples/routing-matrix` through the
  same `load_bundle` and `plan` calls, while a guard prevents fixture identifiers from
  entering production crate source.

Replacing this directory with an unrelated valid deployment bundle requires no core,
API, CLI, or GUI changes; `real_codebase_fixtures.rs` is the executable proof.

## Known gap

Declared lifecycle hooks (`prepare`, `postReady`, `stop`, and `cleanup`) are currently
schema-only and are not executed by the generated-Compose runtime. This fixture does
not declare hooks. Database initialization is instead an explicit runner-script service
with `lifecycle: task`, healthy store dependencies, and downstream
`completed_successfully` dependencies—the supported one-shot lifecycle shape.
