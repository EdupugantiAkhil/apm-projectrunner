# Agent mistakes and lessons

## 2026-07-15 — Phase 5 daemon review corrections

- Live-bind rollback returned early when observing one previously activated router
  failed, discarding the complete attempt vector before SQLite persistence and skipping
  compensation for remaining routers. Correction: record the observation failure as a
  failed rollback attempt and continue the rollback loop. Lesson: error paths for
  multi-target mutations must preserve the full history accumulated so far.
- Lease-heartbeat failure dropped the async handle for blocking live-bind work, allowing
  router mutation to continue without the lease while its attempts became unobservable.
  Correction: signal cooperative cancellation, await the backend to completion, persist
  returned attempts, and only then finish with the lock-lost error. Lesson: blocking work
  must observe cancellation and must never be abandoned merely because its async handle
  was dropped.

## 2026-07-15 — Phase 5 live router control

- The first state update treated an activated candidate's acknowledgement as both the
  observed and previous snapshot, which made `previousVersion` equal `currentVersion`
  on the first recorded bind. Correction: retain the pre-apply observation for the
  previous tuple and derive the post-ack observation from the activated candidate.
  Lesson: an acknowledgement describes the new active snapshot; version visibility
  still needs a distinct pre-mutation observation.
- Adding schema version 3 initially left the migration test expecting only version 2.
  Correction: assert both pending migrations and the complete version sequence.
  Lesson: migration tests should express the ordered suffix from their fixture version,
  not assume only one future migration.
- The first CLI version-summary condition used a let-chain unsupported by the minimum
  compiler available in this workspace. Correction: use nested conditionals and rerun
  the workspace check. Lesson: edition 2024 does not imply every newer language feature
  is available at the declared Rust 1.85 minimum.
- The exact workspace test again reached the environment's `EPERM` listener restriction,
  and Docker Engine access was denied. Correction: run the complete transport-independent
  Phase 5 suite and proof script, retain the exact failures in verification, and do not
  weaken existing network tests to manufacture a pass. Lesson: release evidence must
  distinguish implemented behavior from host capabilities.

## 2026-07-15 — Phase 5 daemon and API

- The first API integration tests started real loopback listeners, but this execution
  sandbox rejects socket creation with `EPERM`, including a pre-existing Unix-socket CLI
  test. Correction: factor the exact Axum router into a transport-independent harness
  and keep loopback binding in the production startup path. Lesson: HTTP behavior,
  concurrency, and streaming can be proven in memory while listener policy is tested
  separately without weakening production restrictions.
- An initial multi-file patch omitted the second file marker, so its context was checked
  against the wrong manifest and rejected. Correction: split the patch at explicit file
  boundaries and verify target context. Lesson: keep dependency and implementation
  edits in clearly delimited patch sections.
- The first workspace Clippy run exposed a pre-existing `format_collect` warning in the
  router's random credential encoding under the current toolchain. Correction: replace
  it with allocation-equivalent direct hexadecimal encoding and rerun the exact command.
  Lesson: repository-wide `-D warnings` can surface toolchain drift outside the changed
  crate; keep such fixes mechanical and behavior-preserving.

## 2026-07-15 — Phase 5 SQLite state

- The first snapshot-upsert SQL used Rust line continuations without preserving spaces,
  joining `SET` to the following identifier. Correction: preserve explicit spaces at
  every continued SQL boundary; the snapshot round-trip and reconciliation tests now
  execute the statement. Lesson: multiline SQL embedded with escaped newlines needs an
  execution test, not only schema compilation.
- The repository test invocation initially attempted a crates.io index refresh in a
  network-restricted shell. Correction: validate the new crate against locally cached
  bundled-SQLite sources first, while retaining the required repository-level commands
  for final verification. Lesson: a newly introduced dependency can require lock/index
  preparation even when its source archive is already cached.
- A public observed-resource query was initially inserted just outside the `StateStore`
  implementation block. Correction: move it into the implementation and rerun tests,
  Clippy, and rustdoc. Lesson: after a large implementation block, anchor method patches
  to the closing method body as well as the surrounding function name.

## 2026-07-14 — Phase 4 routing proof

- A custom-domain listener was initially emitted without `consumer: gateway`, so its
  direct routes were treated as browser-identity routes and returned
  `missing_route_identity`. Correction: direct custom-domain ingress listeners carry
  the consumer used by their configured routes. Lesson: test custom-domain delivery,
  not only listener startup and configuration validation.
- The first invariant implementation made `bind backend group` contradict attached UI
  group expectations. Correction: a complete backend-group mutation updates every
  attached `uiRoutes` expectation in the same planned snapshot. Lesson: duplicated
  cross-layer desired state must move atomically.
- Provider readiness originally passed an unresolved Docker DNS name to Pingora, whose
  peer constructor panicked on lookup failure. Merely spawning the probe contained the
  panic but still invoked the faulty path. Correction: resolve DNS fallibly before peer
  construction and retain task isolation as defense in depth. Lesson: exercise stopped
  container DNS, not only refused loopback ports.
- Every fixture service initially declared the same image build/tag. Parallel Compose
  builds produced different image identities and later `up` operations recreated
  healthy containers. Correction: build the shared fixture image once in one builder
  service. Lesson: one tag must have one build owner in deterministic generated Compose.
- A raw `docker compose restart` invalidated an already-running sidecar joined with
  `network_mode: service:<consumer>` and also changed ephemeral published ports.
  Correction: the recovery proof performs ownership-aware down/up for shared namespace
  reconstruction, and `switchyard up` refreshes the native gateway when publications
  change. Lesson: container restart is not namespace reconstruction; verify DNS and
  loopback publications after lifecycle transitions.
- The local Nix shell exposed a `cargo-fmt` binary whose dynamic loader was unavailable,
  even though Cargo builds worked. Correction: use the working toolchain paths for final
  formatting verification and report environment-specific verification gaps honestly.
  Lesson: distinguish a repository failure from a host toolchain-launch failure.
- A verification wrapper initially assigned to zsh's read-only `status` parameter and
  failed after the test command completed. Correction: rerun it with `rc` and preserve
  the test exit code. Lesson: avoid shell-reserved parameter names in portable wrappers.

## 2026-07-15 — Phase 6 adapter SDK

- The first planner integration replaced the native worktree repository/ref validation
  with `source-git` adapter schema validation without a regression test guarding the
  moved behavior, and review had to add one. Lesson: when validation logic moves across
  a crate boundary, the old behavior needs an explicit test at the new seam before the
  move is trusted.
- A test appended to `planner.rs` reused the local variable name `bundle`, shadowing the
  `bundle()` fixture helper within the same function and failing compilation. Lesson:
  fixture helpers and locals sharing a name cannot coexist in one test body.

## 2026-07-15 — Phase 6 source management

- The first daemon source/worktree handlers ran Git subprocesses and SQLite access
  directly on async worker threads; a slow clone would have stalled unrelated API
  requests. Correction: run each handler body through `spawn_blocking`. Lesson: any
  handler that shells out or does filesystem-heavy work belongs on the blocking pool,
  even when it is "usually fast".

## 2026-07-15 — Phase 6 GUI

- The deployment-definition handlers repeated the async-blocking mistake from the
  source endpoints: planner validation (which invokes git for source identities) ran
  directly on async workers and review had to move it to `spawn_blocking` again.
  Lesson: repo-wide review lessons must be restated in every subsequent brief, not
  assumed remembered.
- The GUI initially exposed only deployment-level logs even though the command
  contract already carried an optional per-instance `target`; review wired instance
  cards to it. Lesson: check the existing contract surface before concluding a
  capability needs new plumbing — and before shipping a screen without it.

## 2026-07-15 — Phase 6 JAS fixture

- The first smoke-test invocation piped output through `tail`, so the reported exit
  code was tail's success while the script had actually failed at variation planning.
  Correction: write output to a file and test the script's own exit status. Lesson:
  never take an exit code from the far end of a pipeline.
- The fixture's UI `java` slot used `host: localhost`, which every existing fixture
  avoided: router listener binds require IP literals and the sidecar exited on config
  parse. Correction: bind `127.0.0.1` (identical service for the unchanged app's
  `localhost` calls) and note the constraint in the deployment definition. Lesson:
  validate generated router configs against the router binary, not only the planner,
  before shipping a fixture.
- The reviewer brief said "post-ready schema-init hook", steering the implementation
  toward the schema-only `hooks.postReady`; Codex correctly stopped on the gap.
  Correction: task-lifecycle init service plus a recorded Phase 7 work item for the
  inert hooks. Lesson: brief wording should name mechanisms verified to exist.

## 2026-07-15 — Phase 7 LAN exposure Part 1

- The first LAN round-trip test reused the general routing-matrix fixture, whose
  sidecar-oriented providers intentionally include non-loopback Docker DNS names, so
  the new host-LAN provider guard correctly rejected it. Correction: make the test's
  upstreams loopback-only, matching host-gateway semantics. Lesson: a shared router
  schema fixture is not automatically valid for every execution mode; tests for
  host-only policy must establish host-mode preconditions explicitly.

## 2026-07-15 — Phase 7 LAN exposure Part 2

- The first preflight draft classified common VPN interface names but did not feed
  `/32` IPv4 and `/128` IPv6 host routes into the same warning. Correction: parse
  read-only `ip -o address show` output behind the command seam and test both address
  families. Lesson: when an acceptance criterion gives multiple detection signals,
  test every signal independently rather than treating examples as alternatives.
- The initial status path returned planned publications as failed when no state existed
  but omitted the check report. Correction: run the same non-mutating injected preflight
  for unstarted status so both `up` and `status` expose check meanings. Lesson: a
  structured diagnostic contract should have the same shape before and after resource
  creation, even when some observations are necessarily unavailable.

## 2026-07-16 — Phase 7 mDNS Part 2 (found only by live verification)

- `find_binary` canonicalizes `avahi-publish-address` to `avahi-publish`, whose
  argv[0]-based dispatch then fails with "No command specified." Correction: pass
  `-a` explicitly. Lesson: canonicalizing a multi-call binary's path changes its
  behavior; sandboxed unit tests with a fake runner cannot catch this.
- `avahi-publish -a` also registers a reverse PTR record, which collides with
  avahi-daemon's own record for the host's primary address ("Local name
  collision"). Correction: pass `-R`. The immediate-exit error now includes the
  publisher's last log line so the next such failure is self-explanatory.
- Publication targeted every exposed non-loopback address, including Tailscale and
  Docker bridge addresses that other LAN devices cannot reach (and avahi may
  refuse). Correction: advertise only non-VPN, non-container-bridge interface
  addresses while preflight still warns about the excluded ones. Lesson: "exposed"
  (listener binds) and "advertisable" (mDNS targets) are different sets.

## 2026-07-16 — Phase 7 Tailscale Part 3

- The first typed status model relied on Serde's `PascalCase` conversion for
  `DNSName`, which produces `DnsName` and rejected realistic canned Tailscale JSON.
  Correction: explicitly rename the acronym-heavy `DNSName` and `TailscaleIPs` fields
  and retain the realistic status fixture. Lesson: case-conversion rules do not
  preserve API acronyms; pin externally defined JSON keys explicitly.

## 2026-07-16 — Phase 7 bundles Part 4

- The first import integrity check treated every absolute-looking string as
  machine-specific state, which would have rejected legitimate container command paths
  such as `/usr/local/bin/...`. Correction: reject absolute paths only in typed
  host-path fields such as sources and overlay file/env references. Lesson: portability
  checks must understand schema meaning; string-shaped data is not automatically a host
  path.

## 2026-07-16 — Phase 7 reliability Part 6

- While relocating compatibility deployment fixtures, an attempted shell rewrite failed
  because path delimiters in the replacement expression were not escaped correctly.
  Correction: make fixture relocations as explicit patches. Lesson: for schema goldens,
  visible diffs are safer than clever bulk text edits.

## 2026-07-16 — Phase 7 reliability tests (Part 6 review)

- Four storm/soak test-design errors survived sandboxed development because they
  only manifest under real socket load: a cross-thread version-monotonicity
  check that races benignly (per-observer state is the sound formulation); a
  zero-incomplete-exchange assertion under a `Close`-policy storm (Close kills
  in-flight connections by design — Pin is the policy whose storm guarantees
  completeness); a serial, nonblocking-socket test stub that collapsed under
  concurrent clients; and 50ms health-check timeouts that manufactured
  fail-closed 503s on a loaded ARM board. Lesson: reliability tests assert what
  the declared policy guarantees, not what a quiet machine happens to produce,
  and their harness must be more robust than the system under test.
