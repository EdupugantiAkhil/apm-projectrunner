# Local GUI

Build the dependency-free React client with the already provisioned Node toolchain:

```text
cd packages/web
npm run build
```

Adopt an existing code folder once, then launch its GUI from anywhere:

```text
switchyard project register path/to/code --name my-project
switchyard daemon install path/to/code
switchyard gui path/to/code
```

Registration preserves existing files, creates project-local Switchyard state and an
empty `deployments/` directory, and registers the folder itself as the first code
source. Repeating the same registration is safe. `switchyard daemon install [project]`
writes and starts a project-specific launchd LaunchAgent on macOS or systemd user unit on
Linux. `switchyard gui [project]` requires that daemon to be running, prints the local URL,
and makes a best-effort attempt to open it with `xdg-open` on Linux or `open` on macOS. If
the daemon is stopped, `gui` reports the exact install command instead of starting it.
Daemon output is appended to `.switchyard/daemon.log`. Failure to start the desktop opener
does not fail the command.

## Supported scope

The React GUI is the default local interactive client. A newly registered empty project
can add sources and devices, create and validate a deployment, edit its full authored
definition, preview or start it, inspect topology and logs, and move instances between
groups. The TUI remains an optional headless/SSH client rather than a required setup
step. Future guided authoring should extend the schema-driven dashboard instead of
introducing a dashboard-only product model.

## Credential handling

The launch URL is `http://127.0.0.1:<port>/gui/#token=<credential>`. The credential is
in the fragment, which is not sent in the HTTP request or included in server access
logs. The application removes the fragment immediately with `history.replaceState`
and retains the credential only in JavaScript memory. Ordinary API calls use the
bearer header. Operation SSE streams use the endpoint's loopback-only `access_token`
query exception because the browser `EventSource` API cannot set headers.

Static files below `/gui/` are public on the loopback listener. This permits the first
page load before the JavaScript client has consumed its fragment credential. It does
not weaken API authentication: all `/api/v1` endpoints remain protected.

GUI operations that start or update routers use a separate project router credential.
The daemon loads or creates `.switchyard/router-token` as an owner-only regular file
and injects it only into its CLI subprocesses and local router-administration calls; it
is never returned to browser code. The credential persists across daemon restarts so
already-running routers remain manageable. An explicitly supplied
`SWITCHYARD_ROUTER_TOKEN` seeds a missing credential file and must match an existing
one, preventing an accidental credential rotation while routers may still be running.

Git clone keeps the same loopback-only bearer boundary: `/api/v1/sources/clone` is an
authenticated API route and no additional listener or unauthenticated credential route
exists. The browser first starts a non-interactive operation that uses the daemon user's
Git credential helper, SSH configuration, and agent. If HTTPS authentication is still
required, the UI displays an in-browser username/password-or-token form for one retry.
The password field is uncontrolled React form state, is cleared immediately after the
request body is created, and submitted material is never rendered back into the page.
The API contract is deserialize-only for these fields and never echoes them.

Credentials pass through memory only: browser form/fetch memory, the daemon request and
clone-task values, Git's child environment, and the one-attempt askpass process. They are
not written to SQLite, `.switchyard/`, operation results, SSE events, or logs. Each
attempt creates an owner-only private temporary directory and an executable owner-only
`GIT_ASKPASS` shell helper containing only environment-variable lookups, never secret
material. Configured Git credential helpers are disabled for the submitted-credential
retry so Git cannot ask one to persist the value; the directory is removed when Git exits. An approved SSH public host key may
briefly be written there as a mode-0600 `known_hosts` file. Clone Git output is not
streamed: the normal operation timeline receives only fixed start/completion messages,
so submitted values cannot become event lines. The general
`switchyard_planner::redact_event_line` filter remains useful for ordinary commands but
cannot guarantee removal of an arbitrary password or token; this clone path therefore
does not rely on it.

An unknown SSH host is never silently accepted. After the ambient clone fails host-key
verification, the daemon obtains the public key with `ssh-keyscan`, derives its SHA-256
fingerprint with `ssh-keygen`, and returns only the host and fingerprint as a secret-free
operation challenge. The UI requires explicit approval and advises verification through
a trusted channel. The retry rescans and requires the exact approved host/fingerprint,
then uses `StrictHostKeyChecking=yes` with the isolated known-hosts file. It never uses
`StrictHostKeyChecking=no` or `accept-new` for clone.

## Deployment workspace

The shell provides keyboard-accessible Deployments, Sources, Devices, Operations, and Block
library views, plus a collapsible event/log drawer. Deployment detail contains a live
patch bay with an instance lane and a group lane; instances are not sorted into typed
lanes, because the schema has no service type to sort them by. The route matrix toggle
exposes the identical topology as a table; viewports below 1280 pixels select that table
automatically.

Select an instance to move it to another group. Selection prepares a modal preview of
the old and new complete ordered memberships and the route snapshots being superseded.
Nothing changes until **Apply membership move** is activated. Close, drain (with
timeout), and pin connection policies map directly to the `switchyard move` CLI options. The resulting
operation acknowledgement or structured rollback failure appears in Operations and
the event drawer.

The Routing panel loads the authored YAML with its optimistic hash. Group and instance
`address`, host-listener, and managed-profile changes show a full line diff and planner diagnostics.
Apply performs a dry-run validation before the definition PUT; an optional follow-up
can plan or run Up. This is deliberately the same portable workflow available without
the GUI: edit `deployments/<name>.yaml`, run `switchyard validate`, then plan or apply.

## Builder and schema forms

The existing deployment builder and schema forms provide the portable
deployment-definition workflow. Some richer guided controls still exist only in the
optional TUI, but the dashboard's full validated definition editor keeps those states
manageable without requiring a TUI session while browser-native interactions evolve.

**New deployment** opens the creation flow. Names use planner DNS-label rules. An
instance selects a source and block, while execution configuration comes entirely from
the chosen adapter's draft 2020-12 JSON Schema. The form supports scalar types, enums,
nested objects, and string arrays; an unsupported schema becomes a labeled JSON editor
with syntax validation. The Block library renders the same schemas read-only, so there
are no product-specific adapter forms in the client.

Builder changes are validated after a short idle period and may also be validated
explicitly. A successful result shows planner-derived expanded service and route data
before save. Save refuses overwrite through the daemon definition API and can
optionally start Up. Sources still supports unmanaged registration and managed
worktree creation/removal; dirty removal has its separate second confirmation.

## Devices

The Devices view lists registered SSH targets with a status badge and last-check time.
Its add form validates name, user, host, and port inline and accepts an optional identity
file path. **Check connection** refreshes the row with `ok`, `unreachable`, or
`auth-failed`; **Remove** requires an explicit confirmation and removes only the
registry entry.

Without an explicit identity, device authentication uses the daemon user's existing SSH
configuration and agent. When an identity path is selected, it is used exclusively for
the SSH probe and Docker operations, preventing unrelated agent keys from being tried
first. The GUI and API do not accept passwords or key contents, and SQLite stores only
the optional identity file path exactly as entered. Switchyard does not modify the
user's persistent SSH configuration.
