# Switchyard repository guidance

do note edit docs/vision/*.md as they are the original source of truth and we should strive to move towords it

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
