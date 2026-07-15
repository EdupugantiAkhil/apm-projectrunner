# Adapter SDK

Switchyard keeps product concepts generic by putting source inspection, execution,
supervision, routing, and health observation behind versioned adapter contracts. The
contracts live in `switchyard-adapter-sdk`; the adapters which express the existing
generated-Compose behavior live in `switchyard-adapters`.

The SDK boundary is framework-neutral. User configuration, execution recovery handles,
and route handles cross it as serializable JSON. State, events, logs, claims, source
identity, and route observations use normalized SDK types. In particular, execution
adapters expose the complete `validate`, `plan`, `prepare`, `start`, `inspect`, `logs`,
`stop`, `cleanup`, and `recover` lifecycle. The current built-ins use that surface at
planning level: they emit declarative resources, argument-array commands, and ownership
claims for the existing Compose generator and runtime. They do not introduce a second
runtime.

## Versioning and compatibility

The current contract is `switchyard.dev/adapter-sdk/v1alpha1`. Every adapter declaration
contains:

- a stable lowercase identifier;
- an implementation semantic version;
- every SDK contract version it supports; and
- protocol, live-update, recovery, and feature capability metadata.

Registration is keyed by adapter kind, identifier, and implementation version. The
registry rejects malformed identifiers, malformed semantic versions, duplicate exact
registrations, and adapters which do not declare the current SDK contract. Each failure
has a stable machine-readable `RegistryErrorCode`. Lookup by kind and identifier selects
the highest compatible registered version; listing returns deterministic declaration,
capability, and schema metadata.

An incompatible SDK declaration is never guessed or coerced. A future stable contract
may define an explicit compatibility range, but the alpha contract requires an exact
contract-version declaration.

## Configuration schemas

Every adapter configuration is a Serde type with a `schemars` 1.2.1 `JsonSchema`
implementation. `switchyard_adapter_sdk::schema_for` deliberately generates draft
2020-12 and every published schema declares that dialect. The SDK compiles and validates
schemas with `jsonschema` 0.47 with its default features disabled, keeping validation
offline and preventing schema resolution from becoming an implicit network operation.

The registry listing is the form-discovery API for the future GUI. A client can select
an adapter by kind and identifier, render basic fields from `configurationSchema`, and
use capability metadata to filter protocol-compatible choices. Adapter-specific screens
are not required for basic configuration.

## Built-in adapters

The built-in registry contains:

| Kind | Identifier | Existing behavior represented |
| --- | --- | --- |
| Source | `source-path` | Existing local directory |
| Source | `source-git` | Existing repository/worktree and requested ref |
| Execution | `execution-container` | Compose image or Dockerfile build |
| Execution | `execution-runner-script` | Service or task in a runner container |
| Supervisor | `supervisor-process-compose` | Process Compose suite in a runner container |
| Route | `route-switchyard` | Sidecar or host-gateway HTTP, HTTPS, WebSocket, gRPC, and raw TCP loopback route |
| Probe | `probe-health` | HTTP, TCP, or command healthcheck |

The deployment YAML remains stable. `switchyard-planner` maps its existing source,
execution, probe, capability, and route-slot model types to these adapter configurations
during validation, then continues through the unchanged deterministic artifact generator.
Filesystem existence, topology completeness, naming, collision, and ownership checks
remain planner responsibilities.

## Conformance suite

Each adapter ships at least one valid and one invalid example. Adapter tests should call
the public suite:

```rust
use switchyard_adapter_sdk::conformance;

conformance::assert_adapter(&my_adapter);
```

The common suite verifies that the declared schema compiles as draft 2020-12, valid
examples pass schema validation and adapter deserialization, invalid examples return
diagnostics, capability declarations contain no duplicates, validation is deterministic,
compatibility supports the current SDK, and example handles serialize losslessly. The
suite also exports `check_runtime_handle` and `check_route_handle` for kind-specific
opaque-handle round trips.

Adapters may add stricter semantic, ownership, recovery, and isolation tests. Passing
the common suite is necessary, not sufficient, for code that controls host resources.

## Trusted host execution is deferred

There is intentionally no `execution-host` registration. Host commands lack Docker
network isolation and can collide on ports, writable directories, background processes,
and other exclusive resources. A host execution adapter may land only after it passes
the same public conformance suite and the ownership and isolation checks in DESIGN.md
section 8, including complete resource claims, durable identity, recovery, safe stop,
and ownership-aware cleanup. A built-in registry test protects this deferral.
