# Security review

Review date: 2026-07-16  
Scope: Phase 7 host listeners, extension permissions, administration channels, Docker
authority, file mounts, and secret handling.

This is a source and test review, not a penetration-test certification. The attacker
models used below include an untrusted LAN/browser client, a local process running as
another user, a malicious or mistaken deployment definition, a compromised container,
and tampered project-local state. A process that can control the Docker daemon is already
host-authoritative: Switchyard ownership labels prevent accidents, but cannot defend
against another Docker-authorized process that can forge those labels or mount the host
filesystem.

## Findings summary

| ID | Severity | Area | Summary |
| --- | --- | --- | --- |
| SR-1 | medium | Admin channels | Public GUI serving follows symlinks outside the configured distribution root |
| SR-2 | high (remediated) | Docker authority | `up --remove-orphans` can delete Compose-project containers before ownership is proved |
| SR-3 | high | File mounts | Script sources may be broad host directories and may be mounted writable |
| SR-4 | high | File mounts | Generated and imported file writes lack symlink-safe containment |
| SR-5 | medium | File mounts | Overlay target validation does not enforce the promised container-symlink boundary |
| SR-6 | medium | File mounts | Script containers do not enforce the non-root default required by the design |
| SR-7 | high | Secret handling | Literal credential-looking environment values are accepted into generated artifacts |
| SR-8 | medium | Secret handling | Daemon command results retain and return raw stdout/stderr despite event redaction |
| SR-9 | informational | Secret handling | Diagnostics redaction is deliberately heuristic and can miss unconventionally named secrets |

There were no critical or low-severity findings. Remediation proposals are design
recommendations; this audit did not change product code.

## 1. Host listeners

### Examined

- Host preflight, LAN acknowledgement, provider restriction, claim reservation, and
  certificate/credential lifecycle:
  [`host_gateway.rs`](../crates/switchyard-router/src/host_gateway.rs#L81-L385) and its
  managed-path checks at
  [`host_gateway.rs`](../crates/switchyard-router/src/host_gateway.rs#L506-L737).
- Router schema enforcement for acknowledged LAN exposure and loopback providers:
  [`v1alpha1.rs`](../crates/router-config/src/v1alpha1.rs#L67-L122) and
  [`v1alpha1.rs`](../crates/router-config/src/v1alpha1.rs#L232-L253).
- HTTP identity selection, header stripping, proxy authentication, and CORS:
  [`lib.rs`](../crates/router-pingora/src/lib.rs#L619-L860) and
  [`lib.rs`](../crates/router-pingora/src/lib.rs#L1040-L1170).
- Managed forward-proxy request parsing and target restriction:
  [`forward_proxy.rs`](../crates/switchyard-router/src/forward_proxy.rs#L116-L220) and
  [`forward_proxy.rs`](../crates/switchyard-router/src/forward_proxy.rs#L220-L445).
- Operator-facing exposure and trust guidance in
  [`router.md`](router.md) and [`browser-routing.md`](browser-routing.md).

### Threat model

The review tried to turn a default local listener into a LAN listener without the
explicit opt-in, widen a LAN gateway's provider connection, claim the same port/domain
twice, spoof tab route authority over LAN, obtain permissive CORS, relay the managed
forward proxy to an arbitrary host, leak proxy credentials upstream, and redirect
certificate writes or cleanup through symlinks.

### Verified protections

- `RouterConfig::validate` and `host_gateway::preflight` both reject a non-loopback bind
  unless `mode: lan` and `acknowledgeLanExposureRisk: true` are present. Wildcard binds
  are expanded into concrete interface addresses for the warning. Tests:
  `exposure_summary_expands_wildcard_binds_to_actual_interfaces` and the invalid fixtures
  `lan-bind-without-opt-in.json` and `lan-opt-in-without-acknowledgement.json`.
- Host provider upstreams remain loopback-only even in LAN mode. Enforcement is
  `HostGatewayError::UnsafeUpstream` in `preflight`; proof is
  `provider_upstreams_remain_loopback_only_with_and_without_lan_mode` and
  `lan-non-loopback-provider.json`.
- Preflight holds every socket reservation until all claims succeed and detects
  normalized domain collisions before writing certificates. Tests:
  `preflight_reports_an_occupied_port_without_writing_certificates` and
  `preflight_rejects_a_domain_claimed_by_different_slots`.
- Explicit `X-Switchyard-Route` identity is ignored on non-loopback listeners even when
  LAN exposure is acknowledged. The proving integration test is
  `explicit_identity_is_rejected_on_non_loopback_listener`. Identity is stripped unless
  both the router-wide policy and selected provider opt in; proof:
  `identity_header_preservation_requires_selected_provider_opt_in`.
- CORS reflects only a configured, matching origin, never `*`, and rejects malformed or
  untrusted preflights. Proof: `browser_routes_enforce_origin_and_answer_cors_preflight`.
- Managed-profile proxy listeners require exactly one constant-time-checked credential,
  remove `Proxy-Authorization`, reject URI/Host credentials and mismatched authorities,
  allow only declared local HTTP targets, and cap headers and request bodies. Proof:
  `managed_profile_listener_requires_and_strips_proxy_credentials` and the Phase-3 exit
  gate's mismatched-authority and credential-leak assertions.
- Managed private keys, credentials, and ownership markers use mode `0600`; generated
  parent directories use `0700`; symlinked path components are rejected; cleanup
  requires deployment/path/digest ownership. Proof:
  `managed_certificates_are_secure_renewable_and_cleanable`,
  `external_certificates_are_never_overwritten_or_removed`,
  `managed_proxy_credentials_are_private_and_owned`, and
  `managed_files_reject_symlinked_parent_directories`.

### Findings

No host-listener defect was found. The negative cases above exercised the documented LAN
opt-in, non-loopback identity boundary, remote provider attempt, duplicate claim,
credential relay, and symlinked certificate path. Public-internet operation remains
unsupported rather than a distinct bind mode; the acknowledgement and expanded
interface warning make exposure non-silent, but host firewall policy remains the
operator's responsibility.

## 2. Extension permissions

### Examined

- Manifest permissions and host access:
  [`manifest.json`](../extensions/switchyard-route/manifest.json#L1-L21).
- Endpoint/route validation, per-tab session rules, header modification, and cleanup:
  [`service-worker.js`](../extensions/switchyard-route/service-worker.js#L1-L131).
- Checked-in route declarations and popup messaging:
  [`routes.js`](../extensions/switchyard-route/routes.js) and
  [`popup.js`](../extensions/switchyard-route/popup.js).
- Installation, disable, and removal instructions:
  [`extensions/switchyard-route/README.md`](../extensions/switchyard-route/README.md) and
  [`browser-routing.md`](browser-routing.md#explicit-tab-header).

### Threat model

The review tried to grant remote-site access, inject the route header outside a declared
endpoint or tab, select a route not present in `routes.js`, preserve rules after tab
closure/disconnect, and find a path for a web page to invoke extension operations.

### Verified protections

- Permissions are limited to `activeTab`, session `storage`, and declarative request
  modification with explicit host access. The checked-in host permission is only
  `http://localhost:10081/*`; there is no content script, broad web host pattern,
  cookies, downloads, native messaging, or `externally_connectable` surface.
- `normalizeEndpoint` accepts only HTTP(S) loopback/`*.localhost` URLs without URL
  credentials, query, or fragment. `configuredRoutes` rejects invalid and duplicate
  identifiers. `connect` resolves the requested identifier against that declaration,
  so the extension cannot create a rule for an undeclared route.
- The declarative rule is restricted by `tabIds`, declared endpoint regexes, resource
  types, and manifest host permissions. It is a session rule, is removed by Disconnect,
  and is removed on `tabs.onRemoved`. The router independently rejects an identifier not
  declared for that destination, so editing `routes.js` cannot create router authority.
- The documentation tells the operator to keep `routes.js` and `host_permissions`
  aligned with the active deployment, and documents Disable as a pause and Remove as
  uninstall without changing application/deployment files.

There is no browser-automation suite for this unpacked extension; the protection claims
above are source-enforced and require Chromium integration verification during release
testing.

### Findings

No extension defect was found. Attempts to expand rule scope encounter three independent
boundaries: local URL normalization, manifest host access, and router-side declared route
resolution. A route ID is not cryptographically tied to one deployment, but it confers
authority only at an endpoint whose active router declares that ID.

## 3. Administration channels

### Examined

- Router Unix socket creation, token validation, framing, and event redaction:
  [`lib.rs`](../crates/switchyard-router/src/lib.rs#L201-L290),
  [`lib.rs`](../crates/switchyard-router/src/lib.rs#L341-L480), and
  [`lib.rs`](../crates/switchyard-router/src/lib.rs#L871-L947).
- Daemon bind enforcement, route middleware, GUI serving, SSE exception, and discovery
  lifecycle:
  [`server.rs`](../crates/switchyard-daemon/src/server.rs#L1040-L1264),
  [`server.rs`](../crates/switchyard-daemon/src/server.rs#L2262-L2284),
  [`server.rs`](../crates/switchyard-daemon/src/server.rs#L2911-L2940), and
  [`server.rs`](../crates/switchyard-daemon/src/server.rs#L3066-L3109).
- Host process state and signal identity:
  [`host_runtime.rs`](../crates/switchyard-cli/src/host_runtime.rs#L439-L499) and
  [`host_runtime.rs`](../crates/switchyard-cli/src/host_runtime.rs#L703-L845).
- mDNS and Tailscale state paths:
  [`lan_preflight.rs`](../crates/switchyard-cli/src/lan_preflight.rs#L370-L445),
  [`lan_preflight.rs`](../crates/switchyard-cli/src/lan_preflight.rs#L850-L932), and
  [`tailscale_publication.rs`](../crates/switchyard-cli/src/tailscale_publication.rs#L140-L280).

### Threat model

The review tried unauthenticated router/daemon calls, oversized router frames,
non-loopback daemon binds, bearer-token use on unrelated query paths, static-file `..`
and symlink traversal, discovery/state replacement, and PID reuse before a stop signal.

### Verified protections

- Router startup rejects an empty admin token. Its Unix socket is mode `0600`, requests
  are newline-delimited and capped at 1 MiB, and tokens are compared without a
  content-dependent early exit. `authenticates_inspects_applies_and_drains` proves the
  authenticated path; `token_comparison_and_redaction_are_safe` proves comparison and
  nested sensitive-key redaction. The frame limit is code-enforced but has no direct
  boundary test.
- Router control events are bounded to 256 and recursively redact keys containing
  authorization, cookie, password, secret, token, or private-key terms before retention.
- The daemon checks the configured address before startup and the resolved listener
  after bind; both must be loopback. `non_loopback_binding_is_refused_before_listener_startup`
  proves the first guard. Every non-GUI route passes bearer middleware.
- Query tokens are considered only for `/api/v1/operations/*/events`; all other paths
  require the header. `gui_static_files_are_public_while_api_and_sse_query_tokens_stay_guarded`
  and `auth_and_versioned_surface_are_enforced` prove the route boundary.
- Daemon discovery uses a random 256-bit token and mode `0600`; clients validate mode,
  API version, non-empty token, and loopback address. Test:
  `discovery_is_private_and_token_comparison_handles_different_lengths`.
- Host, mDNS, and Tailscale runtime directories reject symlinked ancestors/leaves and
  state files must be regular owner-only files. Before host or mDNS signaling, ownership
  fields, Linux process start ticks, executable identity, and command line are rechecked.
  Tests include `runtime_paths_reject_symlinked_ancestors_and_leaves` and
  `publication_state_round_trips_as_owner_only_json`; process identity is primarily
  source-enforced and exercised by lifecycle tests rather than a PID-reuse integration
  test.

### Findings

#### SR-1 — Public GUI serving follows symlinks outside its root (medium)

`serve_gui` rejects non-`Normal` lexical components at
[`server.rs`](../crates/switchyard-daemon/src/server.rs#L1225-L1239), but then calls
`is_file` and `tokio::fs::read` on the joined path. Both follow a symlink inside
`gui_dist`. Because `/gui/*` deliberately bypasses bearer authentication, a symlink such
as `packages/web/dist/assets/state -> ../../../../.switchyard/daemon.json` can expose a
readable file to any local HTTP client. The existing GUI test covers public assets and
API/SSE auth, but not `..` or symlink traversal.

Proposed remediation: canonicalize `gui_dist` and the requested regular file, require
the latter to remain beneath the former, reject symlink components, and add lexical and
symlink escape tests. Prefer opening through an already-open root directory with
no-follow semantics to reduce check/use races.

## 4. Docker authority

### Examined

- Generated ownership labels and loopback publishing:
  [`lib.rs`](../crates/switchyard-planner/src/lib.rs#L1364-L1578),
  [`lib.rs`](../crates/switchyard-planner/src/lib.rs#L1996-L2002), and
  [`lib.rs`](../crates/switchyard-planner/src/lib.rs#L2354-L2361).
- Docker discovery, inspection, down, and cleanup:
  [`runtime.rs`](../crates/switchyard-cli/src/runtime.rs#L205-L310) and
  [`runtime.rs`](../crates/switchyard-cli/src/runtime.rs#L452-L502).
- Host-provider published-port parsing:
  [`host_runtime.rs`](../crates/switchyard-cli/src/host_runtime.rs#L499-L583) and
  [`host_runtime.rs`](../crates/switchyard-cli/src/host_runtime.rs#L754-L782).

### Threat model

The review tried to make cleanup select a same-name, same-project, or deployment-labeled
resource without both ownership labels; expose an application port on all interfaces;
accept a non-loopback provider publication; and find mutating Compose commands that run
without label proof. It also treats access to the Docker socket as equivalent to host
authority, not as a sandbox boundary.

### Verified protections

- Every generated container, network, and named volume carries
  `dev.switchyard.managed=true`, the deployment label, and a resource hash. Application
  ports are generated as `127.0.0.1::<container-port>`.
- `down` discovers both deployment- and Compose-project resources and calls
  `verify_ownership` before Compose mutation. `cleanup` adds an explicit `--yes` gate and
  is the only normal path that passes `--volumes`. Tests:
  `down_does_not_delete_volumes`, `destructive_cleanup_requires_confirmation`, and
  `ownership_rejects_a_resource_without_managed_label`.
- A Docker-published host provider is accepted only when exactly one nonzero loopback
  address is returned. Test: `accepts_exactly_one_nonzero_loopback_publication`.

Ownership labels protect against accidental cross-project mutation. They are not an
authorization mechanism against another Docker-authorized actor, which can forge labels
or directly remove resources. Setup and operational access to Docker must therefore be
limited like root-equivalent host access.

### Findings

#### SR-2 — Apply can remove orphans before ownership proof (high)

`DockerRuntime::up` at
[`runtime.rs`](../crates/switchyard-cli/src/runtime.rs#L205-L223) runs `docker compose up
--remove-orphans` without first discovering the Compose project and calling
`verify_ownership`. In contrast, `down` and `cleanup` perform that proof. A container
carrying the same `com.docker.compose.project` label but not matching Switchyard's
ownership labels can therefore be considered an orphan and deleted during apply. The
`up_builds_then_waits_for_health` test asserts that the destructive flag is present but
does not assert ownership preflight.

Proposed remediation: before any `up --remove-orphans`, discover resources by both
deployment and Compose project and refuse unless every existing resource has matching
Switchyard labels. Add a test with an unowned Compose-project orphan and consider
dropping `--remove-orphans` when no prior owned manifest proves it is safe.

**Remediated during review sign-off:** `DockerRuntime::up` now performs the same
`discover_compose_project` + `verify_ownership` preflight as `down`/`cleanup` before
any compose invocation, proven by
`up_refuses_when_the_compose_project_contains_an_unowned_container`.

## 5. File mounts

### Examined

- Source, volume, and execution model validation:
  [`lib.rs`](../crates/switchyard-planner/src/lib.rs#L313-L470) and
  [`model.rs`](../crates/switchyard-planner/src/model.rs#L130-L250).
- Overlay target validation, controlled roots, content-addressed paths, and
  materialization:
  [`overlay.rs`](../crates/switchyard-planner/src/overlay.rs#L323-L417),
  [`overlay.rs`](../crates/switchyard-planner/src/overlay.rs#L931-L1094), and
  [`overlay.rs`](../crates/switchyard-planner/src/overlay.rs#L1130-L1146).
- Compose source/volume/injected mounts:
  [`lib.rs`](../crates/switchyard-planner/src/lib.rs#L1861-L1993) and
  [`lib.rs`](../crates/switchyard-planner/src/lib.rs#L2078-L2099).
- Portable import conflict handling:
  [`bundle.rs`](../crates/switchyard-planner/src/bundle.rs#L274-L359) and
  [`bundle.rs`](../crates/switchyard-planner/src/bundle.rs#L439-L492).
- Diagnostics output creation:
  [`diagnostics.rs`](../crates/switchyard-cli/src/diagnostics.rs#L110-L136).

### Threat model

The review tried `..` and relative overlay targets, targets outside declared roots,
duplicate overlay writes, source paths such as `/`, writable host mounts, symlinked
generated/import destinations, bundle overwrite/partial import, and diagnostics output
symlinks.

### Verified protections

- Overlay targets must be normalized absolute container paths below `/runtime`, a script
  source mount, or a declared service volume target, and may not equal the root itself.
  Duplicate targets require explicit `replace`. Generated mounts are always `:ro` and
  use content-addressed host paths. Test:
  `overlay_validation_rejects_conflicts_selectors_templates_and_traversal` and
  `overlays_resolve_in_order_trace_shadows_and_materialize_files`.
- Script and Process Compose source mounts default to read-only; `writable: true` is an
  explicit opt-in. Named volume read-only behavior is also emitted explicitly.
- Bundle import checks every definition/overlay destination before its first definition
  write unless `--force` is given, refuses existing files, scaffolds only declared local
  inputs, and validates the result. `example_deployment_exports_imports_and_validates`
  exercises the round trip. The preflight avoids ordinary write conflicts, although an
  I/O failure after writing begins is not transactional rollback.
- Diagnostics refuses a symlink output leaf and writes a mode-`0600` regular file. Its
  recursive collectors also skip symlinks. `recursive_redaction_removes_planted_secrets`
  proves content redaction; output mode/symlink behavior is source-enforced without a
  dedicated unit test.

### Findings

#### SR-3 — Broad host source mounts are accepted, including writable root (high)

Planner source validation at
[`lib.rs`](../crates/switchyard-planner/src/lib.rs#L337-L470) checks that a source is a
directory and that referenced build/Process Compose files exist. It does not reject the
filesystem root, a home directory, or another broad/sensitive host path. Compose
generation at [`lib.rs`](../crates/switchyard-planner/src/lib.rs#L1910-L1974) mounts that
path into script containers and removes `:ro` when `writable: true`. A mistaken or
malicious definition can therefore expose, and in writable mode modify, a broad host
tree from a container. This violates DESIGN.md section 8's broad-bind-mount commitment.

Proposed remediation: canonicalize source paths, reject filesystem roots and configured
sensitive/broad ancestors, require a registered project/repository root, and require a
distinct high-visibility approval for writable host-source mounts. Add `/`, home, parent,
symlink, and writable-root negative tests.

#### SR-4 — Generated and imported writes lack symlink-safe containment (high)

`write_plan` at
[`lib.rs`](../crates/switchyard-planner/src/lib.rs#L222-L269) creates and replaces files
beneath a path assembled from the workspace and deployment, but does not reject a
symlinked `.switchyard`, `generated`, deployment, or `routes` ancestor. Its
[`write_atomic`](../crates/switchyard-planner/src/lib.rs#L303-L307) uses `fs::write` on a
predictable temporary path, which follows an existing symlink. Overlay materialization
similarly uses `create_dir_all` and that writer without no-follow checks. Portable import
at [`bundle.rs`](../crates/switchyard-planner/src/bundle.rs#L283-L340) uses `exists`
followed by `fs::write`; a dangling destination symlink reports non-existent and can
redirect the write outside `--into` even without `--force`. A malicious checkout or
concurrent local process can therefore redirect disposable artifact or import writes to
another path writable by the user. No symlink-output test covers these paths.

Proposed remediation: establish a canonical real-directory root, reject symlinked
ancestors and leaves, create unpredictable `create_new` temporary files with no-follow
semantics, and verify containment again before rename. Add malicious ancestor, leaf,
dangling-leaf, and check/use-race tests for generated routes/overlays and bundle import.

#### SR-5 — Container-symlink target boundary is not enforced (medium)

`target_is_controlled` at
[`overlay.rs`](../crates/switchyard-planner/src/overlay.rs#L1070-L1094) performs lexical
`Path::starts_with` checks only. It cannot establish whether a target component is a
symlink in the selected image or mounted source, yet DESIGN.md section 8 promises to
reject traversal through symlinks. The same overlay file is attached to each application
service in an instance, while allowed roots are unioned across the block's services, so
the check is not tied to the actual target service filesystem either.

Proposed remediation: make injected-file targets service-specific and declared by the
execution adapter. Reject targets whose image path cannot be proven non-symlink, or
materialize them into a dedicated Switchyard-owned mount tree whose components are
created by Switchyard. Add an image fixture containing an escaping symlink and a
multi-service block with different mount roots.

#### SR-6 — Script containers have no enforced non-root default (medium)

The script and Process Compose branches at
[`lib.rs`](../crates/switchyard-planner/src/lib.rs#L1910-L1975) set no Compose `user`, so
the image default applies and is commonly root. Only the router sidecar sets the host
UID/GID at [`lib.rs`](../crates/switchyard-planner/src/lib.rs#L2180-L2209). The schema has
no explicit user/justification field for script execution. This leaves DESIGN.md section
8's non-root script-container commitment unimplemented and increases the impact of
SR-3.

Proposed remediation: default script and Process Compose services to a documented
non-root UID/GID, add an explicit reviewed override with justification, and test both
generated Compose paths.

## 6. Secret handling

### Examined

- Overlay reference parsing, secret-safe provenance, Compose placeholders, and runtime
  resolution:
  [`overlay.rs`](../crates/switchyard-planner/src/overlay.rs#L100-L239),
  [`overlay.rs`](../crates/switchyard-planner/src/overlay.rs#L538-L812),
  [`lib.rs`](../crates/switchyard-planner/src/lib.rs#L2040-L2075), and
  [`runtime.rs`](../crates/switchyard-cli/src/runtime.rs#L323-L361).
- SQLite secret-value admission:
  [`switchyard-state/src/lib.rs`](../crates/switchyard-state/src/lib.rs#L216-L345).
- Portable-bundle sanitization and line redaction:
  [`bundle.rs`](../crates/switchyard-planner/src/bundle.rs#L512-L746).
- Daemon output capture, router events, and diagnostics:
  [`server.rs`](../crates/switchyard-daemon/src/server.rs#L142-L203),
  [`server.rs`](../crates/switchyard-daemon/src/server.rs#L853-L882),
  [`switchyard-router/src/lib.rs`](../crates/switchyard-router/src/lib.rs#L120-L161), and
  [`diagnostics.rs`](../crates/switchyard-cli/src/diagnostics.rs#L285-L365).
- Release archive inputs:
  [`release.sh`](../scripts/release.sh#L38-L67).

### Threat model

The review planted secrets in declared references, credential-looking and ordinary
environment keys, preview/manifest/SQLite structures, captured output, router events,
portable bundles, diagnostics strings, and release inputs. It also checked whether
secret file injection can silently materialize content.

### Verified protections

- Overlay secret references resolve only at apply time from exactly one environment
  variable or file. Generated Compose contains a required synthetic variable, not the
  value; runtime bindings are deliberately non-serializable. Resolved YAML, manifest,
  injected file content, and preview provenance retain markers rather than values. Test:
  `secrets_are_redacted_and_variations_are_disjoint`.
- Secret file injection is explicitly rejected as unsupported, so it cannot copy a value
  into the content-addressed overlay tree.
- `AppliedSnapshot` and `StructuredContext` reject literal values at secret-looking keys
  and accept validated references only. Test:
  `secret_values_are_rejected_and_only_references_are_retained`.
- Portable export replaces credential-looking environment/parameter values, dotenv
  paths, file sources, and machine state with required-local-input placeholders. It uses
  the shared `credential_like_key` heuristic and verifies a content hash on import.
  Tests: `example_deployment_exports_imports_and_validates`,
  `tampered_bundle_is_rejected_with_stable_code`, and
  `unsupported_bundle_api_version_is_rejected_with_stable_code`.
- Router control events redact nested sensitive keys before retention. Daemon SSE events
  replace any output line containing authorization/password/secret/token/private-key
  terms. The proving router test is `token_comparison_and_redaction_are_safe`.
- Daemon-driven router operations use a separate persistent
  `.switchyard/router-token`, created as an owner-only regular file and injected only
  into child commands and local administration calls. The API and GUI never receive
  it, and mismatched environment overrides are rejected instead of silently rotating
  the credential beneath running routers.
- Diagnostics redacts credential-looking object fields, values of credential-looking
  process environment names, the daemon discovery token, the router token when present
  in that environment set, and sensitive-looking log lines before a mode-`0600` write.
  `recursive_redaction_removes_planted_secrets` proves planted field, embedded value,
  router/daemon token, and authorization-line removal.
- Release assembly includes only the three built executables, freshly built GUI assets,
  install/uninstall scripts, and license/notice files. It does not copy deployment YAML,
  `.switchyard`, process environment, credentials, or diagnostics into the archive.

### Findings

#### SR-7 — Literal credential-looking environment values reach artifacts (high)

The ordinary deployment execution and instance environment maps contain plain strings.
Validation accepts them without applying `credential_like_key`, and
`add_runtime_fields` at
[`lib.rs`](../crates/switchyard-planner/src/lib.rs#L2005-L2037) serializes them into
Compose. Overlay `set` also permits a literal at a key such as `DB_PASSWORD`; references
are optional. Such a value consequently appears in authored YAML, generated Compose,
resolved YAML, daemon validation previews, and Docker container configuration. Portable
export sanitizes it later, and SQLite may reject a secret-looking snapshot, but those
checks do not prevent the earlier copies. This conflicts with DESIGN.md section 8's
"reference, do not store" commitment.

Proposed remediation: reject literal values for `credential_like_key` names across
execution, instance, parameter, dotenv, and overlay inputs; require the existing
apply-time reference form (adding it to base deployment fields as needed). Add negative
tests asserting planted values never occur in YAML, Compose, manifest, preview, SQLite,
logs, or diagnostics.

#### SR-8 — Daemon API retains raw command output (medium)

`read_output` at
[`server.rs`](../crates/switchyard-daemon/src/server.rs#L853-L882) emits a redacted event
line but appends the original line to `captured`. `CliBackend::run` returns that raw
stdout/stderr in `CommandResultV1` at
[`server.rs`](../crates/switchyard-daemon/src/server.rs#L168-L203), and recent terminal
operations retain it in memory for authenticated API/CLI retrieval. This is documented
as raw output because arbitrary application logs cannot be classified reliably, but it
does not meet DESIGN.md section 8's unconditional redaction promise.

Proposed remediation: pass declared runtime secret values through an exact-value
redactor before retaining or returning output, and apply the conservative line redactor
as a fallback. If an explicit raw-log mode remains necessary, make it ephemeral,
interactive, and clearly outside persistence/API results. Add a planted-secret command
result test, not only an SSE-event test.

#### SR-9 — Diagnostics cannot recognize every secret (informational)

The redactor at
[`diagnostics.rs`](../crates/switchyard-cli/src/diagnostics.rs#L296-L365) intentionally
uses credential-looking names and known daemon/router values. A secret stored under an
ordinary application key, in a file not represented by such a key, or already emitted
without a sensitive word/value match can survive. [`release.md`](release.md#diagnostics-bundle)
already discloses this limitation.

Proposed remediation: after SR-7 makes declared references universal, feed their exact
resolved values to diagnostics redaction without enumerating unrelated environment
values. Keep the heuristic as defense in depth and add application-specific redaction
configuration where exact declarations are unavailable.

## Cross-cutting DESIGN.md section 8 status

- Backed by code/tests: loopback defaults and explicit LAN acknowledgement; reference-
  based overlay secrets; read-only script-source default; ownership-checked down/cleanup;
  unmanaged worktree non-destruction; explicit rejection of the deferred host-execution
  adapter; and no automatic trust-store mutation.
- Deferred safely: trusted host execution is not registered, so command preview,
  per-block approval, content-hash reapproval, and environment allowlists are not
  reachable promises yet. `docs/adapters.md` records that boundary and the adapter
  registry test rejects `execution-host`.
- Not backed as written: broad host mount rejection (SR-3), symlink-safe generated and
  imported writes (SR-4), symlink-safe injected targets (SR-5), non-root script execution
  (SR-6), universal reference-only secret input (SR-7), and universal log/error redaction
  (SR-8).
- Docker ownership checks are strong accident prevention, not protection from a peer
  with Docker authority. LAN exposure is explicit and inspectable, but firewall and
  public-interface reachability remain external operator controls.
