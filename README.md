# Switchyard

Switchyard is a local development topology orchestrator for running multiple instances
of unchanged application components and selecting how they connect.

You declare a group as an ordered list of instances, and its members share one localhost:

```text
feature-test: [ui-1, backend-1, db-feature]
regression:   [ui-2, backend-2, db-main]
```

That list is the whole configuration. Applications may keep calling fixed addresses such
as `localhost:8001`; Switchyard uses Docker network namespaces and a Rust router sidecar
per member to intercept those addresses without source-code changes, forwarding each call
to the first active member of the caller's group listening on that same port. Every
instance gets its own namespace, so alternatives can stay running on the same ports and a
group switches between them by reordering or disabling a member. A native host router
handles custom local domains, TLS, and browser calls to legacy localhost ports using an
explicit route header, the request origin, or an isolated browser-profile proxy.

## Status

The routing proof, product MVP, browser control plane, and V2 release-usability work are
complete. The team release is still blocked by the security remediation and acceptance
evidence listed in [docs/unfinished-work.md](docs/unfinished-work.md). Verified per-phase
detail is in [PROGRESS.md](PROGRESS.md); the historical implementation checklist remains
in [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md).

Alignment with the product vision in [docs/vision](docs/vision) is tracked in
[docs/v2-roadmap.md](docs/v2-roadmap.md). V2 replaced the original capability, slot,
binding, and direct-route topology with one model: an ordered group membership list whose
members share one localhost, routed port-for-port. The remaining V2 work is the full
lifecycle gate for the vision sample and the final mechanical rename to **APM
ProjectRunner** (`apmpr`); until then the tree uses the `switchyard` name.

The implementation target is:

- Docker Engine with generated Docker Compose for container lifecycle management.
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
protocol are documented in [docs/router.md](docs/router.md). The runnable planner/CLI
proof is documented in
[examples/routing-matrix/README.md](examples/routing-matrix/README.md).
Browser tab identity, the unpacked Chromium extension, and managed-profile fallback are
documented in [docs/browser-routing.md](docs/browser-routing.md).
The current security audit is documented in
[docs/security-review.md](docs/security-review.md), and version compatibility and
deprecation commitments are documented in
[docs/support-policy.md](docs/support-policy.md).

Run the routing-proof release gate from a clean checkout with
`./scripts/phase4-proof.sh`; see the
[routing-matrix guide](examples/routing-matrix/README.md) for its topology, failure
coverage, and backend-group boundary.

The previous shared-PostgreSQL/Portless demo has moved to
[`old/shared-database-portless-demo/`](old/shared-database-portless-demo/). Run its npm
and Compose commands from that directory if you need the historical proof-of-concept.
