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

## Current views

The shell provides keyboard-accessible Deployments, Sources, and Operations views,
plus a collapsible event/log drawer. Deployment detail shows source identities,
resource state, route versions, domains, and bindings. Sources supports unmanaged
registration and managed-worktree creation/removal; dirty removal has a separate
second confirmation. Commands started by this browser session appear in the operation
timeline and stream build, health, route, operation, and log events into the drawer.

The patch-bay editor, domain forms, route switching controls, and JSON Schema form
renderer are intentionally reserved for Part 4b. The `/api/v1/adapters` endpoint and
the three-pane shell are their extension points.
