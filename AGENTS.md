# APM ProjectRunner repository guidance

Treat `docs/vision/*.md` as the original product source of truth. Move the
implementation and `DESIGN.md` toward that vision. Edit the vision only when the project
owner changes a product decision or when an edit is required to remove an internal
contradiction; do not rewrite it merely to describe the current implementation.

## Working model

- Do not create or delegate work to subagents. Work directly in the current agent.
- Treat `DESIGN.md` as the authoritative implementation architecture and update it to
  converge on the vision. Where the two differ, the vision controls the intended product
  and the roadmap records the work needed to close the gap.
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
- `crates/apmpr-router`: the shared sidecar/host router process, local admin
  channel, host-gateway lifecycle helpers, certificates, and managed HTTP proxy.
- `crates/apmpr-planner`: desired-state validation and deterministic generation of
  Compose, router, manifest, and managed-profile artifacts.
- `crates/apmpr-cli`: the `apmpr` command-line workflow for planning,
  applying, inspecting, switching, opening browser profiles, stopping, and cleanup.
- `examples/routing-matrix`: runnable zero-application-change topology fixture and smoke
  proof.
- `extensions/apmpr-route`: dependency-free Chromium extension for tab-scoped
  explicit route identity.
- `docs`: development, router, browser-routing, platform, and operational guidance.
- `scripts`: bootstrap and shared format, lint, test, documentation, and audit checks.
- `old`: archived experiments; do not treat them as the current implementation.
