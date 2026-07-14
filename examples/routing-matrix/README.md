# Routing-matrix contract fixture

This fixture fixes the smallest topology that demonstrates consumer-specific routing:

```text
ui-1 ──► backend-1 ──► feature-services ──┐
ui-2 ──► backend-2 ──► main-services    ──┼──► services-shared/audit
ui-3 ──► backend-1 ──► feature-services ──┘
```

`contract.yaml` is the Phase 0 golden contract. `deployment.yaml` exercises the Phase 2
planner and CLI, while `compose.yaml` is a standalone runtime proof. The fixture
deliberately keeps application behavior free of Switchyard APIs, headers, libraries,
and environment variables.

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

## Run the Phase 2 proof

From the repository root, run:

```sh
./examples/routing-matrix/smoke.sh
```

The script builds one image containing the router and a dependency-free fixture binary,
starts both backends behind namespace-sharing sidecars, and checks their fixed
`localhost:8001` through `localhost:8005` calls. It atomically switches backend-1's
complete five-service group and verifies neither backend restarted. It then runs
`docker compose down` without deleting named volumes, starts the deployment again, and
verifies each backend's persistent request counter increased. Containers are stopped on
exit; the two named volumes remain until explicitly removed with `docker compose -f
examples/routing-matrix/compose.yaml down --volumes`.

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
