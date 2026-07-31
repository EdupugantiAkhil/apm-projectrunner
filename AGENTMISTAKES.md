# Agent mistakes and lessons

## 2026-07-31 — A mocked router gate can hide Compose ownership

- The first Part 3 gate replaced generated provider endpoints with test listeners. That proved
  request selection but bypassed the Compose service named by host-upstream discovery. Routed
  application services share a namespace, so the namespace anchor — not the generated app
  service — owns every published port.
- Lesson: keep the focused router gate, but require a generated-Compose lifecycle fixture for the
  service-name and port-publication boundary. A mock at that boundary cannot protect it.

## 2026-07-31 — Healthchecks cannot assume arbitrary images contain a preferred tool

- Generated TCP probes invoked `nc` unconditionally. The authoritative sample uses stock
  `postgres:16`, which does not contain it, so a ready database remained `unhealthy`.
- Lesson: generated probes must use portable image capabilities or explicit fallbacks. The TCP
  probe now uses `nc` when available and Bash `/dev/tcp` otherwise, with a diagnostic failure if
  neither exists.

## 2026-07-31 — Acceptance isolation must respect the container host

- A privileged Docker-in-Docker attempt was used to avoid a host port collision after the offered
  Linux machine ran out of Docker storage. Nested layer creation produced I/O/read-only errors and
  left Docker Desktop temporarily unavailable, without improving product evidence.
- Lesson: do not improvise privileged nested daemons for a lifecycle gate. Use an ordinary Linux
  host with adequate capacity or arrange the required host port before the run; report the
  environmental limit rather than making the test substrate less reliable.

## 2026-07-31 — CLI acceptance fixtures must run from their deployment directory

- The first lifecycle script invoked `apmpr plan` from the repository root, so generated state was
  written beside the binary instead of beside the temporary deployment.
- Lesson: CLI lifecycle fixtures must `cd` to the authored deployment workspace before validate,
  plan, up, down, and cleanup. Test binaries may live elsewhere; project state may not.

## 2026-07-31 — A guard against a stale value must not read the value

- The Part 7 environment guard reported stale `SWITCHYARD_*` names via `env::vars()`. That
  iterator decodes both names *and values* to `String` and panics on non-UTF-8. The guard
  needs only names, so it took a dependency on data it never uses — and any unrelated
  variable holding non-UTF-8 bytes crashed every command, including `--help`.
- The same guard was placed in one binary. `apmpr-daemon` and `apmpr-router` ship separately
  and read the same variables, so both kept exactly the silent-ignore behavior the guard was
  written to eliminate.
- Lesson: read the narrowest thing that answers the question — `vars_os()` and names only,
  which also guarantees a token value can never be echoed. And when a guard exists because a
  *value* is read somewhere, it belongs wherever that read happens, not only in the binary
  that was open in the editor.

## 2026-07-31 — A test of a helper is not a test of the behavior

- The guard's test called `renamed_environment_variables` directly. Deleting the call site
  in `run()` — the thing that makes the guard exist at all — left the test green. I had
  recorded it as mutation-checked, but had only mutated the helper, not the wiring.
- Lesson: mutation-check the change a user would notice, not the function that implements
  it. For a process-level behavior that means running the real binary; the placement is the
  feature, and only a test that exercises it can protect it.

## 2026-07-31 — Documentation can be confidently, checkably wrong

- I wrote that ownership labels feed the compatibility hashes. They cannot: labels are
  derived *from* the resource hash, a few lines after it is finalized. The hashes really had
  changed and regenerating really was justified, so the conclusion was right and the stated
  reason was fiction — the most durable kind of error, because nothing fails.
- In the same pass I wrote that historical roadmap entries were "kept as written", while the
  sweep I had just run rewrote their identifiers, and gave a recovery command that only
  *listed* Docker containers under prose promising networks and volumes were removable.
- Lesson: a causal claim in a compatibility record is checkable — check it, in the order the
  code actually executes. Do not describe what a change ought to have done; describe what it
  did, and verify any command before offering it as a remedy.

## 2026-07-31 — A rename sweep cannot see encoded, composed, or self-describing text

- The Part 7 rename was planned as a mechanical substitution and verified by searching for
  the old name. That search reported the tree clean while three real defects survived it.
- **Encoded values.** Two tests sent `Proxy-Authorization: Basic
  c3dpdGNoeWFyZDp0ZXN0LXRva2Vu`. That base64 decodes to `switchyard:test-token`, so the old
  name was physically present and textually invisible. Only the suite caught it.
- **Composed identifiers.** `X-Switchyard-Route` became `X-APM ProjectRunner-Route` because
  the prose rule matched before the header rule — a syntactically invalid header name,
  produced by a substitution that was individually correct.
- **Split constants.** The same realm string lived in two crates. One matched the `apmpr`
  rule and the other the prose rule, leaving a listener advertising `realm="apmpr"` in one
  path and `realm="APM ProjectRunner"` in the other, disagreeing with the credential it
  checks.
- Lesson: order substitutions from most specific to least, and treat the test suite — not a
  grep for the old name — as the evidence that a rename is complete. A clean search proves
  the string is gone, not that the meaning survived.

## 2026-07-31 — Renaming a word breaks the sentences that were about the word

- The same sweep rewrote documentation that *discussed* the rename, turning "Working name:
  Switchyard. The intended product name is APM ProjectRunner" into a sentence equating a
  name with itself, and collapsing the roadmap's own checklist into `.apmpr/ → .apmpr/`.
  It also broke two ASCII box diagrams and a flow diagram's arrow column, because the new
  name is nine characters longer, and produced "a APM ProjectRunner".
- Lesson: prose that mentions an identifier is not a mention of the identifier. Historical
  records, migration instructions, and layout that depends on a token's width all need
  review by hand after a sweep; none of them are expressible as a substitution rule.

## 2026-07-31 — A baseline captured through `tail` is not a baseline

- The first attempt at a pre-change baseline piped `cargo test` through `tail -60`, which
  kept only the trailing doc-test summaries and reported "132 passed" — while the run had
  actually raced against the in-progress directory rename and aborted with a missing test
  binary. The real baseline was 336.
- Lesson: capture full output to a file and aggregate every `test result:` line. Record the
  baseline before touching the tree, and never let a measurement run concurrently with the
  change it is supposed to measure.

## 2026-07-31 — Complete hook dependencies require stable default objects

- After adding the dependencies required by React hook lint, `App` still constructed its
  default `ApiClient` in the function parameter. That creates a new client on every render;
  effects correctly depending on `client` would then rerun after every state update.
- Correction: keep one module-level default client. Lesson: clearing a dependency warning
  includes proving each newly listed object or callback has stable identity, especially a
  default prop that ordinary production rendering does not explicitly pass.

## 2026-07-31 — A custom domain name is not a browser URL

- The obvious dashboard change was to wrap each `customDomains` string in an anchor with
  an assumed `http://` prefix. Generated host gateways intentionally bind deterministic
  unprivileged ports, so that link would usually target the wrong socket.
- Correction: preserve the existing name projection and add an API link projection from
  the applied HTTP/HTTPS listener's protocol and port. Lesson: presentation code must not
  reconstruct an endpoint from a display name when the runtime already owns the full
  address.

## 2026-07-31 — A login service does not inherit an interactive shell's tool path

- The first daemon service definitions used an exact `apmpr` executable but did not
  preserve `PATH`. That starts the daemon itself successfully while leaving its Docker,
  Git, and project-command children dependent on the service manager's minimal default
  environment.
- Correction: capture the installer's `PATH` explicitly in both the LaunchAgent and the
  systemd user unit, with format-appropriate escaping. Lesson: validating only the managed
  executable misses every tool it launches; service definitions need the whole runtime
  command-discovery boundary.

## 2026-07-31 — A "vocabulary" audit must check schema surfaces, not just branching

- Part 2b removed role inference from behavior, so the Part 4 audit could easily have
  concluded the tree was clean: nothing branches on `"ui"`, `"backend"`, or `"database"`.
  It was not clean. The managed-profile artifact and the `open` command still *named* a
  generic instance with a field spelled `ui`, so authoring a managed profile for a non-UI
  instance meant filling in a field that claimed otherwise.
- Lesson: role inference has two forms. Behavior that reads a name is the dangerous one;
  a field or command argument named after a role is the one that survives the first
  cleanup, because removing the branch leaves the name behind and nothing fails.

## 2026-07-31 — A compatibility shim needs a test at the boundary it protects, not at the helper

- Part 4 renamed a `ui` field to `instance` in two versioned contracts and preserved both
  with serde: `#[serde(rename = "ui")]` on the artifact, and keeping `ui` as an accepted
  request alias. The test written for it constructed the struct directly and checked the
  resolution helper. Review showed that deleting either serde attribute left every test
  passing — while silently changing the on-disk artifact and rejecting existing clients.
- Lesson: when the thing being preserved is a *serialized form*, the test has to cross
  the serializer. Assert the actual JSON field names, and parse a realistic prior-version
  document. A test that builds the Rust value by hand never exercises the shim at all.

## 2026-07-31 — Removing a schema field can leave a live read that silently returns nothing

- The TUI startup-profile inspector rendered "Capabilities" and "Consumed slots" from
  `service.get("provides")` and `service.get("consumes")`. Part 2b deleted both keys, but
  the reads are untyped `serde_json` lookups, so they kept compiling and printed `none`
  for every service. No test failed; the panel just quietly stopped meaning anything.
- Lesson: after removing a schema field, grep for untyped accessors by string key, not
  just for the Rust type. A typed field removal fails the build; a `get("name")` does not.

## 2026-07-31 — Check whether a doc drifted past vocabulary into being wrong

- Part 4 was scoped as vocabulary alignment, which suggests word replacement. But
  `DESIGN.md` documented an overlay `groups:` replacement and a `routes:` slot that the
  parser rejects outright under `deny_unknown_fields`, and listed a `apmpr group`
  command that has never existed. Following that document would have produced
  configurations that fail to parse.
- Lesson: when a doc is stale, verify its examples against the parser before rewording
  them. Stale terminology is cosmetic; stale examples are instructions that do not work.

## 2026-07-31 — Review recommendations must preserve the vision as product truth

- The Part 3 handoff recommended changing the vision sample from `container` to `script` because
  that matched the current execution architecture. Repository policy says the opposite: the
  implementation and `DESIGN.md` converge on vision unless the project owner changes the decision.
- Correction: record source-backed container execution as the remaining implementation gap and
  change the sample only if the owner explicitly changes the product contract.

## 2026-07-31 — Live planner tests must retain generated topology invariants

- The first vision-sample router gate replaced every planned listener port independently. That
  bypassed the invariant that generated host listeners must own distinct sockets, and the helper
  could return the same released ephemeral port more than once.
- Correction: assert uniqueness before test rebinding and reserve all replacement sockets
  simultaneously so their ports are distinct. Keep runtime-only endpoint substitution explicit.

## 2026-07-31 — Derived compatibility data must be gated by the feature that needs it

- The first automatic host-routing pass inferred host-upstream records for every HTTP-probed
  service, including deployments with no addresses or host router. Validation then correctly
  rejected those orphan mappings, breaking otherwise unrelated planner tests.
- Correction: derive upstreams only when addresses require an automatically generated host
  router; preserve authored advanced routers exactly rather than opportunistically adding
  inferred providers. Lesson: compatibility data derived for one feature must not leak into
  bundles where that feature is absent, especially when validation treats the data as a claim.

## 2026-07-31 — Invalid fixtures can hide the diagnostic a test claims to exercise

- The two-addressed-member test added `ui-a` to a second group even though one-instance-one-group
  validation already made that bundle invalid. Once address resolution used validated active
  membership, the intended ambiguity diagnostic disappeared behind the earlier membership error.
- Correction: remove the other group before adding the second addressed member. Lesson: a
  negative test should violate exactly the invariant named by its assertion unless it explicitly
  verifies diagnostic aggregation.

## 2026-07-31 — Distinguish repository content from linked-worktree metadata

- While implementing Part 2c, "an adopted clone is read and never modified" was interpreted
  as forbidding Git from recording linked-worktree metadata. That made the roadmap appear
  contradictory: `git worktree add` necessarily updates the repository's administrative
  worktree records.
- The owner clarified the product boundary: repositories are shared Git object/metadata
  stores and may be bare; APM ProjectRunner never edits or runs a repository checkout. All working
  files, mounts, edits, and execution belong to source worktrees.
- Correction: managed `url:` repositories are bare stores, adopted `clone:` repositories
  may be bare or ordinary clones, and both back ordinary source worktrees. Lesson: ownership
  language such as "never modified" must name the protected layer precisely—working-tree
  content and execution are different from Git metadata required to provide the feature.

## 2026-07-31 — A rule can faithfully implement wording that its justification does not support

- Part 1 implemented "one provider per capability" exactly as step 8 stated it: any group with two
  members providing the same capability was rejected. The implementation matched the sentence and its
  tests, but the section's reasoning was narrower: it only explained why a consumer with one fixed
  slot cannot route that slot to two providers. The mismatch surfaced when the routing-matrix topology
  needed two UI branches to share one backend and service set; nothing inside the deployment consumes
  `ui`, so the rejected duplicate had no competing listener or route.
- Correction: police the consumer's selected slot, not group membership. Preserve every group member,
  warn only when one consumed slot has several candidates, and route deterministically to the first
  resolved candidate. Keep genuine duplicate listener addresses as planning errors because those are
  actual address conflicts.
- Lesson: when a specification states a broad invariant and then justifies it with a narrower example,
  test the boundary outside the example before encoding the broad statement as a hard rejection. A
  real topology is stronger evidence than internally consistent wording and tests derived from it.

## 2026-07-31 — Do not reuse zsh's special lowercase `path` variable

- While comparing deterministic planner output, a loop assigned a fixture filename to `path`. In zsh,
  lowercase `path` is tied to `PATH`, so the next `cmp`, `python3`, and `wc` commands appeared missing.
  This repeated the existing 2026-07-22 lesson in this file.
- Correction: rerun with `fixture_path`; both plans were byte-identical. Lesson: consult the mistake
  log as a preflight checklist, and use task-specific shell variable names even in one-off verification.

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
  to dodge the existing `apmpr-ops` → `apmpr-daemon` dependency direction, and a
  blanket `#[allow(dead_code)]` hid the resulting module warnings. Correction: extract the
  shared profile domain into the leaf `apmpr-profiles` crate, re-export it from ops,
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
- Registering the project folder as its own source initially made APM ProjectRunner's new
  `.apmpr` database and marker appear as user worktree changes. Correction: only
  project-root source inspection excludes the APM ProjectRunner-owned state directory; other
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
  APM ProjectRunner labels, so correct ownership verification refused teardown and stranded
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
- The first standalone reconciliation call admitted every APM ProjectRunner-labeled Docker
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

- The initial `apmpr init` implementation only accepted a positional directory,
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
  reconstruction, and `apmpr up` refreshes the native gateway when publications
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
  the daemon did not supply the router credential required by `apmpr up`.
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
  APM ProjectRunner itself wrote no secret, Git could have called a persistent helper after a
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

## 2026-07-31 — The product promise must be an acceptance test, not an inferred future property

- Earlier phases thoroughly proved configured fixed-port routing, isolated namespaces, live route
  replacement, browser identity, and recovery. They did not prove the vision's deal-breaking
  promise that group membership alone combines localhosts. Because zero-slot deployments merely
  produced no sidecars, the implementation could pass hundreds of tests while omitting the
  product's primary behavior.
- The first Part 2b design repeated the mistake in a subtler form by planning to infer ports from
  `publish`, probes, and image `EXPOSE`. That still required advance port knowledge and made image
  metadata resolution an architectural blocker. Correction: validate the promise at the network
  boundary first. Disposable containers proved transparent original-destination interception,
  runtime listener discovery, duplicate priority, self-listener preservation, IPv4/IPv6, and
  loopback-only receivers before the planner was changed.
- Lesson: every sentence that explains why the product exists needs a direct end-to-end acceptance
  test. Supporting machinery is necessary evidence, but it is not a substitute for exercising the
  user-visible invariant with all optional declarations removed.

## 2026-07-31 — Linux-only routing paths must compile and run on Linux before completion

- Host tests on macOS did not compile the Linux `SO_ORIGINAL_DST` implementation, so they missed
  both the workspace's `forbid(unsafe_code)` policy and an nftables rejection of a rule containing
  two destination matches. Correction: use socket2's safe original-destination APIs and express the
  Docker DNS exception as an earlier `RETURN` rule. The rebuilt Linux image then started cleanly.
- The first priority implementation tried a sender's own listener before its group. That contradicted
  the vision's unconditional first-listed rule. Moving it into group priority exposed a second edge:
  connecting to the sender's own bridge alias does not traverse `PREROUTING`, so neither listener
  observation nor forwarding reached its loopback-only process. Correction: mark the local member
  explicitly, read its namespace listener table directly, and connect to its loopback with the same
  bypass mark. The live collision proof now warns about both members and selects either one when
  their authored order is reversed.
- Lesson: target-specific code needs a target-native build, and routing-order proofs must include
  the caller itself as a colliding candidate rather than only distinct receiver containers.

## 2026-07-31 — Optional legacy topology is still legacy topology

- Part 2b initially kept `provides:` and `consumes:` as optional remapping overrides after
  group membership became sufficient for normal routing. That left capability/slot
  semantics in `extends:`, collision diagnostics, browser default selection, clients, and
  migration, so the supposedly removed model still controlled important behavior.
- Correction: the V2 authored schema removes capabilities, slots, bindings, direct routes,
  and capability-based inheritance. Transparent routing is port-for-port and group
  membership is the only connection statement.
- Lesson: when a new schema removes a concept, trace every behavior that used it and give
  that behavior a new schema-visible rule or remove it too. Making the old fields optional
  does not complete the migration.

## 2026-07-31 — Markdown backticks in shell arguments execute command substitutions

- A final `rg` verification put Markdown field names with backticks inside a double-quoted
  zsh argument. The shell attempted to execute the enclosed words before running `rg`.
- Correction: use a single-quoted search expression when matching Markdown backticks.
- Lesson: shell quoting applies even to read-only verification commands; never place
  literal Markdown backticks inside a double-quoted command argument.

## 2026-07-31 — An alignment roadmap must not preserve a second product model

- The roadmap treated `user_flow.md` as immutable while retaining obsolete Step 9
  bindings, capability-based group behavior, and roadmap sections for work explicitly
  deferred beyond V2. That made the roadmap a negotiation between conflicting models
  instead of a path to the approved sample schema.
- Correction: update the vision flow when the owner approves a schema decision, then make
  the roadmap follow that source of truth. Membership is the connection, the sample
  configuration is the executable acceptance contract, and deferred work is tracked
  outside this V2 roadmap.
- Lesson: a source of truth must describe one coherent target. Historical implementation
  behavior belongs in migration notes or progress records, not in the target workflow.

## 2026-07-31 — Informal examples must not masquerade as domain concepts

- Vision prose described UI, backend, and database as project "parts" or "segments" and
  then mapped a segment to a startup profile. There is no schema field or deterministic
  rule that can classify those roles, and one profile may expand into several services.
- Correction: define only measurable concepts—repository, source worktree, startup
  profile/block, instance, service, and group. UI/backend/database remain examples.
- Lesson: if the planner cannot derive or validate a noun, do not put it in the glossary
  as a product concept or use it to explain schema relationships.

## 2026-07-31 — A deviation log can become a competing source of truth

- `DEVIATION.md` was linked as the current implementation-gap record after its provider
  maps, capabilities, bindings, routes, segments, address status, and open decisions had
  already been superseded.
- Correction: delete the stale document and keep remaining implementation gaps beside
  their planned work in the version roadmap. Current limitations stay inline in the user
  flow where readers encounter them.
- Lesson: a gap document needs continuous reconciliation or it becomes another product
  model. Prefer one target vision plus versioned roadmaps and implementation progress.

## 2026-07-31 — Sender and receiver exceptions recreate an implicit role model

- The membership rule allowed one runtime instance in several groups if it behaved only
  as a receiver, then deferred rejection until that instance originated a connection.
  Without capabilities or roles, the schema cannot measure that distinction.
- Correction: an instance may appear in at most one group's membership list. Reusing a
  source or startup profile in another group requires a separate instance, and
  multi-group membership fails validation before runtime.
- Lesson: topology rules should use authored, statically measurable facts. Do not make
  schema validity depend on whether a process happens to initiate traffic later.

## 2026-07-31 — A technical consequence was presented as the product reason

- The roadmap said per-instance namespaces exist primarily because same-port listeners
  would collide in one group namespace.
- Correction: the product reason is to keep alternative instances running and switch
  which one a group tests without rebuilds or restarts. Separate localhosts make that
  possible; same-port coexistence is an important consequence.
- Lesson: architecture explanations should lead with the user workflow that requires a
  boundary, then explain the technical properties supplied by that boundary.

## 2026-07-31 — Documentation changes were committed before owner review

- A vision-alignment correction was committed immediately even though the owner had not
  asked for a commit.
- Correction: removed the commit while preserving its file changes locally for review.
- Lesson: phase-sized commits are the repository default, but an owner's request to keep
  work uncommitted controls the handoff.

## 2026-07-31 — Schema removal must include projection vocabulary

- The first Part 2b pass removed capabilities and slots from planner behavior but left operations
  and TUI DTOs named `consumer`, `slot`, and `provider`, effectively preserving the old topology in
  a public projection even though each row had become group membership.
- Correction: model those projections as instance, group, ordered members, and membership changes;
  update client tests and labels to exercise the role-free view.
- Lesson: when a product concept is removed, audit serialized/API and presentation boundaries as
  well as validation and runtime code. Relabeling an old shape is not model removal.

## 2026-07-31 — Package-local tools require the package working directory

- A TypeScript check was accidentally launched from the Rust workspace root. `npx` fetched the
  unrelated deprecated `tsc` package and failed instead of using the Web package's installed
  TypeScript compiler.
- The mistake recurred in Part 6 when a chained verification command reached `npm` from the
  workspace root. It failed immediately because there is no root `package.json`; the complete
  Web gate was rerun from `packages/web`.
- Correction: run `npx tsc -b` from `packages/web` and include the working directory in the
  verification command itself.
- Lesson: an executable name resolving through `npx` is not enough evidence that the intended
  project tool ran; package-local verification includes its package directory.

## 2026-07-31 — Membership changes affect a router set, not one selected router

- The first Part 2d live-move pass treated the selected instance as the only router whose
  configuration changed. Removing it from one group and adding it to another changes the
  complete ordered view for every routed instance in both groups.
- Correction: compare every generated sidecar snapshot with its applied snapshot, apply all
  changed routers through the daemon's existing compensation path, and preserve unchanged
  active snapshots when rewriting generated artifacts. Reject moves that would add or remove
  sidecar resources and require `up` for those.
- Lesson: when authored state is a shared collection, derive the mutation's complete affected
  set from before/after plans. The command's named object is not necessarily the mutation
  boundary.

## 2026-07-31 — An external endpoint is not an implicit host-loopback escape hatch

- The first Part 2e pass interpreted an example loopback address as a request to reach the
  developer host from Docker and began adding special host-gateway behavior.
- Correction: preserve the authored endpoint literally. Group `localhost:<port>` is the
  intercepted caller address; the external host is the upstream destination reached on that
  same port and must be resolvable from the routing sidecar.
- Lesson: distinguish the virtual address presented to group members from the upstream
  endpoint before introducing platform-specific bridging semantics.
- A reachability test dropped a temporary listener and assumed its freed ephemeral port would
  remain closed; the full parallel suite immediately reused it. Correction: use port zero for
  the deliberately invalid runtime-only check. Lesson: a released ephemeral port is never a
  deterministic negative network fixture.
