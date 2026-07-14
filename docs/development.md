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

The shared command and CI temporarily ignore `RUSTSEC-2024-0437`: Pingora 0.8.1 uses
the affected protobuf crate only through Prometheus metrics encoding, so Switchyard
does not expose the vulnerable protobuf decoder to untrusted input. Remove the exception
when Pingora upgrades its Prometheus dependency.

No elevated privileges are expected for builds or unit tests. Configure Docker so the
current user or Docker context can reach the daemon instead of routinely invoking
development commands through `sudo`.
