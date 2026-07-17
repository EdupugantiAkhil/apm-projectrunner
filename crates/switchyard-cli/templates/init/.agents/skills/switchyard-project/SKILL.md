---
name: switchyard-project
description: Inspect, author, validate, and safely operate a Switchyard development topology. Use for deployment.yaml, overlays, startup profiles, sources, devices, instances, service groups, bindings, plans, lifecycle commands, or diagnostics in this project.
---

# Switchyard Project

Reach the same valid desired state as the TUI by editing authored configuration and
using Switchyard's validator and planner. Treat diagnostics as authoritative.

## Inspect before editing

Use this order:

1. Read `deployment.yaml` and every overlay named by `spec.overlays` or supplied by the
   user. Preserve unrelated YAML and comments.
2. Run `switchyard source list` to identify registered repositories and exact
   checkouts. Do not infer registry state from paths alone.
3. Run `switchyard device list` to identify registered SSH devices and their last
   eligibility result.
4. Read `.switchyard/generated/<deployment-name>/` only when comparing the last plan or
   investigating drift. It is derived evidence, never an authoring surface.
5. Run `switchyard status deployment.yaml` to compare desired, applied, and observed
   state. Add `--routes` when investigating bindings.

Never edit `.switchyard/generated/` or `.switchyard/state.sqlite3`. Do not hand-edit
runtime manifests, ownership labels, imported-profile records, or route snapshots.

## Analyze a repository safely

Read repository files; never execute repository scripts merely to discover behavior.
Inspect Dockerfiles, Compose files, Process Compose definitions, package metadata,
READMEs, example environment files, and static configuration. Determine:

- an identifiable, non-interactive startup command and its working directory;
- available container images or safe Docker build context and Dockerfile paths;
- coordinated services in Compose or Process Compose and their dependency order;
- required variables, parameters, file inputs, and credential references;
- fixed listen ports, fixed dependency addresses, and other exclusive resources;
- health/readiness endpoints or safe command/TCP probes; and
- databases, queues, APIs, and other startup or routing dependencies.

Do not run install, bootstrap, migration, start, or other repository-provided scripts
during analysis. Do not turn a command into a profile until its effects and required
inputs are clear. Prefer an existing supported container definition over inventing
language-specific behavior.

## Author startup profiles

A project startup profile is a block under `spec.blocks` in `deployment.yaml`. For a
single containerized provider:

```yaml
spec:
  blocks:
    api:
      services:
        server:
          execution:
            type: container
            build: { context: services/api, dockerfile: Dockerfile }
            command: ["./api", "--port", "8080"]
          provides:
            query: { protocol: http, port: 8080 }
          publish: [8080]
          probe: { type: http, path: /health, port: 8080 }
  instances:
    - { name: api-main, block: api, source: project, device: local }
```

A registered source may instead advertise profiles in exactly
`switchyard-profiles.yaml` at its checkout root. The format is `version: 1`; each key
under `profiles` maps directly to a block `spec` body. Model a coordinated suite as one
profile when its services must be selected, duplicated, and started together:

```yaml
version: 1
profiles:
  application-suite:
    parameters:
      LOG_LEVEL: { required: false, default: info }
    services:
      cache:
        execution: { type: container, image: redis:7-alpine }
        provides:
          cache: { protocol: tcp, port: 6379 }
        probe: { type: tcp, port: 6379 }
      api:
        execution:
          type: container
          build: { context: ., dockerfile: Dockerfile }
        provides:
          application-api: { protocol: http, port: 8080 }
        dependsOn: { cache: healthy }
        probe: { type: http, path: /health, port: 8080 }
```

Discovery reads only this manifest and never executes repository content. A
source-local profile is not runnable until the user explicitly reviews and imports it
in the TUI Profiles view. Changed content requires review and import again. Do not
bypass that trust boundary or write imported-profile state directly.

## Model connections completely

For every routable service:

- declare each `provides` capability with its real protocol and listen port;
- declare each `consumes` slot with the fixed address the unchanged consumer calls;
- define a group whose `providers` maps every required slot to an existing
  `instance/service`; and
- bind a consumer instance to one complete compatible group in `spec.bindings`.

Example:

```yaml
spec:
  blocks:
    client:
      services:
        app:
          execution: { type: container, image: example/client:1 }
          consumes:
            query: { protocol: http, address: { host: 127.0.0.1, port: 8001 } }
  groups:
    main-services:
      providers:
        query: api-main/server
  bindings:
    client-main: main-services
```

Never invent a capability, provider, address, or binding to make validation pass.
Never author a partial group or switch only part of a consumer's binding. If consumers
need different downstream groups, give each an independently routable provider
instance where required by the topology.

## Place instances on devices

Instances default to `local`; write `device: local` when explicit placement helps
review. Before selecting a registered SSH device, run:

```sh
switchyard device check <name>
```

Remote placement supports only container-backed, provider-only instances. Every
provided capability port must appear in `publish`; consumers, routers, process/script
adapters, and cross-device sidecars remain local. The registered device host must be
resolvable and reachable from the local router's containers. Prefer a LAN IP;
`localhost` points back at a container and mDNS often does not resolve in container
DNS. Eligibility proves SSH and Docker access, while `validate` proves workload fit.

## Validate every change

After every authored edit, run:

```sh
switchyard validate deployment.yaml
```

Before any `up`, run:

```sh
switchyard plan deployment.yaml
```

Include the same `--with`, `--variation`, and `--set` inputs the user intends to apply.
Read the entire mutation, resource, placement, and route preview. Fix the authored
definition, not generated output. Do not run `up` while validation or planning reports
an error. Use `switchyard tui .` when explicit profile review or guided authoring is
needed.

## Stop when safe configuration is impossible

If there is no identifiable safe start command, configuration requires credentials,
fixed ports conflict in a way the schema cannot express, the runtime is unsupported,
or another required behavior cannot be established from repository files, do not
produce a best-guess configuration. State clearly that the repository cannot be safely
configured, list each concrete blocker and the evidence inspected, and stop. Ask for a
supported command, secret reference mechanism, port parameterization, containerization,
or runtime adapter as appropriate. Never hide uncertainty behind plausible defaults.

## Safety boundaries

- Use `switchyard down deployment.yaml` for normal stopping; it preserves volumes.
- Run `switchyard cleanup deployment.yaml --yes` only after explicit user intent to
  destructively remove owned resources and persistent volumes.
- Never store passwords, tokens, private keys, or other secret values in YAML, SQLite,
  generated previews, or logs. Use supported external secret references.
- Never bypass source/profile trust, ownership checks, device eligibility, complete
  group routing, validation, or planner diagnostics.
- Preserve source changes. Never reset, clean, rewrite, or delete a checkout to make a
  plan succeed.

For authoritative behavior, read `DESIGN.md` sections **Source-local startup profiles**,
**Device**, **Limited remote container execution**, and **Interactive clients and shared
operations**, plus `docs/tui.md`. The local schema and planner remain authoritative if
an example or description differs from current diagnostics.
