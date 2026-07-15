# Switchyard release {{VERSION}}

Supported platform: Linux-first. This archive was built and smoke-tested for
`{{OS}}/{{ARCH}}`; Switchyard does not claim cross-platform support from this host build.

## Verify the release

From the directory containing the release files:

```sh
sha256sum --check SHA256SUMS
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
