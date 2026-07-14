# Routing-matrix contract fixture

This fixture fixes the smallest topology that demonstrates consumer-specific routing:

```text
ui-1 ──► backend-1 ──► feature-services ──┐
ui-2 ──► backend-2 ──► main-services    ──┼──► services-shared/audit
ui-3 ──► backend-1 ──► feature-services ──┘
```

`contract.yaml` is the golden contract. `deployment.yaml` is the complete generated
Compose and native-gateway proof; `compose.yaml` remains the smaller Phase 2 sidecar
fixture. The applications use ordinary command-line/environment identity configuration,
but contain no Switchyard APIs, headers, libraries, or routing-aware behavior.

Every UI always requests `http://localhost:10081/identity`. Every backend always
requests the same five downstream addresses:

| Slot | Fixed application address |
|---|---|
| `catalog` | `http://localhost:8001/identity` |
| `search` | `http://localhost:8002/identity` |
| `reports` | `http://localhost:8003/identity` |
| `scheduler` | `http://localhost:8004/identity` |
| `audit` | `http://localhost:8005/identity` |

The router, not the applications, supplies consumer identity and selects providers.
`main-services` and `feature-services` each resolve all five slots. Their first four
providers differ; both resolve `audit` to the single `services-shared/audit` provider.

The `identityResponses` section is the golden observable contract. A provider response
identifies its service and concrete provider. A backend response identifies the backend
and embeds all five provider responses. The UI observation identifies the UI and the
backend response it receives. Tests should compare the parsed payload values, rather
than depending on JSON object key order.

## Run the Phase 4 routing proof

From the repository root, run:

```sh
./scripts/phase4-proof.sh
```

The command runs the workspace tests (including HTTP, WebSocket, gRPC, raw TCP, and
connection transition tests), then builds one fixture image and starts the planned
topology. It verifies all three custom domains and Origin routes, switches a UI and a
complete backend group without restarting application containers, rejects unhealthy
snapshots while retaining the active version, and prints snapshot/routing events. It
also exercises delayed readiness, provider/router/application crashes, native-gateway
recovery, a Docker/Compose restart cycle, and persistent-volume recovery. Its trap
stops and deletes only ownership-labelled fixture resources, including test volumes.

The runtime portion alone is available as `./examples/routing-matrix/smoke.sh`. It
requires ports `10081` and `18080` to be free and refuses to touch a pre-existing
deployment named `routing-matrix`.

The equivalent planner workflow is:

```sh
export SWITCHYARD_ROUTER_TOKEN="$(openssl rand -hex 32)"
cargo run -p switchyard-cli --bin switchyard -- validate examples/routing-matrix/deployment.yaml
cargo run -p switchyard-cli --bin switchyard -- up examples/routing-matrix/deployment.yaml
cargo run -p switchyard-cli --bin switchyard -- bind examples/routing-matrix/deployment.yaml backend-1 main-services
cargo run -p switchyard-cli --bin switchyard -- status examples/routing-matrix/deployment.yaml --routes
cargo run -p switchyard-cli --bin switchyard -- down examples/routing-matrix/deployment.yaml
```

`down` preserves the named data volumes. Delete them only with `switchyard cleanup
examples/routing-matrix/deployment.yaml --yes`.

## Backend-group boundary

A backend has one sidecar namespace and therefore one complete downstream group at a
time. `ui-1` and `ui-3` share `backend-1`, so switching that backend changes the group
for both UIs. A single backend cannot infer which inbound browser request caused a later
outbound `localhost:8001` connection; per-UI downstream selection would require the
application to propagate request context, which this proof intentionally forbids.

Deployment `uiRoutes` state makes this requirement explicit. Planning reports
`BackendGroupInvariant` when two UIs request different groups from the same backend and
instructs the user to create two backend instances. Those instances may select the same
source; isolation, rather than a source-code fork, is what provides independent groups.
