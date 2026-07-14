# Development prerequisites

Switchyard development is Linux-first. Native Linux on `x86_64` or `aarch64` with a
Linux Docker Engine and Docker Compose v2 is the supported path; CI runs on `x86_64`
Linux. The runtime relies on Docker-provided Linux network namespaces for
consumer/router sidecar isolation; developers do not need to create namespaces or run
the router as root.

macOS is a planned host-gateway platform and may be used for workspace-only work with
Docker Desktop configured for Linux containers. End-to-end routing support on macOS is
not yet part of Phase 0. On Windows, use a Linux WSL2 distribution with Docker Desktop
integration; native Windows development is not currently supported.

## Bootstrap

Install [rustup](https://rustup.rs/), CMake, and Docker Engine (or Docker Desktop), then
run:

```sh
./scripts/bootstrap
```

The command is diagnostic and does not use `sudo` or change the host. It checks the
pinned Rust compiler, CMake (required to build Pingora), Docker daemon access, Docker
Compose v2, Linux-container mode, and, on native Linux, network namespace availability.
Follow any reported remediation and rerun it until all checks pass.

## Routing-proof platforms

The Phase 4 release gate supports native Linux `x86_64` and `aarch64`, both using Linux
containers, Docker Engine, and Compose v2. The dependency-free fixture and Rust router
build for the host architecture; no architecture-specific image is downloaded. CI runs
the complete proof on `x86_64`, and the same command is verified on `aarch64` during
development. macOS, native Windows, and public/LAN exposure remain outside this gate.

Run the clean-checkout release proof with:

```sh
./scripts/phase4-proof.sh
```

The command needs unoccupied loopback ports `10081` and `18080`, and Docker access for
the current user. It refuses an existing ownership-labelled `routing-matrix` deployment
instead of replacing it.

## Shared checks

Run the local CI-equivalent checks with:

```sh
./scripts/check.sh
```

Individual commands are available as `fmt`, `lint`, `test`, and `doc` arguments. Audit
Rust dependencies after installing `cargo-audit`:

```sh
cargo install cargo-audit --locked
./scripts/check.sh audit
```

The shared command and CI temporarily ignore two narrowly scoped advisories:

- `RUSTSEC-2024-0437`: Pingora 0.8.1 uses the affected protobuf crate only through
  Prometheus metrics encoding, so Switchyard does not expose the vulnerable protobuf
  decoder to untrusted input. Remove this exception when Pingora upgrades its Prometheus
  dependency.
- `RUSTSEC-2026-0009`: the first fixed `time` release, 0.3.47, requires Rust 1.88 while
  Switchyard supports Rust 1.85. Switchyard's direct use is limited to clock arithmetic
  for certificate validity timestamps, and neither Switchyard nor a reachable dependency
  path parses untrusted input with `time`'s RFC 2822 parser, the only affected API. Remove
  this exception when Switchyard's MSRV reaches 1.88 or a Rust-1.85-compatible backport is
  available.

Phase 3's Pingora Rustls support also inherits the unmaintained `rustls-pemfile` crate
through the latest `pingora-rustls` 0.8.1 and `rustls-native-certs`. This is an allowed
maintenance warning, not a known vulnerability; remove it when Pingora upgrades that
dependency chain.

No elevated privileges are expected for builds or unit tests. Configure Docker so the
current user or Docker context can reach the daemon instead of routinely invoking
development commands through `sudo`.
