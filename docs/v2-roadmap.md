# V2 roadmap — aligning the implementation with the vision

The vision is [docs/vision/ABOUT.md](vision/ABOUT.md),
[docs/vision/user_flow.md](vision/user_flow.md), and the executable target shape in
[docs/vision/sample-config.md](vision/sample-config.md). Those files are the source of
truth. This file records where the implementation differs and the plan for closing those
differences.

Scope of V2 is **the shapes the product is authored and reasoned about in**: group
membership, addresses, and the vocabulary. The security/acceptance backlog in
[docs/unfinished-work.md](unfinished-work.md) is a separate track that V2 does not touch.

The sample configuration is the V2 acceptance contract. V2 is not complete until that
file, excluding its explicitly deferred `scripts:` section, validates, plans, starts, and
passes an end-to-end routing smoke test without compatibility-only fields.

There is no `part`, `segment`, UI, backend, or database role in the authored model.
Those words may label examples, but the implementation must not infer or validate them.
Behavior may depend only on schema-visible repositories, sources, blocks, instances,
services, addresses, ordered group membership, and observed listener ports.

## Settled decisions

These were decided before the work started; parts below are written against them.

| Decision | Choice |
| --- | --- |
| Migration | Hard cut. One `apiVersion` bump to `v1alpha2` covering every V2 deployment-schema change, plus a `switchyard migrate` command that rewrites `deployment.yaml` in place. The loader rejects `v1alpha1` with an error naming the command. |
| Naming | Rename to **APM ProjectRunner**, typed as `apmpr`. Binary `apmpr`, state dir `.apmpr/`, crates `apmpr-*`, `apiVersion: apmpr.dev/v1alpha2`, env `APMPR_*`, header `X-Apmpr-Route`. Done **last**, as a pure mechanical sweep over a settled tree. |

**APM ProjectRunner is the intended product name.** `Switchyard` is a temporary
implementation name introduced during development, not a competing product direction.

Because the rename lands last, Parts 1–6 are authored against the current `switchyard`
names throughout. Part 7 renames them all at once.

## Status at a glance

Each part is one focused, reviewable increment, implemented directly in the current
agent, verified, and then committed. A part is only ticked once it is committed with
verification evidence.

| | Part | Commit |
| --- | --- | --- |
| ✅ | 1 — Group membership becomes a list | `bae84bf` |
| ✅ | 2 — Addresses on the group and on the instance | `5d14720` |
| ✅ | 2a — Membership stops being policed by capability | `a24991b` |
| ✅ | 2b — A group shares one localhost; capabilities and slots are removed | `d2bdddf` |
| ✅ | 2c — Repositories are declared once; sources are a repo and a ref | `4e6969a` |
| ✅ | 2d — `bindings:` is deleted; membership is the connection | `6616583` |
| ✅ | 2e — External instances: things already running outside Switchyard | `d0fcdd0` |
| ⬜ | 3 — Serving a whole group from one address (router) | |
| ⬜ | 4 — Vocabulary and documentation alignment | |
| ⬜ | 5 — Daemon-as-service posture | |
| ⬜ | 6 — Release usability items | |
| ⬜ | 7 — Rename to APM ProjectRunner (`apmpr`) | |

Baseline after Part 2a: 307 Rust tests passing, 49 web tests passing, four known React
`exhaustive-deps` lint warnings (cleared in Part 6).

## The parts

---

### Part 1 — Group membership becomes a list ✅

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

This was an intermediate migration step. Part 2b removes capabilities and slots from the
authored schema, and Part 2d removes the bindings and routes that used the derived mapping.
The capability-based `extends:` behavior above is therefore historical, not part of the
final V2 model.

---

### Part 2 — Addresses on the group and on the instance ✅

Vision reference: user_flow step 9. Landed in `5d14720`; 303 tests passing.

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

`docs/vision/user_flow.md` step 8 is kept current with this rule so the source of truth and
the roadmap do not describe different products.

Landed in `a24991b`; 307 tests passing. The warning channel was the design work:
`PlannerWarning` follows the existing `BundleWarning` shape and rides on `Plan.warnings`.
Part 2b supersedes the remaining capability-based warning rule with listener-port
collisions and removes `provides:` and `consumes:` from the authored schema.

---

### Part 2b — A group shares one localhost; capabilities and slots are removed ✅

Landed in `d2bdddf`; workspace tests, Clippy, rustdoc, 49 web tests, and the TypeScript
build pass.

The largest correction in V2, and the one that decides whether the product earns its
existence. Before this part, a deployment only routed if every profile declared
`provides:` and `consumes:`. Omitting them validated clean and produced **zero router
sidecars** — every instance ran isolated. The declarations were not documentation of the
routing; they *were* the routing.

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
metadata, but they are not routing prerequisites. `provides:` and `consumes:` are not
optional labels or an override language in V2: they are absent from the new schema.
Routing is port-for-port. A call to loopback port 5432 is tried against group members on
port 5432.

- [x] Intercept arbitrary `127.0.0.0/8` and `::1` TCP destinations without a port list
- [x] Preserve loopback-only receiver binds through receiver-side interception
- [x] Generate each routed member's sidecar from the ordered group membership, not from
      `consumes:`
- [x] A deployment with **no** `provides:`/`consumes:` anywhere routes correctly — this is
      the acceptance test for the whole part
- [x] Remove `provides:` and `consumes:` from the authored schema and all client authoring,
      validation, compatibility filtering, and projections
- [x] Remove role inference built on capability names. No planner, API, or client branch
      may treat names such as `ui`, `backend`, or `database` as product types
- [x] Remove the fixed-listener override path generated from capabilities and slots; a
      future port-remapping feature, if needed, must have its own explicit schema rather
      than retaining the old topology model
- [x] Two members listening on one port in one group: **warn, first listed wins** —
      sidecars passively report their namespace listener tables; callers warn from those
      observations without opening probe connections to losing application ports
- [x] Migration drops identity, loopback, port-for-port `provides:`/`consumes:` metadata.
      It refuses with an actionable diagnostic when an old slot changes host or port,
      because silently dropping a real remap would change behavior
- [x] Remove `extends:`. Without a capability key there is no honest definition of
      "replace the inherited member that provides the same thing." Every group authors its
      complete ordered `instances:` list; overlays may replace that list explicitly
- [x] `disabled:` removes a member from only that group's active routing while preserving
      its running instance, namespace, and authored priority position

The schema-removal pass also completed the schema-visible portion of Part 3: address
selection is generic, never keyed by a `ui` role, and a bare group address requires
exactly one active member with its own `address:`. Per-request explicit member selection
and the end-to-end group-address fixture remain Part 3 work.

**Why this is not "one namespace per group".** The product goal is to keep alternative
instances running and switch which one a group tests without rebuilding or restarting
them: test `backend-1`, then reorder or disable it so the same callers reach `backend-2`.
Each alternative therefore needs its own namespace and localhost.

There is also an important network reason. Several members of one group may listen on the
same fixed application port. One group namespace would make those legal listeners collide
before Switchyard could apply the authored priority order. Per-instance namespaces both
keep the alternatives alive for switching and let the group select the winner. Each
instance gets its group's ordered member view.

**Container proof completed.** Disposable Docker tests recovered undeclared original
ports, routed late-starting listeners, changed the winner by reordering members, included
self listeners in authored priority, intercepted both IPv4 and IPv6 loopback, and reached
a receiver bound only to its own `127.0.0.1`. No test receiver published or exposed its
application port.

Touches the planner's route generation and the sidecar config, ahead of Part 3's host-router
work. Schema-affecting, so it lands before the Part 7 rename and rides the same `v1alpha2`
migration.

---

### Part 2c — Repositories are declared once; sources are a repo and a ref ✅

Vision reference: user_flow step 4, ABOUT.md "all backed by one clone".

Before this part, `Source` carried `type`, `path`, `repository`, and `ref`, and the project
state store carried a parallel `RegisteredSource` with `repository_path` and
`requested_ref`. Two problems were corrected:

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
    clone: ~/work/legacy-checkout               # existing bare repository or ordinary clone

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
| Repository storage | `.switchyard/clones/<name>`, or your own path when adopted | Switchyard, or you | Git objects/worktree metadata; managed storage is removable |
| Source worktrees | wherever you author `path:` | Switchyard | Created, and removable |

A source is never a repository, and a repository is never a source. Nothing has to work out
which one a directory is, and the adopt-versus-manage question is settled once at the
repository level instead of per source: `url:` means Switchyard creates and owns a bare
repository, while `clone:` adopts existing Git storage (bare or an ordinary clone).
Repositories hold objects and linked-worktree metadata; Switchyard never runs their
checkout. Every editable and runnable tree is a source worktree, created the same way
against either repository form.

**`path:` is mandatory on sources and absent from managed repositories.** A source
directory is something you *use* — you open it in an editor, run commands in it, point
tooling at it — so it must be yours to choose and yours to see written down. A managed
clone is bookkeeping you never work in, so `.switchyard/clones/<name>` is fine and already
exists (`switchyard-sources/src/lib.rs:206`, used by `clone_repository` at line 601).
Exactly one of `url:` or `clone:` is required on a repository.

**The adopted-clone field is `clone:`, not `path:`.** Two fields spelled `path:` that mean
different things — "the worktree Switchyard will create for you" on a source and "existing
Git storage backing worktrees" on a repository — is the kind of collision
that reads fine in the spec and misleads in practice. `clone:` names what it points at and
pairs obviously with `url:`, which is the other way of saying where the clone comes from.

**Missing paths are created, not rejected** (decided). A repository with a `url` is cloned
if absent. A source whose `path` is absent gets `git worktree add` against its repository at
its ref. Nothing on disk at all, and `deployment.yaml` reconstructs the whole tree. What is
present is left alone; this is not a sync that enforces state.

- [x] `repositories:` section — exactly one of `url:` (Switchyard creates and manages a
      bare repository) or `clone:` (existing bare repository or ordinary clone)
- [x] Managed clones land in `.switchyard/clones/<name>`, not authored
- [x] A source is always `{ repository, ref, path }` — all three required
- [x] A repository `clone:` and a source `path:` may never be the same directory, or nested
      one inside the other; that is a validation error, not a warning
- [x] `path` always relative to the deployment file
- [x] `up` creates missing clones and worktrees via the existing `SourceManager`
- [x] Reconcile the deployment `Source` with the state store's `RegisteredSource` — one of
      them should stop being the second home for the same three fields
- [x] Migration: `{ type: worktree, path, repository, ref }` → a `repositories:` entry plus
      a `{ repository, ref, path }` source, collapsing duplicate repository paths into one
      entry and keeping every existing path exactly as authored. The repository an existing
      deployment names is one the user already has, so it migrates to the adopted `clone:`
      form — migration must never turn a directory Switchyard was reading into one it
      manages.

**Plain-path sources are removed.** The sample configuration's two-population model is
the rule: every source is a worktree and every worktree names its repository and ref.
Build contexts resolve inside a source worktree; they are not modeled as standalone
sources. Existing fixtures that use `{ path: . }` or `{ path: ../.. }` migrate by naming
their containing Git clone once and creating a source worktree for the required ref.
Migration refuses a path that is not inside a Git repository instead of inventing a
repository or silently preserving a third source kind.

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

Schema-affecting, so it lands before the Part 7 rename and rides the same `v1alpha2`
migration.

The deployment is now authoritative for planning and `up`; neither path consults the
registered-source store. That store remains a project-level discovery and ownership catalog
for imperative clients, and guided authoring materializes a selected registered worktree
into the deployment's repository/source sections. Existing paths are inspected but never
reset: wrong repository, wrong ref, missing ref, non-worktree paths, duplicate paths, and
project escapes have explicit diagnostics.

Landed in `4e6969a`; workspace tests, Clippy, rustdoc, formatting, 49 Web tests, and the
TypeScript/Vite production build passed.

---

### Part 2d — `bindings:` is deleted; membership is the connection ✅

Vision reference: user_flow steps 8 and 9, including the one-instance-one-group rule.

`bindings:` restates group membership. In the ordinary case it carries no information at
all — remove it from a deployment where every consumer is in exactly one group and planning
fails with four `IncompleteGroup` diagnostics, every one of which the membership list
already answers. Two places to say the same thing is two places to disagree.

The only extra choice it could express is which group supplies an instance's localhost.
The schema no longer needs that choice: an instance may appear in at most one group's
`instances:` list. To reuse the same code or startup profile in another group, the author
creates another instance.

**Decided: delete the section.** Membership gives every grouped instance one routing
context, and schema validation rejects multi-group membership before planning or startup.
This rule is structural; it does not infer whether an instance is a consumer,
receiver-only, UI, backend, or database.

- [x] `spec.bindings` removed from the schema; membership alone resolves every consumer
- [x] Every instance appears in at most one group's `instances:` list; validation names
      both groups when the rule is violated
- [x] A grouped instance gets that group's complete ordered member view
- [x] Acceptance proves that two instances may reuse the same source and startup profile
      while belonging to different groups
- [x] Delete the backend-specific `BackendGroupInvariant`. The generic replacement is an
      instance-in-multiple-groups validation diagnostic; it does not classify that
      instance by role or wait for a runtime connection
- [x] Ops, TUI, and web surfaces that read or write bindings move to membership — the Web
      UI's "desired connections" table becomes group membership editing
- [x] Migration refuses an instance listed in several groups and names every occurrence;
      it cannot silently duplicate a runtime instance or choose which group keeps it
- [x] Migration drops `bindings:` where it agrees with membership; where it disagrees,
      **refuse and report** rather than pick one
- [x] **`spec.routes` goes in the same pass.** It is the other place a connection can be
      authored — bypassing groups entirely — and leaving it is the same mistake as
      keeping `bindings:`: a second way to say what a group says, able to contradict it.
      One mechanism, no escape hatch.

**Sequencing.** This depends on Part 2b: once group membership supplies the shared
localhost, unique membership replaces the routing-context choice that `bindings:` used to
express. Land 2b first, then this.

Landed in `6616583`; workspace tests, Clippy, rustdoc, formatting, 49 Web tests, Web lint,
and the TypeScript/Vite production build pass.

---

### Part 2e — External instances: things already running outside Switchyard

Not everything a group needs is started by Switchyard. A separately managed Postgres,
a shared Elasticsearch on the corporate network, a service on a teammate's box —
today none of these can be a group member, so a deployment that needs one cannot be
expressed at all.

An **external instance** is an instance Switchyard routes to but does not start:

```yaml
instances:
  - { name: shared-db,    external: db.dev.internal,         ports: [5432] }
  - { name: shared-kafka, external: kafka.dev.internal,      ports: [9092, 9093, 2181] }
  - { name: teammate-svc, external: teammate.dev.internal,   ports: ["8000-8010"] }
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

**Use the address as authored.** `search.staging.internal` is resolved from the routing
sidecar and must be reachable from there. Switchyard does not reinterpret a loopback address
as the developer host; host-machine bridging, when wanted, must be named explicitly by an
address reachable from the container network.

- [x] `Instance` gains an external form: `{ name, external, ports }`, with no block, no
      source, no device, and no lifecycle — `up` never starts or stops it
- [x] `ports:` accepts integers and inclusive `"start-end"` range strings in one list;
      ranges are quoted because YAML would otherwise mangle them
- [x] Range bounds validated `start <= end`, and capped (1024 ports) so `"1-65535"` fails
      loudly rather than attempting to bind the machine
- [x] Ranges expand before the collision check, so a clash on one port warns about that
      port rather than the whole range
- [x] Collisions stay positional: a started member listed before an external wins, with the
      warning naming both — the Part 2a rule, unchanged
- [x] Two externals sharing a host with different ports is normal, not a collision — a
      collision is about a port inside one group, never about the target
- [x] An optional `probe:` on an external, reusing the existing `Probe` type, so an external
      that is not actually listening is reported at `up` rather than as a connection refused
      at first request
- [x] Diagnostics distinguish "external not reachable" from "instance failed to start" —
      the remedies are entirely different

**Depends on Part 2b**, which establishes that a group's routing comes from the ports its
members listen on. An external is the one member with nothing to discover from — no
`publish`, no `probe` port, no image metadata — so its `ports:` list is the one place a port
is always authored by hand.

Landed in `d0fcdd0`; workspace tests, all-target/all-feature Clippy, rustdoc, formatting,
49 Web tests, TypeScript, and the Vite production build pass. Focused TCP coverage proves
literal host resolution and port-for-port forwarding from the expanded external allowlist.

---

### Part 3 — Serving a whole group from one address (router)

The substantial piece of step 9, and the reason Part 2 stopped at the schema. Before this part, a
`custom_domain` destination maps to exactly one provider, resolved at config-render time.
Reaching **any** member by one address means the host router resolving a member **per
request**.

- [x] Resolve an explicitly targeted member per request by instance subdomain
      (`backend-1.feature-test.comparison.localhost`) or browser route identity; no
      capability or slot name participates
- [x] Delete the planner's `group_capability_candidates(..., "ui", ...)` selection and
      every diagnostic that requires a member "providing UI"
- [x] The bare group name resolves only when exactly one active member is independently
      browser-addressable through its own `address:`. That schema-visible fact replaces
      the removed `ui` capability as the default-selection rule
- [x] Zero or several browser-addressable members is an error listing the candidates,
      never a first-listed guess
- [x] Checked against browser identity — an `Origin` serving several members must still
      identify the group unambiguously
- [x] A fixture that actually exercises a group address end to end
- [ ] Materialize `docs/vision/sample-config.md` as the acceptance fixture, excluding only
      its deferred `scripts:` section: validate, plan, create its missing clone/worktrees,
      start both groups, open both group addresses, prove their different backends and
      separate database instances reusing one source/profile, reach the external instance,
      exercise `disabled:`, then stop and clean up without compatibility-only schema fields

The sample now validates, plans, and passes a live planner-to-router gate without authored
`hostRouter` or `hostUpstreams`: both bare group domains select their distinct UI and Origin
selects the matching backend, while the disabled canary is absent. The remaining full lifecycle
gate needs execution work outside the router before it can honestly be ticked: the sample invokes
source commands inside plain `container` images, while the execution contract mounts source only
for `script`.
The fixture also needs local substitutes for its illustrative Git and external-service hosts.
Because the vision is authoritative, the implementation must converge on source-backed container
execution unless the project owner explicitly changes that product decision. That work is not a
router implementation detail.

Uses the existing `router-config` direct-route and browser-route contracts and changes
`router-pingora` so a trusted explicit identity can select a member on an otherwise static
host-gateway route. No new authored router schema is required.

---

### Part 4 — Vocabulary and documentation alignment

`DESIGN.md` is the authoritative architecture doc and still describes the pre-V2 shapes.

- [ ] `DESIGN.md`: groups as lists, `address:` on both objects, `ingress:` gone,
      capabilities/slots/bindings/routes/extends gone
- [ ] Audit schema, planner, operations, daemon, CLI, TUI, Web UI, examples, and current
      docs for inferred `part`, `segment`, UI, backend, or database roles. Replace behavior
      with generic instance/service/address/membership/listener rules; retain role words
      only in clearly labeled examples
- [ ] Reconcile the user_flow glossary against the terms diagnostics and UI labels use
- [ ] Current documentation distinguishes shipped behavior from remaining V2 work

---

### Part 5 — Daemon-as-service posture

user_flow step 2 states the intended split plainly: the daemon is a service, and
`switchyard gui` only opens a window onto it. Today `gui` auto-starts the daemon as a
fallback, and the doc itself calls that "a fallback, not the design".

- [ ] A launchd plist and a systemd unit
- [ ] A command that installs them
- [ ] `gui` against a stopped daemon becomes an actionable error naming that command,
      rather than silently starting one

---

### Part 6 — Release usability items that block the vision's flow

Pulled from [docs/unfinished-work.md](unfinished-work.md) because the vision's flow reads
wrong without them, not because they are security work:

- [ ] Running custom domains in the dashboard become normal clickable links opening in
      the default browser
- [ ] Normal link opening kept distinct from the managed-profile fallback
- [ ] Root `README.md` status refreshed to match reality
- [ ] Clear the four React `exhaustive-deps` warnings so web lint is warning-free

---

### Part 7 — Rename to APM ProjectRunner (`apmpr`)

One mechanical sweep over a settled tree, reviewed as a pure rename diff with no
behaviour mixed in.

- [ ] Crate names and paths → `apmpr-*`
- [ ] Binary → `apmpr`
- [ ] `.switchyard/` → `.apmpr/`, with a migration step folded into `apmpr migrate`
- [ ] `apiVersion: switchyard.dev/v1alpha2` → `apmpr.dev/v1alpha2`
- [ ] `SWITCHYARD_*` → `APMPR_*`
- [ ] `X-Switchyard-Route` → `X-Apmpr-Route`
- [ ] The repo directory and every doc; product name in prose is "APM ProjectRunner"


---

## Not in V2

**The security and acceptance backlog** — SR-1 through SR-8 and the missing end-to-end
evidence in [docs/unfinished-work.md](unfinished-work.md). These block a team release
regardless of V2 and are tracked there, not here.

**Remote execution beyond the current cut** — DEVIATION §4. ABOUT.md's "instances on more
than one device" is already honest about the boundary, and widening it is a product
decision rather than an alignment gap.
