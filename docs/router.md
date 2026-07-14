# Router process

Build and run the router with a validated JSON router configuration, a local
administration socket, and a token supplied outside the configuration file:

```sh
cargo build --package switchyard-router
SWITCHYARD_ROUTER_TOKEN="$(openssl rand -hex 32)" \
  cargo run --package switchyard-router -- sidecar router.json /tmp/switchyard-router.sock
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
configuration fixed; those changes require a process restart. Providers with declared
health checks are probed before activation. If candidate readiness fails, the apply
returns `provider_unhealthy` with `status: rolled_back`, the previous version remains
active, and health/reload events record the rejection. DNS failures are resolved before
Pingora peer construction so an unavailable container cannot panic a data-plane worker.
`drain` and `Ctrl-C` perform orderly shutdown.

Inspection exposes active snapshot identity, HTTP and TCP data-plane counters, and a
bounded structured event history. Events never retain request headers or URIs, and
control-event fields with secret-like names are redacted.

## Native host gateway

Host mode performs a non-mutating preflight of every domain, listener port, and
loopback upstream before it creates credentials or starts listeners:

```sh
SWITCHYARD_ROUTER_TOKEN="$(openssl rand -hex 32)" \
  cargo run --package switchyard-router -- host host-router.json /tmp/switchyard-host.sock
```

Host listeners and their container upstreams must both use loopback addresses. Custom
domains are exact, case-insensitive matches; duplicate domain claims and occupied ports
fail before certificate files are changed. Unprivileged ports are the default. Binding
ports below 1024 requires an explicit operating-system capability or redirect and is
not performed by Switchyard.

When a deployment includes `hostRouter`, `switchyard up` starts the native gateway
after Compose reports healthy and waits for all listeners plus its administration
socket. The CLI finds `switchyard-router` beside itself or on `PATH`; set
`SWITCHYARD_ROUTER_BIN` to an explicit binary when developing from separate build
directories. Deployment-scoped PID identity and logs live under
`.switchyard/run/<deployment>` with owner-only permissions. `status` reports Docker and
host state, `down` stops only a PID whose Linux start time, executable, command line,
and generated config all match the ownership record, and `cleanup --yes` removes
marker-owned host credentials after stopping the deployment. Stale or reused PIDs are
never signaled.

For a provider backed by a dynamically published Compose port, set its authored
endpoint to a loopback address with port `0` and add `hostUpstreams.<provider>` with an
instance, service, and container port declared by that service's `publish` list. After
Compose starts, the CLI accepts exactly one nonzero loopback result from
`docker compose port` and writes the resolved configuration under
`.switchyard/run/<deployment>/host-router.json`. Providers with a concrete loopback
port remain valid for externally managed local processes.

Docker may assign a new ephemeral loopback port when a published container namespace
is recreated or restarted. A later `switchyard up` compares the running host config to
current `docker compose port` observations and safely refreshes the owned gateway when
they differ.

## Backend-group invariant

One backend instance owns one consumer network namespace and one complete downstream
group. UIs may share that backend only when they also share its selected group. Without
application-level context propagation, the backend cannot associate an outbound fixed
localhost connection with the inbound UI request that caused it. If two UIs need the
same backend source with different groups, declare two backend instances from that
source. The planner's `uiRoutes` cross-check emits `BackendGroupInvariant` before any
mutation when this rule is violated; `switchyard bind` updates every attached UI's
recorded group expectation together with the backend binding.

An HTTPS listener uses its `tls.certificate` and `tls.privateKey` paths. Missing pairs
are generated as 90-day self-signed identities, the key is mode `0600`, and identities
renew during the final 30 days. Existing pairs without a Switchyard ownership marker
are treated as external and never overwritten or removed. Review trust-store commands
without changing system state, then remove only Switchyard-owned files with:

```sh
cargo run --package switchyard-router -- certificates trust host-router.json
cargo run --package switchyard-router -- certificates cleanup host-router.json
```

Trust installation remains an explicit user action because it changes the OS/browser
security boundary. For non-`*.localhost` names, configure local DNS or `/etc/hosts`
separately; certificate generation does not claim DNS ownership.

Managed profiles are planned into `host-router.json` as dedicated loopback listeners.
Each requires `proxyAuthentication` with a private credential file; host mode creates
missing credentials as mode `0600`, and proxy authorization is never part of the public
managed-profile metadata. The managed forward-proxy fallback currently accepts HTTP
targets only and rejects an HTTPS `startUrl` because CONNECT tunneling is not
implemented. HTTPS remains supported on normal host-gateway listeners using Origin or
the explicit route header.

The planner preserves every unambiguous cleartext local destination used by a managed
profile identity as an exact host-and-port proxy target. The `startUrl` must match one
of those targets, including its effective port, while the same profile may access
other declared UI or API targets. Absolute-form HTTP requests are accepted only when
their URI authority and `Host` header agree and match one declared target; CONNECT,
undeclared ports, remote hosts, and origin-form requests fail closed.
