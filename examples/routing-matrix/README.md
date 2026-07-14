# Routing-matrix contract fixture

This fixture fixes the smallest topology that demonstrates consumer-specific routing:

```text
ui-1 ──► backend-1 ──► feature-services ──┐
ui-2 ──► backend-2 ──► main-services    ──┼──► services-shared/audit
ui-3 ──► backend-1 ──► feature-services ──┘
```

`contract.yaml` is a Phase 0 contract, not a runnable deployment. Runtime applications
are added in a later phase. The contract deliberately keeps application behavior free
of Switchyard APIs, headers, libraries, and environment variables.

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
