# Overlays and variations

An overlay is an ordered, portable change to a deployment. It can select deployment
instances, set or unset environment variables, load strict dotenv files, override
parameters and routes, and inject read-only files without modifying a source checkout.

## Format

```yaml
apiVersion: switchyard.dev/v1alpha1
kind: Overlay
metadata:
  name: mongodb-development
spec:
  selectors:
    deployment:
      matchLabels: { product: identity }
    instances:
      matchLabels: { tier: backend }
      names: [identity-a]
    optional: false
  environment:
    envFiles: [env/common.env]
    set:
      LOG_LEVEL: DEBUG
      API_TOKEN: { environmentVariable: IDENTITY_API_TOKEN }
    unset: [LEGACY_DATABASE_URL]
  variables: { enableNewSearch: "true" }
  parameters: { migrationPolicy: isolated-database }
  routes:
    database: mongodb-main
  files:
    - source: config/application-mongodb.yml
      target: /runtime/config/application.yml
      mode: "0644"
    - content: |
        newSearch=${overlay.variables.enableNewSearch}
        instance=${instance.name}
      target: /runtime/config/features.env
      template: true
      mode: "0600"
```

`selectors.deployment.matchLabels`, `selectors.instances.matchLabels`, and
`selectors.instances.names` are exact matches and combine with AND semantics. An empty
selector selects every instance. A required selector which matches nothing is an error;
use `optional: true` only when a no-op is intentional. Deployment labels live at
`metadata.labels`, and instance labels at `spec.instances[].labels`.

Deployment files may list overlays in `spec.overlays`. Paths are resolved relative to
the deployment file. CLI `--with` paths are appended in command-line order:

```yaml
spec:
  overlays:
    - overlays/common.yaml
    - overlays/local.user.yaml
```

Dotenv files accept only `KEY=VALUE`, blank lines, and whole-line `#` comments. Values
are literal: quotes, `$()`, backticks, and `${...}` have no shell behavior. Duplicate or
invalid keys are rejected.

## Precedence and origins

Values resolve in this order:

```text
adapter/block defaults
  < deployment overlays, in listed order
  < authored deployment instance values
  < CLI --set KEY=VALUE
```

Maps merge by key, while `environment.unset` removes an inherited key. A second overlay
which writes the same file target or route slot must explicitly opt into replacement.
For files use `replace: true` on the file; routes accept
`slot: { provider: name, replace: true }`. `spec.replace: true` applies to every keyed
entry in that overlay. Lists do not merge implicitly.

`switchyard plan` and `switchyard overlay diff` print each final value with its source
and warnings for shadowed layers. The generated manifest contains the same secret-safe
origin records. File values are represented by their SHA-256 content hash.

## File injection and templates

Files are written only below:

```text
.switchyard/generated/<deployment>/overlays/<instance>/<sha256>/
```

Each generated Compose application service receives a read-only bind mount at the
declared target. Targets must be normalized absolute container paths beneath `/runtime`,
the execution adapter's source mount, or a declared service volume target. Parent
traversal and targets outside those controlled roots are rejected.

Templates perform lookup substitution only. The complete namespace is:

- `overlay.variables.<name>`
- `instance.name`
- `deployment.name`
- `parameters.<name>`

Unknown or unterminated variables fail validation. There are no expressions, shell
commands, JavaScript, includes, or function calls. Text such as `$(command)` remains
literal text and is never executed.

## Secrets

Environment values may reference an environment variable or a file:

```yaml
PASSWORD: { environmentVariable: DATABASE_PASSWORD }
TOKEN: { file: /run/user/1000/switchyard/token }
```

Plans, resolved YAML, manifests, diffs, logs, and state show only a marker such as
`«secret: DATABASE_PASSWORD»`. Compose contains an opaque interpolation variable, never
the secret or its source reference. At apply time the CLI reads the reference, supplies
the value only to the Compose child process environment, and retains no copy. A single
trailing newline in a file reference is removed.

Secret-backed injected files are deliberately rejected with a “not yet supported”
diagnostic. Inline file content must therefore never contain a secret.

## CLI and concurrent variations

```text
switchyard overlay validate overlays/mongodb.yaml
switchyard overlay diff deployment.yaml --with overlays/mongodb.yaml
switchyard plan deployment.yaml --with overlays/mongodb.yaml --set LOG_LEVEL=TRACE
switchyard up deployment.yaml --with overlays/mongodb.yaml --variation mongo
switchyard status deployment.yaml --variation mongo
switchyard down deployment.yaml --variation mongo
```

`--with`, `--variation`, and `--set` may also be used by `plan`, `up`, `status`, and
`down`; `--with` and `--set` are repeatable. A variation changes the effective
deployment name to `<deployment>--<variation>`. Compose projects, containers, networks,
volumes, generated directories, and ownership labels consequently remain disjoint.
Fixed host-listener claims must also be disjoint; collision validation rejects two
variation plans which claim the same host and port.

The change preview compares against the current generated artifacts. Route-only changes
are `live`, environment/parameter/file changes are `restart`, and image or build changes
are `rebuild`, reported deterministically per service.
