# Portable deployment bundles

Portable bundles are reviewable JSON files for sharing a deployment definition and the
overlay definitions needed to reproduce its shape on another machine.

```sh
switchyard bundle export deployments/demo.yaml --with overlays/team.yaml \
  --output demo.switchyard-bundle.json

switchyard bundle import demo.switchyard-bundle.json --into imported/demo
switchyard validate imported/demo/demo.yaml
switchyard plan imported/demo/demo.yaml
```

The bundle format is `switchyard.dev/bundle/v1alpha1`. A bundle contains:

- bundle metadata: deployment name, deterministic created-at value, and source tool
  version;
- the deployment definition;
- referenced overlays embedded by overlay name and written back under `overlays/` on
  import;
- `requiredLocalInputs`, listing source directories, dotenv files, file inputs, or
  local values the receiver must provide;
- a SHA-256 `contentHash` over the canonical JSON payload. Import rejects a tampered
  bundle with `bundle_hash_mismatch` and rejects unknown bundle versions with
  `bundle_unsupported_api_version`.

Bundles deliberately omit generated and machine-local state. Anything under
`.switchyard/`, generated Compose, certificates, sockets, logs, daemon SQLite state,
and live source identity observations are never portable bundle content.

## Secrets and local inputs

Secrets are references, never values. Secret references in overlays remain references
to the receiver's environment variable or secret file. Literal values for
credential-looking keys such as `PASSWORD`, `TOKEN`, `SECRET`, `CREDENTIAL`,
`API_KEY`, and `*_KEY` are replaced by required local inputs and reported as warnings
during export.

Source paths are exported as `required-local-inputs/source-...` placeholders. Import
creates placeholder directories and small scaffold files only so `validate` and `plan`
can preview the imported topology. Replace those placeholders with real local paths or
edit the deployment sources before running `up`.

Do not commit machine-local overlays. Keep personal paths, credentials, and private
dotenv files in ignored files such as `overlays/local.user.yaml`; share only portable
configuration and secret references.

## Import reports

Import writes files and performs no Docker mutation. It reports compatibility, required
local inputs, the normal plan mutation preview, and conflicts:

- `name_conflict`: the deployment name already exists in `.switchyard/generated/` or
  daemon state. Rename the imported deployment or remove the old local deployment.
- `domain_conflict`: a custom domain is already claimed by another local manifest or
  daemon deployment. Choose a different local domain before applying.
- `port_conflict`: a host-router listener bind collides with another generated
  deployment. Change the listener port or stop the other deployment.
- `live_port_conflict`: a requested host listener is not currently bindable. Stop the
  process using it or edit the deployment.
- `external_resource_conflict`: a deterministic Docker container, network, or volume
  name already exists with another owner or no Switchyard ownership labels. Inspect the
  resource before cleanup.
- `docker_unavailable`: Docker was not reachable, so external resource checks were
  skipped. The import still made no Docker changes.

## Safe sharing

Review a bundle before import. It is plain JSON so normal code review can inspect block,
deployment, group, route, and overlay definitions.

Treat domains and ports as claims on the receiving machine, not as globally safe names.
An imported deployment can be valid but still conflict with another local project.

Share reusable blocks and groups as ordinary definitions when possible; use bundles
when you need one self-contained handoff that embeds overlays and records every local
input the receiver must supply.
