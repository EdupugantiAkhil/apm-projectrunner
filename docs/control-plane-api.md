# Local control-plane API

Switchyard's persistent control plane is available as either `switchyard daemon run`
or the equivalent `switchyard-daemon` binary. The CLI command is the normal developer
entry point because it can reuse the exact `switchyard` executable for script-compatible
operations. The separate binary is useful to service managers and packagers; set
`SWITCHYARD_CLI` when `switchyard` is not on its `PATH`.

The daemon always binds a loopback address. Its default is an ephemeral port on
`127.0.0.1`; a non-loopback `SWITCHYARD_DAEMON_BIND` is rejected. The global limit for
heavy build/start work defaults to two and can be set with
`SWITCHYARD_DAEMON_MAX_HEAVY`.

## Discovery and authentication

On startup, the daemon migrates `.switchyard/state.sqlite3`, reconciles generated
manifests and best-effort Docker label observations through `switchyard-state`, and
atomically writes `.switchyard/daemon.json`:

```json
{
  "apiVersion": "v1",
  "address": "127.0.0.1:49152",
  "token": "<random bearer credential>",
  "pid": 1234
}
```

The file is mode `0600`. Clients reject files accessible by group or other users,
incompatible versions, empty credentials, and non-loopback addresses. Every API request
requires `Authorization: Bearer <token>`. Missing and invalid credentials both receive
HTTP 401 and error code `unauthorized`; token comparison has no content-dependent early
exit. Credentials and secret-bearing output lines are never written to daemon events.

The API bearer credential is distinct from the router-administration credential. For
GUI and daemon-proxied commands, the daemon loads or creates the owner-only
`.switchyard/router-token`, passes it to `switchyard up` through the child environment,
and uses the same value for native binding changes. The value is not included in the
discovery document or any API response. Persisting it allows a restarted daemon to
continue administering routers that outlive the control-plane process. If
`SWITCHYARD_ROUTER_TOKEN` is explicitly set, it seeds a missing file and must match an
existing project credential.

## Version 1 endpoints

All supported routes are below `/api/v1`. JSON errors have stable `code`, `message`, and
optional `context` fields. Framework types are not part of the public Rust contract.

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/api/v1/system/status` | Daemon identity, PID, active count, and heavy-operation limit |
| `POST` | `/api/v1/system/shutdown` | Graceful authenticated shutdown |
| `POST` | `/api/v1/commands/validate` | Validate desired state |
| `POST` | `/api/v1/commands/plan` | Render the deterministic plan |
| `POST` | `/api/v1/commands/apply` | Apply/build/start (`switchyard up`) |
| `POST` | `/api/v1/commands/bind` | Change a binding |
| `POST` | `/api/v1/commands/status` | Inspect status, optionally including routes |
| `POST` | `/api/v1/commands/routes` | Inspect route snapshots |
| `POST` | `/api/v1/commands/logs` | Observe deployment or service logs |
| `POST` | `/api/v1/commands/open` | Open a managed browser profile |
| `POST` | `/api/v1/commands/down` | Stop while preserving volumes |
| `POST` | `/api/v1/commands/cleanup` | Ownership-checked destructive cleanup |
| `GET` | `/api/v1/operations/{id}` | Fetch current or durable terminal operation state |
| `POST` | `/api/v1/operations/{id}/cancel` | Request cooperative cancellation |
| `GET` | `/api/v1/operations/{id}/events` | Observe or resume the operation SSE stream |
| `GET` | `/api/v1/deployments/{deployment}/routes` | Query route versions and activation history |
| `GET` | `/api/v1/deployments` | List known deployments with applied hashes, latest operation, domains, and bindings |
| `GET` | `/api/v1/deployments/{deployment}` | Read the applied snapshot, generated source identities, resources, and reconciliation summary |
| `GET` | `/api/v1/deployments/{deployment}/definition` | Read authored YAML, absolute path, and SHA-256 edit hash |
| `POST` | `/api/v1/deployments` | Validate and atomically create a non-existing authored definition |
| `PUT` | `/api/v1/deployments/{deployment}/definition` | Validate and atomically replace an authored definition using its expected hash |
| `GET` | `/api/v1/adapters` | List built-in adapter declarations and configuration JSON Schemas |
| `GET` | `/api/v1/sources` | List registrations with live source identity |
| `POST` | `/api/v1/sources` | Register an existing path as unmanaged |
| `DELETE` | `/api/v1/sources/{name}` | Forget a registration without deleting files |
| `GET` | `/api/v1/worktrees?repository={source}` | Inspect every Git worktree for a registered repository |
| `POST` | `/api/v1/worktrees` | Create a managed linked worktree |
| `DELETE` | `/api/v1/worktrees/{name}` | Remove a managed worktree after the dirty-state guard |

A command request always contains `bundle`. Command-specific fields are `consumer` and
`group` for bind, `routes` for status, `target` for logs, `ui` for open, and `confirmed`
for cleanup. A bind may also carry `transition` as `close`, `pin`, or `drain` with a
`timeoutMs`; the CLI exposes the same choice through `--transition` and
`--drain-timeout-ms`. Creation returns HTTP 202 and a versioned operation document. Status is
`pending`, `running`, `succeeded`, `failed`, or `cancelled`. Script-compatible stdout,
stderr, and exit code remain available while an operation is active or retained in
memory; terminal status and structured error data are durable in SQLite across restart.
Raw output is deliberately not persisted because it can contain application secrets.

Definition creation accepts `{ "name": "demo", "yaml": "..." }`. Setting
`"validateOnly": true` runs the identical planner load/validate/plan path and returns
structured diagnostics plus a resource preview without retaining a file. A normal
create writes `deployments/<name>.yaml` atomically and returns HTTP 409
`deployment_exists` instead of overwriting. Definition replacement accepts
`{ "yaml": "...", "expectedHash": "<sha256>" }`; a stale hash receives HTTP 409
`definition_conflict`. Both writes validate before mutation. Planner validation errors
use HTTP 422 `validation_failed` with `context.diagnostics`; absent definitions use
`deployment_definition_not_found`. GET prefers a definition path recorded in an
applied snapshot when present and otherwise checks the project-local deployments
directory.

Source registration accepts `{ "name": "app", "path": "/code/app" }` and always
records the path as `unmanaged`. Worktree creation accepts `repository`, `ref`, and
optional `name` and `path`; any explicit path must remain under the project's
`.switchyard/worktrees` root. Worktree removal requires a JSON body containing
`allowDirty`. Omitting it or setting it to false returns `source_dirty` with staged,
unstaged, and untracked counts. Setting it to true is the distinct destructive
confirmation, but it still cannot target unmanaged or out-of-root paths. Expected
validation failures use stable codes including `source_path_not_found`,
`repository_unregistered`, `source_target_exists`, `source_ref_unknown`,
`source_unmanaged`, `source_dirty`, and `source_outside_managed_root`.

The daemon keeps the 64 most recent terminal operations in memory, including their raw
output and resumable event logs. Older terminal operations are evicted from memory;
`GET /api/v1/operations/{id}` continues to return their durable status from SQLite, but
their raw output and SSE replay are no longer available. Existing SSE streams retain
their event log until the connected client finishes.

Mutations acquire an expiring, heartbeated `switchyard-state` lease keyed by planned
deployment ID. A second mutation receives HTTP 409 and `operation_lock_contended`;
reads remain independent. Apply operations additionally acquire the global heavy-work
semaphore. Shutdown and cancellation terminate child commands cooperatively, record
`cancelled`, and release the lease.

Bind operations are daemon-owned rather than delegated to the one-shot CLI. The daemon
renders complete candidates for the consumer sidecar and running host gateway, then
uses their existing local administration sockets. A target becomes `active` in SQLite
only when its acknowledgement matches the candidate version and checksum and reports
`activated`. A timeout, rejection, stale acknowledgement, or checksum mismatch retains
the previously observed active version and records a failed attempt. If one target has
already activated when a later target fails, the daemon reapplies the prior routes at a
new monotonic version and records the rollback. Router provider-health rollbacks are
also retained with their structured, secret-safe diagnostic context.

The deployment route endpoint returns each router/binding's desired, current, previous,
and observed versions and checksums, transition policy, status, and last error code,
followed by append-only activation history. `switchyard status --routes` and
`switchyard routes` add a compact version summary when they execute through a daemon.

## Server-Sent Events

`GET /api/v1/operations/{id}/events` emits standard SSE records. IDs start at one and
increase monotonically per operation stream. Event names are `operation`, `build`,
`health`, `route`, and `log`. Send the last processed ID in `Last-Event-ID` to replay
retained later events. Streams retain the latest 2,048 records while their operation
remains among the 64 most recent terminal operations and close after the terminal
event. Disconnecting an observer does not cancel work, and an already connected stream
remains valid if its operation is evicted from the daemon's lookup map.

An SSE request may authenticate with the normal `Authorization` header or with an
`access_token` query parameter. The query form exists only because browser
`EventSource` cannot set request headers. It is accepted only for operation event
paths, and the daemon remains loopback-only. The GUI obtains the credential in a URL
fragment, removes the fragment immediately, holds the credential only in memory, and
places it in the SSE URL only when opening the local stream. API clients that can set
headers should continue to use bearer headers.

## Bundled GUI

The daemon serves static files below `/gui/`, with `index.html` as the fallback for
client-side views. Static GUI requests are deliberately exempt from bearer auth; every
`/api/v1` route remains guarded. The default directory is `packages/web/dist` under
the project root and can be changed with `DaemonConfig::gui_dist` or
`SWITCHYARD_GUI_DIST`. See [`gui.md`](gui.md) for build, launch, and security details.

## Compatibility policy

The URL prefix and `apiVersion` fields version the contract independently of the daemon
executable. Version 1 may gain optional fields, new event data, new error codes, and new
endpoints. Existing field meanings, required request fields, enum values, and endpoint
behavior do not change within v1. An incompatible change requires `/api/v2`; v1 remains
available for at least the next minor release and until the bundled CLI and supported
GUI have migrated.

## CLI behavior and test boundary

Overlay and variation arguments currently execute through the CLI's one-shot planner
and runtime path so apply-time secret references remain confined to the invoking process.
See [`overlays.md`](overlays.md) for the supported commands and safety model. The
versioned daemon command contract remains byte-compatible for overlay-less calls.

Ordinary `switchyard` commands inspect secure project-local discovery first. A reachable
daemon returns the existing stdout, stderr, and exit code. Missing or stale discovery
falls back to the original one-shot path, so scripts do not require a daemon.
`SWITCHYARD_BYPASS_DAEMON=1` is an internal recursion guard for the daemon backend.

Auth, versioning, SSE replay, locking, the heavy limit, cancellation, restart state, and
CLI output parity are tested with an in-memory HTTP service and stub operations, without
Docker. Real startup additionally needs permission to bind a loopback socket. Apply,
status, logs, down, cleanup, and Docker-label observation need a working Docker CLI for
meaningful runtime results; startup continues when Docker observation is unavailable.
