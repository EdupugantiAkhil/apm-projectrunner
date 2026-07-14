# Switchyard repository guidance

## Working model

- Do not create or delegate work to subagents. Work directly in the current agent.
- Treat `DESIGN.md` as the authoritative architecture and `IMPLEMENTATION_PLAN.md` as
  the phased execution checklist. Mark work complete only after implementation,
  verification, and relevant documentation are finished.
- Maintain `PROGRESS.md` with the current implementation and verification status, and
  maintain `AGENTMISTAKES.md` with mistakes, corrections, and lessons that should guide
  future work. Update both files as relevant while completing each phase.
- Keep changes focused, avoid unnecessary code and tests, preserve unrelated user
  changes, and commit reviewed phase-sized increments so they are easy to revert.

## Project structure

- `crates/router-config`: versioned router configuration types, validation, and schema
  compatibility contracts.
- `crates/router-core`: immutable route snapshots, browser identity resolution, and
  atomic route activation independent of any network implementation.
- `crates/router-pingora`: HTTP, HTTPS, WebSocket, gRPC, CORS, and browser-facing data
  plane built on Pingora.
- `crates/router-tcp`: raw TCP routing and connection-transition behavior.
- `crates/switchyard-router`: the shared sidecar/host router process, local admin
  channel, host-gateway lifecycle helpers, certificates, and managed HTTP proxy.
- `crates/switchyard-planner`: desired-state validation and deterministic generation of
  Compose, router, manifest, and managed-profile artifacts.
- `crates/switchyard-cli`: the `switchyard` command-line workflow for planning,
  applying, inspecting, switching, opening browser profiles, stopping, and cleanup.
- `examples/routing-matrix`: runnable zero-application-change topology fixture and smoke
  proof.
- `extensions/switchyard-route`: dependency-free Chromium extension for tab-scoped
  explicit route identity.
- `docs`: development, router, browser-routing, platform, and operational guidance.
- `scripts`: bootstrap and shared format, lint, test, documentation, and audit checks.
- `old`: archived experiments; do not treat them as the current implementation.

## End goal

Switchyard is a local development topology orchestrator. It must run multiple instances
of unchanged application components while letting each consumer select independent
backend and service-group routes, even when applications keep using fixed addresses
such as `localhost:8001` or `localhost:10081`.

The finished product should provide:

- deterministic Docker Compose lifecycle management with private per-deployment
  networking, persistent named volumes, and safe ownership-aware cleanup;
- one Rust router codebase for container sidecars, the native host gateway, browser
  identity routing, and raw TCP traffic;
- versioned desired state, durable applied snapshots and observations, recoverable
  process/source orchestration, and auditable mutations;
- CLI and desktop control planes for creating, inspecting, switching, opening, stopping,
  and recovering deployments without changing application source code;
- explicit, reversible LAN/team workflows and release hardening without expanding into
  a public-internet or general production scheduler.

The release milestones are defined in `IMPLEMENTATION_PLAN.md`: Phases 0-4 complete the
routing proof, Phases 5-6 deliver the local product MVP, and Phase 7 delivers the
team-ready release.
