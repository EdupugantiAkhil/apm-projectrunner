# Switchyard release {{VERSION}}

This native archive was built and smoke-tested for `{{OS}}/{{ARCH}}`. Supported targets
are Linux `x86_64`/`aarch64` and Apple Silicon `darwin/arm64` on macOS 26 or newer;
Switchyard does not claim support for a different target from this host build.

## Verify the release

From the directory containing the release files:

```sh
sha256sum --check SHA256SUMS
# stock macOS:
shasum -a 256 --check SHA256SUMS
```

When `SHA256SUMS.sig` is present, obtain the project's trusted `allowed_signers` file
through a separate authenticated channel, then run:

```sh
ssh-keygen -Y verify -f allowed_signers -I switchyard-release -n switchyard-release \
  -s SHA256SUMS.sig < SHA256SUMS
```

The signer identity in `allowed_signers` is `switchyard-release`. An absent signature
means the release is checksum-protected but unsigned.

## Upgrade and recovery

Follow [the upgrade and recovery procedures](../docs/upgrade-recovery.md) before
replacing a binary set. The release archive's installer supports ownership-checked
replacement of an existing manifest-owned installation.

## Changes

{{CHANGELOG}}
