# Agent mistakes and lessons

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
