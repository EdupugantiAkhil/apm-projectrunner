# Development prerequisites

The persistent daemon and authenticated versioned API are documented in
[`control-plane-api.md`](control-plane-api.md).
Overlay composition, secret handling, change previews, and concurrent variations are
documented in [`overlays.md`](overlays.md).
Portable export/import bundles and safe sharing guidance are documented in
[`bundles.md`](bundles.md).
The Phase 7 source audit and its tracked findings are in
[`security-review.md`](security-review.md). Configuration, HTTP API, state-file, and
SQLite compatibility commitments are in [`support-policy.md`](support-policy.md).

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

Create a minimal project from the embedded reference template with:

```sh
switchyard init
cd my-project
switchyard validate deployment.yaml
```

The guided initializer asks for a lowercase deployment name and destination directory,
then creates the folder with the deployment, development overlay, README, gitignore,
and a project-local `.agents/skills/switchyard-project` workflow skill.
For scripts, `switchyard init <directory>` remains available; use `--name <project-name>`
to override the directory-derived deployment name. Existing scaffold files are preserved
unless `--force` is explicitly supplied.

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

Run the Phase 6 product-MVP proof from a clean checkout with:

```sh
./scripts/phase6-proof.sh
```

This command requires Rust, Node.js with npm, Docker Engine, and Docker Compose v2. It
runs workspace formatting, tests, Clippy and rustdoc with warnings denied; performs a
clean GUI dependency install followed by its build and tests; and runs the live JAS
fixture smoke. `examples/routing-matrix/smoke.sh` remains the standalone routing-proof
command.

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

## Reliability Suite

The heavy reliability tests are opt-in and are not called by `scripts/check.sh`:

```sh
./scripts/reliability.sh
```

The script builds the needed test binaries, then runs only `#[ignore]` tests with
`--ignored`. It does not require Docker, but several tests bind loopback sockets and
must be run on a host where local socket binding is permitted. The default runtime is
short for review loops: `SWITCHYARD_RELOAD_STORM_SECONDS=30`,
`SWITCHYARD_SOAK_SECONDS=30`, and `SWITCHYARD_CONCURRENCY=16`. Increase those
environment variables for longer soak or higher client-load runs.

The suite covers router-core snapshot reload storms, TCP and HTTP data-plane reload
storms with Linux `/proc` fd/RSS leak checks, an HTTP soak with health flapping, and an
in-process daemon API concurrency test. Each test duration is printed by the script,
and any failed assertion exits non-zero.

## Sources and worktrees

Switchyard distinguishes ownership at registration time. An existing developer path is
always `unmanaged`: registration records its canonical path and live-inspects Git, but
never grants Switchyard permission to reset, clean, checkout, remove, or otherwise
modify it. Deregistration only forgets that database record. A source cannot later be
promoted from unmanaged to managed.

Managed linked worktrees are created below `.switchyard/worktrees/`; managed clones
created through the library are below `.switchyard/clones/`. Every mutation verifies
the canonical target remains below its matching root and never passes Git `--force`.
Removal refuses staged, unstaged, and untracked changes by default. The explicit
`--allow-dirty` flag is the only override and still cannot remove an unmanaged or
out-of-root path.

```sh
switchyard source register product /code/product
switchyard source list
switchyard source list --json
switchyard worktree create product feature/api --name feature-api
switchyard worktree remove feature-api
# only after reviewing the exact dirty counts:
switchyard worktree remove feature-api --allow-dirty
switchyard source deregister feature-api
switchyard source deregister product
```

These commands use the authenticated daemon when it is running and the same synchronous
state/source libraries as a one-shot fallback otherwise. Git absence and plain paths
degrade to explicit unknown inspection fields; they do not block use of a plain-path
deployment. Generated manifests and `switchyard status` append the source path,
repository, requested ref, commit, and dirty flag captured for each instance at plan
time. Live commit and dirty observations are never persisted as registry truth.

Registered SSH devices can host provider-only container instances. Set an instance's
`device` to the registered name and publish every provided capability port explicitly.
Planning emits `compose.<device>.yaml`; lifecycle and log commands use Docker's SSH
transport with batch authentication. `switchyard status` includes each resource's
device and reports an unreachable remote explicitly while retaining its last observed
resources for reconciliation. Remote consumers, process adapters, remote routers, and
cross-device sidecars remain outside this limited cut.

```sh
switchyard device add builder dev@192.168.1.40 --identity ~/.ssh/id_ed25519
switchyard device check builder
switchyard device list
```

The check first verifies SSH, then asks the remote Docker server for its version through
`DOCKER_HOST=ssh://...`; it records eligibility or a concrete SSH, Docker availability,
or permission failure without storing credentials. Use a device host that the local
router's containers can resolve and reach. A LAN IP is preferable to `localhost` or an
mDNS name because routes use the registered host with the published service port.
