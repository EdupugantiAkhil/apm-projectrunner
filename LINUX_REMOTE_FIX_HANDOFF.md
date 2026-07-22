# Linux and remote-device verification fix handoff

Date recorded: 2026-07-23

Baseline under test: commit `5b87686` (`Complete Apple Silicon macOS support`)

This document records the issues found while verifying the macOS portability changes
on a real Linux/aarch64 NixOS host and while driving that host from macOS.

## Resolution

Resolved on 2026-07-23. Platform-specific short test roots now preserve macOS socket
limits without assuming `/private/tmp` on Linux. Docker SSH operations use the shared
`switchyard-docker-ssh` process-scoped transport, which applies an explicit identity as
a distinct argument with `BatchMode=yes` and `IdentitiesOnly=yes`. Full locked workspace
tests pass on macOS and the real Linux/aarch64 host; the macOS-to-device lifecycle also
passes with no usable agent and an identity path containing spaces. Details are in
`PROGRESS.md`.

## Verification environment

- macOS client: Apple Silicon macOS 26 with Docker Desktop in Linux-container mode.
- Remote host: `akhil@192.168.1.167`, NixOS/aarch64 on a Snapdragon 845.
- The supplied `pocof1.local` name did not resolve; the LAN address did.
- Remote Docker Engine: 28.5.1, Linux/arm64.
- Remote Docker Compose: 2.39.4.
- The exact pinned Rust 1.88 toolchain was used for the Linux build.
- `./scripts/bootstrap` passed on the Linux host.

## Issue 1: macOS-only temporary paths broke Linux tests

Two test helpers unconditionally create temporary directories below `/private/tmp`:

- `crates/switchyard-cli/src/host_runtime.rs`, in
  `failed_startup_cleanup_allows_a_clean_retry`.
- `crates/switchyard-router/src/host_gateway.rs`, in `test_directory`.

`/private/tmp` is the short canonical macOS temporary root. It does not exist on a
normal Linux installation. These hard-coded paths were introduced in commit `5080179`
while shortening Unix-socket paths for macOS.

Observed Linux results:

- One CLI test failed before its assertion because `tempdir_in("/private/tmp")`
  returned `NotFound`.
- Ten host-gateway tests failed through the shared helper for the same reason.
- With only the affected cases filtered out, the workspace completed with 247 passed,
  0 failed, and 5 normally ignored tests.

Required correction:

- Retain a deliberately short `/private/tmp` root on macOS so Unix-domain socket paths
  stay below macOS `SUN_LEN` limits.
- Use the platform temporary directory, or `/tmp`, on Linux.
- Keep the choice in one small test helper rather than repeating platform literals.
- Do not weaken the symlink, ownership, socket-length, or cleanup assertions.

Suggested shape:

```rust
#[cfg(target_os = "macos")]
fn short_test_temp_root() -> &'static Path {
    Path::new("/private/tmp")
}

#[cfg(not(target_os = "macos"))]
fn short_test_temp_root() -> &'static Path {
    Path::new("/tmp")
}
```

The helper may instead return `std::env::temp_dir()` outside macOS if the affected
socket path remains bounded in all tests.

## Issue 2: registered `--identity` is ignored by Docker's SSH transport

Switchyard correctly passes the registered identity to its direct SSH reachability
probe with `ssh -i <path>`. It then invokes Docker using `DOCKER_HOST=ssh://...` and
sets `DOCKER_SSH_OPTS` in these paths:

- `crates/switchyard-ops/src/devices.rs::docker_environment`
- `crates/switchyard-cli/src/runtime.rs::project_environment`
- `crates/switchyard-daemon/src/server.rs` in the generated-device Docker check

Docker Desktop's Docker CLI does not consume `DOCKER_SSH_OPTS`. The live command it
executed was equivalent to:

```text
ssh -l akhil -p 22 -o ConnectTimeout=30 -T -- 192.168.1.167 docker system dial-stdio
```

There was no `-i` argument. With several unrelated agent keys available, Docker failed
with `Too many authentication failures`, even though Switchyard's immediately preceding
direct SSH probe succeeded with the registered identity.

The same device check and complete remote lifecycle succeeded after loading only the
test identity into a clean `ssh-agent`. This proves the remote runtime itself works and
isolates the failure to identity propagation.

Docker officially documents SSH-agent or OpenSSH configuration for SSH-backed daemon
connections; it does not document `DOCKER_SSH_OPTS`:

<https://docs.docker.com/engine/security/protect-access/#use-ssh-to-protect-the-docker-daemon-socket>

Required correction:

- If `identity_file` is present, every Docker SSH operation must actually use that
  identity: eligibility, plan/apply lifecycle, status, logs, down, cleanup, daemon
  operations, and recovery.
- Preserve `BatchMode=yes` and add `IdentitiesOnly=yes` when an explicit identity is
  selected, preventing unrelated agent keys from being tried first.
- Do not construct a shell command containing an unquoted identity path. Paths with
  spaces and shell metacharacters must remain single argument values.
- When no identity is selected, preserve normal OpenSSH agent/config behavior.
- Do not mutate the user's persistent `~/.ssh/config` or leave a long-lived private
  agent behind.

One viable implementation direction is an ownership-safe temporary `ssh` launcher used
only in the environment of each Docker subprocess. Docker resolves an executable named
`ssh`; the launcher can invoke the absolute real SSH binary with argument-vector-safe
`-o BatchMode=yes`, `-o IdentitiesOnly=yes`, and `-i <identity>`, followed by Docker's
arguments. If this design is used:

- create the launcher below an owner-only directory;
- use a constant launcher body and pass paths through environment variables or a
  compiled helper, never interpolated shell text;
- resolve the real SSH executable before prepending the launcher directory to `PATH`;
- reject symlink/ownership substitution;
- scope its lifetime to the operation and clean it on success, failure, or cancel;
- centralize it so CLI, ops, and daemon execution cannot drift apart.

An internal compiled helper is preferable to a generated shell script if it avoids
packaging or lifecycle complexity. Requiring an agent/config and removing the
`--identity` feature would also be internally consistent, but it would be a product
scope reduction and should not be done silently.

Tests must validate observable subprocess behavior. Existing tests only assert that
the ineffective `DOCKER_SSH_OPTS` variable is present, which allowed the bug through.
A fake Docker executable should invoke the resolved `ssh` command and record its
argument vector, proving that:

- the selected identity is used;
- `BatchMode=yes` and `IdentitiesOnly=yes` are present;
- user, host, and non-default port survive;
- an identity path containing spaces remains one argument;
- no-identity mode continues to use normal SSH configuration;
- failure and cancellation remove temporary transport state.

## Live remote lifecycle evidence

Using a clean SSH agent, the macOS client successfully performed:

1. device registration and Docker eligibility (`docker 28.5.1`);
2. deployment validation and remote Compose generation;
3. remote provider startup to Compose `healthy`;
4. `switchyard status`, reporting `InSync` and the correct device/ownership labels;
5. `switchyard down` and ownership-aware `cleanup`;
6. an orphan check showing zero labeled containers and zero labeled networks.

The generated remote project used a deterministic device-scoped network and complete
Switchyard ownership labels.

## Hardware limitation that is not a Switchyard regression

The Snapdragon host's vendor kernel does not pass traffic between the host and Docker
bridge containers. Its container reached internal `healthy`, but macOS could not reach
the published HTTP port. This limitation was already recorded in `PROGRESS.md`; the
host's resident workloads use host networking for the same reason.

Use this machine to verify SSH eligibility, remote Compose lifecycle, health, labels,
status, recovery, and cleanup. Use a normal Linux Docker host for a routed
macOS-to-remote-provider traffic acceptance test.

## Required verification after fixing

Local/macOS:

```sh
cargo fmt --all -- --check
cargo test -p switchyard-ops devices
cargo test -p switchyard-cli host_runtime
cargo test -p switchyard-cli runtime
cargo test -p switchyard-daemon
cargo test -p switchyard-router host_gateway
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Linux/aarch64:

```sh
./scripts/bootstrap
cargo test --locked --workspace --all-features
```

Real macOS-to-Linux device proof, without preloading the identity into `ssh-agent`:

1. register a disposable key through `switchyard device add ... --identity <path>`;
2. ensure the agent contains unrelated keys, or use an empty agent;
3. require `switchyard device check` to report eligible;
4. run a provider-only remote deployment through up/status/logs/down/cleanup;
5. verify zero labeled remote containers, networks, and volumes remain;
6. repeat with an identity path containing spaces;
7. remove the disposable authorized key and local private key.

## Cleanup status from the verification run

- The disposable remote authorized key was removed.
- Temporary local and remote source, build, Rust toolchain, and Cargo data were removed.
- The proof left zero labeled containers and networks.
- Ordinary pulled Docker image cache was left intact because it may be shared.
- The repository worktree was clean before this handoff file was added.
