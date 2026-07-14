# Router process

Build and run the Phase 1 router with a validated JSON router configuration, a local
administration socket, and a token supplied outside the configuration file:

```sh
cargo build --package switchyard-router
SWITCHYARD_ROUTER_TOKEN="$(openssl rand -hex 32)" \
  cargo run --package switchyard-router -- router.json /tmp/switchyard-router.sock
```

The process assembles Pingora HTTP/WebSocket/gRPC listeners and Tokio raw-TCP
listeners from one immutable route snapshot. HTTP requests pin the snapshot selected at
request start. TCP reloads apply the configured `close`, `drain`, or `pin` policy to
connections using the previous target.

The Linux-first administration channel is a mode-`0600` Unix socket. Each connection
accepts one newline-terminated JSON object of at most 1 MiB and returns one JSON object.
Every request includes the token:

```json
{"token":"...","operation":"current-version"}
```

Supported operations are `validate`, `apply`, `current-version`, `routes`, `health`,
`drain`, `counters`, and `events`. `validate` and `apply` include a `config` field.
Live apply accepts complete, strictly newer snapshots and keeps listener and identity
configuration fixed; those changes require a process restart. `drain` and `Ctrl-C`
perform orderly shutdown.

Inspection exposes active snapshot identity, HTTP and TCP data-plane counters, and a
bounded structured event history. Events never retain request headers or URIs, and
control-event fields with secret-like names are redacted.

Phase 1 supports cleartext HTTP/1.1, h2c/gRPC, WebSocket upgrades, and raw TCP. Local
TLS certificate management and HTTPS listener termination are intentionally added with
the native host gateway in Phase 3.
