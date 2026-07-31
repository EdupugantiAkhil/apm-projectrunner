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
| ⬜ | 3 — Serving a whole group from one address (router) | |
| ⬜ | 4 — Run actions become a flat `scripts:` map | |
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

### Part 4 — Run actions become a flat `scripts:` map

Closes [DEVIATION §6](../DEVIATION.md#6-run-actions-carry-a-structuredshell-split-that-may-not-earn-its-keep).
Vision reference: user_flow step 6.

The structured/shell split goes. A run action becomes one name and one command line,
following the `package.json` `scripts` model:

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
