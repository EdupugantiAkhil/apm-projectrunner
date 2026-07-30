# Unfinished work

This is the consolidated checklist for work that is implemented incompletely, still
requires acceptance evidence, or is deliberately deferred. `IMPLEMENTATION_PLAN.md`
remains the authoritative release checklist; when a release item changes, update that
file and this index together. Detailed evidence and procedures remain in the linked
documents rather than being duplicated here.

Last reconciled: 2026-07-30.

## Team-release blockers

The team release remains incomplete until every item in this section is implemented,
tested, and documented.

### Security remediation

Source: [security review](security-review.md) and
[Phase 7](../IMPLEMENTATION_PLAN.md#phase-7--lan-team-workflows-and-hardening).

- [ ] **SR-3 (high):** reject overly broad host source mounts and make writable source
      mounts an explicit, reviewed exception.
- [ ] **SR-4 (high):** make generated and imported writes symlink-safe and contained
      beneath canonical managed roots.
- [ ] **SR-7 (high):** reject literal credential-looking environment values so secrets
      reach artifacts only through supported references.
- [ ] **SR-1 (medium):** prevent public GUI file serving from following symlinks outside
      the configured distribution root.
- [ ] **SR-5 (medium):** enforce the promised container-symlink boundary for overlay
      targets.
- [ ] **SR-6 (medium):** run script containers as a non-root user by default, with an
      explicit reviewed override where required.
- [ ] **SR-8 (medium):** redact or avoid retaining sensitive command stdout/stderr in
      daemon operation results.

### Release usability

- [ ] Make each running custom domain in the dashboard a normal clickable link that
      opens in the user's default browser.
- [ ] Keep normal link opening distinct from the managed-profile fallback; require
      Chromium or Chrome for Testing only when proxy-authenticated, isolated localhost
      routing is actually requested.
- [ ] Add a repository-level Node.js 24 pin (for example `.nvmrc`) and make development
      or release preflight report the required version before a long build.
- [ ] Refresh the root README status so it reports the completed product MVP and the
      in-progress team release rather than only the Phase 4 routing proof.

## Required acceptance and automation

These capabilities have implementation or lower-level test coverage, but the named
end-to-end evidence is still missing. The manual procedures are in
[the MVP acceptance audit](mvp-acceptance.md#manual-procedures).

### macOS and browser routing

- [ ] On a clean supported macOS user account, install Chromium or Chrome for Testing
      and verify `switchyard open` launches an isolated profile.
- [ ] In that profile, verify the private proxy-auth extension loads, fixed localhost
      requests traverse the selected managed proxy, and credentials do not appear in
      arguments, generated metadata, logs, or operation results.
- [ ] Drive a real browser through all three identity modes: explicit tab header,
      Origin, and managed-profile proxy. The live routing proof currently drives Origin
      requests at the router boundary rather than through browser automation.

### Product-MVP acceptance

- [ ] Register a monorepo plus two already-existing linked worktrees and confirm that
      deregistration leaves every directory and Git ref untouched (criterion 1,
      procedure A).
- [ ] Run the exact one-database, five-UI, two-Python-suite, three-Java-suite topology
      together (criterion 3, procedure B).
- [ ] While the live fixture is running, verify path, ref/branch, and commit are shown
      for every instance (criterion 7).
- [ ] Verify combined and per-instance Docker logs through the CLI and browser GUI
      (criterion 14, procedure C).
- [ ] Run two overlay variations concurrently without source edits or resource
      collisions (criterion 18, procedure D).
- [ ] Exercise Docker-label collection and reconciliation through a restarted real
      daemon, not only through split daemon/state tests.
- [ ] Add browser end-to-end coverage for the common command-bar actions that currently
      have component or API coverage only: Plan, Up, Down, Cleanup, logs, and normal UI
      link opening.

### TUI verification

- [ ] Run `scripts/tui-smoke.py` with `pyte` installed on the supported macOS target.
- [ ] Add the PTY smoke suite to an enforced verification environment so a missing
      `pyte` dependency cannot silently turn the gate into a successful skip.

## Non-blocking engineering follow-ups

These do not currently block the team release unless their scope is promoted in
`IMPLEMENTATION_PLAN.md`.

- [ ] Resolve the four existing React exhaustive-dependency warnings in `App.tsx` and
      `DeploymentBuilder.tsx`, then make web lint warning-free.
- [ ] Reduce the Home signal loader's three requests per deployment and avoid rerunning
      planner validation after every command on large projects.
- [ ] Replace the exact `(name, deployment)` startup-profile join with durable profile
      provenance, or document and test the unlisted-block behavior as the permanent
      contract.
- [ ] Expose the shared operations-layer profile mutation before adding profile editing
      to interactive clients; profile editing currently produces a preview only.
- [ ] Evaluate zeroizing in-memory repository credentials instead of retaining them in
      ordinary `String` allocations until freed memory is overwritten.

## Deferred ideas

These are intentionally outside current release gates. They become required work only
after their product requirement and acceptance test are approved.

- [ ] Evaluate Podman as a runtime adapter.
- [ ] Evaluate Kubernetes, containerd, or Nomad adapters.
- [ ] Evaluate non-Chromium browser extension support.
- [ ] Evaluate broader multi-host scheduling beyond the current limited remote-device
      cut.
- [ ] Evaluate multi-user authentication and authorization.
- [ ] Evaluate public plugin distribution and sandboxing.

## Supported boundaries, not unfinished work

Do not treat these explicit product boundaries as compatibility bugs or pending release
tasks:

- Intel Macs and macOS releases older than 26 are unsupported.
- Automatic `.local` mDNS publication is Linux-only; macOS supports loopback and
  explicitly addressed LAN gateways.
- Public-internet exposure is unsupported.
- Managed-profile proxying is HTTP-only and does not perform HTTPS `CONNECT` or local
  TLS interception.
- Remote consumers, process adapters, routers, and cross-device sidecars are outside
  the limited remote-device cut.
- Trusted host-script execution and secret-file injection are rejected rather than
  silently approximated.
