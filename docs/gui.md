# Local GUI

Build the dependency-free React client with the already provisioned Node toolchain:

```text
cd packages/web
npm run build
```

Start the loopback daemon from the project root, then launch the GUI:

```text
switchyard daemon run
switchyard gui
```

`switchyard gui` requires a reachable daemon. It prints the local URL and makes a
best-effort attempt to open it with `xdg-open` on Linux or `open` on macOS. Failure to
start the desktop opener does not fail the command.

## Supported scope

The React GUI is a supported secondary client for deployment monitoring and operations.
It retains its deployment status, topology, route operations, logs, source inspection,
and lifecycle controls.

New control-plane authoring workflows are TUI-only: source-local startup profiles, the
guided instance-creation wizard, the connections matrix, and device placement. The GUI
is not scheduled to receive these workflows. Adding GUI authoring parity requires a
separate milestone and design decision rather than following TUI work automatically.
The existing operational route table, topology visualization, and complete binding
changes remain supported; they are not the new Connections authoring workflow.

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

## Deployment workspace

The shell provides keyboard-accessible Deployments, Sources, Devices, Operations, and Block
library views, plus a collapsible event/log drawer. Deployment detail contains a live
patch bay with UI-consumer, backend/provider, and provider-group lanes. Cables carry a
direction arrow and a capability label as well as their capability color. The route
matrix toggle exposes the identical topology as a table; viewports below 1280 pixels
select that table automatically.

Select a consumer node to change its complete provider-group binding. The select lists
only groups that satisfy every current slot. Selection prepares a modal preview of all
old and new slot providers and the route snapshot being superseded. Nothing changes
until **Apply complete change** is activated. Close, drain (with timeout), and pin
connection policies map directly to the `switchyard bind` CLI options. The resulting
operation acknowledgement or structured rollback failure appears in Operations and
the event drawer.

The Routing panel loads the authored YAML with its optimistic hash. Domain listener,
`uiRoutes`, and managed-profile changes show a full line diff and planner diagnostics.
Apply performs a dry-run validation before the definition PUT; an optional follow-up
can plan or run Up. This is deliberately the same portable workflow available without
the GUI: edit `deployments/<name>.yaml`, run `switchyard validate`, then plan or apply.

## Builder and schema forms

The existing deployment builder and schema forms remain available for their current
deployment-definition workflow. They do not constitute parity with the new TUI-only
authoring workflows described above.

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

Device authentication uses the daemon user's existing SSH keys, configuration, and
agent. The GUI and API do not accept passwords or key contents, and SQLite stores only
the optional identity file path exactly as entered.
