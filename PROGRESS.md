# Switchyard implementation progress

Updated: 2026-07-31

## Release status

- Routing proof (Phases 0–4): complete.
- Product MVP (Phases 5–6): complete.
- Team release (Phase 7): in progress.
- Web UI plan (`docs/web-ui-plan.md`): complete. Parts 1 through 13, including follow-up
  Parts 11a–11c, and Part 13's security review with its two fixes.

## 2026-07-31 — V2 Part 3 group-address routing increment

- Group addresses now select browser-reachable members per request. The planner generates
  `<instance>.<group-address>` domains and trusted explicit-header routes for each active
  HTTP/HTTPS member; Pingora accepts those identities on loopback consumer listeners, rejects
  unknown identities, and strips the routing header before forwarding.
- A bare group address still requires exactly one active independently addressed member. That
  member is only the default, not a designated group front door. Disabled and TCP-only members
  do not become browser targets, while all active members remain available through shared
  localhost routing.
- Deployments with addresses no longer need compatibility-only `hostRouter` or `hostUpstreams`
  topology. The planner derives a deterministic unprivileged host gateway, dynamic providers, and
  browser localhost routes from membership plus services whose HTTP/HTTPS probe port is also
  published. Disabled members are excluded from those inferred providers. Existing authored
  router configurations retain their advanced merge path.
- The executable vision-sample test extracts the YAML in `docs/vision/sample-config.md`, removes
  only deferred `scripts:`, and proves it validates and plans without compatibility fields. A
  live planner-to-Pingora gate opens both group domains, observes different UIs and backends,
  and verifies that the disabled canary has no generated domain.
- The final full-lifecycle sample gate remains open. Its UI/backend profiles invoke source-tree
  commands in plain `container` images, but the settled execution contract mounts source only
  for `script`; its illustrative Git and external hosts also need local fixture replacements.
  Choosing script-backed sample profiles versus broadening container source semantics is a
  product decision, not router work, and has not been guessed here.

Verification passes: full workspace tests with the five declared reliability ignores; all-target,
all-feature Clippy with warnings denied; rustdoc with warnings denied; formatting and diff checks;
the focused planner suite; and all five live Phase 3 router gates.

## 2026-07-31 — V2 Part 2e external instances

- Added the strict `{ name, external, ports, probe? }` instance form. External instances
  have no block, source, device, Compose resources, source identity, or lifecycle; down and
  cleanup therefore never target them.
- Added integer and quoted inclusive-range ports with nonzero, ordered, duplicate, and
  1024-port range validation. Ranges expand before routing so priority and collision
  diagnostics remain per-port and use the existing first-listed rule.
- Extended transparent route members with an optional declared-port allowlist. Started
  members continue to advertise live listeners; external members use authored ports and
  connect to the authored host on the unchanged destination port. Same-host externals with
  disjoint ports remain independent members.
- Added optional external HTTP, HTTPS, TCP, and command probes to apply plans. `up` runs them
  after managed services become healthy and reports `external instance ... is not reachable`,
  distinct from Docker or managed-instance startup failures.
- External instances remain visible in manifests, membership projections, TUI/Web deployment
  rows, and connection details without appearing as local device placements.
- Verification passes: full workspace tests with the five declared reliability ignores;
  workspace all-target/all-feature Clippy with warnings denied; rustdoc with warnings denied;
  formatting and diff checks; all 49 Web tests, TypeScript compilation, and the Vite production
  build. Focused routing coverage also opens a real TCP listener and proves that an authored
  external port connects to the same port while an undeclared port is not a candidate.

## 2026-07-31 — V2 Part 2d membership is the connection

- Removed deployment `spec.bindings` and `spec.routes` from the strict authored schema.
  Planning now derives each routed instance's complete ordered view solely from its one group
  membership. Validation rejects an instance appearing in several groups and reports both
  paths with guidance to create a separate instance.
- Replaced the CLI/API binding operation with `switchyard move FILE INSTANCE GROUP` and
  `/api/v1/commands/membership`. Live moves compare every sidecar in the source and destination
  groups, apply every changed immutable snapshot through the daemon's rollback/compensation
  path, and retain unchanged active snapshot metadata in generated artifacts. A move that
  changes the routed-instance resource set is refused as a live change and requires `up`.
- Changed operations, TUI, and Web projections to memberships. The stopped Web workspace edits
  each complete ordered group list; the running workspace previews and applies an instance
  move. Deployment summary/detail APIs now expose the membership projection derived from
  `groups.*.instances`.
- Migration removes agreeing legacy bindings and empty routes, refuses a binding that disagrees
  with membership, refuses non-empty direct routes, and reports every occurrence of
  multi-group membership without writing.
- Updated the routing-matrix fixture to use distinct main/feature audit instances that share
  the same source and block, proving reuse without sharing one runtime instance across groups.
  The vision's current-differences note now drops the completed repository and connection-field
  gaps.
- Verification passes: workspace tests with the five declared reliability ignores; all-target,
  all-feature Clippy with warnings denied; rustdoc with warnings denied; formatting and diff
  checks; 49 Web tests; and the TypeScript/Vite production build. Web lint exits zero with the
  four pre-existing exhaustive-dependencies warnings. Implementation landed in `6616583`.

## 2026-07-31 V2 Part 2c — repository stores and source worktrees

- Added the authored `repositories:` map. A `url:` produces one managed bare Git store
  under `.switchyard/clones/<name>`; `clone:` adopts an existing bare repository or
  ordinary clone. Repository stores are never mounted or run.
- Replaced plain/path/worktree source variants with the single required
  `{ repository, ref, path }` form. Source paths are project-contained, relative to the
  deployment, unique, and disjoint from repository storage.
- `up` now performs a pure planning preflight, creates missing repository stores and
  detached source worktrees, then replans against their live Git identities before any
  Docker mutation. Existing worktrees are inspected and refused on repository or commit
  mismatch rather than moved or reset.
- Deployment sources are authoritative for planning/startup and no longer depend on the
  SQLite registered-source catalog. Guided CLI/daemon/Web authoring converts a selected
  registered worktree into repository/source declarations.
- Extended `switchyard migrate` to collapse repeated legacy repository paths into one
  adopted repository, preserve distinct existing worktree paths, and refuse repository
  checkouts or non-Git plain paths instead of retaining a third source kind.
- Migrated examples, compatibility fixtures, daemon/profile previews, init scaffolding,
  generated agent guidance, and Web draft authoring. Managed repositories are bare stores;
  all editable and runnable code lives in ordinary source worktrees.
- Verification: workspace tests with all features passed with the five declared reliability
  ignores; 49 Web tests, the TypeScript/Vite production build, workspace Clippy with all
  targets/features and warnings denied, rustdoc with warnings denied, and formatting passed.
  Web lint exited zero with the four pre-existing exhaustive-dependencies warnings.

## 2026-07-31 One instance, one group

- Replaced sender/receiver-dependent membership behavior with one structural rule: an
  instance may appear in at most one group's `instances:` list.
- Clarified the reason for per-instance namespaces: alternative members remain running so
  a group can switch from one to another by ordering or disabling, without an application
  rebuild or restart. Same-port coexistence is an enabling consequence, not the primary
  product motivation.
- Multi-group membership is now a schema validation error naming both groups, detectable
  before planning or startup without inferring an instance's role or runtime behavior.
- Updated the vision, sample, V2 roadmap, and architecture to use separate database
  instances backed by the same source and startup profile instead of sharing one runtime
  instance between groups.
- Documentation-only clarification. No implementation changed and no code tests were run.

## 2026-07-31 Vision tracking cleanup and V2.1 roadmap

- Deleted the stale `DEVIATION.md`. The V2 roadmap now directly records implementation
  gaps instead of presenting a second, obsolete capability/binding product model as
  current guidance.
- Repaired relative links from `docs/vision`, and linked the vision directly to the V2
  implementation roadmap and the new V2.1 multi-project roadmap.
- Added `docs/v2.1-roadmap.md` for the one-service, one-window, multi-project experience,
  covering the global registry, project-scoped daemon/API behavior, Web UI switcher,
  migration, and end-to-end acceptance.
- Corrected the sample configuration's scenario description: its groups compare UI and
  backend checkouts while deliberately also exercising a disabled canary and an external
  member.
- Clarified `AGENTS.md`: the vision controls intended product behavior, `DESIGN.md`
  controls implementation architecture while converging on it, and vision edits require
  an owner decision or an internal-consistency correction. Removed the V2 roadmap's
  conflicting subagent workflow.
- Documentation-only phase. `git diff --check` passed, all relative Markdown link targets
  under the root and `docs/` resolved, and no code tests were run.

## 2026-07-31 Vision vocabulary — remove unmeasurable part/segment roles

- Removed `part` and `segment` as product-model vocabulary from the target vision.
  Startup profiles are reusable definitions that expand into services; instances are
  source-backed runtime copies; groups contain ordered instances.
- Kept UI, backend, and database only as example instance/service names in the target
  model. The current planner still contains legacy `"ui"` capability selection and a
  `BackendGroupInvariant`; the V2 roadmap now explicitly removes both and requires a
  repository-wide role-inference audit.
- Documentation-only clarification. No implementation changed and no code tests were run.

## 2026-07-31 V2 roadmap and vision-flow reconciliation

- Made `docs/vision/user_flow.md` agree with the membership-only schema: Step 8 now authors
  and edits complete ordered groups without capabilities, slots, `extends:`, bindings, or
  direct routes. The redundant Connections step is removed, and Addresses is now Step 9
  with browser members selected without a `ui` capability.
- Aligned repository/source setup and the glossary with `docs/vision/sample-config.md`.
  Repositories are declared once, every source is a repository/ref/path worktree, missing
  managed clones and worktrees are created by `up`, and plain-path sources are not retained.
- Made the sample configuration, excluding its explicitly deferred `scripts:` section, the
  V2 acceptance contract. The roadmap now requires an end-to-end fixture proving worktree
  creation, two group addresses, different backends, separate database instances reusing
  one source/profile, an external member, and `disabled:` without compatibility-only fields.
- Removed run-action and multi-project work from the V2 roadmap. Renumbered the remaining
  parts and retained the final rename to the intended **APM ProjectRunner** / `apmpr` name.
- Documentation-only phase. No implementation changed and no code tests were run.

## 2026-07-31 V2 schema clarification — no capabilities or slots

- Clarified the final V2 topology in `DESIGN.md` and `docs/v2-roadmap.md`: group membership
  is the only authored connection model. `provides:`, `consumes:`, `bindings:`, `routes:`,
  and capability-based `extends:` are removed rather than retained as optional overrides.
- Routing remains transparent and port-for-port. Runtime listener observation supplies
  collision information; a future port-remapping feature must use a separate explicit
  schema rather than keeping the old capability/slot model alive.
- Defined behavior that had previously depended on removed fields: groups author complete
  ordered lists; a member shared across groups may receive in all of them but gets an
  ambiguity error if it originates a loopback connection; a bare group address requires
  exactly one active member with its own browser address.
- Documentation-only clarification. No implementation or vision file changed, and no code
  tests were run.

## 2026-07-31 V2 Part 2a — group membership stops being policed by capability

- Removed the hard duplicate-provider rejection and its diagnostic code. Group resolution now keeps
  every candidate in resolved membership order while retaining the first as the selected provider;
  a planner warning is emitted only when a bound consumer slot actually has several candidates.
  The warning names the slot, consumer, group, ordered candidates, and winner.
- The deterministic inheritance rule is unchanged and now defines collision order precisely: resolve
  the parent first; remove every inherited member whose provided capabilities overlap any child
  member; append surviving child `instances` in authored order. The first candidate in that resolved
  order wins. Focused tests prove inherited order is preserved, capability overrides replace inherited
  candidates, and reversing `instances` flips both the warning and generated route.
- Proved the intended boundary by experiment. A copied routing-matrix definition with `ui-1` and
  `ui-3` in one group planned both Compose services with zero warnings because no consumer slot asks
  for `ui`. A copied collision fixture warned for both the base binding and its inherited binding and
  routed to `provider-main`; reversing the list routed both to `provider-replica`. A consumer with two
  slots on `127.0.0.1:8001` still failed validation with `ListenerConflict`.
- Added the warning channel to serialized `Plan` output and the daemon validation contract. CLI
  `validate`, `plan`, and `up` print the established `Warning [provider_collision] path: message`
  shape. Direct command runs confirmed all three; the controlled `up` experiment printed both
  warnings before deliberately failing against a nonexistent Docker socket. The daemon API test made
  an in-process request and asserted the returned code, path, and message. The Web UI now renders
  warnings on the selected deployment as soon as definition validation returns, and also retains the
  draft builder, desired-connections, and routing-editor surfaces. The TUI needs no separate channel:
  F7/F8/F9 stream the same CLI stdout into its ordered Operations timeline, whose warning rendering
  and filtering are covered.
- Corrected the crossed JAS group names: `ai-main` now owns `ai-main.jas-base.localhost` and
  `ai-feature` owns `ai-feature.jas-base.localhost`, with the compatibility fixture and generated
  address assertions updated together. The JAS definition hash is now
  `0a06182fe9337f4d580eebe3f2c0724e1854cb488e51e957d0db154a2cea11f9`; its resource hash remains
  `1f6e979ac8162d3480ac098ad9282b18ee36533fca273c6c57df674cbeba3e9e`.
- Left routing-matrix membership unchanged. Adding `ui-1` and `ui-3` to `feature-services` is accepted
  by the planner and emits no warning, but the fixture's real bind-preview proof then changes from five
  routed providers to six and fails its contract (`left: 6`, `right: 5`). The group intentionally
  models the backend's five downstream slots while the three instance addresses keep the UI peers
  symmetric, so changing it here would repeat the Part 2 regression rather than clarify the fixture.
- Compatibility output was regenerated from the updated fixture and checked directly. Two independent
  `switchyard plan` runs were byte-identical for JAS base (37,583 bytes) and routing matrix (28,125
  bytes); the compatibility test also serializes two plans and compares complete output, Compose,
  routes, and hashes.
- Verification: `cargo fmt --all -- --check` produced no output; workspace Clippy with all targets,
  features, and warnings denied passed; `cargo test --workspace --all-features` passed 307 tests with
  0 failures and the five declared reliability ignores. Relative to the 303-test Part 2 baseline, the
  net four additions are three planner tests after replacing the removed rejection test, plus one
  daemon API test. Web TypeScript passed with no output; lint exited zero with exactly the four
  pre-existing exhaustive-dependencies warnings in `App.tsx` and `DeploymentBuilder.tsx`; 49 web
  tests passed across four files, one more than baseline for the visible planner-warning surface.

## 2026-07-30 V2 Part 1 — group membership becomes a list

- Replaced authored service-group provider maps with `instances` lists. The planner now derives
  capability-to-provider routing from each member's declared services, rejects duplicate providers
  with the exact one-provider-per-capability diagnostic, applies inherited overrides by capability,
  and retains explicit `instance/service` member references for multi-service instances.
- Bumped deployment definitions to `switchyard.dev/v1alpha2`. The loader rejects v1alpha1
  deployments with an actionable `switchyard migrate` error, and the new CLI transform converts
  provider maps to deduplicated member lists while preserving the per-transform seam for later V2
  schema changes. Independent review hardened the transform to compare resolved provider maps before
  and after conversion and refuse without writing when the list model would change capabilities or
  produce an invalid group.
- Restored all six migrated repository YAML files from their original text and applied only the
  version and group-schema edits. This removed serialization churn, restored the routing-matrix
  anchors, and fixed the daemon placement test whose literal flow-style replacement had silently
  stopped matching.
- Kept migration as a validated parse/transform/serialize operation rather than adding a fragile
  line-oriented YAML rewriter. Before any write the CLI now warns that comments, anchors, and hand
  formatting are not preserved; after success it lists the API-version change and each converted
  group with capability and unique-member counts. Tests prove the warning hook runs before the
  destructive write, migration round-trips, a second run is idempotent, and an unrepresentable
  provider map leaves the original file untouched.
- Removed the connection-detail fallback that substituted a literal `service` name if provider
  resolution failed; invariant failures now propagate. Updated the generated project-authoring skill
  to teach `instances` lists and corrected the support policy to list Deployment v1alpha2 separately
  from Overlay v1alpha1.
- Compatibility hashes were regenerated from planner output and then checked by the determinism
  test: routing matrix definition/resource hashes are `32966889…`/`fa0a1790…`; JAS base hashes are
  `31236b80…`/`9a02afbf…`. The test still plans each fixture twice and compares Compose, routes, and
  both hashes.
- Verification: `cargo fmt --all -- --check` produced no output; workspace Clippy with all targets,
  features, and warnings denied passed; `cargo test --workspace --all-features` passed 292 tests
  with 0 failures and the five declared reliability ignores. The previous baseline was independently
  confirmed at 282: the ten additions are four planner regressions (migration-required plus the
  three group rules), one CLI parser test, four migration tests, and one ops invariant test. Web
  TypeScript passed; lint exited zero with exactly the four baseline-confirmed
  exhaustive-dependencies warnings in `App.tsx` and `DeploymentBuilder.tsx`; all 48 web tests passed
  across four files.

## 2026-07-30 consolidated unfinished-work checklist

- Added `docs/unfinished-work.md` as the consolidated index of unfinished work while
  retaining `IMPLEMENTATION_PLAN.md` as the authoritative release checklist.
- Separated team-release blockers from missing end-to-end acceptance, non-blocking
  engineering follow-ups, deliberately deferred ideas, and explicit supported
  boundaries. This prevents optional platform expansion from being mistaken for a
  release blocker.
- Expanded the seven open security-review findings into individually checkable tasks and
  linked the existing MVP manual procedures rather than duplicating their instructions.
- Recorded two newly identified Phase 7 tasks in `IMPLEMENTATION_PLAN.md`: normal
  default-browser links for running custom domains, separate from managed-profile
  routing, and a discoverable Node.js 24 pin with an early version check.
- Reconciliation was documentation-only. Markdown links and checklist formatting were
  verified; no implementation status was promoted to complete.

## 2026-07-25 web UI Part 13 — security review and fixes

- Ran the security review Part 13 called for and was merged without. Two findings, both
  confirmed by experiment rather than by reading, both fixed here.
- **Credentials in cleartext to a remote host.** `has_embedded_http_credentials` and
  `is_http_repository` treated `http://` and `https://` identically and nothing rejected
  plaintext, so a browser-submitted password reached an `http://` remote as an
  `Authorization: Basic` header. Verified against a local listener, which captured
  `Basic dXNlcjpTRUNSRVQxMjM=` — a decodable `user:SECRET123`. New `is_cleartext_remote`
  refuses credentials for plain-HTTP remotes at the entry point, and suppresses the
  credential challenge itself so the browser never prompts for a secret it may not send.
  Loopback is exempt per the maintainer's call, keeping local registries and test servers
  usable: `localhost`, any `.localhost` name, and any address whose `IpAddr::is_loopback`
  holds, so `127.4.5.6` and `[::1]` qualify while `192.168.1.10` does not. A bare hostname
  that is not an address could resolve anywhere and is treated as remote.
- **Order-dependent host-key pinning.** `scan_ssh_host` pinned only the first key
  `ssh-keyscan` returned, but that order is unstable — three consecutive scans of
  github.com led with ECDSA, RSA, RSA. Approving on one scan and retrying on another
  therefore raised `host_key_changed`, the active-MITM alarm, on a benign retry. This
  failed closed, so it was never exploitable, but a security error that cries wolf teaches
  users to click through it. `ScannedHostKey` now carries every fingerprint, sorted for a
  stable approval prompt, pins all scanned keys into `known_hosts`, and matches an approval
  against the set.
- Both regression tests were mutation-checked: reverting each fix makes its test fail. The
  first attempt at the host-key test passed against the unfixed code because the
  `ssh-keygen` stub emitted a fixed order regardless of input; real `ssh-keygen -lf -`
  preserves input order, and only after the stub did too did the test reproduce the bug.
- Corrected the browser copy, which promised credentials "pass through memory only" and are
  "never saved". Both true, but they are claims about storage that read as claims about
  safety; the clone form and credential dialog now name the transport requirement.
- Verification: `cargo fmt --all --check` clean; workspace clippy clean under `-D warnings`;
  282 Rust tests passed, 0 failed (280 before, plus the two new regression tests); `npx tsc -b`
  clean; `npm run lint` exited zero with exactly the four pre-existing exhaustive-dependencies
  warnings; 48 web tests passed across four files.
- Remaining accepted risk: credentials are plain `String` with no `zeroize`, so they persist
  in freed heap until overwritten. Standard for a loopback-only daemon and low priority, but
  recorded rather than left implicit.

## 2026-07-25 web UI plan — bookkeeping reconciliation

- `docs/web-ui-plan.md` still had all 65 acceptance boxes unticked, including Part 1's, which
  shipped 20 commits earlier. The checkboxes were never maintained as work landed, so the file
  read as entirely outstanding while `PROGRESS.md` recorded it complete. Ticked them against
  verified code rather than against this file's own claims: `deregisterSource` and the
  `kind === 'managed'` branch for Part 1, the `/api/v1/operations` route and `destructive` field
  for Part 2, `ProfilesView`/`RunActionsView`/`HomeView` and their documented endpoints for
  Parts 3/9/12, `placedInstances` with the disabled removal button for Part 4, and
  `instanceResources`/`serviceResources` for Part 11.
- Two boxes deliberately left unticked. Part 9's shell-authoring item was struck through as
  out of scope; rewrote it as a plain "Not built" line, since a ticked-looking strikethrough
  reads ambiguously. Part 13's security-review item is genuinely open — that part was merged
  on a code-level audit without human sign-off, so ticking it would have recorded a review
  that did not happen. Noted the same caveat in Release status.
- Added a Status section to the plan naming `PROGRESS.md` as the authoritative record, so the
  two files cannot drift into contradiction again, and carried forward the two implementation
  follow-ups worth remembering: Part 12's Home loader cost, and Part 11c's exact-name profile
  join.

## 2026-07-25 web UI Part 13 — merge onto current main

- Part 13 was authored in an isolated worktree that had branched from `5ed8139`, 25 commits
  behind `main`, so it predated Parts 10, 11, 11a, 11b, and 12. Its own verification run was
  therefore not evidence about the current tree: `cargo test --workspace` aborted with a
  SIGABRT in a `router-pingora` websocket test that passes on `main`. Rebasing the change onto
  current `main` removed that abort entirely, confirming it was a stale-baseline artifact and
  not caused by the clone work.
- Conflicts resolved by keeping `main` and layering the clone additions on top, rather than
  taking either side wholesale. Notably `SourcesView` had to keep Part 1's unmanaged
  deregistration (`kind === 'managed'` branch and `deregisterSource`), which the stale branch
  would have reverted to managed-only worktree removal. `CommandKind` needed both `RunAction`
  (Part 9) and `Clone` in the enum, its `segment()` match, its rejection arm, and the web
  union.
- Two semantic gaps closed by hand after the merge built: the clone operation predated the
  `destructive` field (Part 2) and the `instance` field (Part 11b). Clone is registered as
  non-destructive via the existing `operation_is_destructive` predicate and carries
  `instance: None`, since it is a source-scoped operation with no authored instance.
- The unrelated `crates/router-tcp/tests/tcp_proxy.rs` edit in the stale branch was dropped; it
  was baseline drift, not Part 13 work.
- Verification on the merged tree: `cargo fmt --all --check` clean; workspace clippy clean with
  `-D warnings`; 280 Rust tests passed, 0 failed, 5 pre-declared reliability ignores; `npx tsc -b`
  clean; `npm run lint` exited zero with exactly the four pre-existing exhaustive-dependencies
  warnings; 48 web tests passed across four files.

## 2026-07-25 web UI Part 11c — startup-profile provenance

- Closed the last Part 11 caveat. Startup profiles are blocks: `project_profile_rows`
  (`crates/switchyard-profiles/src/lib.rs:285-296`) maps every entry of `spec.blocks` to a profile
  of the same name, so an instance's `block` is its profile identity. The inspector now joins the
  already-loaded profile library on `(name, deployment)` and names the profile with its origin and
  trust rather than reporting provenance as unavailable.
- The join is by exact name and deployment, not inference. A block with no matching library entry
  is named and marked `not listed in the profile library` rather than being given invented origin
  or trust.
- Moved `originLabel`/`trustLabel` into a new `packages/web/src/profileModel.ts` shared by
  `ProfilesView` and the inspector, following the `connectionModel.ts` precedent. Exporting them
  from `ProfilesView.tsx` directly would have added two `react(only-export-components)` lint
  warnings; the shared module keeps the count at the four pre-existing warnings.
- Added web coverage for the resolved profile line and for the unlisted-profile branch.
- Verification: `npx tsc -b` passed with no output; `npm run lint` exited zero with exactly the
  four pre-existing exhaustive-dependencies warnings; 41 web tests passed across three files;
  `cargo fmt --all --check`, workspace clippy with `-D warnings`, and 276 Rust tests (0 failed,
  5 pre-declared ignores) all passed.

## 2026-07-25 web UI Part 11b — instance-scoped operations

- Added ordered schema migration 008 and bumped the state schema to version 8. Operations now
  carry a nullable `instance` column plus an instance/time cursor index; opening an existing v7
  database creates the documented pre-migration backup before applying the column and index.
- Preserved honest legacy semantics: rows written before migration 008 remain readable with
  `instance` null. Null means the operation is not attributed to one specific instance, whether
  because it is deployment-wide, its target could not be validated, or it predates attribution.
- Persist genuine instance scope for bind consumers, validated logs targets (`instance` or
  `instance/component`), and managed-profile opens whose profile name is also an authored instance.
  Deployment-wide commands and structured/shell run actions remain null rather than inventing scope.
- Added exact instance filtering in state and daemon operation lists while preserving newest-first
  `(startedAt, id)` cursor pagination. Operation responses now expose nullable `instance`; the old
  `unsupported_operation_filter` rejection has been removed.
- The inspector requests the selected instance's durable operation page and shows instance-scoped
  records only. Deployment-wide and legacy null rows are deliberately not blended because they do
  not identify which instance they affected; an instance with no attributed rows gets an explicit
  empty state.
- Added state coverage for v7 migration backup, legacy null readability, exact filtering, and
  filtered cursor pagination; daemon coverage for a genuinely targeted logs operation; and web
  coverage for filtered rendering and the honest empty state.
- Verification: `cargo fmt --all --check` passed with no output; workspace Clippy with all targets
  and warnings denied passed; `cargo test --workspace` passed across 53 test/doc-test binaries with
  276 tests passing and the five declared reliability tests ignored; `npx tsc -b` passed with no
  output; `npm run lint` exited zero with exactly the four pre-existing exhaustive-dependencies
  warnings; all 40 web tests passed across three files.

## 2026-07-25 web UI Part 11a — per-service resource attribution

- Added the typed `dev.switchyard.service` ownership label beside the existing instance label and
  apply both to every planned service path: plain containers, remote containers, and the namespace,
  application, and router services emitted for sidecar-routed consumers. Service-owned volumes
  retain the same attribution.
- Expanded the state persistence allowlist only for the specific instance and service ownership
  labels. Round-trip coverage proves those labels survive while unrelated observed labels are still
  discarded at the trusted persistence boundary.
- Replaced browser-side resource-name substring matching with exact persisted ownership-label
  matching. Instance placement and service state, health, and resource placement now use those
  labels; a service without a matching observation explicitly renders `not observed`.
- No schema migration or version bump was added because `labels_json` already stores the trusted
  label map and the schema shape is unchanged. Existing version-7 resource rows cannot be honestly
  backfilled from names, remain readable without a backup or migration, and stay unattributed in
  the UI until a later reconciliation replaces them with newly labeled observations.
- Added planner coverage across plain, remote, and sidecar emission, state coverage for ownership-
  label retention and legacy current-schema rows, and web coverage for labeled service rendering
  plus the honest unavailable fallback for an old unlabeled resource whose name happens to match.
- Verification: `cargo fmt --all --check` passed with no output; workspace Clippy with all targets
  and warnings denied passed; `cargo test --workspace` passed across 53 test/doc-test binaries with
  274 tests passing and the five declared reliability tests ignored; `npx tsc -b` passed with no
  output; `npm run lint` exited zero with exactly the four pre-existing exhaustive-dependencies
  warnings; all 39 web tests passed across three files.

## 2026-07-25 web UI Part 11 — per-instance inspector

- Unified instance inspection in the existing right-hand Inspector. Instance-card Inspect actions
  and runtime patch-bay instance nodes now drive the same lifted selection; the former inline
  `DeploymentWorkspace` node inspector was removed, leaving one inspector surface.
- The selected instance view shows authored and observed placement, source identity, expanded
  block services, active incoming and outgoing connections through the shared
  `connectionModel.ts`, and the existing complete-provider-group editor and switch report.
- Closed by Part 11c: `DeploymentSnapshot.spec.instances` records an expanded block, and blocks are
  profiles, so the inspector joins the profile library on `(name, deployment)` and names the
  startup profile with its origin and trust. Blocks absent from the library are named and marked
  as not listed rather than given invented provenance.
- Closed by Part 11a: planner-emitted instance and service ownership labels now survive resource
  persistence, and the inspector uses them for per-service state, health, and placement without
  matching resource names. Legacy resource rows recorded before Part 11a remain explicitly
  unavailable until observed again because their missing attribution cannot be honestly backfilled.
- Closed by Part 11b: operation records now have nullable instance attribution, the daemon
  supports an exact persisted `instance` filter, and the inspector requests only the selected
  instance's operations. Deployment-wide and legacy null-instance rows remain readable but are
  explicitly excluded rather than blended into the instance list.
- Added coverage for the single-inspector invariant, patch-bay selection updating that same
  inspector, authored and observed placement, expanded service inventory and its explicit
  unavailable observations, active connections, startup-profile provenance absence, and the
  documented deployment-scoped operations approximation.
- Verification: `npx tsc -b` passed with no output; `npm run lint` exited zero with exactly the
  four pre-existing exhaustive-dependencies warnings in `App.tsx` and
  `DeploymentBuilder.tsx`; all 39 web tests passed across three files; `cargo fmt --all --check`
  passed with no output.

## 2026-07-25 web UI Part 10 — rich operation and log filtering

- Added one accessible, case-insensitive free-text filter to the event drawer. It composes
  with the existing deployment selector using AND semantics and matches each visible output
  line plus its parent operation's deployment, command kind (the available operation label),
  and operation ID. Copy plain text continues to serialize exactly that filtered event set.
- Scope caveat: emitted operation events contain only `line` and `stderr` metadata, while the
  durable parent operation provides deployment, kind, and ID. There are no structured instance
  or service fields to filter. Instance/service searching therefore honestly matches those names
  when they occur in the output line; no daemon fields or inferred metadata were fabricated.
- Added timeline markers driven solely by `Operation.destructive`; the client does not maintain
  a destructive command-kind list. Added web coverage for case-insensitive narrowing,
  deployment-plus-text composition, filtered copying, and marker presence/absence from the field.
- Added the same free-text filter to the durable Operations timeline itself, matching
  deployment, kind, operation ID, status, and captured stdout/stderr over the loaded page.
  Its help text names that scope, so the control does not imply it searches records the
  browser has not fetched.
- Verification: `npx tsc -b` passed with no output; `npm run lint` exited zero with exactly the
  four pre-existing exhaustive-dependencies warnings in `App.tsx` and `DeploymentBuilder.tsx`;
  all 37 web tests passed across three files; `cargo fmt --check` passed with no output.

## 2026-07-25 web UI Part 9 — project run actions (partial scope)

- Extracted the shared run-action file model, validation, CRUD, shell acknowledgement, typed
  operation specification, and process construction from `switchyard-ops` into the
  `switchyard-run-actions` leaf crate. Ops now re-exports that domain and the daemon consumes
  it directly, without a cross-crate source include or a new frontend dependency.
- Added authenticated list, structured-only CRUD, and preview/hash-confirmed execution routes.
  The server rejects shell-shaped authoring and mutations targeting an existing shell action
  with `shell_run_action_authoring_forbidden`. Shell execution requires a server-side
  `acknowledgeShellWarning` request before the project-local acknowledgement marker is written
  and execution begins.
- Added the Run actions rail view with structured and shell action presentation, structured
  authoring only, an explicit CLI/TUI shell-authoring boundary, deployment selection for
  structured actions, and an exact command/argv confirmation dialog before every run.
- Documented every run-action endpoint and its confirmation, validation, and acknowledgement
  contracts. Added daemon API coverage for list/CRUD, authoring rejection, acknowledgement
  enforcement, and both run paths; added API-client and view coverage for request shapes, the
  visible authoring boundary, and structured/shell confirmation previews.
- Verification: `cargo fmt --all -- --check` passed with no output; workspace Clippy with all
  targets/features and warnings denied passed; `cargo test --workspace --all-features` passed
  with the five declared reliability tests ignored; all 32 web tests passed; `npx tsc -b`
  passed with no output; `npm run lint` exited zero with the four pre-existing
  exhaustive-dependencies warnings in `App.tsx` and `DeploymentBuilder.tsx`.

## 2026-07-25 web UI Part 8 — connection transition and rollback details

- Typed the complete existing deployment-routes response in the web client, including every
  desired/current/previous/observed checksum and version field, transition JSON, timestamps,
  and append-only route-history fields. The daemon already exposed everything required, so no
  Rust or API-documentation change was needed.
- Replaced the collapsed route version with separate desired, observed, and previous columns,
  plus explicit transition state, apply status/error, and rollback availability. The browser
  shows the latest five activation records per router/binding, matching the TUI projection and
  rollback wording.
- Binding observation now returns the terminal operation to the workspace. After a complete
  switch, a result dialog reports atomic success or failure, command/error detail, durable
  desired/observed/status/transition/error observations, and recorded or available rollback
  information.
- Added web coverage for the exact typed history shape, separate route versions, transition and
  previous-version rendering, rollback history, and post-switch reports for both successful and
  failed terminal operations.
- Verification: all 30 web tests passed; `npx tsc -b` passed with no output; `npm run lint`
  exited zero with the four pre-existing exhaustive-dependencies warnings in `App.tsx` and
  `DeploymentBuilder.tsx`.

## 2026-07-25 web UI Part 7 — authored connection view while stopped

- Replaced the stopped deployment's runtime-patch-bay placeholder with a separate desired
  connection matrix loaded from the authored definition. It validates the definition to use
  the API's parsed definition preview, lists consumed slots and compatible complete groups,
  and includes consumers that have no binding yet.
- Kept runtime and authored topology deliberately separate. The stopped view is headed
  `Desired connections (authored state)` and says it is desired/authored rather than
  observed/runtime state; the running view is now headed `Observed runtime patch bay` and
  identifies its applied-snapshot source.
- Offline changes update only the authored `spec.bindings` mapping, then save through
  `updateDefinitionValidated` with the definition response's hash as `expectedHash`. Saved
  bindings are therefore concurrency-checked and take effect on the next Up; the stopped
  reconciliation callout and Run Up action remain unchanged.
- Extracted resolved-group, consumed-slot, consumer-list, definition-preview, and targeted
  binding-edit helpers into `packages/web/src/connectionModel.ts`. Both the runtime patch bay
  and stopped authored view use the same consumed-slot derivation.
- Added GUI coverage for stopped authored rendering with bound and unbound consumers, offline
  validated persistence with the expected hash, and the observed runtime patch bay while
  running.
- Verification: all 29 web tests passed; `npx tsc -b` passed with no output; `npm run lint`
  exited zero with the four pre-existing exhaustive-dependencies warnings in `App.tsx` and
  `DeploymentBuilder.tsx`.

## 2026-07-25 web UI Part 5 — guided instance authoring

- Added an `Add instance` entry point on existing deployments while retaining the existing
  whole-deployment builder. The new path is one progressively revealed form for registered
  checkout, trusted/valid profile, eligible device, instance identity, and schema-rendered
  profile parameters.
- Checkout changes validate only trusted/imported profile records against the selected
  registered source and target deployment. Untrusted or invalid profiles remain visible as
  unavailable explanations but cannot be selected. Ineligible devices are disabled in the
  selector and their server-provided eligibility reason is shown inline.
- Extended the existing profile validation endpoint with optional instance-authoring inputs
  and a non-mutating validated draft. Its planner-derived response now includes expanded
  service names plus per-service published ports and volumes. The browser persists that exact
  draft through the existing optimistic definition PUT, avoiding YAML parsing or expansion
  reimplementation in the client.
- `SchemaForm` now accepts field errors so planner diagnostics can be attached to generated
  parameter inputs. Profile, device, and parameter diagnostics are mapped to their respective
  controls; no new global validation banner was added.
- Added web coverage for the existing-deployment append path, checkout/profile trust and
  validity filtering, disabled ineligible devices with visible reasons, SchemaForm profile
  parameters, services/ports/volumes preview, and field-level diagnostics. Extended daemon API
  coverage for the richer default report and named instance/device/parameter draft preview.
- Verification: all 28 web tests passed; `npx tsc -b` passed with no output; `npm run lint`
  exited zero with the four pre-existing exhaustive-dependencies warnings; `cargo fmt --all --
  --check` passed with no output; workspace Clippy with all targets/features and warnings denied
  passed; `cargo test --workspace --all-features` passed across all unit, integration, and doc
  test binaries with only the repository's declared ignored reliability tests.

## 2026-07-25 web UI Part 4 — device eligibility and placement visibility

- The device API now returns the implicit `local` device server-side, followed by
  registered SSH devices. Each row separates raw persisted check status, SSH reachability,
  and runtime eligibility with an explicit reason, and includes authored
  `{deployment, instance}` placements.
- Device checks now use the same SSH-plus-Docker eligibility semantics as the TUI and CLI.
  The shared domain was extracted from `switchyard-ops` into the new leaf
  `switchyard-devices` crate; ops re-exports it and the daemon depends on the leaf directly,
  avoiding the existing ops-to-daemon dependency cycle.
- The daemon rejects removal of `local` and returns HTTP 409 `device_has_placements` with
  the blocking placements when an SSH device is still referenced. The Devices view shows
  reachability and eligibility in distinct columns, explains the eligibility reason,
  lists placements in the removal dialog, and disables occupied-device removal.
- Instance cards now show authored placement from the applied snapshot separately from
  observed placement on reconciled resources.
- Added daemon API coverage for eligibility projection, server-side local inclusion,
  placement listing, and the authoritative removal guard, plus web coverage for the two
  device columns, blocked removal, and authored/observed instance placement.
- Verification: `cargo fmt --all -- --check` passed with no output; workspace Clippy
  with all targets/features and warnings denied passed; `cargo test --workspace
  --all-features` passed across 51 test binaries/doc-test suites with 272 passed and the
  five declared reliability ignores; all 25 web tests passed; `npx tsc -b` passed with no
  output; `npm run lint` exited zero with the four pre-existing exhaustive-dependencies
  warnings in `App.tsx` and `DeploymentBuilder.tsx`.

## 2026-07-25 web UI Part 3 — startup-profile library and trust workflow

- Added authenticated profile list/detail/manifest-review/validate/import/remove endpoints.
  The shared discovery, hashing, trust projection, import, and removal domain now lives in
  the leaf `switchyard-profiles` crate. `switchyard-ops::profiles` and the existing ops-root
  exports re-export that crate's single compiled set of types, while the daemon depends on
  it directly; `switchyard-ops` can therefore retain its daemon dependency for standalone
  reconciliation without a Cargo cycle or a cross-crate source include.
- Added the only ops-layer API needed by the HTTP trust boundary: verbatim manifest review
  with a SHA-256 review hash, and reviewed import that compares and parses those exact bytes
  before recording trust. A changed manifest returns `profile_manifest_review_changed` and
  cannot be imported until the client retrieves and displays the new content.
- The Profiles web view shows origin/trust/shadow badges, expanded definitions, source
  manifest review before import or re-import, imported-profile removal, and checkout
  validation with expanded services and structured diagnostics. It explicitly states that
  profile editing is unavailable until a shared mutation exists.
- Added profile-domain reviewed-import coverage in `switchyard-profiles`, one daemon API
  workflow covering discovery, detail, validation, stale review refusal, import,
  changed-content re-review/re-import, and removal, plus web client and view coverage. The
  daemon keeps its direct `yaml_serde` dependency for profile-validation request authoring;
  no frontend dependency was added.
- Verification: `cargo fmt --all -- --check` passed with no output; workspace Clippy with
  all targets/features and warnings denied passed; the workspace test command passed with
  271 tests and five declared reliability ignores (daemon 7 unit, daemon API 19 passed and
  one ignored, profiles 8, ops 17, and all doc tests); all 24 web tests passed; `npx tsc -b`
  passed with no output; `npm run lint` exited zero with the four pre-existing
  exhaustive-dependencies warnings in `App.tsx` and `DeploymentBuilder.tsx`.

## 2026-07-25 web UI Part 2 — durable operations list

- Added authenticated `GET /api/v1/operations`, backed only by the SQLite operations
  table. It returns newest-first pages of 50 records, exact deployment/kind/status
  filters, stable operation-ID cursors, and a server-computed destructive marker for
  `down` and `cleanup`.
- The persisted schema has no instance column. The accepted `instance` query parameter
  therefore returns `unsupported_operation_filter` rather than inferring an inaccurate
  value or adding an unrequested migration.
- The web client types the page and filters, and the Operations view reloads durable
  records whenever it is entered. In-memory command results remain visible for operations
  started by the current browser, while CLI/daemon records survive browser reloads and
  active durable records retain the existing cancellation action.
- Added state query coverage, daemon API coverage for filters/cursors/destructive records,
  and web client/view coverage.
- Verification: `cargo fmt --all -- --check` passed with no output; workspace Clippy with
  all targets/features and warnings denied passed; daemon/state tests passed (daemon 7
  unit, 18 API with the declared reliability test ignored, state 18, all doc tests); all
  21 web tests passed; `npx tsc -b` passed with no output; `npm run lint` exited zero with
  only the four pre-existing exhaustive-dependencies warnings in `App.tsx` and
  `DeploymentBuilder.tsx`.

## 2026-07-25 web UI Part 6 — initial connection authoring

- `DeploymentWorkspace` now derives each consumer's consumed slots from the deployment
  spec's blocks and instances rather than from existing routes alone, mirroring the
  TUI's semantics in `crates/switchyard-tui/src/tabs/connections.rs:191-223` and
  `crates/switchyard-ops/src/connections.rs:76-118`. Consumers with required slots and
  no binding are listed as unbound instead of omitted.
- The provider-group selector renders for any consumer with required slots, offering an
  explicit unbound placeholder, and reuses the existing compatibility filter, preview,
  and bind command rather than a parallel path. A first bind states that there is no
  current provider group instead of leaving the old-provider column blank.
- Scope note: the workspace reads the applied snapshot, so this covers binding after
  the first `Up`. Authoring connections while stopped remains Part 7.
- Verification: 19 GUI tests pass (1 new), `tsc -b` clean, `oxlint` clean apart from the
  pre-existing exhaustive-deps warnings.

## 2026-07-25 web UI Part 1 — unmanaged source deregistration

- Added `ApiClient.deregisterSource(name)` against the already-routed
  `DELETE /api/v1/sources/{name}` (`server.rs:1299`), which takes no body and returns
  204; the endpoint was already documented in `docs/control-plane-api.md`.
- `SourcesView` now offers Remove for unmanaged sources as well as managed ones. The
  confirmation dialog states per kind whether the directory is deleted or only the
  registration is forgotten, and the dirty-worktree two-step guard applies to managed
  removal only, since unmanaged deregistration destroys nothing.
- Verification: 18 GUI tests pass (3 new), `tsc -b` clean, `oxlint` clean apart from
  the pre-existing exhaustive-deps warnings.

## 2026-07-23 browser-first registered projects

- Added non-destructive, idempotent `switchyard project register [directory] [--name]`.
  It writes a versioned `.switchyard/project.json`, initializes project-local SQLite,
  creates an empty authored `deployments/` directory, and registers the existing folder
  itself as the first unmanaged source without modifying pre-existing files.
  Project-root Git inspection excludes `.switchyard` so tool-owned state does not make
  a clean checkout look modified; user changes remain visible.
- `switchyard gui [project]` now selects that project from any working directory,
  reuses its daemon or starts one in the background, records daemon output under
  `.switchyard/daemon.log`, and opens the existing fragment-authenticated dashboard.
- Added authenticated `GET /api/v1/project`; the dashboard rail shows the project name
  and exposes its canonical root as hover context. Unmarked initialized projects remain
  compatible, and the TUI remains available for headless use.
- Updated the architecture, implementation checklist, GUI, development, API, and
  support documentation to make the dashboard the default local interactive client.
- Focused verification passes: CLI/daemon/state tests (61 CLI unit tests plus daemon
  parity, 7 daemon unit tests, 16/17 daemon API tests with the declared reliability
  ignore, and 17 state tests), all 16 GUI tests, and the production GUI build. A live
  temporary non-empty folder smoke preserved its existing file, registered its root
  source, auto-started the dashboard daemon, reported healthy status, and stopped it.
- Final `./scripts/check.sh` passes end to end: formatting, workspace tests with only
  the declared reliability ignores, all-target/all-feature Clippy with warnings denied,
  and rustdoc with warnings denied. The final GUI run also passes all 16 tests and its
  production TypeScript/Vite build.

## 2026-07-23 Linux portability and explicit remote identity correction

- Replaced the ineffective `DOCKER_SSH_OPTS` integration with one shared
  process-scoped Docker SSH transport used by ops eligibility, CLI lifecycle/status/
  logs/cleanup, and daemon reconciliation. An explicit registered identity is supplied
  as an argument-safe value with `BatchMode=yes` and `IdentitiesOnly=yes`; paths with
  spaces remain one argument. The private launcher directory is mode `0700`, is scoped
  to the synchronous Docker operation, and is removed on drop. Devices without an
  explicit identity retain normal OpenSSH configuration and agent behavior.
- Direct SSH eligibility probes now also set `IdentitiesOnly=yes` whenever an explicit
  identity is selected, so their authentication behavior agrees with Docker lifecycle
  operations.
- macOS socket-length tests retain their short `/private/tmp` root, while Linux uses
  `/tmp`; this removes the eleven Linux failures introduced by the macOS portability
  work without weakening socket, symlink, ownership, or cleanup assertions.
- Focused transport, ops, CLI host/runtime, daemon, and router host-gateway tests pass
  on macOS. Workspace Clippy with all targets/features and warnings denied is clean.
- Full locked workspace tests pass on the real Linux/aarch64 NixOS host with Rust 1.88;
  bootstrap also passes there. The board required debug symbols/incremental artifacts
  disabled and serial final linking because its loop-backed root had only about 2 GiB
  free; an initial transfer-only AppleDouble metadata failure was removed and rerun.
- A real macOS-to-device lifecycle passed with no usable SSH agent and an identity path
  containing spaces: eligibility, validate, plan, up/healthy, `InSync` status, followed
  logs, down, destructive cleanup, and a remote zero-leftover check. The disposable
  authorized key and verification state were removed. Routed traffic was not retested
  because this Snapdragon vendor kernel's known Docker bridge defect is unchanged.

## 2026-07-22 macOS portability — product and release completion

- Apple Silicon macOS 26 or newer is now a supported local host target with Docker
  Desktop in Linux-container mode. Bootstrap rejects Intel Macs and older macOS
  releases explicitly. Apple Containers are not required: the shared Docker/Compose
  runtime remains intact, while sidecar control crosses Docker Desktop through the
  ownership-checked exec bridge instead of a shared-filesystem socket.
- The exec bridge now inspects and verifies the complete ownership tuple and then uses
  the immutable inspected container ID for execution, closing the name-reuse race.
  Router integration coverage exercises authentication, apply, inspect, counters,
  events, drain, redaction, malformed frames, and the 1 MiB bound through the bridge.
- `examples/jas-base/smoke.sh`: PASSED on macOS after the platform fixes. It covers the
  image, legacy-script, Process Compose, worktree, custom-domain, independent binding,
  persistent-volume initialization, restart, and ownership cleanup workflows. The final
  unified Phase 6 proof passed Rust formatting/tests/Clippy/rustdoc, the React clean
  install/build and all 16 GUI tests, and the live JAS fixture in one run.
- Native macOS router tests pass for HTTP, HTTPS, streaming, WebSocket, gRPC, raw TCP,
  CORS, browser identity, managed proxy, and transactional rollback. The daemon API
  worktree failure was a real `/var` versus `/private/var` canonical-path bug and is now
  fixed using the durable registered source relationship rather than lexical roots.
- The opt-in routing-matrix Docker Desktop restart proof passed: it preserved applied
  route state, recovered stopped Compose resources, refreshed the host gateway after
  Docker reassigned published ports, restored the selected route, and completed
  ownership-safe stop and cleanup. Daemon restart, stale-state, interrupted-operation,
  and sidecar crash recovery remain covered by the native and fixture proofs.
- Release assembly now works with stock macOS `shasum` and Bash 3.2, produces a
  `darwin-arm64` archive, performs checksum verification, fresh install, in-place
  ownership-checked upgrade, executable invocation, uninstall, and an empty-prefix
  assertion. The final archive smoke passed, and a throwaway Ed25519 signing key
  produced and successfully verified its checksum signature with the fixed
  `switchyard-release` identity/namespace.
- GitHub Actions now has an Apple Silicon `macos-26` Rust/GUI/release job. Live Docker
  routing is gated to an explicitly enabled self-hosted Apple Silicon runner labelled
  `switchyard-docker`, because hosted arm64 macOS runners do not provide nested
  virtualization.
- macOS HTTPS guidance prints reversible per-user Login Keychain commands and never
  invokes privilege escalation. Automatic `.local` publication remains explicitly
  Linux-only; loopback and acknowledged explicit-address LAN binding remain available
  on macOS. The complete reliability suite passed its core, raw-TCP, HTTP reload,
  health-flap soak, and daemon-concurrency checks on macOS; fd/RSS bounds remain the
  precise Linux-only `/proc` evidence.
- No supported Chromium/Chrome for Testing bundle is installed on this host and a clean
  macOS user account is not available, so the final real-browser clean-account launch
  acceptance item remains open. Discovery, supported-version enforcement, profile
  isolation, private proxy-auth extension creation, and native `open` integration are
  implemented and covered below that external acceptance boundary.

## 2026-07-22 macOS portability — native-host foundation

- The ordered implementation and release checklist is tracked under `macOS support
  track` in `IMPLEMENTATION_PLAN.md`. The intended product target is now explicitly
  limited to Apple Silicon on macOS 26 or newer; Intel Macs and older macOS releases
  are not deferred compatibility work. The targeted macOS platform remains
  workspace-only until that exit gate passes.

- Bootstrap now verifies the native macOS host-process tools, and the CLI builds and
  passes its focused test and Clippy suites on Apple Silicon macOS with Docker Desktop
  running Linux containers.
- The native host gateway now launches without Linux `setsid` and preserves
  ownership-safe PID reuse, executable, and command-line checks using macOS process
  metadata. Managed-browser discovery also covers standard Chromium and Chrome for
  Testing application bundles.
- The routing proof is not yet complete on macOS. Docker Desktop can create a Unix
  socket inode in the host bind mount used by container sidecars, but its cross-VM
  shared filesystem rejects required socket permission/operation calls with
  `EINVAL`/`ENOTSUP`. A portable sidecar admin transport is still required before macOS
  can be promoted to a supported end-to-end platform. The failed proof cleaned all
  owned fixture resources.
- The proof also exposed stale Rust 1.85 fixture images after the workspace compiler
  floor moved to 1.88; both maintained fixture Dockerfiles now use Rust 1.88.
- Verification completed: `./scripts/bootstrap`, all 59 CLI unit tests plus daemon
  parity, and focused CLI Clippy with warnings denied. The live routing matrix builds
  its native and Linux/arm64 binaries and starts provider containers, then stops at the
  documented Docker Desktop sidecar-admin socket boundary.

## 2026-07-22 macOS portability — portable sidecar administration

- Replaced host-bind-mounted sidecar sockets with mode-`0600` Unix sockets that remain
  inside each sidecar filesystem. The shared typed client verifies the exact container's
  managed, deployment, instance, and resource-hash labels before sending the
  authenticated request over stdin to a bounded `docker exec` admin bridge. No admin
  port is published and the token is absent from command arguments and generated plans.
- CLI binding and route inspection, daemon multi-router apply and rollback, diagnostics,
  and the maintained routing fixture now use the shared endpoint abstraction. The
  native host gateway retains its directly reachable owner-only Unix socket.
- `examples/routing-matrix/smoke.sh`: PASSED on Apple Silicon macOS 26.5.2 with Docker
  Desktop. This verifies fixed localhost isolation, custom domains, live sidecar and
  host switching, unhealthy-provider rollback, sidecar/application/host crash recovery,
  Compose restart, persistent volume state, and ownership-safe cleanup. The generated
  sidecars no longer mount a host runtime directory.
- Focused verification passed: workspace compile, planner/admin/CLI tests, the router
  bridge integration test, and Clippy with warnings denied for all affected crates. A
  pre-existing daemon source/worktree API test remains red on this host because it
  returns HTTP 400 where the test expects 409; the failure reproduces in isolation and
  is unrelated to router administration.

## 2026-07-18 Phase D part 1 remote-device runtime

- Real-device teardown follow-up: every remote Compose project now declares its own
  deterministic named bridge network, attaches every remote service to it, and labels
  the network with the deployment ownership tuple plus its device. Remote named volumes
  remain supported and carry the same ownership/device labels. Local Compose output is
  unchanged. `down` and destructive cleanup now attempt the local project and every
  remote project even after failures, aggregate all failures, and identify remote
  ownership failures by device and exact resource.
- Follow-up verification: the complete planner suite and compatibility check pass, the
  focused CLI down/cleanup regressions pass, workspace Clippy is clean with warnings
  denied, and formatting is clean. `cargo test --workspace` again progressed through
  the transport-independent router suites before the sandbox rejected the existing
  `router-pingora/tests/grpc_h2c.rs` listener with `EPERM`.
- Device-aware planning now validates the provider-only remote cut: registered devices,
  container execution, no consumer slots, and explicit publication of every provided
  capability port. Local-only Compose and compatibility hashes remain unchanged.
- Remote provider instances are partitioned into deterministic
  `compose.<device>.yaml` projects suffixed with the device name and labeled with their
  placement. Local sidecars and the host router target the registered device host and
  published capability port.
- Docker lifecycle, logs, discovery, status, and cleanup carry per-command
  `DOCKER_HOST` and SSH transport state, gate every remote with `docker version` before
  mutation, start remotes before local consumers, and stop/clean in reverse order. The
  original `DOCKER_SSH_OPTS` implementation was replaced by the 2026-07-23 correction.
- Generated manifests persist remote project/device placement. Reconciliation observes
  each referenced daemon, tags resources by device, and records an explicit
  `device_unreachable` diagnostic while retaining the last remote observations.
- Verification: all planner tests (including unchanged compat goldens), all state tests,
  all ops tests, and the four focused remote-runtime tests pass. Workspace Clippy is
  clean with warnings denied and formatting is clean. `cargo test --workspace` compiled
  the workspace and progressed through the router suites, then the sandbox rejected
  the existing `router-pingora/tests/grpc_h2c.rs` listener with `EPERM`; no network or
  Docker test was claimed. Real LAN execution remains the Phase D end-to-end follow-up;
  no TUI implementation or TUI documentation changed here.

## 2026-07-17 repository/worktree instance UX

- The project TUI now distinguishes repositories, linked worktrees, and ordinary
  directories instead of presenting every checkout as an undifferentiated source.
  Ownership and parent-repository context remain visible.
- Pressing `w` on a selected repository or linked worktree opens a one-field managed
  worktree form. The entered checkout name becomes a new branch based on the selected
  checkout's exact HEAD commit. The non-destructive `SourceManager` creates and
  registers it under `.switchyard/worktrees`, making it an instance choice immediately.
- Instance creation now presents blocks as reusable startup profiles and sources as
  checkouts/worktrees. Project run scripts remain deployment-level operations rather
  than becoming a second, conflicting instance execution format.
- When a newly registered worktree is selected for an instance, targeted YAML insertion
  preserves `type: worktree`, repository path, and requested ref.
- Verification: focused TUI and source-manager coverage includes minimal worktree-form
  selection, automatic branch creation from an exact base commit, and authored
  worktree relationship preservation. All 12 source-manager and 27 TUI tests pass;
  focused Clippy is clean with warnings denied, and formatting/diff checks pass.

## 2026-07-17 standalone TUI reconciliation and initialized skill

- The TUI now invokes the daemon's shared synchronous reconciliation path before
  loading deployment rows. Standalone lifecycle commands therefore refresh generated
  manifests and labeled Docker observations even when no daemon process is running.
- The Instances table merges runtime services with authored block/source context and
  falls back to authored-only rows before first apply, removing the duplicate
  authored/runtime rows and stale `not applied`/`stopped` display after a healthy `up`.
- `switchyard init` now scaffolds a concise project-local
  `.agents/skills/switchyard-project` skill with Codex UI metadata. It guides agents to
  validate and plan authored YAML, use the TUI or explicit lifecycle commands, preserve
  volumes by default, and avoid editing generated state or embedding credentials.
- Verification: the complete workspace suite passes with the five declared reliability
  ignores, all 25 TUI tests pass, workspace Clippy is clean with `-D warnings`, and the
  skill validator accepts both the embedded template and a freshly initialized project.
  A live TUI run against the reported project now displays `state: running` after
  reconciling its healthy container and excludes unrelated host deployments.

## 2026-07-17 native Git authentication handoff

- Replaced the intermediate SSH credential/askpass layer with a native terminal handoff.
  The TUI leaves raw and alternate-screen modes, invokes `git clone` with inherited
  stdin/stdout/stderr and no authentication overrides, then restores the full-screen UI.
  Git credential helpers and OpenSSH agent/config/key selection and prompts therefore
  work identically to a shell clone.
- SIGINT is handled while Git owns the terminal so Ctrl-C interrupts Git without leaving
  the parent TUI in a corrupted terminal state. Failed/interrupted clones retain Git's
  visible output until Enter is pressed, clean partial clone targets, and return an
  actionable error to the source dialog.
- Live pseudo-terminal verification cloned and registered a local repository across the
  suspend/resume boundary. A second run against the reported GitHub URL displayed the
  native OpenSSH `Enter passphrase for key ...` prompt, accepted Ctrl-C, waited for Enter,
  and restored the TUI with an interrupted-clone retry message.

## 2026-07-16 TUI source-dialog UX and Git SSH authentication

- Follow-up: Git clone submission now always passes through the options review popup
  instead of cloning immediately from the location screen. Enter opens the review,
  Enter there yields to native Git, and authentication failures return to that popup
  with retry guidance.
- Replaced the four-field source form with a mode selector and exactly one location
  input. Local directories and Git clone addresses derive stable source names from the
  final path/repository segment; collisions receive the first available numeric suffix.
- Git ref and authentication settings moved into a dedicated `F2` popup with contextual
  descriptions. Authentication is delegated to native Git/OpenSSH behavior.
- Terminal bracketed-paste mode now delivers a pasted location atomically to its focused
  field and strips trailing CR/LF, preventing URLs from spilling into adjacent inputs.
- Background/non-interactive source APIs remain prompt-free. Interactive TUI clones use
  the dedicated native terminal handoff.
- Clone validation rejects embedded HTTP credentials and option-like/control-character
  refs before invoking Git. Failed clone directories are removed so an authentication
  correction can be retried immediately.
- Verification: all 11 source-manager and 24 TUI tests pass, including one-location
  input, inferred naming, isolated bracketed paste, required authentication review and
  terminal-handoff queueing, native interactive clone registration, credential
  rejection, and failed-clone cleanup. The complete workspace test suite passes with
  only its five declared reliability ignores;
  workspace Clippy passes for all targets and features with `-D warnings`; workspace
  formatting and diff checks are clean.

## 2026-07-16 standalone project TUI workflow

- The intended fresh-project path is now `switchyard init` followed by
  `switchyard tui .`: Sources, Devices, and Instances are first-class keyboard views,
  with cyclic forward/back navigation and updated in-app help.
- Devices support inline validated registration, background SSH connectivity checks,
  persisted check detail, selectable rows, and confirmed registry-only removal. The TUI
  reuses the state and SSH safety contracts, including option-injection guards and no
  password/key-material storage.
- The Instances view now presents authored instances even before they are running. Its
  add-instance form selects an existing block and a declared or registered source; a
  newly selected registered source is inserted into `spec.sources`. Targeted YAML
  insertion preserves unrelated scaffold content and comments, validates by planning a
  same-directory draft, and atomically replaces the definition only after success.
- The pairing selector exposes consumer/provider-group changes with incompatible groups
  omitted and applies the selected complete replacement through typed, shell-free
  `switchyard bind` arguments. Durable generated binding state is reloaded after the
  operation.
- Runtime placement remains explicitly local. Registered devices are currently SSH
  connectivity targets; no inert per-instance device field or fake remote placement was
  added ahead of a distributed-runtime design.
- Verification: all 19 TUI tests pass, including the exact initialized deployment
  template with a registered-source instance, view navigation, device rendering and
  validation, YAML preservation, and bind argument construction. The complete workspace
  test suite passes with only its five declared reliability ignores; workspace Clippy
  passes for all targets and features with `-D warnings`; workspace formatting is clean.
  A live pseudo-terminal smoke initialized and validated a fresh project, launched
  `switchyard tui <project>`, accepted `q`, and restored the terminal cleanly.

## 2026-07-16 device registry and SSH checks

- SQLite schema v5 adds validated, uniquely named device registrations and durable
  last-check status without any password or key-material fields. Historical v4 stores
  upgrade transactionally with the existing pre-migration backup guarantees.
- The authenticated v1 API and CLI provide device list/add/remove/check parity. Checks
  invoke `ssh` with a direct argument vector, batch mode, a five-second timeout, and
  host-key `accept-new`; status mapping and missing-SSH behavior are covered without a
  live network dependency.
- The GUI adds a Devices rail view with inline add validation, persisted status and
  timestamps, row refresh after checks, and confirmed removal.
- Verification: state and daemon suites passed (including 16 API tests plus one
  pre-existing ignored reliability test); the CLI suite passed with only the
  sandbox-blocked host-runtime socket test filtered; workspace Clippy passed with
  `-D warnings`; all 16 GUI tests and the production GUI build passed. The exact
  combined Rust command was attempted first and stopped at that pre-existing socket
  test with `EPERM`.
- Review fixes applied after the Codex pass: device user/host values may no longer
  start with `-` (and the user may not contain `@`), closing an SSH option-injection
  path where a crafted user such as `-oProxyCommand=...` became the leading token of
  the `user@host` destination argument; and the daemon check endpoint no longer holds
  the state-store mutex across the SSH subprocess, so a slow connect cannot stall
  unrelated API requests.
- Local re-verification after the fixes: full `switchyard-state`/`switchyard-daemon`/
  `switchyard-cli` suites pass unfiltered (including the socket test the Codex sandbox
  blocked), workspace clippy passes with `-D warnings`, all 16 GUI tests and the
  production build pass. Live proof on this machine: device add/check/list/remove via
  the built CLI against a LAN host and an unreachable TEST-NET address produced real
  `ssh` runs with correct `unreachable` mapping for both timeout and
  connection-refused, persisted timestamps/details in the store, and the injection
  attempt was rejected with `invalid_device_user`.

`IMPLEMENTATION_PLAN.md` remains the task-level checklist. This file records the
implemented shape and the evidence used to close a phase.

## 2026-07-16 project TUI Sources view

- Added `switchyard tui [<project-dir>]` and a Ratatui/Crossterm terminal shell with
  panic-safe terminal restoration, responsive resize/event handling, Sources and
  placeholder Instances tabs, a footer/spinner, and an in-app help overlay.
- The Sources view lists live registry/Git inspection, registers local paths, creates
  managed URL clones on a background thread, and confirms safe removal. Every mutation
  remains in `SourceManager`/`StateStore`; managed deletion retains ownership and dirty
  guards.
- The view dispatcher isolates Sources and Instances modules for the follow-on
  Instances implementation. State-machine and `TestBackend` rendering tests cover form
  validation, confirmation cancellation, and inline errors.
- Verification: the TUI suite passed with the cached Ratatui stack on Rust 1.94. The
  combined CLI/TUI test reached 49 passing CLI tests before the sandbox rejected the
  pre-existing host-runtime socket test with `EPERM`; the filtered CLI suite and daemon
  parity test then passed. Focused TUI/source Clippy passed with `-D warnings`. Workspace
  Clippy on Rust 1.94 stopped on new lints in pre-existing daemon code; Rust 1.85
  verification awaits a fetch of pinned `instability` 0.3.1, which is not present in
  the offline cache.
- Review pass after the Codex run: the add-form's string-sentinel action channel
  (`__submit__`/`__close__` smuggled through the error field) was replaced with an
  explicit `FormAction` enum handled directly in the key handler, and the renderer's
  sentinel filter was removed. Local re-verification with network and the pinned
  toolchain: full workspace test suite passes (43 suites, including the socket test the
  Codex sandbox blocked), workspace clippy passes with `-D warnings`, and the lockfile
  resolves. Live pty proof: the TUI launches in a scaffolded project, renders tabs,
  toggles help, quits on `q` with the terminal restored, and an end-to-end add flow
  through the modal cloned a local git URL on a background thread and registered it as
  a managed source visible to `switchyard source list`.

## 2026-07-16 project TUI Instances view

- Replaced the Instances placeholder with durable deployment/resource presentation,
  including the reconciliation-aware stopped state for applied deployments with no
  observed resources, service status/health rows, latest operations, and multiple
  definition selection.
- Direct up/status/down/plan actions and structured run-script presets share typed CLI
  argv construction. Work and stdout/stderr consumption run off the event loop, with a
  scrollable output tail and terminal exit-code reporting. The CLI remains the entry
  point so daemon-compatible actions retain automatic daemon delegation, while overlay,
  variation, and set options remain representable.
- Added lenient project-local `.switchyard/run-scripts.yaml` loading, UI-level
  create/edit/delete validation, structured and arbitrary-shell forms, and a shell
  execution notice. Round-trip/malformed-file, argv mapping, modal/confirmation state,
  and TestBackend rendering coverage were added.
- Verification: `cargo test -p switchyard-tui` passes (13 tests), workspace Clippy
  passes for all targets with `-D warnings`, and workspace formatting is clean.
- Review pass found no defects; local live proof through a pty on a scaffolded
  project with Docker: `u` brought the deployment up to healthy, `s` refreshed status,
  `x`/`y` tore it down to the stopped presentation with zero leftover containers; a
  structured `plan` preset with the dev overlay ran through typed argv; a shell preset
  triggered the one-time warning, ran after `y` with its output streamed into the
  pane, persisted the acknowledgement file, and ran without a second warning
  afterwards. The TUI exited cleanly each time with the terminal restored.

## 2026-07-16 GUI serving correction

- The bundled GUI is served by the daemon below `/gui/`. Its Vite build now emits
  relative asset URLs, so JavaScript, CSS, and favicon requests remain under that
  prefix rather than falling through to the authenticated server root.
- Verification: `npm run build` passed in `packages/web`; a live daemon returned HTTP
  200 for `/gui/`, its generated JavaScript asset, and an authenticated
  `/api/v1/deployments` request.

## Phase 4 implementation

- The planned routing-matrix contains three independently sourced UIs, two
  independently sourced backends, two five-service groups, and a shared audit provider.
- UI custom domains and fixed `localhost:10081` browser routing run through the native
  gateway; backend fixed ports `8001`–`8005` run through namespace-sharing sidecars.
- `uiRoutes` cross-checks Origin-to-backend routing, backend bindings, and downstream
  group expectations. Conflicts fail with `BackendGroupInvariant` and duplication
  guidance. `bind` updates all attached UI expectations with the backend group.
- Candidate snapshots are provider-health-gated. An unhealthy candidate returns a
  rollback diagnostic and leaves the active version unchanged.
- Provider DNS is resolved before Pingora peer construction, and health probes are
  task-isolated so an upstream resolution failure cannot take down a router worker.
- Generated long-running Compose services use `restart: unless-stopped`. The host
  runtime detects changed ephemeral Docker publications and refreshes its owned gateway.
- `examples/routing-matrix/smoke.sh` covers live UI/group switching, complete snapshot
  observations, rollback, delayed readiness, provider/router/application/host crashes,
  Docker/Compose recovery, custom domains, fixed addresses, and volume persistence.
- `scripts/phase4-proof.sh` is the clean-checkout release command; CI runs it on Linux
  `x86_64`, and it was run locally on Linux `aarch64`.

## Verification

- `cargo test -p switchyard-cli -p switchyard-planner --all-features`: passed.
- `cargo test -p router-pingora --test http_proxy --all-features`: passed.
- `cargo test --workspace --all-features`: passed, including router health rollback,
  DNS containment, protocol, transition, and shutdown coverage.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: passed.
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps`: passed.
- `./scripts/phase4-proof.sh`: passed as the final one-command release check.
- `examples/routing-matrix/smoke.sh`: passed on Linux `aarch64` with Docker Engine
  29.5.2 and Docker Compose 5.1.4; its cleanup left zero owned containers and volumes.
- Rust formatting was checked with the available Nix-provided Rust 1.95 `rustfmt`; the
  shell's `cargo-fmt` shim could not launch because its dynamic loader is absent.

## Phase 5 implementation

### SQLite state

- `switchyard-state` is a synchronous, daemon-neutral library using bundled SQLite at
  an explicit caller-provided path; `.switchyard/state.sqlite3` is the documented
  per-project convention.
- Two ordered embedded migrations establish applied deployment snapshots, append-only
  deployment/operation/resource/health/route history, immutable route-snapshot
  activation records, and expiring operation leases. Existing databases receive a
  non-overwriting pre-migration file backup, and newer schemas are refused.
- Applied snapshots and structured contexts reject literal values in secret-bearing
  fields. The public secret type represents environment-variable and file references,
  and reconciliation retains only Switchyard ownership labels from Docker observations.
- Reconciliation compares generated manifest definition/resource hashes, nullable
  last-applied state, and injected Docker-label observations. It records observations
  without changing runtime resources or promoting recovered manifests to desired state.
  Stable drift codes cover missing, mismatched, multiply hashed, and invalidly owned
  state. A deleted or older restored database therefore recovers observations without
  inventing a successful apply.
- Focused offline evidence: 9 unit tests passed; isolated crate Clippy passed with
  `-D warnings`; isolated crate rustdoc passed with `RUSTDOCFLAGS=-D warnings`; and
  workspace formatting passed.
- The required repository-level state test, workspace test, workspace Clippy, and
  workspace rustdoc commands were attempted, but Cargo stopped before compilation
  because this shell could not resolve `index.crates.io` while fetching the pre-existing
  `bytes` dependency of `router-pingora`. They must be rerun in a network-enabled or
  fully vendored environment; this is an environment verification gap, not a recorded
  pass.

### Daemon and API

- `switchyard-daemon` provides a standalone binary and the developer-facing
  `switchyard daemon run/status/stop` group. It binds loopback only, runs migrations and
  startup reconciliation, writes an atomic mode-0600 discovery document, and cancels
  and joins active operations before graceful shutdown.
- Axum is the small HTTP routing layer on the existing Tokio runtime. Versioned serde
  contract types remain framework-neutral. Every endpoint is under `/api/v1`, uses
  stable JSON error codes, and requires a random project-local bearer credential.
- The subprocess backend reuses the exact one-shot CLI implementation with an internal
  recursion guard, preserving stdout, stderr, and exit codes. Secure discovery selects
  the daemon when reachable; absent or stale discovery retains the old one-shot path.
- Mutations use heartbeated `switchyard-state` deployment leases; apply work also uses a
  configurable global semaphore. Reads acquire neither. Cancellation, shutdown,
  subprocess completion, durable status updates, and lock release share a terminal path.
- Per-operation SSE publishes operation, build, health, route, and log events with
  monotonic IDs, retains 2,048 records, and replays records after `Last-Event-ID`.
  Status and structured errors survive restart in SQLite; raw command output and event
  buffers remain memory-only to avoid persisting possible application secrets.
- Phase 5 review hardening retains live-bind and rollback attempts across partial
  failures, cancels and joins blocking bind work after lease loss, bounds in-memory
  terminal operations to the most recent 64, waits through SSE with backed-off polling
  fallback, authenticates discovery peers with daemon status, and applies bearer
  authentication exactly once in router middleware.
- Docker-free tests cover auth, versioned-only routing, every SSE category and replay,
  mutation contention, global limiting, mid-operation cancellation, SQLite restart
  recovery, no-daemon fallback, and byte-identical API-backend CLI output. The production
  listener and Docker observation paths remain integration boundaries; this execution
  sandbox rejects socket creation with `EPERM`.
- Verification for this increment: `cargo test -p switchyard-daemon --all-features`
  passed (6 tests plus doc tests); the focused CLI fallback/API parity integration test
  passed; workspace Clippy with `-D warnings` passed; workspace rustdoc with
  `RUSTDOCFLAGS="-D warnings"` passed; and workspace formatting passed. The exact
  workspace test built successfully and passed every test reached before the first
  socket-based Pingora integration test (`grpc_h2c`) failed to bind with sandbox
  `EPERM`. An earlier isolated CLI run reached the same restriction in its pre-existing
  Unix-socket host-runtime test. This is the sole repository-test verification gap.

### Live router control

- Router administration is now a shared typed crate used by both the one-shot CLI and
  daemon. It retains the existing newline-delimited Unix-socket protocol, provides
  configurable timeouts, and decodes snapshot identities and activation
  acknowledgements without exposing credentials in errors.
- The real daemon backend owns binding changes. It plans from the last generated
  resolved state, pushes complete monotonic snapshots to the selected consumer sidecar
  and a running host gateway, and requires matching version, checksum, and `activated`
  status before recording success or replacing generated artifacts.
- Multi-router changes compensate for partial activation by reapplying the prior route
  configuration at a newer version. Timeouts, invalid/stale acknowledgements,
  provider-health rollback, compensation success, and compensation failure are stored
  as secret-safe route history and linked to the durable operation ID.
- SQLite schema version 3 adds per-router/binding desired, current, previous, and
  observed version/checksum state, transition policy, status, and last error code.
  `/api/v1/deployments/:deployment/routes` returns this state and append-only history;
  daemon-backed `status --routes` and `routes` append a compact version summary.
- Bind requests and `switchyard bind` accept additive close, drain (with timeout), and
  pin controls. The selected policy is applied consistently to HTTP, HTTPS, WebSocket,
  gRPC, and TCP fields in the router's existing transition contract.

### Phase 5 exit gate

- Successful daemon applies persist the resolved desired snapshot and definition hash.
  A transport-independent restart test proves custom domains and bindings remain in
  SQLite, while a live-binding test proves failed and rolled-back route history and all
  visible versions survive daemon reconstruction.
- The same recovery test deletes SQLite and verifies startup rediscovers the generated
  routing-matrix manifest with `applied_state_missing` drift instead of inventing an
  apply. State-layer coverage injects owned Docker-label observations and proves the
  same safe recovery path for runtime resources.
- CLI parsing, daemon request generation, no-daemon fallback, byte-compatible command
  output, additive route-version output, and the shared transition policy contract are
  automated. Existing command output remains unchanged before the additive version
  section.

## Phase 5 verification

- `cargo test -p switchyard-daemon --all-features --test api`: passed (8 tests),
  including restart, domain/binding persistence, route failure/rollback persistence,
  lock-loss cancellation with attempt persistence, bounded terminal retention, and
  deleted-database recovery.
- `cargo test -p switchyard-state -p switchyard-router-admin -p switchyard-daemon
  --all-features --no-fail-fast`: passed (state, shared client, daemon, integration, and
  doc tests).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: passed.
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps`: passed.
- `cargo fmt --all -- --check`: passed after formatting the increment.
- `cargo test --workspace --all-features`: compilation succeeded and all tests reached
  passed until the pre-existing `router-pingora` `grpc_h2c` socket test; its listener
  failed with sandbox `EPERM`. The exact workspace command therefore did not pass in
  this environment.
- `./scripts/phase5-proof.sh`: daemon/recovery portion passed; the Docker routing-matrix
  gate was explicitly skipped because access to `/var/run/docker.sock` was denied.
  Docker Compose 5.1.2 is installed, but the Engine is unavailable to this sandbox.

## Phase 6 implementation

### Adapter SDK (Part 1)

- `switchyard-adapter-sdk` defines the versioned `switchyard.dev/adapter-sdk/v1alpha1`
  contracts for source, execution, supervisor, route, and probe adapters. Configuration
  and recovery handles cross the boundary as serializable JSON; states, events, logs,
  claims, source identity, and route observations use normalized SDK types.
- Every adapter declares id, semantic version, supported SDK contract versions, and
  protocol/live-update/recovery/feature capabilities, and must publish a draft 2020-12
  JSON Schema (schemars generation, offline jsonschema validation). The registry rejects
  malformed ids/versions, duplicates, and incompatible contract declarations with stable
  `RegistryErrorCode`s; listing returns declaration + schema metadata for schema-driven
  forms.
- A public conformance suite checks schema compilation and dialect, valid/invalid
  examples, deterministic validation, capability consistency, compatibility, and
  lossless opaque-handle round trips.
- `switchyard-adapters` implements the seven built-ins (`source-path`, `source-git`,
  `execution-container`, `execution-runner-script`, `supervisor-process-compose`,
  `route-switchyard`, `probe-health`) at planning level; execution remains owned by the
  existing generated-Compose runtime. Trusted host execution is explicitly deferred and
  guarded by a registry test.
- `switchyard-planner` validation resolves sources, executions, probes, provider
  capabilities, and route slots through the built-in registry while keeping the
  deployment YAML, diagnostics style, and deterministic artifact generation unchanged.
  A regression test proves worktree sources still require an existing repository and a
  non-empty ref through the adapter path.
- Documentation: `docs/adapters.md`.

### Phase 6 Part 1 verification

- `cargo fmt --all -- --check`: passed.
- `cargo test --workspace --all-features`: passed on this host (all suites, including
  the socket-based router integration tests unavailable to the implementation sandbox).
- `cargo test -p switchyard-planner --test planner`: 12 passed, including the new
  worktree adapter-path regression test.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: passed.
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps`: passed.

## Phase 7 import/export and collaboration — Part 4

- `switchyard-planner` owns the portable bundle contract in `bundle.rs`, because it is
  the crate that already owns strict deployment/overlay parsing and validation. The CLI
  keeps only local-machine conflict checks and presentation.
- `switchyard bundle export <deployment.yaml> [--with <overlay.yaml>]... [--output
  <file>]` writes one deterministic, reviewable
  `switchyard.dev/bundle/v1alpha1` JSON file with a SHA-256 content hash over the
  canonical payload. Export embeds deployment and overlay definitions, replaces local
  source/file/dotenv inputs with `requiredLocalInputs`, preserves secret references, and
  warns/replaces credential-looking literal keys.
- `switchyard bundle import <bundle-file> --into <directory> [--force]` verifies
  apiVersion and content hash, rejects machine-state paths in typed host-path fields,
  writes the deployment and overlay YAML without overwriting unless forced, scaffolds
  placeholder local inputs, validates through the existing planner path, prints the
  normal mutation preview, and starts no runtime resources.
- Import conflict reporting is CLI-only and read-only: generated manifests, live daemon
  deployment summaries, live bind checks, and Docker `inspect` probes detect
  `name_conflict`, `domain_conflict`, `port_conflict`, `live_port_conflict`,
  `external_resource_conflict`, and `docker_unavailable`.
- Docker conflict probing degrades to `docker_unavailable` in sandboxes without Docker.
  No new daemon endpoint was added; a future daemon-aware import workflow remains a
  follow-up.
- `docs/bundles.md` documents bundle contents, omitted machine state, secret/local-input
  handling, conflict codes, and safe sharing of block, deployment, group, and overlay
  definitions. `docs/development.md` links it from the documentation index.

### Phase 7 Part 4 verification

- `cargo fmt --all --check`: passed.
- `cargo test -p switchyard-planner`: passed, including export/import validation,
  tampered-hash rejection, and unsupported-apiVersion rejection.
- `cargo test -p switchyard-planner -p switchyard-cli`: compiled and passed all planner
  tests and the new CLI parser test, then hit the pre-existing
  `host_runtime::tests::failed_startup_cleanup_allows_a_clean_retry` Unix-socket bind
  sandbox failure (`Operation not permitted`). This is the same class of socket
  restriction recorded earlier and not a bundle regression.
- `cargo test -p switchyard-cli cli::tests::parses_bundle_commands`: passed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- CLI smoke: `switchyard bundle export examples/routing-matrix/deployment.yaml` to
  `/tmp`, followed by `switchyard bundle import ... --into /tmp/... --force`, passed.
  Import produced placeholder local inputs, a create-artifacts mutation preview, and
  read-only conflict diagnostics; Docker probing degraded to `docker_unavailable` in
  this sandbox.

### Source and worktree management (Part 2)

- `switchyard-sources` is a synchronous, daemon-neutral library: read-only Git
  inspection (repository root, linked-worktree detection, branch/detached HEAD, commit,
  staged/unstaged/untracked summary, ahead/behind), managed worktree/clone creation
  under `.switchyard/worktrees` and `.switchyard/clones`, and non-destructive removal.
  Non-repo paths and a missing git binary degrade to explicit unknown codes.
- Every mutating operation passes one `guard_mutation` gate: unmanaged sources are
  never mutated (deregistration only forgets the record), canonicalized paths must stay
  inside the managed roots, dirty working trees refuse removal without an explicit
  `allow_dirty` override, and unknown Git state refuses removal. No git command ever
  uses `--force`.
- SQLite schema version 4 (`registered_sources`) persists name, immutable
  managed/unmanaged kind, path, repository path, requested ref, and managed-relative
  location; live Git observations are always derived, never persisted as truth.
- `/api/v1/sources` (GET/POST/DELETE) and `/api/v1/worktrees` (GET/POST/DELETE) follow
  the existing bearer-auth and stable-error-code conventions. Review hardening moved
  all five handlers onto the Tokio blocking pool so a slow clone or worktree operation
  cannot stall async workers.
- CLI: `source list [--json]`, `source register/deregister`, `worktree create/remove
  [--allow-dirty]` with daemon-first execution and byte-stable one-shot fallback.
- Plans, manifests, and `switchyard status` now carry per-instance live source
  identities (path, repository, ref, commit, dirty) captured at plan time; definition
  and resource hashes still derive only from desired state.
- Documentation: `docs/control-plane-api.md` endpoints and a sources/worktrees section
  in `docs/development.md`.

### Phase 6 Part 2 verification

- `cargo fmt --all -- --check`: passed.
- `cargo test --workspace --all-features`: passed on this host (Codex-side run reached
  the known sandbox socket restriction only).
- Post-review daemon/sources rerun after the blocking-pool hardening: passed
  (daemon 4 unit + 9 API + parity, sources 6).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: passed.
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps`: passed.

### Overlays and variations (Part 3)

- Overlay documents (`kind: Overlay`) support deployment/instance selectors (required
  selectors must match unless `optional: true`), ordered environment (`envFiles`
  strict dotenv, `set`, `unset`), file injection (path or inline content, optional
  restricted templates, `replace: true` conflicts), parameters, and route selection.
  Deployments list overlays in order via `spec.overlays`; instances gained optional
  selector labels.
- Resolution follows the DESIGN.md precedence chain (block defaults < deployment
  overlays in order < instance values < `--set` ephemeral overrides), merges maps by
  key, honors `unset`, and records an origin trace with full shadowing history for
  every resolved environment value, parameter, file, and route.
- Injected files materialize only under
  `.switchyard/generated/<deployment>/overlays/<instance>/<content-hash>/` and are
  bind-mounted read-only; targets reject relative paths and `..` traversal and must
  fall under controlled container mount roots. Templates support only fixed-namespace
  `${...}` lookup (overlay variables, instance/deployment names, parameters) with
  unknown variables rejected — no execution of any kind.
- Secret overlay values are environment-variable or file references; previews, origin
  traces, resolved YAML, manifests, and Compose show only placeholders. Generated
  Compose interpolates `${SWITCHYARD_OVERLAY_SECRET_<hash>:?}` and the runtime injects
  real values solely into the `docker compose` process environment at apply time.
  Secret file injection is explicitly rejected as unsupported.
- `overlay validate` and `overlay diff --with ...` provide stable diagnostics and a
  per-service live/restart/rebuild classification against currently generated
  artifacts. `plan`/`up`/`down`/`status` accept `--with`, `--variation`, and `--set`;
  variations rename the deployment through existing deterministic naming with
  cross-variation listener/publication collision checks. Overlay-less output remains
  byte-stable.
- Documentation: `docs/overlays.md`.

### Phase 6 Part 3 verification

- `cargo fmt --all -- --check`: passed.
- `cargo test --workspace --all-features`: passed on this host (planner 17, CLI 32,
  all router/daemon suites green).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: passed.
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps`: passed.

### Schema-driven GUI foundation (Part 4a)

- Daemon additions: `GET /api/v1/deployments` (+ per-deployment detail with applied
  snapshot, reconciliation summary, resources, and manifest source identities),
  `GET /api/v1/adapters` (registry declarations plus JSON Schemas for schema-driven
  forms), and `/gui/` static serving of `packages/web/dist` (configurable, SPA
  fallback, traversal-safe). Static assets bypass bearer auth; `/api/v1` is unchanged
  except that operation SSE additionally accepts the credential via `access_token`
  query parameter because EventSource cannot set headers (loopback-only rationale
  documented).
- `switchyard gui` prints and best-effort-opens `http://127.0.0.1:<port>/gui/#token=…`
  using daemon discovery; the credential travels only in the URL fragment, which the
  web client captures into memory and strips from the location immediately.
- `packages/web` (Vite + React 19 + TypeScript, committed scaffold with pre-installed
  dependencies): typed API client with structured errors, operation polling and SSE
  subscription; DESIGN.md shell (deployment rail, canvas, inspector, collapsible
  event/log drawer, exact color tokens); deployment list/detail with per-instance
  source identity, live route versions, domains, and bindings; sources view with
  register-unmanaged and worktree create plus a two-step dirty-removal dialog;
  operations timeline with cancel and failure detail; guarded destructive commands
  (typed confirmation for down/cleanup, dirty-worktree acknowledgement before up);
  keyboard navigation, aria-live announcements, reduced-motion support, responsive
  fallbacks.
- Verification: workspace tests passed on this host (daemon API 12); fmt, workspace
  clippy `-D warnings`, and rustdoc `-D warnings` passed; `npm run build` passed and
  `npm test` passed (6 Vitest tests).

### Schema-driven GUI completion (Part 4b)

- Deployment definition API: `GET /api/v1/deployments/{name}/definition`,
  `POST /api/v1/deployments` (validate-first, `validateOnly` dry-run with plan
  preview, atomic hard-link create refusing overwrite), and
  `PUT .../definition` (SHA-256 optimistic concurrency, validate-first, atomic
  rename). All definition and source handlers run on the Tokio blocking pool because
  planner validation shells out to git for source identities.
- Patch bay: typed consumer/provider/group lanes, SVG cables colored by capability
  with direction arrows, node inspector (source, health, resources, active routes),
  keyboard-first switching through compatible-group selects (incompatible groups are
  omitted with an explanatory count), an always-available accessible route-matrix
  table that is also the narrow-viewport rendering, and reduced-motion compliance.
- Atomic switching: selecting a group prepares a pending change set; a preview dialog
  shows the complete replacement route table (old→new provider per slot) and the
  superseded snapshot version, with close/drain(timeout)/pin transition selection;
  apply goes through the existing `bind` command and surfaces
  acknowledgement/rollback results.
- Deployment builder: name validation, block instances with schema-driven adapter
  configuration, source selection from registered sources, parameters, continuous
  validation through the dry-run endpoint, expanded-service/compose preview, save,
  optional follow-up Up.
- `SchemaForm` renders draft 2020-12 object schemas (string/number/integer/boolean/
  enum/nested object/string array, required markers, descriptions) and degrades to a
  validated JSON textarea for unsupported constructs; a read-only block library lists
  registered adapters from `/api/v1/adapters`. No hard-coded per-adapter forms exist.
- Routing panel: custom domains, browser identity routes, and managed profiles are
  edited through the authored definition with a full line diff, validate-first gating,
  and optional plan/up follow-through — the CLI/API equivalent is the definition file
  plus `switchyard validate`.
- Per-instance log access from instance cards passes the existing `target` command
  field (review addition), completing combined and per-service logs in the GUI.

### Phase 6 Part 4 verification

- `cargo fmt --all -- --check`: passed.
- `cargo test --workspace --all-features`: passed on this host.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: passed.
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps`: passed.
- `npm run build`: passed; `npm test`: 12 Vitest tests passed.

### Real-codebase validation (Part 5)

- `examples/jas-base/` is a self-contained generic stand-in for the JAS legacy
  workspace: two image-backed database stand-ins with named volumes and a one-shot
  `lifecycle: task` schema-initialization service (`dependsOn: healthy`, consumers
  gated on `completed_successfully`), a fixed-port legacy shell script in a runner
  image for the Java stand-in, a five-process Process Compose suite per AI instance,
  and Dockerfile-built UIs with custom domains. The DESIGN.md topology is expressed
  verbatim (`ui-a → jas-main + ai-feature`, `ui-b → jas-feature + ai-main`, shared
  `db-main`), with both Java stand-ins consuming identical fixed `127.0.0.1:8001–8005`
  and `9101/9102` slots and both UIs consuming `127.0.0.1:10081`.
- Planner tests (`real_codebase_fixtures.rs`): full expansion assertions for the
  fixture; a fixture-swap test planning jas-base and routing-matrix through the
  identical deterministic path; an overlay/variation disjointness test; and a guard
  test proving no `jas` identifier exists in any production crate source.
- Discovered gap recorded in the plan (Phase 7): declared `LifecycleHooks`
  (`prepare`/`postReady`/`stop`/`cleanup`) are schema-only — nothing generates or
  executes them; the fixture deliberately uses task-lifecycle services instead and
  documents the gap in its README.
- Review fixes: the UI `java` slot originally declared `host: localhost`, which the
  router rejects (`invalid IP address syntax`) because listener binds require IP
  literals — changed to `127.0.0.1`, which serves the unchanged app's
  `localhost:10081` calls identically inside the namespace. The smoke script's
  variation demonstration now skips with a notice when another generated deployment
  legitimately claims `127.0.0.1:10081` (the collision guard working as designed in a
  shared workspace).

### Phase 6 Part 5 verification

- `cargo test -p switchyard-planner --all-features`: passed (21 tests including the
  four new fixture tests).
- `cargo fmt --all -- --check`, workspace clippy `-D warnings`, rustdoc `-D warnings`:
  passed.
- `examples/jas-base/smoke.sh`: PASSED end to end on this host (Docker Engine 29.4.0,
  Compose 5.1.2, Linux aarch64): build, registered unmanaged source + managed
  worktree, typed topology observations for both UIs and both Java stand-ins,
  task-based schema initialization, live AI-group switch without restarting the Java
  stand-in, source identity in status, database volume persistence across down/up,
  and zero owned resources after cleanup with the workspace git status unchanged.

### Phase 6 exit gate (Part 6)

- `docs/mvp-acceptance.md` audits every DESIGN.md §14 criterion (1–21) against named
  Rust tests, Vitest tests, and smoke-script assertions, deliberately distinguishing
  complete automation from partial automation; criteria 1, 3, 7, 14, and 18 carry
  documented manual procedures for their remaining manual portions. The CLI/API/GUI
  parity matrix covers every common operation; the two gaps it found were closed
  during review: `switchyard operation cancel <id>` (daemon-backed arbitrary
  operation cancellation from the CLI) and an instance-card **Open** button for
  managed-profile instances in the GUI.
- `docs/upgrade-recovery.md` documents test-backed upgrade (ordered migrations,
  pre-migration backups, newer-schema refusal, backup-based downgrade) and recovery
  procedures (daemon restart, deleted/restored SQLite, drift review, data-safety
  guarantees), each referencing the proving test by name.
- `scripts/phase6-proof.sh` is the one-command Phase 6 check: `scripts/check.sh`
  (fmt, workspace tests, clippy `-D warnings`, rustdoc `-D warnings`), a clean GUI
  `npm ci`/build/test, and the live `examples/jas-base/smoke.sh`.
- Honest residual limits recorded in the audit: browser routing is live-proven with
  Origin-bearing requests rather than a driven browser; Docker-label recovery by a
  restarted real daemon and Docker Engine restarts remain integration boundaries;
  concurrent variation execution is proven at planning level with a manual live
  procedure; the lifecycle-hooks execution gap is tracked as Phase 7 work.

## Phase 6 verification

- `./scripts/phase6-proof.sh`: PASSED on this host (Linux aarch64, Docker Engine
  29.4.0, Compose 5.1.2, Node 24): workspace formatting, full workspace tests,
  clippy `-D warnings`, rustdoc `-D warnings`, GUI clean install/build and 12 Vitest
  tests, and the complete live jas-base smoke (topology, worktree sources, live group
  switching, task initialization, volume persistence, ownership-scoped cleanup).
- `cargo test -p switchyard-cli --all-features`: passed (35 unit tests including the
  new `parses_operation_cancel`, plus the daemon-parity integration test).
- Earlier per-part verification is recorded in the Part 1–5 sections above; the
  routing proof remains covered by `scripts/phase4-proof.sh`.

## Post-phase-6 full review and re-verification (2026-07-15)

- `./scripts/phase6-proof.sh`: re-run PASSED end to end on this host, including the
  live jas-base smoke with clean ownership-scoped teardown.
- `examples/routing-matrix/smoke.sh`: re-run PASSED (the standing live gate for
  Phases 4 and 5; `phase4-proof.sh`/`phase5-proof.sh` are this plus already-passed
  workspace/daemon tests).
- `./scripts/check.sh audit`: cargo-audit 0.22.1 (0.22.2 needs rustc 1.88; the
  workspace toolchain is 1.85) with the two documented protobuf ignores.
- Manual code review of the highest-risk paths (daemon auth middleware and SSE
  query-token scope, GUI static serving traversal guard, definition create/update
  atomicity and optimistic concurrency, live-bind rollback/compensation, state-store
  lease acquire/heartbeat/release, sources `guard_mutation` containment, overlay file
  injection and secret placeholder/runtime injection, daemon discovery client): no
  major defects found.
- Review fix: `PUT /api/v1/deployments/{name}/definition` now validates the
  deployment name before deriving the definition path (the GET already did),
  closing a percent-encoded traversal-shaped read; covered by a new 404 assertion in
  `definition_absence_and_validation_failures_have_stable_structured_errors`.
- Follow-up review fixes: `api_for_tests` now prepares the daemon with empty runtime
  observations (production `start_with_backend` still observes Docker), so the daemon
  test suite is hermetic against Switchyard-labeled resources on the host — proven by
  rerunning the empty-state test with a live decoy-labeled container. `check.sh audit`
  now names the toolchain-compatible install (`cargo install cargo-audit --locked
  --version 0.22.1`; 0.22.2+ needs rustc 1.88, the workspace pins 1.85).

## Phase 7 LAN and private-network access — Part 1

- Added the versioned `spec.exposure` host-router contract. Omission remains
  loopback-only; LAN binding requires both `mode: lan` and
  `acknowledgeLanExposureRisk: true`. Stable validation codes cover non-loopback binds
  without opt-in, missing acknowledgement, and non-loopback providers in LAN mode.
- Host mode now accepts acknowledged non-loopback listener binds while keeping provider
  upstreams loopback-only. Wildcard binds expand to concrete local interface addresses,
  emitted in a structured `lan_exposure_warning` startup event and retained in the
  shared exposure summary.
- CLI apply/status output and daemon deployment list/detail inspection surface the
  effective mode and addresses. A changed owned host-router definition is stopped and
  replaced during normal re-apply, so reverting to loopback closes LAN listeners before
  the replacement starts.
- Contract round-trip and invalid-fixture tests cover the secure default and all three
  LAN validation failures; host-gateway tests cover concrete wildcard expansion. Final
  verification: `cargo fmt --all --check` and workspace/all-target clippy with
  `-D warnings` passed; router-config passed all 8 tests; daemon passed all 18 tests;
  CLI passed all 34 non-socket unit tests plus daemon parity; router passed all 10
  non-socket unit tests plus its tokenless host-command test. The sandbox refused the
  existing TCP/Unix-listener tests with `Operation not permitted`; those tests and the
  requested second-machine LAN reachability check remain for reviewer execution on a
  socket-capable host.

### Part 1 reviewer verification (2026-07-15)

- Reviewer fix: `explicit_identity_is_rejected_on_non_loopback_listener`
  (router-pingora, socket-dependent, outside the Codex sandbox's reach) now opts in
  to acknowledged LAN exposure so validation passes, and proves the explicit
  identity header stays untrusted on non-loopback listeners even in LAN mode.
- `./scripts/check.sh`: PASSED end to end (fmt, full workspace tests, clippy
  `-D warnings`, rustdoc `-D warnings`).
- Live LAN proof on this host (192.168.1.10) against a second machine
  (poco-f1-nixos, 192.168.1.167): LAN-mode host router on `0.0.0.0:18980` emitted
  the structured `lan_exposure_warning` listing every concrete interface address;
  a remote curl through the custom domain returned 200 with proxied backend
  content; the same config without the exposure opt-in was refused with
  `LanExposureNotEnabled`; reverting the bind to loopback made the remote curl
  unreachable again while local traffic kept working.

## Phase 7 LAN and private-network access — Part 2

- Added CLI-owned `.local` mDNS publication for acknowledged LAN host gateways. The CLI
  derives only custom domains ending in `.local`, expands them across concrete exposed
  non-loopback addresses, and launches one `avahi-publish-address` process per pair only
  after gateway readiness. Owner-only state records deployment/definition ownership,
  PID start ticks, executable, exact name/address arguments, and the check report.
- Gateway stop, replacement, `down`, `cleanup`, and re-apply to loopback now terminate
  identity-verified publishers and remove their state. Missing `avahi-utils`, an
  unreachable Avahi daemon, or an immediately exiting publisher fails apply with an
  actionable diagnostic and cleans partial publication state.
- Added structured preflight results for Avahi tools/daemon reachability, usable LAN
  interfaces, VPN-style names and `/32`/`/128` host routes, best-effort firewalld/ufw/
  nftables visibility, the always-on link-boundary limitation, and post-publication
  local name resolution. CLI `up` and `status` show checks plus per-name/address
  published/failed state.
- Daemon deployment list/detail now expose optional `mdnsPublication`, derived from the
  CLI's owner-only state; the daemon does not manage Avahi processes. Router docs cover
  setup, check meanings, detection limits, same-link/guest/VPN/firewall/NSS constraints,
  reversal, and the unsupported public-internet boundary.
- Hermetic tests cover `.local` selection, loopback exclusion, state JSON/permissions,
  preflight report shaping, firewall result shaping through command injection,
  VPN/host-route classification, and daemon list/detail projection. Verification run:
  `cargo fmt --all --check`, all 18 daemon tests, 39 CLI unit tests plus daemon parity,
  and workspace/all-target clippy with `-D warnings` passed. The exact requested
  combined package test reached 39/40 CLI tests; only the pre-existing Unix-listener
  startup-cleanup test was blocked by the sandbox's `Operation not permitted`, so the
  CLI suite was re-run successfully with that one socket test filtered out.
- Live verification remains required on a Linux host with `avahi-utils`, Avahi and
  sockets available: confirm publication and local resolution, resolve/connect from a
  second same-LAN machine, observe firewall and VPN warnings on representative hosts,
  verify publisher cleanup on down/re-apply, and exercise the immediate-exit diagnostic
  with Avahi stopped.

### Part 2 reviewer verification (2026-07-16)

- Reviewer fixes after live testing (details in AGENTMISTAKES.md): spawn publishers
  with `-a -R` (argv[0] dispatch and reverse-PTR collision), advertise only
  non-VPN/non-bridge interface addresses while preflight warns on the rest, and
  include the publisher log tail in immediate-exit errors.
- `./scripts/check.sh`: PASSED end to end after the fixes.
- Live proof (radxa 192.168.1.10 publishing, poco-f1-nixos 192.168.1.167
  observing): `switchyard up` on the LAN-enabled routing-matrix fixture published
  `ui-1.routing-matrix.local -> 192.168.1.10` with the full check report (pass:
  avahi binary, avahi-daemon, lan-interface; warn: vpn-interface for tailscale0,
  firewall indeterminate under nftables, network-boundaries, name-resolution
  without nss-mdns). A unicast mDNS query from the second machine returned the
  correct A record and a curl through the published name returned 200 via the
  gateway. `switchyard down` stopped the owned publisher, removed the state file,
  and the name stopped answering.
- Environmental limitation observed and documented: this Wi-Fi network does not
  propagate the radxa host's outbound multicast (its own hostname `.local` record
  also never reaches other devices), so passive discovery from the second machine
  fails while unicast queries and TCP connects succeed — exactly the failure mode
  the preflight's `network-boundaries`/`firewall-udp-5353` warnings describe.

## Phase 7 LAN and private-network access — Part 3

- Added the explicit `GatewayExposure.publishTailscale` opt-in, omitted by default and
  valid only with acknowledged LAN exposure. Router validation exposes a stable error
  code and fixture for invalid combinations, with serialization round-trip coverage.
- Extended the adapter SDK with the `Publication` kind, `PublicationAdapter` contract,
  and structured private-network reachability/check records. The built-in
  `publication-tailscale` adapter validates its JSON Schema configuration, runs only
  `tailscale status --json` behind a command seam, requires a running backend and a
  gateway-exposed Tailscale IP, and derives the ts.net name, Tailscale IPs, and ports.
- CLI `up` now performs the advisory check after gateway readiness and atomically
  persists an owner-only deployment/version-bound record. `status` re-derives current
  tailnet reachability and reports stale/missing state without mutation; gateway stop,
  down, cleanup, and disabling the opt-in remove the record because no process or
  tailnet resource is owned.
- Daemon deployment list/detail project the guarded state as optional
  `tailscalePublication`. Router documentation covers checks, custom-domain resolution
  through MagicDNS split DNS or client-side resolution, and the strict boundary that
  Switchyard never runs Tailscale mutation commands or Funnel/public exposure.
- Hermetic adapter tests cover running, stopped, and missing-binary status through the
  command seam. `cargo fmt --all --check`, workspace/all-target clippy with
  `-D warnings`, and the requested package tests pass except for the pre-existing
  socket-bound CLI startup-cleanup test blocked by the sandbox (`Operation not
  permitted`); rerunning with only that test skipped passes 40 CLI tests plus all
  config, SDK, adapter, daemon, parity, and doc tests. Live two-machine tailnet
  verification remains with the reviewer.

### Part 3 reviewer verification (2026-07-16)

- `./scripts/check.sh`: PASSED end to end.
- Live tailnet proof (radxa publishing, poco-f1-nixos on the same tailnet):
  `switchyard up` with `publishTailscale: true` reported
  `radxa-dragon-q6a.warg-firefighter.ts.net via 100.106.209.100, fd7a:...` with all
  four checks passing. From the second machine over the tailnet, a request to the
  raw ts.net name failed closed with structured `route_not_found` (custom domains
  are not tailnet-resolvable by default, as documented), and a Host-resolved
  request to the custom domain through the tailscale address returned 200.
  `switchyard down` removed the owner-only publication state file.

### Part 4 reviewer verification (2026-07-16)

- Reviewer fix: import now pre-checks every destination path before writing any
  file, so a `bundle_write_conflict` can no longer leave a partially imported
  bundle behind.
- `./scripts/check.sh`: PASSED end to end.
- Live CLI proof: `bundle export` of routing-matrix produced a deterministic
  envelope with 8 source paths replaced by required local inputs and
  `local_path_replaced` warnings; `bundle import` into a clean directory
  reported compatibility ok, scaffolded the inputs, validated, and printed the
  full mutation preview with `Conflicts: none`. Importing into this repository
  (where fixtures already exist) reported `name_conflict` for the existing
  generated routing-matrix and a genuine `port_conflict`: jas-base also claims
  `127.0.0.1:10081`. A tampered bundle was rejected with `bundle_hash_mismatch`
  naming both hashes.

## Phase 7 reliability — Part 5: lifecycle hooks resolved by removal

- The reserved per-service `hooks` field (`prepare`, `postReady`, `stop`,
  `cleanup`) was removed from the planner schema instead of gaining an executor:
  it was never read by any runtime path, no fixture used it, and the real
  initialization mechanism (`execution: script` with `lifecycle: task`, gated via
  `dependsOn: completed_successfully`) already carries logs, status, ownership,
  and recovery like any service. Declaring `hooks` now fails closed with an
  unknown-field error naming the field
  (`declared_lifecycle_hooks_are_rejected_not_silently_ignored`); the supported
  pattern and the removal rationale are documented in `docs/adapters.md`.
- Reviewer verification (2026-07-16): `./scripts/check.sh` PASSED end to end,
  and the live `examples/jas-base/smoke.sh` PASSED, proving task-lifecycle
  database initialization, live group switching, persistence, and
  ownership-scoped cleanup all still work after the removal.

## Phase 7 reliability — Part 6: upgrade and heavy reliability tests

- Added fast SQLite upgrade-matrix tests for schema versions 1, 2, and 3 in
  `switchyard-state`. The fixtures are built through the actual historical DDL
  embedded in `src/migrations` rather than committed binary databases; this keeps
  the rows readable in review, avoids SQLite-file portability churn, and still
  exercises the production migration and backup path. Each version inserts
  representative values into every table that existed at that version, verifies
  current-schema migration to version 4, asserts row values, checks the
  pre-migration backup, and runs `PRAGMA integrity_check` plus foreign-key checks.
- Added a failed-migration recovery test that uses a test-only migration list to
  create the same pre-migration backup production would create, leaves the
  original version-2 database intact after a transaction failure, restores the
  backup to a new path, and verifies the normal current migration succeeds.
- Added schema compatibility goldens: router-config pins a Phase-7 host-router
  JSON fixture with `exposure` LAN/Tailscale fields; switchyard-planner pins
  copied compat deployments for `examples/routing-matrix` and `examples/jas-base`
  with expected definition/resource hashes and deterministic generated router
  configs.
- Added ignored heavy reliability tests and `scripts/reliability.sh`. The suite
  covers router-core reload storms, TCP and Pingora HTTP reload storms under
  concurrent clients, Linux fd/RSS leak sampling, an HTTP soak with health-check
  flapping, and in-process daemon API concurrency with global heavy-operation
  limiting plus per-deployment lock contention. Socket-bound tests are compiled
  here but must be executed by the reviewer on a host that permits loopback
  binding.

### Part 6 reviewer verification (2026-07-16)

- Reviewer fixes, all in the new tests (no product defects found; details in
  AGENTMISTAKES.md): the router-core storm's version-monotonicity check is now
  per-observer-thread (the global fetch_max compare raced benignly across
  threads); the TCP storm flips targets under `Pin`, where every client exchange
  must complete intact (asserting zero incomplete exchanges under `Close` denies
  the policy's defined behavior; `Close` stays covered by its dedicated test and
  the pre-storm sequence, and the pre-storm sequence now asserts that a pinned
  connection survives a later `Close` reload, matching
  `pin_policy_survives_later_route_changes`); the HTTP test upstream stub handles
  connections concurrently on blocking sockets and tolerates dirty disconnects
  (single-threaded serial handling with inherited nonblocking sockets collapsed
  under storm load); the storm providers declare no health checks and the soak
  uses a generous 2s health timeout (50ms timeouts manufactured fail-closed 503s
  under load); soak flap correlation uses timestamped windows with recovery slack
  instead of a boolean read after the response; fd-leak assertions compare
  growth (`end <= warmup`) instead of exact equality.
- `./scripts/reliability.sh` (defaults): PASSED — router-core storm 30s,
  router-tcp storm+leak 30s, HTTP storm+leak 31s, HTTP soak+flap 30s, daemon
  high-concurrency 2s.
- 120-second HTTP soak: PASSED with zero unexpected errors, all
  provider_unhealthy rejections inside flap windows, and no fd/RSS growth.
- `./scripts/check.sh`: PASSED end to end (fast suite runtime unchanged; all
  heavy tests are `#[ignore]`).

## Phase 7 reliability — Part 7: release packaging and diagnostics

- Added native host release assembly in `scripts/release.sh`: Rust release builds for
  `switchyard`, `switchyard-daemon`, and `switchyard-router`; a clean Node.js 24 GUI
  build; a version derived from the workspace version plus `git describe`; a platform
  tarball; generated release notes; mandatory SHA-256 checksums; and optional SSH
  signatures in the fixed `switchyard-release` namespace. No cross-compilation or
  host-dependent GPG tooling is used.
- The archive contains ownership-aware prefix installation and uninstallation. Upgrade
  replacement and deletion require the prior installed-files manifest plus matching
  per-file hashes, non-Switchyard paths are never overwritten, the default prefix is
  user-writable `~/.local`, and the daemon discovers the GUI installed below that
  prefix. `scripts/release-smoke.sh` provides the fast no-Docker artifact checksum,
  extraction, install, executable, uninstall, and clean-prefix proof.
- Added `switchyard diagnostics <deployment.yaml> [--output <path>]`. Its one-file JSON
  report gathers host/tool/runtime versions, planner validation and definition identity,
  daemon detail or deployment-scoped generated/runtime state, host-gateway logs, live
  router events when authenticated locally, and best-effort read-only Docker ownership
  observations. Missing external/runtime services remain structured unavailable data.
- Redaction is recursive and happens before the owner-only file write. Diagnostics and
  daemon event capture now share the planner's line convention; diagnostics also reuse
  the portable-bundle credential-key heuristic, replace process environment and known
  router/daemon token values, and never resolve overlay secret references. Unit tests
  plant credential fields, embedded environment values, router/daemon tokens, and an
  authorization log line and assert none survive while redaction markers do.
- `docs/release.md` documents build, checksum/signature verification, install,
  ownership-checked upgrade/uninstall, the authoritative upgrade/recovery pointer, and
  diagnostics contents and guarantees. Full release and GUI builds require reviewer
  execution where Cargo/npm network or caches are available; verification status is
  recorded below.

### Part 7 sandbox verification (2026-07-16)

- `cargo fmt --all --check`: PASSED.
- `cargo clippy --workspace --all-targets -- -D warnings`: PASSED.
- `bash -n scripts/release.sh scripts/release-smoke.sh`: PASSED; the packaged install
  and uninstall assets also pass `bash -n`.
- `cargo test -p switchyard-cli -p switchyard-planner`: the new diagnostics/parser
  tests and all planner tests pass. The unfiltered command reaches the pre-existing
  `host_runtime::tests::failed_startup_cleanup_allows_a_clean_retry` sandbox failure
  (`Operation not permitted` while exercising process signaling); rerunning with that
  one host-permission test skipped passes 43 CLI unit tests, daemon parity, all 26
  planner unit/integration tests, and planner doc tests.
- A real `target/debug/switchyard diagnostics` run against `routing-matrix` wrote an
  owner-only (`0600`) JSON report, captured generated/runtime/log state, and represented
  unavailable Docker access as best-effort structured data. A synthetic package using
  the built executables passed fresh install, manifest-owned upgrade with obsolete GUI
  removal, executable placement, hash-checked uninstall, and clean-prefix assertions.
- `scripts/release.sh`, signed/unsigned artifact generation, and the full
  `scripts/release-smoke.sh` remain for reviewer execution because the requested clean
  `npm ci`/release build may require network access unavailable in this sandbox.

### Part 7 reviewer verification (2026-07-16)

- Reviewer fix: the diagnostics redactor now scrubs only the values of
  credential-looking process environment variable names (shared planner
  heuristic) plus the daemon discovery and router tokens, instead of every
  process environment value — replacing benign values like `$HOME` erased every
  absolute path from the report (proven on a live bundle), and a variable
  holding a common short word would have mangled arbitrary text.
  `docs/release.md` states the scoped guarantee.
- `./scripts/check.sh`: PASSED end to end.
- `./scripts/release.sh`: PASSED unsigned and signed (throwaway ed25519 key);
  `ssh-keygen -Y verify` accepted `SHA256SUMS.sig` and `sha256sum -c` passed.
- `./scripts/release-smoke.sh`: PASSED (checksum verification, temp-prefix
  install, installed binaries invoke, ownership-checked uninstall, clean
  prefix).
- Live `switchyard diagnostics` against the running routing-matrix deployment
  with a planted `SWITCHYARD_ROUTER_TOKEN`: token absent from the report,
  output mode 0600, all sections present, paths still readable after the
  scoped-redaction fix.
- `/dist/` added to `.gitignore` so release artifacts cannot be committed.

## Phase 7 security and support policies — Part 8

- Audited host listeners, browser-extension permissions, router and daemon
  administration channels, host/mDNS/Tailscale state, Docker ownership and cleanup,
  overlay/script/bundle/diagnostics file paths, secret references and redaction, and
  release archive inputs against DESIGN.md section 8.
- Published `docs/security-review.md` with concrete implementation/test evidence,
  adversarial checks, and nine stable findings. Severity count: critical 0, high 4,
  medium 4, low 0, informational 1. No product code was changed; remediation remains for
  reviewer triage.
- Published `docs/support-policy.md` covering alpha configuration and state schemas,
  deliberate compatibility goldens, the one-minor/90-day parsing and API overlap window,
  additive `/api/v1` evolution, same-release CLI/daemon support, ordered forward-only
  SQLite migration/backups, newer-schema refusal, and backup-only downgrade.
- Linked both policies from `docs/development.md` and the repository README. The Phase 7
  implementation-plan checkboxes remain untouched for reviewer verification.
- Part 8 verification: `cargo fmt --all --check` passed; every new relative Markdown
  link target was inspected and exists; `git diff --check` passed.

### Part 8 reviewer verification and Phase 7 exit gate (2026-07-16)

- Security review (`docs/security-review.md`): the reviewer independently
  verified the four high findings against the code. SR-2 (unowned Compose-project
  orphans deletable via `up --remove-orphans` without the ownership proof that
  `down`/`cleanup` already required) was confirmed and fixed during sign-off:
  `DockerRuntime::up` now runs the same `discover_compose_project` +
  `verify_ownership` preflight, proven by
  `up_refuses_when_the_compose_project_contains_an_unowned_container`. SR-3, SR-4,
  and SR-7 (high) and the four mediums are accurate and recorded as an explicit
  unchecked remediation item in Phase 7 — their fixes need deliberate design
  decisions, not rushed patches. Support/deprecation policies published in
  `docs/support-policy.md`.
- Exit gate evidence:
  - LAN sharing explicit/inspectable/reversible/secure-by-default: Parts 1–3
    live proofs (opt-in + acknowledgement, exposure warnings and status/API
    surfacing, remote reachability and revert-to-loopback closure verified from
    a second machine, mDNS withdrawal on down, advisory-only tailnet
    publication).
  - Bundle round-trip across supported machines: routing-matrix exported here,
    imported and validated with the *installed release binary* on a second
    aarch64 Linux machine (poco-f1-nixos, NixOS): checksum verified,
    `Compatibility: ok`, required-local-inputs scaffolded, definition validates;
    sanitization tests prove no secrets/absolute paths embedded.
  - Release artifacts: `release-smoke.sh` locally plus on the second machine a
    full checksum-verify → install → run → reinstall (upgrade) → uninstall
    sequence ending with zero files in the prefix; an accidental default-prefix
    install was also fully removed by the manifest-driven uninstall, a
    real-world ownership-cleanup proof. Recovery procedures remain covered by
    the tested `docs/upgrade-recovery.md` paths (pre-migration backups,
    newer-schema refusal, SQLite delete/restore rebuild).
- Phase 7 remains open only on the tracked security-remediation item; every
  other Phase 7 task and the exit gate are complete.

## 2026-07-16 — Cleaned-up deployment GUI state

- The GUI now interprets an empty observed-resource set plus the
  `observed_resources_missing` reconciliation diagnostic as a stopped/cleaned-up
  deployment, instead of presenting it as reconciled or filling instance cards with
  ambiguous `state unknown` labels.
- Stopped deployments show the reconciliation reason and a prominent `Run Up` action;
  runtime-only actions are disabled, runtime domains and active routes are explicitly
  unavailable, and the interactive patch bay is replaced by a stopped-state message.
- The selected deployment rail entry and inspector project the same state, and command
  completion refreshes both deployment summary and detail so a successful Up can clear
  the stopped presentation without a page reload.
- Verification: all 8 `App.test.tsx` GUI tests pass, including the new cleaned-up-state
  regression; the production TypeScript/Vite build passes; oxlint completes with only
  the four pre-existing React hook dependency warnings.

## 2026-07-16 — GUI Up router credential propagation

- Fixed daemon-backed `Up` operations failing with
  `SWITCHYARD_ROUTER_TOKEN must be set when starting routers`: the real CLI backend now
  receives a persistent project router credential and injects it into daemon-spawned
  commands alongside the recursion guard.
- The daemon loads or creates `.switchyard/router-token` as an owner-only regular file.
  It reuses the value across daemon restarts, accepts an environment value only when
  seeding a missing file or matching the existing value, and does not expose the token
  through its API, GUI, or debug output.
- Native live binding now receives the same managed credential explicitly instead of
  reading ambient process environment, keeping Up and later route changes consistent.
- Verification: all 6 daemon unit tests and 14 daemon API tests pass (the opt-in
  reliability test remains ignored), daemon doc tests pass, and the CLI daemon-parity
  integration test passes. Workspace/all-target/all-feature clippy passes with warnings
  denied, and the rebuilt CLI succeeds. New tests cover child credential injection,
  persistence, owner-only permissions, and mismatched-override refusal.

## 2026-07-16 — `switchyard init` reference-template scaffolding

- New `switchyard init <directory> [--name <project-name>] [--force]` command scaffolds
  a base project from templates embedded in the binary: a minimal but real
  `deployment.yaml` (one nginx container service with provides/probe/publish plus
  commented sources/consumer examples), `overlays/dev.yaml`, `README.md` with the
  standard command sequence, and a `.gitignore` covering `.switchyard/`.
- Project names default to the sanitized directory basename (DNS-label rules) and can
  be overridden with `--name`; existing scaffold files are enumerated and refused
  without `--force`. After writing, the command validates the generated deployment
  through the same `load_and_plan` path as `switchyard validate`, so the template
  cannot silently rot.
- Verification: all 47 `switchyard-cli` unit tests plus the daemon-parity integration
  test pass locally; workspace clippy with `-D warnings` passes. End-to-end proof on
  this machine: `init` → `validate` → `plan` (dev overlay origin attributed) →
  `up` (container reaches healthy under Docker) → `down` (zero leftover resources),
  plus conflict refusal on re-run and `--force` overwrite.

## 2026-07-16 — Interactive `switchyard init`

- `switchyard init` now starts a guided initializer when no directory is supplied. It
  asks for a valid deployment name and an optional destination (defaulting to a new
  folder named after the project), then creates and validates the complete reference
  template. The existing directory-based command remains available for automation.

## 2026-07-17 — TUI control plane Phase A: architecture and contracts

- Documented in `DESIGN.md`: the retained-Ratatui-TUI decision and the shared
  `switchyard-ops` operations/projection crate boundary; a retroactive device model
  (project-scoped SSH records, implicit `local`, placement is validated never ignored,
  global config deferred); the `switchyard-profiles.yaml` source-local startup-profile
  manifest with explicit import, content-hash trust, and project-over-source
  precedence; the final user-facing terminology table (the handwritten
  "project / project instance" naming was rejected); and the scoped limited remote
  container execution cut (Docker SSH transport, local router, published addresses,
  labeled remote resources, eligibility validation, no silent orphaning).
- Declared the React GUI a supported secondary monitoring/operations client in
  `docs/gui.md` and `docs/support-policy.md`; the new authoring workflows are TUI-only
  with no implicit parity schedule.
- Appended the TUI control-plane milestone (Phases A–E) to `IMPLEMENTATION_PLAN.md`.
- Verification: documentation-only diff (five permitted files), fixed the manifest
  example's `provides` shape to the canonical capability map during review.
- Phase D readiness proven early: the LAN device `poco-f1-nixos` (192.168.1.167,
  aarch64) accepts key-based SSH and `docker -H ssh://akhil@poco-f1-nixos` reaches its
  Docker 28.5.1 daemon.

## 2026-07-18 — TUI control plane Phase B: guided configuration

- `switchyard-ops` crate extracted from the TUI (execution, run scripts, projections)
  with zero behavior change; TUI now consumes it (commit cc3bc88).
- Source-local startup-profile domain per DESIGN.md: `switchyard-profiles.yaml`
  discovery (read-only, planner-validated), state schema v6 `imported_profiles` with
  canonical content hashes, trust/shadowing projections (commit 016a408).
- New Profiles TUI tab: origin/trust/services table, per-source manifest diagnostics,
  inspector, explicit full-definition review before import, changed-hash re-review,
  import removal (commit 56dbfb1). Pty-verified end to end: discovery → review →
  import persisted in SQLite → manifest edit flips the row to "changed — review".
- Guided instance creation: planner `Instance.device` (only `local` validates, absent
  means local), ops `preview_instance`/`create_instance` (trust gate, one-time block
  materialization pruned of nulls/empties, source declaration, parameter emission,
  validate-then-replace), TUI form for profile/checkout/name/device/parameters with
  plan-backed preview and field-attached diagnostics before the write.
- Verification: full workspace tests including planner compat goldens, clippy
  `-D warnings`, fmt, and pty-driven flows creating a real instance from an imported
  profile (`demo1` with `device: local` and a clean materialized block).

## 2026-07-18 — TUI control plane Phase C: routing workflow

- Connections tab: consumer×slot route matrix with compatible-group drafting,
  old/new provider preview via `plan_with_binding`, atomic apply through the
  existing `switchyard bind` operation, and route version/transition/error status
  projected from `router_bindings` and `route_history` (commit 6533a16).
- Review fixes: stored deployments whose authored definition lives outside the
  project now fall back to `.switchyard/generated/<name>/resolved-deployment.yaml`
  so the matrix and bind work at the dogfooding repo root; connection-matrix load
  errors are surfaced in the view instead of being swallowed into an empty state;
  `[`/`]` deployment switching now also works in the Connections view.
- Exit gate verified end to end on the live routing-matrix fixture: driving the
  real TUI over a pty, backend-1 was switched feature-services → main-services;
  the preview listed exactly the four changing providers (audit shared, unchanged);
  live identity traffic confirmed all four providers flipped atomically; the
  docker container-ID set was identical before and after (no restarts); backend-2
  stayed on its own group. State restored to feature-services afterwards.
- Verification: full workspace tests, clippy `-D warnings`, fmt.

## 2026-07-18 — Phase D remote execution verified on real devices

- Fixed during verification: a deployment whose instances are all remote produced an
  empty local Compose project and `up` failed with "no service selected"; the plan
  now carries `local_service_count` and the runtime skips the empty local project in
  up/down/cleanup/logs.
- Real-device proof on `poco-f1-nixos` (aarch64 LAN device over SSH): eligibility
  gate, remote compose project, healthy device-labeled container, device-aware
  status, and clean `down` with zero leftover containers/networks. The follow-up
  network-label fix (9791e8c) was required by this run. Bridged container networking
  is broken in that device's vendor kernel (host↔container traffic never passes;
  its resident Home Assistant container runs host-networked), so routed traffic
  cannot terminate there; this is a device limitation, not a Switchyard defect.
- Full routed proof over a real SSH device at the local machine's LAN address:
  local consumer with fixed 127.0.0.1:8080 + sidecar router + remote provider
  started via `DOCKER_HOST=ssh://` — traffic reached nginx on the provider through
  `192.168.1.10:80` per the published-address design, and teardown left nothing.
- Operational note: a registered device's `host` is used verbatim as the router
  upstream host, so it must be an address resolvable and reachable from inside
  containers (LAN IP preferred over `localhost` or mDNS names).

## 2026-07-18 — Phase D part 2: TUI remote placement visibility

- Device checks now persist an explicit remote-container eligibility outcome after an
  SSH probe and a Docker-server version query over Docker's SSH transport. Legacy
  SSH-only successful checks remain visibly unchecked until rerun.
- The Devices view presents eligibility and its concrete failure; the guided instance
  selector presents local and every registered device with persisted eligibility while
  leaving workload compatibility to the existing planner preview.
- Instance/service projections and the TUI now show `local` or the registered placement
  device. Reconciliation device-unreachable diagnostics replace misleading stopped or
  missing-resource presentation for affected remote manifest rows.
- Documentation now states the limited cut: container-only provider instances with
  published ports, a locally reachable device host, and no remote consumers, process
  adapters, routers, or cross-device sidecars.
- Verification: all changed-crate tests pass (CLI 57, ops 24, state 16, TUI 38, plus
  daemon parity and doc tests); workspace clippy with `-D warnings` and fmt pass. The
  unrestricted workspace test command is sandbox-blocked by `Operation not permitted`
  in the known socket-binding router tests and one CLI host-runtime socket test.

## 2026-07-18 — Phase D complete: TUI device eligibility and placement

- Device checks now probe SSH then Docker over the SSH transport, persisting
  eligible/ineligible with the Docker version (state schema v7; legacy SSH-only
  results demote to unchecked). Devices tab shows eligibility in words with the
  concrete failure reason; the instance form labels remote devices and states the
  cut's requirements; instance/service rows show true placement and unreachable
  devices are rendered explicitly.
- Live verification: `switchyard device check poco` reports "eligible for remote
  container execution (docker 28.5.1)" against the real LAN device and the TUI
  Devices tab renders the same over a pty.

## 2026-07-18 — TUI control plane Phase E complete: milestone closed

- Expanded the `switchyard init` AI skill to 198 lines implementing every section-7
  requirement: ordered inspection, read-only repository analysis, project and
  source-local profile authoring with the import trust boundary, complete
  groups/bindings, device placement rules for the limited remote cut, the
  validate/plan loop, and the explicit cannot-safely-configure failure mode. The
  init test now pins the skill's key content and the scaffold still validates.
- Documentation pass: docs/tui.md navigation refresh, state schema v7 noted in
  release/upgrade-recovery docs with remote-device recovery guidance, GUI scope
  confirmed already documented, IMPLEMENTATION_PLAN.md Phase B–E items checked,
  new_tui_features.md marked implemented through Phase E.
- Final verification on the closing tree: scripts/check.sh fully green (fmt,
  clippy all-features -D warnings, workspace tests, rustdoc -D warnings); pty
  sweep rendered all six TUI tabs; release assembly produced
  dist/switchyard-0.1.0+2c6a3df-dirty-linux-aarch64.tar.gz with verified
  SHA256SUMS.

## 2026-07-18 — TUI local device visibility

- The Devices table now includes `this device` as its first, always-available option,
  with non-applicable SSH metadata rendered as `-`, matching the implicit `local`
  placement already used by the planner and instance form.
- Selection includes both `this device` and registered SSH devices; SSH check and
  removal are guarded for the implicit entry and explain why those actions do not
  apply.

## 2026-07-18 — TUI checklist documentation reconciliation

- Reconciled the duplicated Phase A–E and acceptance checklists in
  `docs/new_tui_features.md` with the completed authoritative entries in
  `IMPLEMENTATION_PLAN.md`. The feature document's status already said implemented,
  but its individual checkboxes had accidentally remained unchecked.
- Verification: `docs/new_tui_features.md` contains no remaining unchecked checklist
  entries, and the change is documentation-only.

## 2026-07-18 — Connections view navigation consistency

- Left/Right now switch views from Connections just as they do from every other TUI
  view, preventing an arrow-key user from becoming trapped after entering the tab.
- Compatible provider-group drafting remains available on `h`/`l`; all inline key
  hints and TUI documentation now describe the non-conflicting bindings.
- Verification: all 39 TUI library tests, strict TUI clippy, formatting, and diff
  checks pass.

## 2026-07-18 — AppCUI TUI rewrite (branch tui-appcui-rewrite)

- Design accepted: docs/tui-appcui-design.md (7-tab single-window AppCUI shell,
  F-key action scheme, re-exec terminal handoff). Toolchain bumped to Rust 1.88
  for appcui 0.4.13; new clippy lints fixed workspace-wide.
- Part 1 (shell + Home + handoff loop) and part 2 (Code tab: register, clone with
  handoff, worktrees, safe remove) implemented by Codex, reviewed, verified by
  pty-driven smoke runs (register + local clone handoff end to end). Review fixes:
  re-exec handoff (input-thread leak), timer-based Code-tab restore, F-key
  bindings, no SearchBar, human-readable inspection age.
- Parts 3–6 briefs ready in .codex-refs/briefs/ (Profiles, Instances,
  Connections, Devices+Operations), part 7 hardening to follow.

## 2026-07-19 — AppCUI TUI rewrite complete (parts 3–7)

- Parts 3–6 (Profiles, Instances + wizard + background operations, Connections
  route matrix, Devices + Operations) implemented by Codex and reviewed with
  per-part pty smoke passes; notable review fixes: Markdown code-fence hang
  (data-driven text moved to read-only TextArea), F-key action scheme (list
  controls consume Insert/Space/letters), re-exec terminal handoff.
- Part 7: shell.rs refactored 1,897 → 883 lines (per-tab impl blocks moved into
  tab modules), UX fixes (first-row auto-select, empty-state visibility, add
  dialog focus), scripts/tui-smoke.py (14 pty assertions), docs/tui.md rewrite.
- Review fixes on part 7: deterministic preview-dialog focus + Enter accept;
  refreshes no longer hold the mutation gate.
- Verification: cargo fmt, workspace clippy -D warnings, workspace tests,
  rustdoc -D warnings, cargo audit (new scoped quick-xml exception documented),
  tui-smoke 14/14 in three consecutive runs. time bumped to 0.3.53 and
  RUSTSEC-2026-0009 exception removed now that the toolchain is Rust 1.88.
- Not yet re-verified on this branch: the Docker-based routing acceptance
  fixture and real LAN-device remote execution through the new TUI (ops layer
  unchanged; CLI-level coverage from the previous milestone still applies).


## 2026-07-25 web UI Part 12 — Project Home and onboarding

- Added Home to the top-level rail and Left/Right navigation cycle. Projects whose deployment
  list is empty move from the existing initial Deployments state to Home after that list loads;
  projects with one or more deployments retain the existing Deployments landing behavior.
- Added an accessible five-item ordered setup checklist and a keyboard-reachable recommended
  action that changes the real application view: Sources, Profiles, the new-deployment builder,
  Deployments for startup/connection work, or Operations after completion.
- Checklist signals are all client-visible API data: source uses non-empty `GET /sources`; profile
  uses a non-shadowed `trusted` or `imported` profile; instance uses `spec.instances` from the
  validated authored-definition preview (falling back explicitly to the applied snapshot only when
  that preview omits the spec); startup uses a non-null deployment-summary `appliedAt`; connection
  uses `consumedSlots`/`resolvedGroups` from `connectionModel.ts` and requires one consumer to have
  a provider for every consumed slot.
- Caveats: the profile API has no separate durable “selected profile” record, so that step means a
  usable trusted/imported profile is available for the guided builder. The deployment API has no
  structured project-wide running boolean, so startup completion uses the durable apply timestamp
  and does not infer runtime state from resource-state strings. When source/profile/deployment data
  cannot be loaded, or an authored spec cannot be obtained, the affected checklist step renders
  `Unavailable / unknown` rather than incomplete. An applied-snapshot fallback is named as an
  incomplete signal instead of being presented as current authored state.
- Added project-wide problem categories from real fields: source `inspection.unknownCode`; profile
  `sourceErrors`; deployment reconciliation diagnostics; device reachability and eligibility plus
  the server-provided eligibility reason; operations with `status === failed`; and consumers whose
  authored consumed slots lack providers, derived through `connectionModel.ts`. Load/spec failures
  render each affected category as unavailable or incomplete while preserving any known problems.
- Added five App tests for empty-project Home landing, unchanged non-empty landing, all checklist
  completion transitions, recommended-action navigation into the builder, and aggregation across
  source/profile/operation categories.
- Verification: `npx tsc -b` passed with no output; `npm run lint` exited zero with exactly the four
  pre-existing `react-hooks(exhaustive-deps)` warnings (two in `App.tsx`, two in
  `DeploymentBuilder.tsx`); `npm test` passed all 46 tests across three files.
## 2026-07-25 — Web UI Part 13 Git clone with in-browser credentials

- Added authenticated `POST /api/v1/sources/clone` as a normal heavy `clone`
  operation with pending/running/terminal state, SSE timeline messages, cancellation
  observation, source-scoped mutation locking, and managed source registration. The
  first attempt preserves the existing non-interactive Git posture:
  `GIT_TERMINAL_PROMPT=0`, ambient credential helpers, SSH agent/config, and
  `ssh -o BatchMode=yes`; URL credentials, malformed refs, and invalid identity paths
  remain rejected before Git runs.
- HTTPS authentication failures return only a secret-free credentials challenge. The
  browser retries with an uncontrolled password/token form that is reset immediately
  after request construction and never renders the submitted value. The daemon DTO is
  deserialize-only; credentials live only in request/task values and the Git/helper
  child environment for one attempt. A private owner-only temp directory holds an
  owner-only askpass script containing no secret material and is removed on return;
  configured credential helpers are disabled for that submitted-credential retry so
  Git cannot ask one to persist the value. No credential reaches SQLite, `.switchyard/`, operation results/errors, SSE events,
  logs, or API responses. Clone events are fixed lifecycle messages rather than raw Git
  output because the general planner line redactor cannot guarantee removal of an
  arbitrary submitted token.
- Unknown SSH hosts produce a challenge containing the scanned SHA-256 fingerprint.
  Approval is explicit in the UI; retry rescans and requires the exact host/fingerprint,
  then writes only the public key to a temporary mode-0600 isolated `known_hosts` file
  and uses `StrictHostKeyChecking=yes`. No `no`/`accept-new` shortcut is used.
- Added a new `CloneSource.test.tsx` for credential prompting, explicit host-key
  approval, and no secret rendering; daemon coverage checks clone registration plus
  absence of a submitted sentinel from operation JSON, SSE payloads, and SQLite, and
  retains the embedded-URL credential rejection. Documentation records the endpoint,
  loopback bearer posture, memory/disk lifetimes, redaction boundary, and host approval.
- Caveats: `ssh-keyscan` supplies an unauthenticated candidate key, so the UI explicitly
  tells the user to verify its fingerprint through a trusted channel. The clone path
  deliberately does not expose raw Git progress text; progress is the normal operation
  lifecycle plus fixed start/completion messages. Final full-workspace and web
  verification output is recorded in the implementation report.

## 2026-07-31 — V2 Part 2 addresses on groups and instances

- Replaced `spec.uiRoutes` with one optional singular `address` on each group and instance.
  Planner validation accepts plausible DNS hostnames, compares claims case-insensitively with a
  trailing-dot normalization, rejects duplicates across both object kinds, and retains the
  loopback-only host-router default without changing `router-config` or its LAN acknowledgement.
- Extended the existing host-router generation seam. An addressed group resolves its bare name to
  the sole member providing `ui`; an addressed instance resolves its single reachable service. The
  planner merges generated `custom_domain` destinations into authored listeners, derives Origin
  browser routes from the UI's explicit-header routes, preserves identical authored entries, and
  rejects same-domain/different-slot or same-origin/different-provider conflicts. Group-address
  backend checks still emit `BackendGroupInvariant` at `spec.groups.<name>.address`, retain the
  "duplicate the backend instance" guidance, and reject groups with zero or multiple UI candidates.
- Extended `switchyard migrate` as a second transform in the Part 1 seam. It converts a legacy
  `uiRoutes` origin to the downstream group's address, adds the named UI and backend to that group,
  removes only custom-domain and Origin entries that can be proved redundant from the UI provider
  and explicit-header template, preserves unrelated authored routes sharing the same Origin,
  validates the Part 1 provider-map transform before adding the new capabilities, writes only after
  the complete migrated bundle loads, and is byte-idempotent on a second run. Multiple legacy routes
  naming one downstream group are refused because one object may carry only one address.
- Hand-migrated both examples and compat fixtures without reserialization. `jas-base` now models its
  two complete addressed combinations directly. `routing-matrix` keeps its downstream service groups
  unchanged and gives all three peer UIs instance addresses, preserving the documented ability to
  switch `backend-1` between complete downstream groups while generating the same three domains and
  Origin routes. Compat hashes are pinned from full byte-identical planner outputs across two runs.
- Updated ops projections and compatibility expectations, CLI resolved-state handling, web snapshot
  types and routing help, acceptance/router documentation, and removed DESIGN.md's unimplemented
  `ingress:` block. No router-pingora, router-core, router-tcp, router-config, vision, or roadmap
  files were changed.
- Verification: `cargo fmt --all -- --check` passed with no output; workspace clippy passed under
  `-D warnings`; `cargo test --workspace --all-features` passed 303 tests with 0 failed and 5
  ignored, up from the Part 1 baseline of 292. Address generation, each group-resolution error,
  normalized uniqueness, both authored-router conflict forms, identical merge, the backend-group
  invariant, migration round-trip/idempotence, and multi-route refusal now have independently
  localizing coverage. Web `npx tsc -b` passed with no output; lint exited zero with exactly the four
  pre-existing exhaustive-dependency warnings; all 48 web tests passed.

## V2 roadmap expansion — Parts 2b through 2e, and Part 4 deferred (planning only)

No code changed. This entry records design decisions taken while writing
`docs/vision/sample-config.md`, an annotated end-to-end deployment added to the vision
directory. Each claim below was checked against the current build with the CLI rather than
read off the schema.

- **Part 2b — a group shares one localhost.** Omitting `provides:`/`consumes:` validates
  clean but generates **zero router sidecars** (verified: 12 sidecars with them, 0 without),
  so the declarations are not documentation of the routing, they are the routing. That makes
  slots the price of entry rather than an override, which contradicts ABOUT.md's "auto
  routing magically happens". Planned: derive routing from the ports a group's members
  listen on, discovered from `publish:`, `probe:`, and image metadata. Open: static
  discovery versus runtime observation.
- **Part 2c — repositories declared once.** Sources currently repeat the repository path and
  their git fields are inert: `{ type: worktree, repository, ref }` validates, but nothing in
  the planner calls `create_worktree`, so directories must pre-exist. Planned: a
  `repositories:` section (`url:` managed, `clone:` adopted), sources always
  `{ repository, ref, path }`, and `up` creating what is missing. Notes that
  `validate_containment` must be re-scoped from `.switchyard/worktrees` to the project
  directory, and that migration must produce the adopted form so a directory Switchyard was
  reading never becomes one it manages. Unresolved: plain-path sources used as build context
  (`{ path: . }`, `{ path: ../.. }`) are not worktrees.
- **Part 2d — `bindings:` and `routes:` deleted.** Removing `bindings:` from a
  single-group-per-consumer deployment produces four `IncompleteGroup` diagnostics, every one
  already answered by membership. The initial audit proposed allowing receiver-only instances
  in several groups, but the later product decision removed that unmeasurable distinction:
  every instance belongs to at most one group. `routes:` goes in the same pass.
- **Part 2e — external instances.** `{ name, external, ports }` for things already running
  outside Switchyard. `ports:` takes integers and inclusive range strings, mapping
  port-for-port; ranges expand before the Part 2a collision check so a clash names the port
  rather than the range.
- **Part 4 deferred to V3.** A run action is `$SHELL -c` in the project directory
  (`run-actions/src/lib.rs:403`), and it cannot reach the deployment it is about: `publish:`
  emits `127.0.0.1::8080` so host ports are ephemeral, the group localhost exists only inside
  sidecars, and the promised environment is absent (`run-actions` never calls `.env()`;
  `SWITCHYARD_BUNDLE` appears nowhere in the workspace). Landing the flat map first would
  migrate everyone onto a shape that then changes again. Candidates recorded: export group
  addresses, `switchyard exec <instance> -- <cmd>`, and a group-scoped script form.
- One-time exception noted in Part 2a stands; no other vision file was edited. Verification
  for this entry was CLI experiments against scratch deployments only — no test suite was run,
  because no code changed.

## 2026-07-31 — V2 Part 2b automatic shared localhost

- Replaced the planned static port-discovery design with namespace-local transparent TCP
  interception. A sidecar captures arbitrary IPv4 and IPv6 loopback connections, recovers the
  original destination port, and routes that same port to active group members in authored order.
  `publish`, probes, image `EXPOSE`, `provides`, and `consumes` are no longer prerequisites.
- Added receiver-side interception. Deployment-network connections are redirected back to the
  selected member's loopback, so a receiver bound only to `127.0.0.1` remains reachable without
  widening its application bind address. Marked router sockets bypass interception without
  recursion; a sender's own listeners participate at their authored group position.
- Changed generated local topology to one namespace anchor per routed instance. Every expanded
  service joins that namespace and its DNS alias points to the anchor. Transparent sidecars receive
  only `NET_ADMIN`, drop all other capabilities, use `no-new-privileges`, and install ownership-
  scoped IPv4/IPv6 rules that are idempotent across sidecar restarts and removed on clean shutdown.
- Added passive runtime listener exchange on a second reserved internal port. Sidecars read their
  namespace TCP listener tables, callers query all members concurrently, warn when more than one
  active member owns the original port, and select the first listed without opening probe
  connections to losing application ports. Direct connection attempts remain a startup-race
  fallback.
- Added `groups.<name>.disabled`. Disabled names must resolve to group members, are local rather
  than inherited through `extends`, disappear from routing/collision candidates, retain their
  authored list position, and do not affect the resource hash. Namespace/sidecar resources remain
  present so re-enabling is a route update rather than an application restart.
- Existing `provides`/`consumes`, bindings, and direct routes remain compatible as staged explicit
  remapping and selection overrides. The router's fixed listeners sit behind transparent
  interception, while a sender with one unambiguous active membership can already derive its group
  without a binding ahead of Part 2d's schema removal.
- Added `examples/automatic-localhost/deployment.yaml`, whose receiver and caller declare no
  routing metadata or ports. Planner acceptance tests assert empty fixed listener/provider tables,
  ordered transparent members, disabled behavior, local inheritance semantics, and schema
  compatibility.
- Disposable Docker proofs passed for undeclared ports, late-starting listeners, reordered
  duplicate priority, self listeners at group priority, IPv4 loopback, IPv6 loopback, and a receiver
  bound only to its own `127.0.0.1`.
- The repository router image then built successfully on Docker Desktop's Linux VM. The complete
  `automatic-localhost` deployment became healthy with no authored routing metadata or application
  ports; its caller reached `localhost:18081` and `[::1]:18081`, both returning the receiver bound
  only to `127.0.0.1`. Starting a second listener on the caller produced the two-member collision
  warning and kept routing to the first member; reversing the authored order selected the caller.
- Workspace tests, all-target/all-feature Clippy with `-D warnings`, and rustdoc with `-D warnings`
  pass. Group priority order and `disabled` are excluded from the resource hash, so those route
  changes do not require application-resource recreation.

## 2026-07-31 — V2 Part 2b schema and projection removal

- Removed authored `provides:`, `consumes:`, capability, route-slot, and `extends:` types. Groups
  now resolve only their complete ordered `instances:` list plus group-local `disabled:` entries;
  planner-generated sidecars use transparent membership routing exclusively.
- Removed the capability-derived fixed-listener planning path, protocol compatibility filtering,
  capability collision validation, and role inference. Planner, operations, TUI, and Web
  projections now expose generic instances, groups, ordered members, and membership changes.
- Added safe migration for the removed fields. Identity loopback port-for-port metadata is dropped,
  inherited groups are materialized before `extends:` is removed, and host or port remaps are
  refused with a field-specific diagnostic instead of silently changing behavior.
- Updated initialization templates and the JAS/routing-matrix compatibility fixtures to the
  membership-only schema. Fixed-port fixture applications now bind their original distinct ports,
  allowing transparent port-for-port routing without authored dependency metadata.
- Completed the schema-visible Part 3 address rule while removing role inference: an instance
  address selects that instance generically, and a bare group address requires exactly one active
  member with its own address. Zero or multiple candidates fail with candidate-list diagnostics.
- `bindings:` and direct `routes:` remain compatibility-only until Part 2d; the planner rejects
  non-empty direct routes and clients label the remaining binding action as a temporary whole-group
  membership selection rather than a capability/slot connection.
- Verification passes: `cargo test --workspace --all-features`, all-target/all-feature Clippy with
  `-D warnings`, rustdoc with `-D warnings`, Web `npm test -- --run` (49 tests), and Web
  `npx tsc -b`.
