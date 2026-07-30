# Agent mistakes and lessons

## 2026-07-30 — A migrated document must preserve resolved semantics, not merely parse

- The first migration validation stopped after the rewritten YAML deserialized as v1alpha2. A valid
  v1alpha1 group could therefore be rewritten into an invalid or behaviorally broader v1alpha2
  group when a listed instance provided capabilities that its old provider map did not select.
- Correction: resolve the inherited v1alpha1 capability-to-provider maps before transforming,
  resolve the derived v1alpha2 maps afterwards, and refuse without writing if resolution fails or
  any capability mapping changes. A regression test covers an extra capability that would otherwise
  appear silently.
- The same review found a literal `"service"` fallback masking an impossible provider-resolution
  failure in connection details, plus stale generated authoring guidance and schema support text.
  Propagate invariant failures instead of manufacturing display data, and include templates and
  support-policy prose in schema sweeps.

## 2026-07-30 — Schema migration must not rewrite fixture text incidentally

- Running the new deployment migration across repository fixtures used `serde_yaml` to
  reserialize each complete document. The intended schema delta was small, but the rewrite changed
  flow collections to block style, expanded anchors and aliases, and obscured the review under
  hundreds of unrelated formatting lines.
- One daemon test edited the shared fixture with a literal replacement of a deliberate flow-style
  instance line. Reserialization made that replacement a silent no-op, so the test continued with
  no device placement and failed much later as an empty API result.
- Correction: restore every affected YAML file from its original text and hand-apply only the API
  version and `providers`-to-`instances` changes. For the user-facing migration command, warn before
  the validated parser rewrite that comments, anchors, and formatting will be lost, and list every
  semantic change after it succeeds.
- Lesson: a mechanical rewrite of files that tests or tools may consume textually is a behavioral
  change even when parsed data is equivalent. Preserve original text for repository fixtures,
  review schema migrations as focused diffs, and never assume parser round-tripping is harmless.

## 2026-07-25 — A green test proves nothing until you watch it fail

- The first regression test for order-dependent host-key pinning passed against the unfixed
  code. The `ssh-keygen` stub printed a fixed fingerprint order regardless of its input, so
  the scenario it described — a scan whose key order varies — never actually occurred. Real
  `ssh-keygen -lf -` fingerprints its input in arrival order; once the stub did too, the test
  failed on the old code as intended.
- Correction: mutation-check every security regression test by reverting the fix and
  confirming the test fails. Both fixes in that review were checked this way.
- Lesson: a passing test is evidence only after it has been seen to fail for the right
  reason. This matters most for security fixes, where an over-obliging stub turns a real
  vulnerability into a permanently green suite.

## 2026-07-25 — Security claims need an experiment, not a careful reading

- An earlier audit of the browser clone path reported it clean on six properties, all of
  which held. It missed that credentials reach a plain `http://` remote in cleartext, because
  every check asked "is the secret handled correctly?" and none asked "where does it go?" A
  ten-line probe against a local listener captured `Authorization: Basic dXNlcjpTRUNSRVQxMjM=`
  in seconds. Reading the same code a third time would not have found it.
- The same audit accepted host-key pinning as correct. Running `ssh-keyscan` three times
  showed the key order changing between runs, which the pinning logic assumed was stable.
- Lesson: for a security property, run the thing. Send the request to a listener you control,
  invoke the tool twice and diff, revert the fix and watch the test fail. Prefer a cheap
  experiment over another read of the source, and be most suspicious of the properties an
  earlier pass already declared fine.
- Repeat of the 2026-07-15 Phase 6 JAS fixture lesson ("never take an exit code from the
  far end of a pipeline"): `cargo fmt --all --check | head` printed a reassuring "FMT OK"
  during this same review while fmt had actually failed, because `head` masked the exit
  code. Check `$?` or `PIPESTATUS`, not whether the output looked fine.

## 2026-07-25 — A plan's checkboxes drift unless ticked in the same commit as the work

- Every one of `docs/web-ui-plan.md`'s 65 acceptance boxes was still unticked after all 13
  parts had shipped. Each part was committed with its `PROGRESS.md` entry but never with its
  own boxes, so the plan read as 100% outstanding for 20 commits while `PROGRESS.md` recorded
  it complete. Anyone reading the plan first would have concluded no work had been done.
- The trap when reconciling: ticking boxes from `PROGRESS.md` just copies one document's claims
  into another. Each box was instead checked against the code that satisfies it — the route in
  `server.rs`, the exported client method, the rendered component — which is also what caught
  the two that must stay unticked.
- Do not tick a box for an acceptance criterion that was not actually met. Part 13 required a
  human security review before merge and got a code-level audit instead; ticking it would have
  destroyed the only record that the review is still owed.
- Lesson: a checkbox list is a claim about state, so update it in the commit that changes that
  state, or do not keep one. When two documents track the same work, name one authoritative in
  the other so a reader can tell which to believe.

## 2026-07-25 — Isolated worktrees do not branch from current HEAD

- Part 13 was delegated with worktree isolation to stop two concurrent agents from racing on one
  checkout. The worktree branched from `5ed8139`, an old merge commit 25 commits behind `main`,
  not from current `HEAD`. The agent therefore built against a tree predating Parts 10, 11,
  11a, 11b, and 12, and its green verification run described that stale tree: `cargo test
  --workspace` aborted on a `router-pingora` SIGABRT that does not reproduce on `main`, and its
  `SourcesView` would have reverted Part 1's unmanaged deregistration had the diff been taken
  wholesale.
- Correction: back the diff up, apply it to a branch off current `main` with `git apply --3way`,
  resolve each conflict by keeping `main` and layering the new work on top, then re-verify.
- Lesson: verify a delegated worktree's base commit at launch, not at review. A subagent's
  verification output is only evidence about the tree it ran in, so confirm that tree is the one
  you intend to merge into before trusting any of its numbers.

## 2026-07-25 — Empty label fixtures need an explicit record type

- The first Part 11a web fixture mixed a labeled resource object with a bare `labels: {}` legacy
  resource. TypeScript inferred optional ownership-label properties with `undefined` values for the
  union, which is not assignable to `Record<string, string>`. Correction: type the empty legacy map
  explicitly as `Record<string, string>`. Lesson: when a fixture deliberately contrasts typed and
  absent map entries, annotate the empty map at the boundary instead of relying on union inference.

## 2026-07-25 — Instance inspectors must not manufacture missing relationships

- The first Part 11 draft treated an expanded block name as startup-profile provenance, matched
  resources to services through container-name substrings, and approximated instance operations
  by searching operation IDs and output. Correction: label profile provenance and per-service
  observations unavailable, show the expanded block separately, and use an explicitly deployment-
  scoped recent-operation list without the rejected instance filter or text inference. Lesson:
  inspect persistence boundaries as well as frontend types before presenting a loose string or
  planner convention as durable API metadata.

## 2026-07-25 — Deployment rail button names include visible status text

- The first Part 10 composition test queried the staging rail button by the exact name
  `staging`, but its accessible name also includes the visible `unknown` status. Correction:
  match the stable deployment-name prefix. Lesson: queries for composite controls must account
  for all visible child text, while still anchoring on the stable identifying portion.

## 2026-07-25 — Additive API modes must preserve the original lookup path

- The first guided-authoring extension always resolved the requested deployment through the
  general deployment-definition lookup, including legacy profile-validation calls with no
  target override. A project using root `deployment.yaml` then returned a false missing-file
  validation result. Correction: retain the profile record's exact definition path for the
  original mode and same-deployment previews, using target lookup only for a genuinely
  different deployment. Lesson: when extending a read/validate endpoint with an optional
  target, keep the no-option path byte-for-byte equivalent before adding cross-target logic.

## 2026-07-25 — Package verification must set the package directory

- The first Part 4 web-test command ran `npm test` from the Rust workspace root, despite
  the earlier recorded correction for this exact mistake, and failed because no root
  `package.json` exists. Correction: run npm with `--prefix packages/web` and run `npx`
  from `packages/web`. Lesson: consult existing mistake records as executable preflight
  constraints, not merely historical notes.

## 2026-07-25 — Shared domains need a real leaf crate

- A cross-crate `#[path]` include compiled the profile domain into the daemon a second time
  to dodge the existing `switchyard-ops` → `switchyard-daemon` dependency direction, and a
  blanket `#[allow(dead_code)]` hid the resulting module warnings. Correction: extract the
  shared profile domain into the leaf `switchyard-profiles` crate, re-export it from ops,
  and depend on it directly from the daemon. Lesson: resolve a shared-domain dependency
  cycle by introducing one owned leaf crate, not by compiling another crate's source twice.

## 2026-07-25 — Durable operation refresh should not add hook debt

- The first Operations-view refresh used a new `useEffect` keyed to the active view,
  adding another exhaustive-dependencies warning to a file that already has known hook
  warnings. Correction: refresh through the existing mouse and keyboard navigation entry
  points instead, while retaining the local operation update path for commands started by
  the browser. Lesson: when a component intentionally uses local loader functions, prefer
  explicit user-entry refreshes over adding another effect unless the loaders are first
  stabilized consistently.

## 2026-07-23 — Project adoption must be compensating and non-destructive

- The first registration draft inserted the root source before writing the project
  marker, which could leave half-registered state if the marker write failed.
  Correction: preflight both identities, atomically publish the marker with an
  exclusive temporary file, then register the source and remove the new marker if that
  final mutation fails. Lesson: a workflow spanning SQLite and the filesystem needs an
  explicit mutation order and compensation path even when each mutation is safe alone.
- Registering the project folder as its own source initially made Switchyard's new
  `.switchyard` database and marker appear as user worktree changes. Correction: only
  project-root source inspection excludes the Switchyard-owned state directory; other
  source roots retain ordinary Git semantics. Lesson: adopting a source in place must
  keep tool-owned local state out of source-identity and dirty-worktree decisions.
- A combined follow-up command ran the GUI's npm step from the Rust workspace root,
  where there is intentionally no `package.json`. Correction: rerun the unchanged GUI
  test/build from `packages/web`. Lesson: mixed workspace verification commands must
  set the package-specific working directory explicitly rather than inheriting the
  preceding Rust command's root.

## 2026-07-22 — Verify against stock platform tools and kernel limits

- The release path initially claimed native archives while requiring GNU
  `sha256sum`, Bash 4 associative arrays/`mapfile`, and GNU `sort -z`. Correction:
  support stock macOS `shasum` and Bash 3.2 throughout assembly, install, upgrade,
  verification, and uninstall, then run the packaged scripts rather than treating a
  successful Rust build as release proof. Lesson: native support includes the scripts
  shipped around the binary, not only the binary itself.
- A shell inspection loop used `path` as a zsh variable and temporarily replaced the
  shell's command search path, making ordinary commands appear missing. Correction:
  use task-specific names such as `candidate`. Lesson: zsh's lowercase `path` is tied
  to `PATH`; avoid common shell/system variable names in diagnostics as well as scripts.
- The unpaced reliability client exhausted macOS's smaller ephemeral TCP port range and
  reported incomplete proxy exchanges even though ordinary protocol tests passed.
  Correction: retain full Linux pressure while pacing macOS clients below host tuple
  exhaustion. Lesson: a portability stress test must bound incidental OS resources or
  it measures host defaults instead of the component invariant.
- macOS canonicalizes temporary paths from `/var` to `/private/var`, and accepted
  sockets can inherit nonblocking state differently from Linux. Correction: compare
  canonical source paths, create security-sensitive test roots below the canonical temp
  directory, and set blocking mode explicitly in synchronous test servers. Lesson:
  never use Linux path aliases or accepted-socket behavior as portable identity.
- A pre-mutation host-runtime check treated unavailable published Docker ports as a
  fatal mismatch after Docker Desktop restarted, preventing Compose from restoring the
  containers that would make those ports available. Correction: preserve ownership
  and state errors, but defer port comparison startup errors until after runtime `up`.
  Lesson: recovery preflights must not require the stopped dependency they recover.

## 2026-07-22 — Keep platform commitments demand-driven

- The initial macOS checklist retained future Intel verification without a current
  product requirement. Correction: define the supported target as Apple Silicon on
  macOS 26 or newer and remove Intel and older-release compatibility from the backlog.
  Lesson: unsupported platforms should not silently become deferred release work; add
  them only when demand and acceptance hardware justify the maintenance cost.

## 2026-07-22 — macOS routing requires a portable sidecar admin channel

- The initial macOS port focused on the native host gateway's `setsid` and `/proc`
  dependencies. The live proof then showed that Docker Desktop creates the sidecar Unix
  admin socket inode in a host bind-mounted directory but rejects required permission
  and socket operations across the VM boundary with `EINVAL`/`ENOTSUP`. Correction:
  keep macOS support provisional until the authenticated sidecar admin transport is
  reachable without a bind-mounted Unix socket. Lesson: host-process portability and
  container-to-host control-channel portability are separate release gates.
- Resolution: keep the socket inside the sidecar filesystem and reach it through an
  ownership-verified, stdin-framed `docker exec` helper. This avoids both Docker
  Desktop shared-filesystem socket semantics and a new TCP administration listener.
- The routing proof's startup-order assertion parsed Docker's nanosecond RFC 3339
  timestamps with Python's microsecond-limited `datetime.fromisoformat`, so the first
  post-transport proof stopped after healthy startup. Correction: compare equal-offset
  RFC 3339 strings directly. Lesson: fixture assertions must accept Docker's documented
  timestamp precision across engines and host platforms.
- The clean proof initially failed before routing because maintained fixture
  Dockerfiles still pinned Rust 1.85 while `rust-toolchain.toml` and the workspace now
  require 1.88. Correction: align both fixture build images with Rust 1.88. Lesson:
  every compiler-floor change must search container build stages as well as CI and host
  toolchain declarations.

## 2026-07-18 — Remote Compose resources need explicit ownership

- The first remote-runtime cut removed each remote service's explicit network and let
  Compose create an implicit `<project>_default` network. That network had no
  Switchyard labels, so correct ownership verification refused teardown and stranded
  the remote container. Correction: generate a deterministic device-scoped network
  with ownership and device labels, attach every remote service, and cover remote
  networks and supported named volumes at the serialized-YAML boundary. Lesson: every
  Compose-created resource must be explicit and owned when cleanup is ownership-gated;
  service/container labels do not transfer to implicit networks.
- Teardown initially returned on the first local or remote project failure, which could
  strand every later remote project. Correction: attempt all projects, retain every
  failure, and return one aggregate whose remote entries name the device and resource.
  Lesson: multi-host cleanup is a best-effort sweep with an aggregated failure result,
  not a fail-fast transaction.

## 2026-07-18 — Remote eligibility must precede drift status

- The first runtime wiring left the existing `status` drift preflight ahead of the new
  remote Docker eligibility check. An unreachable device would therefore have failed as
  generic unknown drift instead of naming the required `docker version` check and its
  SSH stderr. Correction: expose the eligibility check and invoke it before status in
  the up workflow, while retaining the check inside `DockerRuntime::up` for direct
  callers. Lesson: adding a precondition inside a lower runtime layer is insufficient
  when an older higher-level preflight can terminate the workflow first.

## 2026-07-17 — Worktree UX relationship preservation

- The first worktree UX pass reused the existing instance-source insertion, which would
  have flattened a managed linked worktree into a generic path in `deployment.yaml`.
  Correction: carry worktree, repository, and requested-ref metadata through the source
  choice and author the full worktree source shape. Lesson: a clearer presentation must
  preserve the same relationship in durable desired state, not merely label it in the
  UI.
- The first creation form also asked for an existing Git ref even though pressing `w`
  on a checkout already supplies the intended base. Correction: capture that checkout's
  exact HEAD commit and derive both the new branch and worktree from the entered name.
  Lesson: do not ask users to restate context that the selected UI object provides.
- The first branch-validation condition used let-chain syntax unsupported by the
  workspace's pinned Rust toolchain. Correction: use a nested conditional and rerun the
  package checks. Lesson: code to the repository's declared compiler floor, not newer
  language syntax accepted elsewhere.

## 2026-07-17 — TUI reconciliation and initialized skill

- The skill-creator initializer existed but did not have its executable bit set, so a
  direct invocation failed with permission denied. Correction: invoke the provided
  Python script through `python3`. Lesson: check an optional script's executable mode or
  use its interpreter explicitly before assuming direct execution is supported.
- A new table-projection test used unconstrained `.into()` calls in an expected array;
  the workspace has several valid `From<&str>` targets, so type inference failed.
  Correction: construct expected `String` values explicitly. Lesson: use concrete
  constructors in assertions when dependency trait implementations make conversion
  targets ambiguous.
- The first standalone reconciliation call admitted every Switchyard-labeled Docker
  resource returned by the host-wide observer, which inserted an unrelated deployment
  into the project's state during live verification. Correction: filter observations
  to deployment IDs in the project's generated manifests before reconciling. Lesson:
  shared host observers must be scoped at the project boundary before persistence.

## 2026-07-16 — TUI source dialog follow-up

- A custom masked credential and SSH askpass bridge still diverged from normal terminal
  behavior and prevented Git/OpenSSH from owning key selection and prompts. Correction:
  yield the controlling terminal and execute native `git clone` with inherited standard
  streams and no authentication overrides. Lesson: when a mature CLI already specifies
  an interactive protocol, a TUI should suspend and delegate instead of reimplementing
  that protocol.
- The first SSH askpass helper retained its writable temporary-file descriptor while
  executing the helper, which Unix rejected with `ETXTBSY`. Correction: close the file
  descriptor by retaining only the auto-deleting temporary path before execution.
  Lesson: executable temporary helpers must be finalized and closed before spawning.
- A follow-up test initially used `assert_eq!` on an internal action enum that does not
  need comparison/debug traits in production. Correction: assert the returned variant
  with pattern matching. Lesson: tests should not expand production trait surfaces just
  for assertion convenience.
- The redesigned dialog initially left authentication review behind optional `F2`, so
  Enter could start an SSH clone with the default agent before the user saw any auth
  choice. Correction: make authentication review a required second step for every Git
  clone and keep clone failures there for correction and retry. Lesson: a discoverable
  optional action is not equivalent to a required workflow step.
- The first source dialog exposed name, local path, Git URL, and Git ref simultaneously,
  making mutually exclusive choices look like required fields and making paste behavior
  hard to understand. Correction: ask for one mode-specific location, infer the name,
  and move Git-only settings to a separate popup. Lesson: mutually exclusive workflows
  should be separate interaction states, not parallel empty inputs.
- A full-screen Git clone must not fall through to an invisible password or key-
  passphrase prompt. The first correction forced batch mode, but that also removed normal
  terminal prompt behavior. A later askpass form still intercepted native selection.
  Final correction: suspend the TUI and hand the terminal directly to Git/OpenSSH.
  Lesson: terminal UIs must preserve expected subprocess prompting end to end.

## 2026-07-16 — Standalone TUI workflow

- A registered source is durable project state, but an instance's `source` must still
  reference `spec.sources` in the authored deployment. Correction: the TUI instance
  form inserts a newly selected registered source declaration before inserting the
  instance, then validates the whole draft. Lesson: registry membership and desired-
  state references are separate contracts and UI workflows must bridge them explicitly.
- The device registry describes SSH connectivity for future execution; it is not a
  distributed placement scheduler. Correction: expose full device management while
  labeling instance runtime placement as local. Lesson: an interactive selector must
  not persist choices that the runtime cannot honor.

## 2026-07-16 — Interactive initializer follow-up

- The initial `switchyard init` implementation only accepted a positional directory,
  despite project initialization being a discovery-oriented workflow. Correction: keep
  the positional form for automation and make the no-argument form prompt for the
  project name and destination. Lesson: initialization commands should provide a
  guided default while retaining an explicit non-interactive path for scripts.

## 2026-07-16 — TUI Instances view

- The first health-label projection checked for `healthy` before `unhealthy`, even
  though the latter contains the former as a substring. Correction: test the negative
  state first. Lesson: ordered substring classifiers must put the more specific token
  before its suffix or prefix.

## 2026-07-16 — Device registry migration

- Adding schema v5 initially left two recovery tests with hard-coded v4 migration
  suffixes. Correction: update the expectations and retain the dedicated v4-to-v5
  upgrade test. Lesson: every schema increment must search for all literal migration
  sequences, even when the main historical test already derives its suffix.
- The first GUI check handler briefly mutated the `devices` prop before reloading it.
  Correction: treat props as immutable and use the requested server refresh as the
  sole row update. Lesson: an immediate refresh does not make transient prop mutation
  safe or necessary.

## 2026-07-16 — GUI asset base path

- The GUI build used Vite's default root-absolute asset paths even though the daemon
  serves it below `/gui/`. Asset requests therefore fell through to the authenticated
  server root and caused a blank page with `unauthorized`. Correction: set Vite's base
  to `./` and verify the generated index references relative assets through the live
  daemon. Lesson: deployment-path assumptions must be covered by a served-build smoke
  check, not only a standalone Vite build.

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

## 2026-07-16 — Phase 7 release and diagnostics Part 7

- The first diagnostics redactor used `if let` chains, which this workspace's pinned
  Rust 1.85 compiler still rejects even under edition 2024. Correction: use explicit
  nested matches for optional discovery-token parsing. Lesson: edition selection does
  not imply stabilization of adjacent language features; compile new syntax against
  the repository's pinned compiler before relying on it.

## 2026-07-16 — Phase 7 security review Part 8

- A documentation link-check loop assigned a filename to lowercase `path`, which is a
  special zsh array tied to `PATH`; subsequent commands in that shell could not be found.
  Correction: use a non-special name such as `relative_path`. Lesson: avoid zsh special
  parameter names in repository scripts and ad hoc verification loops.

## 2026-07-16 — Cleaned-up deployment GUI regression

- The first stopped-state test used a singular text query for a status intentionally
  repeated on every instance card, so the test failed on the correct UI output.
  Correction: assert the expected collection size. Lesson: accessibility tests for
  repeated per-resource state should verify the collection, while singular queries are
  reserved for unique status banners and actions.
- The stopped-state usability fix made `Run Up` prominent but initially verified only
  its presentation, not the daemon-to-CLI execution boundary. A live click exposed that
  the daemon did not supply the router credential required by `switchyard up`.
  Correction: provision one persistent project credential and test its injection into
  the real subprocess backend. Lesson: a recovery CTA is not complete until its
  end-to-end command prerequisites are exercised, especially credentials intentionally
  absent from browser state.

## 2026-07-16 — Project TUI Sources view

- Ratatui 0.29 declares `instability` with a compatible lower bound, but an unconstrained
  offline lock update selected `instability` 0.3.12 and its Rust 1.88 minimum, violating
  this workspace's Rust 1.85 contract. Correction: pin the lockfile to 0.3.1 and use the
  newer installed toolchain only for provisional cached compilation. Lesson: a direct
  dependency's MSRV does not constrain the resolver's choice of newer transitive
  releases; verify and pin loose proc-macro dependencies against the workspace MSRV.

## 2026-07-18 — Model serialization feeds compatibility hashes

Attempted to silence `null`/empty-map noise in materialized profile YAML by adding
`skip_serializing_if` across planner model structs. The compat golden test failed:
`definition_hash` is computed from planner serialization, so "cosmetic" serializer
changes are compatibility breaks that would mark every existing deployment as
drifted. Fix: revert the model change and prune nulls/empties only in the ops-layer
YAML emission for newly authored blocks. Lesson: never change serde output of
planner model types without treating it as a schema-compatibility change; the
`compat.rs` goldens are the tripwire. A second, self-inflicted lesson: reverting a
file with `git checkout` during an uncommitted feature also removed the feature's
own field (`Instance.device`) — prefer targeted edits over whole-file reverts while
reviewing uncommitted work.

## 2026-07-18 — Device status enums are constrained in SQLite too

The first eligibility implementation extended `DeviceCheckStatus` but did not widen
the `devices.last_check_status` SQL check constraint, so persisting `eligible` failed
and was misleadingly mapped as a duplicate registration. The first migration patch also
missed the separate `SCHEMA_VERSION` constant. Correction: add a preserving schema
migration, advance the declared version, and verify the new status through a state round trip. Lesson: before
extending a persisted enum, inspect both parser/serializer code and database constraints;
exercise the new value against a real migrated store, not only an in-memory formatter.

## 2026-07-18 — Duplicated milestone checklists must close together

The TUI milestone closing pass checked the authoritative `IMPLEMENTATION_PLAN.md` and
changed `docs/new_tui_features.md` to say “implemented through Phase E,” but left every
duplicated feature and acceptance checkbox unchecked. Correction: reconcile both lists
against the same completion evidence. Lesson: when a secondary design document mirrors
an execution checklist, search for remaining unchecked entries before claiming the
milestone is closed.

## 2026-07-18 — AppCUI: singletons, second Apps, and greedy list controls

Three empirical AppCUI 0.4.13 findings from the TUI rewrite review, all invisible
in the sandbox and caught only by pty-driving the real binary:

1. Building a second `App` in the same process after `App::run()` returns leaks
   the previous termios backend input thread, which then randomly steals
   keystrokes (`ps -T` shows 3 threads). The clone terminal-handoff therefore
   re-execs the process instead of rebuilding the App in-process.
2. `Tab::set_current_tab` from a window constructor does not update
   `focused_child_index`/the command bar; deferred restoration through a one-shot
   timer works.
3. List controls consume `Insert`/`Space`/`Shift+arrows` (selection) and — with
   the `SearchBar` flag — every printable character including the character half
   of `Ctrl+Q`. Per-tab actions bind only to F-keys/`Delete`/`Enter`, and list
   controls are created without `SearchBar`.

Lesson: verify interactive-framework assumptions with a pty end-to-end pass per
part; unit tests and clippy prove nothing about key routing or process lifecycle.

## 2026-07-18 — AppCUI Markdown hangs on language-tagged code fences

The part-3 Profiles inspector froze the whole TUI at startup: the AppCUI 0.4.13
`Markdown` control busy-loops (100% CPU, never paints) on any fenced code block
with a language tag (```text, ```yaml, ```rs); bare ``` fences are fine. Fixed by
rendering all data-driven content (profile expansion, YAML fallbacks, verbatim
source manifests) through read-only `TextArea` controls and keeping Markdown only
for static authored text without tagged fences. Also: a pty probe that does not
set TIOCSWINSZ reports every AppCUI app as hung (0x0 terminal) — set a real
window size before concluding anything.

## 2026-07-19 — Modal initial focus is nondeterministic; make defaults explicit

The pty smoke suite exposed that the instance-operation preview dialog's initial
focus differed between a fresh TUI session and a post-handoff re-exec'd one, so
Enter sometimes pressed Cancel and silently did nothing. Corrections: request
focus on the non-destructive action button explicitly, implement `on_accept` for
Enter, and stop letting the read-only automatic refresh hold the mutation gate
(a confirm racing a background refresh was silently dropped with a notice that
the next refresh immediately cleared). Lesson: never rely on a UI framework's
implicit first-focus or on notices as the only evidence an action ran; assert
observable outcomes end to end, repeatedly, in both fresh and restarted
sessions.

## 2026-07-23 — Verify transport behavior and platform paths on their real consumers

The macOS portability pass hard-coded `/private/tmp` in tests that also run on Linux,
causing eleven Linux failures. The remote-device implementation also asserted an
environment variable named `DOCKER_SSH_OPTS` instead of verifying the SSH subprocess
that Docker actually launches; Docker Desktop ignored that variable, so a registered
identity worked for the direct probe but not for Docker and unrelated agent keys caused
`Too many authentication failures`. Corrections: choose the deliberately short test
root per platform, and use a shared process-scoped SSH launcher with an executable-level
argument-vector test, including an identity path containing spaces and cleanup after
drop. Lesson: portability tests must run on every claimed host, and subprocess adapters
must be tested at their observable process boundary rather than by asserting assumed
environment variables.

The isolated Linux verification initially let `rustup-init` update the remote user's
`.zshenv` to source the temporary Cargo environment. Removing the toolchain then left a
broken shell-startup reference. Correction: remove the exact generated line and the now
empty file, and verify a fresh remote shell starts without a warning. Lesson: temporary
toolchain installers must use their no-profile-modification option, and cleanup must
audit shell startup files as well as the requested cache directories.

## 2026-07-25 — Web verification must start in the package directory

- The Part 9 focused web test was first invoked from the Rust workspace root, where npm
  cannot find `package.json`, despite existing project guidance. Correction: rerun it from
  `packages/web`; it passed. Lesson: treat the required package working directory as part
  of every npm command, including focused tests.


## 2026-07-25 — Typed API spies require typed shared fixtures

- The first Part 12 checklist test passed existing loosely inferred source and device fixtures into
  typed `ApiClient` spies. TypeScript had widened their discriminants (`kind`, reachability, and
  eligibility) to `string`, so `tsc` rejected the otherwise valid fixture data. Correction: annotate
  the shared fixtures as `SourceRecord` and `DeviceRecord`. Lesson: fixtures used only behind JSON
  helpers may tolerate widened literals, but reusing them at a typed mock boundary requires the API
  record type at declaration time.
## 2026-07-25 — Browser clone test harness reused consumed responses

- The first clone-flow mock stored one `Response` object per operation. Both the normal
  operation observer and the clone challenge controller poll the same operation, so the
  second reader failed with `Body is unusable`. Correction: retain plain terminal JSON
  and construct a fresh `Response` for every GET. Lesson: mocks for pollable resources
  must model repeatable reads rather than one-shot fetch bodies.
- The new EventSource test stub initially used a TypeScript parameter property, which
  this package rejects under `erasableSyntaxOnly`. Correction: declare and assign the
  field explicitly. Lesson: compile new test scaffolding against the repository's exact
  TypeScript restrictions before treating a concise syntax form as available.
- The first askpass retry left configured Git credential helpers enabled. Even though
  Switchyard itself wrote no secret, Git could have called a persistent helper after a
  successful retry, contradicting the memory-only contract. Correction: add the
  per-command `-c credential.helper=` override only on submitted-credential attempts.
  Lesson: a one-shot input channel also has to disable downstream credential-store
  callbacks; controlling where a secret enters is not enough to control where Git sends
  it after authentication.

## 2026-07-31 — Part 2 address verification corrections

- The first hand migration of the routing-matrix fixture gave one of two peer UIs a group address,
  gave the other an instance address, and added UI/backend members to the downstream service
  groups. Static planning passed, but the documented `backend-1` group switch then failed the new
  group-address invariant, so the fixture no longer proved runtime switching. Correction: keep the
  original downstream groups unchanged and put all three domains on their UI instances. Lesson:
  verify a contract fixture's documented mutations as well as its initial generated artifacts, and
  do not force a legacy route into a group address when it actually names one instance.
- The migration initially removed every Origin browser route sharing a migrated origin, including
  unrelated authored destinations. Correction: remove only routes that exactly match an explicit-
  header template for that UI, and only remove custom-domain destinations whose listener slot
  resolves to that UI's host provider. Lesson: cleanup of generated-looking configuration must be
  provenance-safe; matching one field is not enough to prove an entry is redundant.
- A throwaway zsh script assigned a temporary filename to `path`, which is a special array tied to
  `PATH`; subsequent commands became unavailable. Correction: use a neutral variable such as
  `file`. Lesson: avoid zsh special parameter names in verification scripts.
