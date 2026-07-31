# Releases and diagnostics

APM ProjectRunner release archives are native host-platform builds for supported Linux hosts
and Apple Silicon on macOS 26 or newer. `scripts/release.sh` records the current
operating system and architecture in the artifact name and release notes and does not
cross-compile. Intel macOS archives are not supported.

## Build a release

Use the pinned Rust 1.88 toolchain and Node.js 24. Building requires the normal Cargo
dependency cache/network access and a clean GUI dependency install:

```sh
./scripts/release.sh
```

The script builds the three release binaries, runs `npm ci && npm run build`, and writes
these files under `dist/`:

- `apmpr-<workspace-version>+<git-describe>-<os>-<arch>.tar.gz`
- `RELEASE_NOTES.md`, generated from `docs/release-notes-template.md` plus commits since
  the newest reachable tag (or all commits when there is no tag)
- `SHA256SUMS` covering the archive and release notes

Linux archives use `sha256sum`; macOS archives use the system `shasum -a 256` when GNU
coreutils is absent. The installer, verifier, upgrade path, and uninstaller accept the
same checksum format and run with the stock macOS Bash 3.2. A Darwin release built on a
supported host is named with the `darwin-arm64` platform suffix.

Run the fast, Docker-free packaging proof against that output:

```sh
./scripts/release-smoke.sh
```

Pass `--build` to build first. The proof checks hashes, extracts into a temporary
directory, installs into a temporary prefix, invokes all three installed executables,
uninstalls, and requires the prefix to be empty.

## Verify checksums and signatures

Checksums are mandatory for every release:

```sh
cd dist
sha256sum --check SHA256SUMS
```

On macOS, the equivalent stock command is `shasum -a 256 --check SHA256SUMS`.

CI runs the Rust, GUI, documentation, and release-assembly gates on GitHub's Apple
Silicon `macos-26` image. Docker Desktop cannot run on that hosted VM because nested
virtualization is unavailable, so the workflow enables the live routing proofs only
when repository variable `APMPR_MACOS_DOCKER_RUNNER=true` selects a self-hosted
Apple Silicon runner labelled `apmpr-docker`.

Signing is optional so local and development builds do not depend on host GPG state.
Set `APMPR_SIGNING_KEY` to an SSH private-key file to produce
`SHA256SUMS.sig`:

```sh
APMPR_SIGNING_KEY=/secure/path/release-key ./scripts/release.sh
```

Distribute the corresponding allowed-signers entry through an authenticated channel.
Its identity must be `apmpr-release`, for example:

```text
apmpr-release ssh-ed25519 AAAAC3... trusted-release-key
```

Verify the exact checksum file with the fixed namespace and identity:

```sh
ssh-keygen -Y verify \
  -f allowed_signers \
  -I apmpr-release \
  -n apmpr-release \
  -s SHA256SUMS.sig < SHA256SUMS
```

An absent signature is explicit: the build prints that it is unsigned and continues.
It must still pass the platform checksum command above. APM ProjectRunner does not use GPG or
minisign.

## Install, upgrade, and uninstall

Extract the archive and run its installer. The default prefix is `~/.local` and does
not require root:

```sh
tar -xzf apmpr-<version>-<os>-<arch>.tar.gz
./apmpr-<version>-<os>-<arch>/install.sh
```

Use `--prefix <path>` for another user-writable prefix. The installer prints every
file it writes. It refuses to replace an existing path unless that path is recorded in
the previous APM ProjectRunner install manifest and its current checksum still matches. An
upgrade also hash-checks and removes files owned by the old manifest that are absent
from the new archive, so obsolete GUI assets cannot become unowned leftovers. The
GUI is installed below `share/apmpr/web`; the packaged daemon discovers it there,
or `APMPR_GUI_DIST` can explicitly select another build.

Before replacing an existing binary set, follow
[Upgrade and recovery](upgrade-recovery.md). That document is authoritative for daemon
shutdown, SQLite migration backups, downgrade recovery, and post-upgrade inspection.
The release notes link to the same procedure rather than copying it.

The current project-state schema is v7: v6 introduced reviewed source-profile imports,
and v7 records remote-device eligibility. Upgrades migrate older state forward in order
and retain the existing pre-migration backup behavior. After an upgrade, inspect both
local deployments and any per-device remote resources before applying further changes.

Uninstall through the installed helper, using the same prefix:

```sh
~/.local/bin/apmpr-uninstall
# or: /chosen/prefix/bin/apmpr-uninstall --prefix /chosen/prefix
```

The helper reads `share/apmpr/installed-files.manifest`, verifies every current
file against its recorded checksum, and only then removes those files and the manifest.
It refuses a partial or ambiguous cleanup when an installed file is missing, replaced,
modified, or symlinked. Uninstalling binaries does not delete project-local
`.apmpr` state or deployment resources; use the documented deployment lifecycle
commands separately.

## Diagnostics bundle

Create one JSON report from an authored deployment:

```sh
apmpr diagnostics deployment.yaml
apmpr diagnostics deployment.yaml --output /tmp/report.json
```

The default name is
`apmpr-diagnostics-<deployment>-<unix-timestamp>.json`. The report contains tool,
Git, host, Docker, and Compose version observations; deployment validation and hashes;
daemon deployment detail when reachable or deployment-scoped generated/runtime JSON
state otherwise; a recent host-gateway log tail; live router events when the router
token is available; and best-effort read-only Docker ownership-label observations.
Missing commands, Docker, daemon, state, logs, or routers are recorded as unavailable
instead of preventing collection.

Redaction runs recursively over every string before the owner-only output file is
written. It uses the same conservative line convention as daemon events, masks values
whose JSON keys look credential-related using the planner's portable-bundle heuristic,
and removes the values of credential-looking process environment variables (the same
name heuristic) wherever they appear. Benign environment values such as `$HOME` are
not scrubbed out of paths — erasing them would destroy diagnosability without hiding
a secret. Overlay secret references are never resolved into the report; neither
their environment/file values nor the process environment is enumerated. The daemon
discovery token and `APMPR_ROUTER_TOKEN` are used only for local read-only
requests and are included in the exact-value redaction set. Unit coverage verifies
that credential fields, authorization/token log lines, and embedded secret
environment values do not survive.

Redaction is deliberately conservative and can hide harmless lines, but no heuristic
can recognize every application-specific secret. The command therefore always prints
a reminder to review the JSON before sharing it.
