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

## The parts

Each part is one reviewable increment, run by one subagent, committed after review.

---

### Part 1 — Group membership becomes a list

Closes [DEVIATION §1b](../DEVIATION.md#1b-group-membership-is-a-mapping-where-a-list-would-do).
Vision reference: user_flow step 8.

`ServiceGroup.providers: BTreeMap<slot, instanceRef>` becomes `instances: Vec<instanceRef>`.
The slot→provider mapping is *derived* by the same `provider_for` search that already
resolves it, run the other direction: for each consumed slot, find the member whose block
declares a service `provides`ing that capability.

- `provides`/`consumes` already carry everything needed, so the map restates a
  relationship the profiles declare. Deriving it removes the restatement.
- Two members providing one capability is an **authoring error**, not an ambiguity to
  resolve. New diagnostic: ``db-main and db-replica both provide `database`; a group may
  contain one provider per capability``.
- `extends:` override is matched **by capability** — a member replaces the inherited
  member providing the same capability rather than joining it.
- `instance/service` reference syntax (`ai-main/ingest`) stays available inside the list;
  it resolves a different ambiguity (one instance, several services providing one
  capability).
- Diagnostics referring to a list entry say **"group member"**, not "instance".

Also lands the `apiVersion` bump to `v1alpha2` and the first transform in
`switchyard migrate`. Subsequent schema parts extend the same transform, so a user
migrates exactly once.

Touches: `switchyard-planner` (model, `resolve_groups`, `validate_routes`, `provider_for`),
`switchyard-ops/connections.rs`, `packages/web/src/connectionModel.ts`,
`switchyard-cli` (new `migrate` command), examples, compat fixtures.

---

### Part 2 — Addresses on the group and on the instance (schema and planner)

Closes [DEVIATION §1a](../DEVIATION.md#1a-addresses-on-the-group-and-on-the-instance).
Vision reference: user_flow step 10.

One rule replaces two mechanisms: **anything addressable carries `address:`, declared on
the thing it names.**

- `address: <domain>` on a group in `groups:` — one name reaches the whole combination.
- `address: <domain>` on an instance in `spec.instances` — optional, absent by default.
- `spec.uiRoutes` is **removed**. Its planner invariants carry over unchanged as rules
  about group addresses.
- The domain folds inline. Today an address means editing two places that must agree
  (`uiRoutes` plus a `custom_domain` destination on a `hostRouter` listener); after this
  part the planner **generates** the `custom_domain` destinations and `browserRoutes`
  from `address:`. Hand-authored `hostRouter` listeners remain valid for everything else.
- `DESIGN.md`'s unimplemented `ingress:` block is dropped rather than built. An
  `address:` on the instance cannot outlive its instance, so the dangling-reference class
  disappears with it.
- The one-backend-one-group invariant survives verbatim: two groups may not route through
  one backend instance to different downstream groups; planning fails and tells you to
  duplicate the backend.
- **One address per object** for now (`address:` singular, not `addresses: [a, b]`).
  Plural is additive later if it earns its keep; starting singular keeps resolution
  unambiguous.

---

### Part 3 — Serving a whole group from one address (router)

The substantial piece of step 10, and the reason Part 2 stops at the schema. Today a
`custom_domain` destination maps to exactly one provider, resolved at config-render time.
Reaching **any** member by one address means the host router resolving a member **per
request**.

- Resolution by subdomain (`backend.feature-test.comparison.localhost`), by path, or by
  requested slot.
- The bare group name needs a default, because opening it in a browser sends one request.
  It resolves to the member providing the **UI capability**; no UI member, or more than
  one, is an error listing what it could have meant rather than a guess.
- Must be checked against browser identity: an `Origin` of
  `feature-test.comparison.localhost` is what currently identifies which combination a
  request belongs to, so a domain serving several members must still identify the group
  unambiguously.

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

The convenience is the **environment**, not the schema: the runner puts Switchyard's own
binary directory on `PATH` and exports `$SWITCHYARD_PROJECT` and `$SWITCHYARD_BUNDLE`.

Removes `StructuredCommand`, `OperationSpec::Structured`, `from_script`, and most of
`validate()`. Browser authoring is **dropped, not widened** — the browser lists and runs;
authoring is one line in one file.

**On attribution** (the open question in DEVIATION §6): keep it, and recover it from the
run rather than from the schema. The deployment target is already selected in the UI at
run time; the runner records that selection, so the operation stays tagged in the timeline
and still counts against the heavy-operation limit. That is the whole win the structured
form was buying, and it survives without a second authoring format.

Note the daemon currently *rejects* shell run actions at the operation backend
(`run_action_backend_unsupported`). That path has to start working, since after this part
every action is a shell action.

---

### Part 5 — Vocabulary and documentation alignment

`DESIGN.md` is the authoritative architecture doc and still describes the pre-V2 shapes.
Bring it to the vision: groups as lists, `address:` on both objects, the `ingress:` block
removed, `scripts:` as a flat map. Reconcile the user_flow glossary against the terms the
diagnostics and UI labels actually use, and update `DEVIATION.md` to record which sections
V2 closed.

`docs/vision/*.md` are not edited.

---

### Part 6 — Daemon-as-service posture

user_flow step 2 states the intended split plainly: the daemon is a service, and
`switchyard gui` only opens a window onto it. Today `gui` auto-starts the daemon as a
fallback, and the doc itself calls that "a fallback, not the design".

- Ship a launchd plist and a systemd unit, with a command that installs them.
- Make `gui` against a stopped daemon an actionable error naming that command, rather
  than silently starting one.

---

### Part 7 — Release usability items that block the vision's flow

Pulled from [docs/unfinished-work.md](unfinished-work.md) because the vision's flow reads
wrong without them, not because they are security work:

- Running custom domains in the dashboard become normal clickable links opening in the
  default browser, kept distinct from the managed-profile fallback.
- Root `README.md` status refreshed to match reality.

---

### Part 8 — Rename to APM ProjectRunner (`apmpr`)

Closes [DEVIATION §5](../DEVIATION.md#5-naming). One mechanical sweep over a settled tree,
reviewed as a pure rename diff with no behaviour mixed in.

`switchyard` → `apmpr` in: crate names and paths, the binary, `.switchyard/` → `.apmpr/`,
`apiVersion: switchyard.dev/v1alpha2` → `apmpr.dev/v1alpha2`, `SWITCHYARD_*` →
`APMPR_*`, `X-Switchyard-Route` → `X-Apmpr-Route`, the repo directory, and every doc.
The product name in prose is "APM ProjectRunner".

The state-directory rename needs its own migration step, folded into `apmpr migrate`.

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
