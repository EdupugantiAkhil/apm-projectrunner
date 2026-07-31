# V2 roadmap — aligning the implementation with the vision

The vision is [docs/vision/ABOUT.md](vision/ABOUT.md) and
[docs/vision/user_flow.md](vision/user_flow.md). Those two files are the source of truth
and are not edited. [DEVIATION.md](../DEVIATION.md) records where the implementation
differs from them today; this file is the plan for closing those differences.

Scope of V2 is **the shapes the product is authored and reasoned about in**: group
membership, addresses, run actions, and the vocabulary. Multi-project support is
deliberately out (see "Not in V2"), and the security/acceptance backlog in
[docs/unfinished-work.md](unfinished-work.md) is a separate track that V2 does not touch.

## Settled decisions

These were decided before the work started; parts below are written against them.

| Decision | Choice |
| --- | --- |
| Migration | Hard cut. One `apiVersion` bump to `v1alpha2` covering every V2 schema change, plus a `switchyard migrate` command that rewrites `deployment.yaml` and `run-scripts.yaml` in place. The loader rejects `v1alpha1` with an error naming the command. |
| Multi-project | Deferred to V2.1. |
| Naming | Rename to **APM ProjectRunner**, typed as `apmpr`. Binary `apmpr`, state dir `.apmpr/`, crates `apmpr-*`, `apiVersion: apmpr.dev/v1alpha2`, env `APMPR_*`, header `X-Apmpr-Route`. Done **last**, as a pure mechanical sweep over a settled tree. |

Because the rename lands last, Parts 1–7 are authored against the current `switchyard`
names throughout. Part 8 renames them all at once.

## Status at a glance

Each part is one reviewable increment: written by one subagent, verified by a second,
then committed. A part is only ticked once it is committed with verification evidence.

| | Part | Commit |
| --- | --- | --- |
| ✅ | 1 — Group membership becomes a list | `bae84bf` |
| ✅ | 2 — Addresses on the group and on the instance | `5d14720` |
| ✅ | 2a — Membership stops being policed by capability | `a24991b` |
| ✅ | 2b — A group shares one localhost; slots stop being required | |
| ⬜ | 2c — Repositories are declared once; sources are a repo and a ref | |
| ⬜ | 2d — `bindings:` is deleted; membership is the connection | |
| ⬜ | 2e — External instances: things already running outside Switchyard | |
| ⬜ | 3 — Serving a whole group from one address (router) | |
| ⏸ | 4 — Run actions become a flat `scripts:` map | deferred to V3 |
| ⬜ | 5 — Vocabulary and documentation alignment | |
| ⬜ | 6 — Daemon-as-service posture | |
| ⬜ | 7 — Release usability items | |
| ⬜ | 8 — Rename to APM ProjectRunner (`apmpr`) | |

Baseline after Part 2a: 307 Rust tests passing, 49 web tests passing, four known React
`exhaustive-deps` lint warnings (cleared in Part 7).

## The parts

---

### Part 1 — Group membership becomes a list ✅

Closes [DEVIATION §1b](../DEVIATION.md#1b-group-membership-is-a-mapping-where-a-list-would-do).
Vision reference: user_flow step 8. Landed in `bae84bf`; 292 tests passing.

- [x] `ServiceGroup.providers` map → `instances: Vec<String>`
- [x] Slot→provider mapping derived from declared capabilities, not restated
- [x] `extends:` overrides by capability, on the resolved group
- [x] `instance/service` reference form still resolves inside the list
- [x] Diagnostics say "group member", not "instance"
- [x] `apiVersion` bump to `v1alpha2`, loader names `switchyard migrate`
- [x] `switchyard migrate` with a per-transform seam for later parts
- [x] In-repo definitions and compat fixtures migrated
- [ ] ~~Two members providing one capability rejected~~ — reversed by Part 2a

The slot→provider mapping is derived by the same `provider_for` search run the other
direction, so the map no longer restates what the profiles already declare. Touched
`switchyard-planner`, `switchyard-ops/connections.rs`,
`packages/web/src/connectionModel.ts`, `switchyard-cli`, examples, and compat fixtures.

---

### Part 2 — Addresses on the group and on the instance ✅

Closes [DEVIATION §1a](../DEVIATION.md#1a-addresses-on-the-group-and-on-the-instance).
Vision reference: user_flow step 10. Landed in `5d14720`; 303 tests passing.

One rule replaces two mechanisms: **anything addressable carries `address:`, declared on
the thing it names.**

- [x] `address:` on a group — one name reaches the whole combination
- [x] `address:` on an instance — optional, absent by default
- [x] `spec.uiRoutes` and `UiRoute` removed
- [x] Planner generates `custom_domain` destinations and `browserRoutes` from `address:`
- [x] Authored `hostRouter` content still merges; conflicts fail rather than overwrite
- [x] One-backend-one-group invariant survives, with paths rewritten to the address field
- [x] `DESIGN.md`'s unimplemented `ingress:` block dropped rather than built
- [x] `uiRoutes` → address migration, all-or-nothing and idempotent
- [x] **One address per object** (singular, not `addresses: [a, b]`)

Two defects were caught in independent review: the `routing-matrix` migration had broken
the group switch the fixture exists to prove, with an ops test changed to expect the
failure; and the migration could delete an unrelated authored browser route sharing a
migrated Origin.

---

### Part 2a — Group membership stops being policed by capability ✅

Reverses the rejection Part 1 built. Decided after Part 2, when the `routing-matrix`
migration showed the rule blocking an ordinary topology: two UI branches sharing one
backend and database could not be members of one group, because both provide `ui`.

The vision's justification for the rule only ever argued about a **consumer's slot** —
"a second database in the group would have nothing pointed at it". That reasoning holds
for a consumed capability and comes apart for an unconsumed one. Nothing inside a
deployment consumes `ui`; the browser reaches it from outside by `Origin`. So the rule
was stated more broadly than the reason given for it.

A group is a shared address space, and the only thing that matters is whether two members
would answer at the same address:

- [x] Remove the rejection and `DiagnosticCode::DuplicateProvider`
- [x] Warn when a **consumer's slot** has several candidates, naming them and the winner
- [x] **First listed wins** — `instances:` order matters only where there is a collision
- [x] Two UIs in one group produce **no warning**; nothing consumes `ui`
- [x] A warning channel through the planner out to CLI, daemon API, and web UI
- [x] Deterministic ordering rule after `extends:` — resolve the parent, drop inherited
      members the child overrides, append the child's own in authored order
- [x] A genuine listener conflict still fails planning — a different thing entirely
- [x] Fix the crossed `jas-base` group addresses (`ai-main` answered to `ui-b...`)
- [x] Remove Part 1's tests asserting the rejection
- [ ] Decide whether `routing-matrix` should now use a group address — left as instance
      addresses; revisit in Part 3, which needs a group-address fixture anyway

`docs/vision/user_flow.md` step 8 was edited for this — the "One provider per capability"
section is now "Address collisions, and who wins". This is the one deliberate exception to
treating the vision as immutable, made with the owner's explicit approval, because the
rule was an oversight rather than an intent.

Landed in `a24991b`; 307 tests passing. The warning channel was the design work:
`PlannerWarning` follows the existing `BundleWarning` shape and rides on `Plan.warnings`.

---

### Part 2b — A group shares one localhost; slots stop being required

The largest correction in V2, and the one that decides whether the product earns its
existence. Today a deployment only routes if every profile declares `provides:` and
`consumes:`. Omitting them validates clean and produces **zero router sidecars** — every
instance runs isolated, talking to nothing. The declarations are not documentation of the
routing; they *are* the routing.

That is the wrong default. ABOUT.md's promise is that "the auto routing magically happens
— you do not wire up addresses, edit config files, or change ports", and that "every UI
still calls the backend at the address it was always written to call". A schema that
requires you to restate every one of those addresses before anything is routed has moved
the wiring rather than removed it. Reduced to that, Switchyard is SSH port forwarding with
a YAML file.

**The rule: everything in a group shares one localhost.** Each member's namespace
transparently intercepts arbitrary IPv4 and IPv6 loopback TCP connections. The router
recovers the original destination port and tries active group members on that same port
in authored order. Receiver-side interception forwards deployment-network traffic back
to the receiver's own loopback, so an application may continue binding only
`127.0.0.1`.

This removes the false requirement that Switchyard predict every port before a program
starts. `publish:`, probes, and image `EXPOSE` remain useful lifecycle and host-ingress
metadata, but they are not routing prerequisites.

- [x] Intercept arbitrary `127.0.0.0/8` and `::1` TCP destinations without a port list
- [x] Preserve loopback-only receiver binds through receiver-side interception
- [x] Generate each routed member's sidecar from the ordered group membership, not from
      `consumes:`
- [x] A deployment with **no** `provides:`/`consumes:` anywhere routes correctly — this is
      the acceptance test for the whole part
- [x] `provides:`/`consumes:` survive as **overrides**, for the genuine remap case where a
      consumer calls a port the provider does not listen on
- [x] Capability names become optional labels for readability, not the wiring mechanism
- [x] Two members listening on one port in one group: **warn, first listed wins** —
      sidecars passively report their namespace listener tables; callers warn from those
      observations without opening probe connections to losing application ports
- [x] Migration leaves existing `provides:`/`consumes:` working untouched; they simply
      stop being required
- [x] `disabled:` removes a member from only that group's active routing while preserving
      its running instance, namespace, and authored priority position

**Why this is not "one namespace per group".** Members may listen on the same port, and a
receiver-only instance may be shared by several groups. One group namespace would make
both legal cases collide. Switchyard therefore uses one namespace per instance and gives
each sender its selected group's ordered receiver view.

**Container proof completed.** Disposable Docker tests recovered undeclared original
ports, routed late-starting listeners, changed the winner by reordering members, included
self listeners in authored priority, intercepted both IPv4 and IPv6 loopback, and reached
a receiver bound only to its own `127.0.0.1`. No test receiver published or exposed its
application port.

Touches the planner's route generation and the sidecar config, ahead of Part 3's host-router
work. Schema-affecting, so it lands before the Part 8 rename and rides the same `v1alpha2`
migration.

---

### Part 2c — Repositories are declared once; sources are a repo and a ref

Vision reference: user_flow step 4, ABOUT.md "all backed by one clone".

Today `Source` carries `type`, `path`, `repository`, and `ref` (`model.rs:77`), and the
project state store carries a parallel `RegisteredSource` with `repository_path` and
`requested_ref` (`switchyard-state/src/lib.rs:248`). Two problems:

- **The repository is repeated per source.** Four worktrees of one repository means writing
  `repository: ./sources/monorepo` four times, with nothing enforcing agreement. ABOUT.md
  says the branches are "all backed by one clone"; the schema only repeats a path.
- **The git fields are descriptive, not operative.** `{ type: worktree, repository: ...,
  ref: ... }` validates, but nothing in the planner ever calls `create_worktree` — it is
  reachable only imperatively from the CLI, TUI, and daemon HTTP. Validation still requires
  the directory to already exist. You cannot hand someone a `deployment.yaml` and have the
  worktrees appear.

Two sections replace one, splitting *where the code comes from* from *which checkout an
instance runs*:

```yaml
repositories:
  monorepo:
    url: git@github.com:acme/monorepo.git      # cloned into .switchyard/clones/monorepo
  legacy:
    clone: ~/work/legacy-checkout               # a clone you already have; read, never modified

sources:
  ui-main:         { repository: monorepo, ref: main,        path: ./sources/ui-main }
  ui-feature:      { repository: monorepo, ref: feature-a,   path: ./sources/ui-feature }
  backend-feature: { repository: monorepo, ref: backend-fix, path: ./sources/backend-feature }
```

A source becomes a repository plus a ref plus where it lives — which is what a worktree
*is*. Adding a fourth branch to compare is one line, which is the ABOUT.md promise.

**Every source is a worktree, and the repository always lives elsewhere** (decided). This
is the rule that makes the rest fall out. Two directory populations with no overlap:

| | Where | Who owns it | Ever modified by Switchyard |
| --- | --- | --- | --- |
| Repository clones | `.switchyard/clones/<name>`, or your own path when adopted | Switchyard, or you | Managed clones only |
| Source worktrees | wherever you author `path:` | Switchyard | Created, and removable |

A source is never a repository, and a repository is never a source. Nothing has to work out
which one a directory is, and the adopt-versus-manage question is settled once at the
repository level instead of per source: `url:` means Switchyard clones and owns it, `path:`
means read this and never touch it. Every worktree below is created by Switchyard either
way, so worktree handling has exactly one case.

**`path:` is mandatory on sources and absent from managed repositories.** A source
directory is something you *use* — you open it in an editor, run commands in it, point
tooling at it — so it must be yours to choose and yours to see written down. A managed
clone is bookkeeping you never work in, so `.switchyard/clones/<name>` is fine and already
exists (`switchyard-sources/src/lib.rs:206`, used by `clone_repository` at line 601).
Exactly one of `url:` or `clone:` is required on a repository.

**The adopted-clone field is `clone:`, not `path:`.** Two fields spelled `path:` that mean
different things — "the worktree Switchyard will create for you" on a source and "a
repository that already exists, do not touch it" on a repository — is the kind of collision
that reads fine in the spec and misleads in practice. `clone:` names what it points at and
pairs obviously with `url:`, which is the other way of saying where the clone comes from.

**Missing paths are created, not rejected** (decided). A repository with a `url` is cloned
if absent. A source whose `path` is absent gets `git worktree add` against its repository at
its ref. Nothing on disk at all, and `deployment.yaml` reconstructs the whole tree. What is
present is left alone; this is not a sync that enforces state.

- [ ] `repositories:` section — exactly one of `url:` (Switchyard clones and manages) or
      `clone:` (a clone you already have, read and never modified)
- [ ] Managed clones land in `.switchyard/clones/<name>`, not authored
- [ ] A source is always `{ repository, ref, path }` — all three required
- [ ] A repository `clone:` and a source `path:` may never be the same directory, or nested
      one inside the other; that is a validation error, not a warning
- [ ] `path` always relative to the deployment file
- [ ] `up` creates missing clones and worktrees via the existing `SourceManager`
- [ ] Reconcile the deployment `Source` with the state store's `RegisteredSource` — one of
      them should stop being the second home for the same three fields
- [ ] Migration: `{ type: worktree, path, repository, ref }` → a `repositories:` entry plus
      a `{ repository, ref, path }` source, collapsing duplicate repository paths into one
      entry and keeping every existing path exactly as authored. The repository an existing
      deployment names is one the user already has, so it migrates to the adopted `clone:`
      form — migration must never turn a directory Switchyard was reading into one it
      manages.

**Unresolved: plain-path sources are not all worktrees.** "Every source is a worktree" is
right for the branch-comparison case the product exists for, but every source in the
repository today is a plain path, and several are not repository checkouts at all:
`fixture-root: { path: . }` and `repository-root: { path: ../.. }` in `jas-base`, and
`{ path: ../.. }` in `routing-matrix`, exist so a block can build a Dockerfile out of the
repository. They have no ref and no branch, and forcing them into `{ repository, ref, path }`
would be fiction. Three ways out, in preference order:

1. **Keep a plain `{ path }` source as a third kind**, documented as "a directory, not a
   checkout" — honest, and the validation cost is one variant rather than a rule with
   exceptions. Loses the clean two-population split above.
2. **Move build context off sources entirely** — a block's `build.context` is not really a
   source, and treating it as one is what created these entries. Cleaner model, wider blast
   radius.
3. Force them through the repository form, which would have `jas-base` declare its own
   repository as a repository and check itself out. Rejected.

Decide before implementation; the checklist above assumes (1) unless overridden.

**The containment guard has to change, and that is the one real cost.** Managed creation is
currently guarded by `validate_containment`, which rejects any target outside
`.switchyard/worktrees` with `source_outside_managed_root`. Author-chosen paths are outside
it by definition, so the guard cannot stay as written. Replacing it with nothing would let
a deployment file create directories anywhere on disk, including outside the project. The
replacement should be: **contained within the project directory**, refusing absolute paths
and any `../` that escapes it, with the same error code and a message naming the offending
path. That keeps the property worth having — a deployment file cannot write outside the
project it belongs to — while allowing `./sources/ui-main`.

**Other cases needing defined behaviour, because `up` now mutates git state.** A path that
exists but is not a worktree of the named repository; a path that is a worktree of a
*different* ref than authored; a dirty worktree whose ref moved; a ref absent from the
repository; two sources authored with the same path. Each needs a diagnostic rather than a
raw `git` error surfacing.

Schema-affecting, so it lands before the Part 8 rename and rides the same `v1alpha2`
migration.

---

### Part 2d — `bindings:` is deleted; membership is the connection

Vision reference: user_flow step 9, and the one-backend-one-group rule in step 10.

`bindings:` restates group membership. In the ordinary case it carries no information at
all — remove it from a deployment where every consumer is in exactly one group and planning
fails with four `IncompleteGroup` diagnostics, every one of which the membership list
already answers. Two places to say the same thing is two places to disagree.

The only case it could express is a consumer in **several** groups — and that case is not
supported and will not become supported. One process cannot infer per-request downstream
context; the one-backend-one-group rule already refuses it and says to duplicate the
instance. Keeping a field whose sole purpose is to describe a rejected topology invites
people to try it.

**Decided: delete the section.** One rule, stated once:

> An instance that consumes may belong to exactly one group. An instance that consumes
> nothing may belong to any number.

The second half is not an accommodation, it is the same rule: with nothing consumed there
is no downstream to be ambiguous about. That is `db-new` shared by `feature-test` and
`regression` — ABOUT.md's own example ("both groups use the same UI instance and the same
database"), and it must keep working.

- [ ] `spec.bindings` removed from the schema; membership alone resolves every consumer
- [ ] An instance that **consumes** and belongs to two groups is a planning error naming
      both groups and saying to duplicate the instance
- [ ] An instance that consumes nothing may belong to any number of groups — covered by a
      test using the ABOUT.md shape, not just asserted here
- [ ] `BackendGroupInvariant` rewritten against membership rather than bindings; its "a
      group's own binding is checked for the same agreement" clause disappears with the field
- [ ] Ops, TUI, and web surfaces that read or write bindings move to membership — the Web
      UI's "desired connections" table becomes group membership editing
- [ ] Migration drops `bindings:` where it agrees with membership; where it disagrees,
      **refuse and report** rather than pick one
- [ ] **`spec.routes` goes in the same pass.** It is the other place a connection can be
      authored — per slot, bypassing groups entirely — and leaving it is the same mistake as
      keeping `bindings:`: a second way to say what a group says, able to contradict it.
      One mechanism, no escape hatch.

**Sequencing.** This depends on Part 2b: once a group is one shared localhost, "which
group's localhost does this instance see" is exactly the question an instance in two groups
cannot answer, so 2b sharpens the ambiguity rather than softening it. Land 2b first, then
this.

---

### Part 2e — External instances: things already running outside Switchyard

Not everything a group needs is started by Switchyard. A Postgres installed natively on the
machine, a shared Elasticsearch on the corporate network, a service on a teammate's box —
today none of these can be a group member, so a deployment that needs one cannot be
expressed at all.

An **external instance** is an instance Switchyard routes to but does not start:

```yaml
instances:
  - { name: host-db,      external: 127.0.0.1,              ports: [5432] }
  - { name: host-kafka,   external: 127.0.0.1,              ports: [9092, 9093, 2181] }
  - { name: host-devsvc,  external: 127.0.0.1,              ports: ["8000-8010"] }
  - { name: staging-es,   external: search.staging.internal, ports: [9200, 9300] }

groups:
  feature-test:
    address: feature-test.comparison.localhost
    instances: [ui-1, backend-1, host-kafka]
```

**It is an instance kind, not a new group section** (decided). Groups stay one list of
members, the collision rule keeps working unchanged, and an external reads as what it is:
a member Switchyard happens not to start. The alternative — an `external:` map on the group
— would have needed a separate rule for ordering against members, because a YAML map has no
meaningful order and Part 2a settled collisions as *first listed wins*.

**`external:` is the host and `ports:` the list, mapping port-for-port.** 5432 inside the
group reaches 5432 on the target. Splitting the address this way is what makes ranges
possible: a range whose two sides differ would need arithmetic, and "which end does 8005
land on" is a question worth not having. It also means one entry per external service
rather than per port.

**Not only the host.** `search.staging.internal` is the same mechanism as `127.0.0.1`,
which is why the field is `external:` rather than `host:` and takes a full hostname.

- [ ] `Instance` gains an external form: `{ name, external, ports }`, with no block, no
      source, no device, and no lifecycle — `up` never starts or stops it
- [ ] `ports:` accepts integers and inclusive `"start-end"` range strings in one list;
      ranges are quoted because YAML would otherwise mangle them
- [ ] Range bounds validated `start <= end`, and capped (1024 ports) so `"1-65535"` fails
      loudly rather than attempting to bind the machine
- [ ] Ranges expand before the collision check, so a clash on one port warns about that
      port rather than the whole range
- [ ] Collisions stay positional: a started member listed before an external wins, with the
      warning naming both — the Part 2a rule, unchanged
- [ ] Two externals sharing a host with different ports is normal, not a collision — a
      collision is about a port inside one group, never about the target
- [ ] An optional `probe:` on an external, reusing the existing `Probe` type, so an external
      that is not actually listening is reported at `up` rather than as a connection refused
      at first request
- [ ] Diagnostics distinguish "external not reachable" from "instance failed to start" —
      the remedies are entirely different

**Depends on Part 2b**, which establishes that a group's routing comes from the ports its
members listen on. An external is the one member with nothing to discover from — no
`publish`, no `probe` port, no image metadata — so its `ports:` list is the one place a port
is always authored by hand.

---

### Part 3 — Serving a whole group from one address (router)

The substantial piece of step 10, and the reason Part 2 stops at the schema. Today a
`custom_domain` destination maps to exactly one provider, resolved at config-render time.
Reaching **any** member by one address means the host router resolving a member **per
request**.

- [ ] Resolve a member per request: by subdomain
      (`backend.feature-test.comparison.localhost`), by path, or by requested slot
- [ ] Bare group name resolves to the **UI-capability** member. Part 2a allows several,
      so it follows the same collision rule: warn, take the first listed
- [ ] No UI member is still an error listing what it could have meant
- [ ] Checked against browser identity — an `Origin` serving several members must still
      identify the group unambiguously
- [ ] A fixture that actually exercises a group address end to end

Touches `router-pingora` and `router-config`, not only the schema.

---

### Part 4 — Run actions become a flat `scripts:` map — **deferred to V3**

Closes [DEVIATION §6](../DEVIATION.md#6-run-actions-carry-a-structuredshell-split-that-may-not-earn-its-keep).
Vision reference: user_flow step 6.

**Deferred.** The schema change below is small and well understood, but it would land a
shape whose central question is unanswered: a script cannot currently reach the deployment
it is about. Shipping the flat map first would mean migrating everyone onto a format we
then change again once that is solved. Left whole for V3.

The intended shape, unchanged:

```yaml
scripts:
  dev-up: switchyard up $SWITCHYARD_BUNDLE --with overlays/dev.yaml
  smoke: ./scripts/smoke.sh --target feature-test
```

- [ ] Flat name→command map replaces the seven-field record
- [ ] Runner puts the binary directory on `PATH`, exports `$SWITCHYARD_PROJECT` and
      `$SWITCHYARD_BUNDLE` — the convenience is the environment, not the schema
- [ ] Remove `StructuredCommand`, `OperationSpec::Structured`, `from_script`, most of
      `validate()`
- [ ] Browser authoring **dropped, not widened** — the browser lists and runs
- [ ] The daemon's `run_action_backend_unsupported` rejection of shell actions must start
      working; after this part every action is a shell action
- [ ] Migration transform for existing `run-scripts.yaml`

**On attribution** (the open question in DEVIATION §6): keep it, and recover it from the
run rather than from the schema. The deployment target is already selected in the UI at
run time; the runner records that selection, so the operation stays tagged in the timeline
and still counts against the heavy-operation limit. That is the whole win the structured
form was buying, and it survives without a second authoring format.

#### Why it is deferred: a script cannot reach the deployment

A run action is `$SHELL -c "<string>"` with `current_dir` set to the project
(`switchyard-run-actions/src/lib.rs:403`, `run` at 418). Your shell, your machine, your
permissions. That is right for `switchyard up` and wrong for anything that needs to *talk
to* what is running, and the vision's own example is the second kind:
`smoke: ./scripts/smoke.sh --target feature-test`.

It cannot work today. `publish:` generates `127.0.0.1::8080` — an empty host port, so Docker
assigns an ephemeral one at start and nothing can hardcode it. The group's shared localhost
exists only inside sidecar namespaces, so `curl 127.0.0.1:8080` from your shell reaches
nothing. And the environment step 6 promises is not implemented: `run-actions` never calls
`.env()`, and `SWITCHYARD_BUNDLE` appears nowhere in the workspace.

Three ways to close it, kept here as ideas rather than commitments:

**(1) Export the addresses.** A group already has a stable name —
`feature-test.comparison.localhost`, served by the host router. Give scripts
`$SWITCHYARD_GROUP_FEATURE_TEST` and per-instance equivalents and `smoke.sh` curls
something that does not move. No containers, no namespaces; just telling the script what it
cannot discover. Smallest change, covers testing a group from outside.

**(2) `switchyard exec`.** Run a command *inside* a member's namespace:

```yaml
scripts:
  migrate: switchyard exec backend-1 -- ./gradlew flywayMigrate
```

The command then sees exactly the localhost `backend-1` sees — `127.0.0.1:5432` is that
group's database. This is the honest answer to "how does a script reach the group": not by
forwarding a port out, but by joining. It is also the real answer to "can a script pick its
image" — what that question wants is the instance's *network*, not its base image.

**(3) A script that names a group** and is given its own sidecar:

```yaml
scripts:
  smoke: { group: feature-test, run: ./scripts/smoke.sh }
```

Most convenient and most magical. It costs the flat map: an entry becomes a string *or* an
object, which is the structured/shell split this part exists to delete, reintroduced under
a new name. Weigh that against what (2) already gives.

Preference if nothing changes before V3: **(1) and (2)**. Together they cover both real
cases — test a group from outside by name, run a task from inside — and neither costs the
flat map. Both are one `switchyard` invocation in an ordinary shell, which is the `npm run`
bargain the vision asks for.

**On choosing an image per script** (asked during planning, recorded so it is not
re-litigated): no. A script *acts on* the deployment from outside and needs your Docker,
your daemon socket, and your credential helper; a block *is* the isolated thing under test
and needs a pinned image. Putting `switchyard up` in a container containerizes the remote
control rather than the appliance. A script that genuinely needs a runtime already has
`docker run --rm -v $PWD:/w -w /w node:22 …` — one explicit line, no schema. Revisit only
if that prefix turns out to be on most scripts in practice.

---

### Part 5 — Vocabulary and documentation alignment

`DESIGN.md` is the authoritative architecture doc and still describes the pre-V2 shapes.

- [ ] `DESIGN.md`: groups as lists, `address:` on both objects, `ingress:` gone,
      `scripts:` as a flat map
- [ ] Reconcile the user_flow glossary against the terms diagnostics and UI labels use
- [ ] `DEVIATION.md` records which sections V2 closed
- [ ] `AGENTS.md` reflects that the vision was edited once, deliberately, in Part 2a

---

### Part 6 — Daemon-as-service posture

user_flow step 2 states the intended split plainly: the daemon is a service, and
`switchyard gui` only opens a window onto it. Today `gui` auto-starts the daemon as a
fallback, and the doc itself calls that "a fallback, not the design".

- [ ] A launchd plist and a systemd unit
- [ ] A command that installs them
- [ ] `gui` against a stopped daemon becomes an actionable error naming that command,
      rather than silently starting one

---

### Part 7 — Release usability items that block the vision's flow

Pulled from [docs/unfinished-work.md](unfinished-work.md) because the vision's flow reads
wrong without them, not because they are security work:

- [ ] Running custom domains in the dashboard become normal clickable links opening in
      the default browser
- [ ] Normal link opening kept distinct from the managed-profile fallback
- [ ] Root `README.md` status refreshed to match reality
- [ ] Clear the four React `exhaustive-deps` warnings so web lint is warning-free

---

### Part 8 — Rename to APM ProjectRunner (`apmpr`)

Closes [DEVIATION §5](../DEVIATION.md#5-naming). One mechanical sweep over a settled tree,
reviewed as a pure rename diff with no behaviour mixed in.

- [ ] Crate names and paths → `apmpr-*`
- [ ] Binary → `apmpr`
- [ ] `.switchyard/` → `.apmpr/`, with a migration step folded into `apmpr migrate`
- [ ] `apiVersion: switchyard.dev/v1alpha2` → `apmpr.dev/v1alpha2`
- [ ] `SWITCHYARD_*` → `APMPR_*`
- [ ] `X-Switchyard-Route` → `X-Apmpr-Route`
- [ ] The repo directory and every doc; product name in prose is "APM ProjectRunner"


---

## Not in V2

**Multi-project support** — a project registry, a daemon holding more than one workspace,
and an in-window project switcher (user_flow step 3). It is the largest single workstream
in the vision, and it is additive plumbing: it changes no shape that Parts 1–8 change, so
it sequences cleanly afterwards as V2.1.

**The security and acceptance backlog** — SR-1 through SR-8 and the missing end-to-end
evidence in [docs/unfinished-work.md](unfinished-work.md). These block a team release
regardless of V2 and are tracked there, not here.

**Remote execution beyond the current cut** — DEVIATION §4. ABOUT.md's "instances on more
than one device" is already honest about the boundary, and widening it is a product
decision rather than an alignment gap.
