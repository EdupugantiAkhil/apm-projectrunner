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
but contain no APM ProjectRunner APIs, headers, libraries, or routing-aware behavior.

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
`main-services` and `feature-services` are complete ordered memberships. Their first four
service instances differ; separate audit instances reuse the same source and startup
profile while preserving the one-instance-one-group rule.

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
export APMPR_ROUTER_TOKEN="$(openssl rand -hex 32)"
cargo run -p apmpr-cli --bin apmpr -- validate examples/routing-matrix/deployment.yaml
cargo run -p apmpr-cli --bin apmpr -- up examples/routing-matrix/deployment.yaml
cargo run -p apmpr-cli --bin apmpr -- move examples/routing-matrix/deployment.yaml backend-1 main-services
cargo run -p apmpr-cli --bin apmpr -- status examples/routing-matrix/deployment.yaml --routes
cargo run -p apmpr-cli --bin apmpr -- down examples/routing-matrix/deployment.yaml
```

`down` preserves the named data volumes. Delete them only with `apmpr cleanup
examples/routing-matrix/deployment.yaml --yes`.

## Instance-group boundary

An instance has one sidecar namespace and belongs to at most one complete group.
`backend-1` moves between the two groups in this proof, so every member of its destination
group receives the same ordered localhost view.

When two groups need the same code or startup profile simultaneously, the deployment
declares two instances, as it does for the audit service. The instances may reuse the
same source and block; the runtime instance itself is never shared across groups.
