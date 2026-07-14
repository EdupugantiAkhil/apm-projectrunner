# Switchyard

Switchyard is a local development topology orchestrator for running multiple instances
of unchanged application components and selecting how they connect.

The central use case is a topology such as:

```text
ui-1 ──► backend-1 ──► feature service group
ui-2 ──► backend-2 ──► main service group
ui-3 ──► backend-1 ──► feature service group
```

Applications may keep calling fixed addresses such as `localhost:8001`. Switchyard uses
Docker network namespaces and a Rust router sidecar per consumer to intercept those
addresses without source-code changes. A native host router handles custom local
domains, TLS, and browser calls to legacy localhost ports using an explicit route
header, the UI origin, or an isolated browser-profile proxy.

## Status

Implementation is proceeding from the authoritative specification and constraints in
[DESIGN.md](DESIGN.md). Progress is tracked as markable phase checklists in
[IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md).

The implementation target is:

- Docker Engine with generated Docker Compose for Phase 1 lifecycle management.
- A Rust `switchyard-router` built on Pingora for HTTP-family traffic and Tokio for raw
  TCP.
- Native host-gateway and container-sidecar modes from one router codebase.
- Versioned YAML desired state plus SQLite-backed applied snapshots, control state, and
  observations in the product phase.
- Docker named volumes for persistent application data.

## Repository structure

```text
Cargo.toml               Rust workspace for routing components
crates/                  router configuration and data-plane crates
docs/                    development and platform documentation
scripts/                 bootstrap and shared development checks
DESIGN.md                authoritative architecture and roadmap
IMPLEMENTATION_PLAN.md   phased implementation checklist
old/                     archived experiments; not the current implementation
```

## Development

Run `./scripts/bootstrap` to check the pinned Rust toolchain, Docker, Compose, and host
capabilities. Then run all formatting, lint, unit-test, and documentation checks with
`./scripts/check.sh`. See [docs/development.md](docs/development.md) for supported host
platforms and individual commands. The router binary and authenticated local control
protocol are documented in [docs/router.md](docs/router.md).

The previous shared-PostgreSQL/Portless demo has moved to
[`old/shared-database-portless-demo/`](old/shared-database-portless-demo/). Run its npm
and Compose commands from that directory if you need the historical proof-of-concept.

New implementation directories will be added according to the repository layout in
`DESIGN.md`; empty scaffolding is intentionally not committed before implementation
starts.
