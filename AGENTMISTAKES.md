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
