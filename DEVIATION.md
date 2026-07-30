# Deviations between ABOUT.md and the implementation

[ABOUT.md](ABOUT.md) is written from the project's stated goal, in the owner's words. This
file records where the current repository differs, so the difference is visible instead of
being quietly smoothed over in the description. These are observations to look into, not
a list of defects.

## 1. "A group with one entry of each segment"

ABOUT.md describes a group as one entry per segment — one UI, one backend, one database —
selected together.

The implementation splits this into two concepts:

- A **service group** (`groups:` in `DESIGN.md` §3) names a set of **providers** only. It
  is the thing being consumed, not a whole topology. A group can also `extends:` another
  group and override selected providers.
- A **binding** (`bindings:` / `switchyard bind`) points **one consumer** at one group.

So a UI is not a member of the group it uses; it is bound to it. Describing a topology
like the ABOUT.md example takes several bindings (UI → backend group, backend → database
group) rather than one group listing all three. `routes:` also allows direct per-slot
connections without a group at all.

Worth deciding: whether the product should grow a first-class "one entry of each segment"
object matching the mental model, or whether ABOUT.md's framing is a simplification that
maps onto bindings well enough in practice.

### 1a. Addresses: on the group, and on the instance

ABOUT.md's "open a group by its address" does exist — as `uiRoutes`
(`crates/switchyard-planner/src/model.rs:316`), whose entries carry `origin`, `backend`, and
`downstreamGroup`. That is a group-level address: one name resolving to a UI, a backend, and
that backend's group. The planner enforces it as a real relationship rather than a label
(`lib.rs:1216-1287`): `downstreamGroup` must match the backend's actual binding, and two
entries may not request one backend with different groups — the diagnostic tells the author
to duplicate the backend instance, because one process cannot infer per-request downstream
context.

Three problems with how this is currently expressed:

- **It hangs off the wrong object.** The address names a combination, so it belongs on the
  group. Keying it by UI makes a group-level fact look like a property of one instance, and it
  is why §1 above reads as though ABOUT.md's "group with an address" does not exist — it does,
  in the wrong place.
- **The name.** `uiRoutes` describes the implementation (a route keyed by UI) rather than the
  concept.
- **The domain is elsewhere.** `uiRoutes` has no `domain` field: the domain is declared as a
  `custom_domain` destination on a `hostRouter` listener, so authoring one address means
  editing two places that must agree. Separately, `DESIGN.md` §"Ingress names are desired
  state" documents an `ingress:` block of `instance` + `domain` for the simple one-instance
  case; that field is not implemented anywhere in the crates, and the shape below drops it
  rather than building it.

[user_flow.md](user_flow.md) step 10 is written against one rule rather than two mechanisms:
**anything addressable carries `address:`, declared on the thing it names.**

- **Group address** — `address:` on a group in `groups:`. One name reaches the whole
  combination; `instances:` is the group's existing member list, so nothing has to be kept in
  sync. No member is a designated entry point, matching how the rest of the system works: a
  requester asks for a capability and the group answers.
- **Instance address** — `address:` on an instance in `spec.instances`, optional and absent by
  default, for a memorable name for one instance with no combination implied.

The separate `ingress:` block is dropped rather than implemented. Its entry spends three names
on two facts — a block key, an `instance:` that repeats it, and the domain — and it introduces a
reference that can dangle when the instance it names is deleted. An `address:` field on the
instance cannot outlive its instance, and removes a top-level concept from the vocabulary. The
term *ingress* then has nothing left to name; the step is called addresses.

So: move `uiRoutes` onto the group, fold the domain inline, add `address:` to the instance
record — plus a migration for existing definitions and their generated host-router configs. The
planner invariants above carry over unchanged; they become rules about group addresses rather
than about UI routes.

Open, and worth settling once for both kinds rather than letting them diverge: whether one
object may have several names (`addresses: [a, b]`) or exactly one.

The substantial piece is **reaching any member by one address**. A `custom_domain` destination
currently maps to exactly one provider, so this needs the host router resolving a member per
request (subdomain, path, or requested slot) rather than at config-render time — work in
`router-pingora`, not only a schema change. It also interacts with browser identity: an
`Origin` of `feature-test.comparison.localhost` is what currently identifies which combination
a request belongs to (`DESIGN.md` §browser routing), so a domain serving several members must
still identify the group unambiguously. Worth confirming that holds before committing to the
shape.

Also open: the bare group name needs a default member for browser navigation. `user_flow.md`
proposes resolving it to the UI-capability member, erroring when there is none or more than
one. That is a resolution rule rather than a schema field, so it can be settled later.

### 1b. Group membership is a mapping where a list would do

Related, and the reason `entry:` looked necessary at first. `Group.providers`
(`crates/switchyard-planner/src/model.rs:308`) is a `BTreeMap<String, String>` from slot name
to instance reference. But `provider_for` (`lib.rs:1437-1448`) resolves a provider by searching
the named instance's services for one whose `provides` map contains the slot — so the mapping
restates a relationship the profiles already declare. Given a plain list of instances,
Switchyard can derive which member fills which slot by the same search run the other direction.

Two problems with the map form:

- **It reads as a taxonomy.** `providers: { backend: backend-1, database: db-new }` looks like
  it classifies instances into kinds. It does not — instances are uniformly one checkout
  through one startup profile, and the key is a slot name, not a type.
- **The field name names the wrong end of the relationship.** Everywhere else the consumer is
  the protagonist: a block `consumes`, a slot declares the address the app already calls, a
  binding points a consumer at a group. `providers` names the group from the far side.

Intended shape, per [user_flow.md](user_flow.md) step 8: `instances:` taking a plain list, and
no second form. `instances` also matches what every client already calls these objects.

The mapping form is not needed even for the case that appears to require it. Two instances
providing one capability is an authoring error, not an ambiguity to resolve: a consumer slot is
the fixed address the unchanged application already calls, and `validate_listeners`
(`lib.rs:1036-1042`) already rejects one instance declaring two slots at the same `(host, port)`.
So a consumer with one `database` slot makes one kind of database call and has exactly one
provider to route it to; a second `database` provider in the group would have nothing pointed at
it. An application that really talks to two databases calls two distinct addresses, so its
profile declares two differently-named slots, each filled by its own provider — which a list
expresses without help.

The validation to add is therefore a rejection, not a disambiguation: *"`db-main` and
`db-replica` both provide `database`; a group may contain one provider per capability."*

Two details to confirm when implementing: that `instance/service` reference syntax (`ai-main/ingest`,
already used in existing definitions) stays available inside the list — it resolves a different
ambiguity, one instance with several services providing the same capability, per `provider_for`
(`lib.rs:1444-1448`) — and whether any existing definition relies on a group key that differs
from the provider's declared capability name, which would need rewriting rather than mechanical
migration.

Note also that `instance` will name both the declaration in `spec.instances` and the reference
from a group. That reads unambiguously in YAML, but diagnostics should say "group member" for
the reference — *"`db-replica` is not a member of group `dual-write`"*, not "is not an instance
of".

## 2. "Auto routing magically happens"

ABOUT.md presents routing as automatic once the group exists.

In the implementation, routing is deliberately explicit and validated, by design
principle: *"A UI never silently connects to whichever backend happens to be available.
Its selected dependencies are visible and inspectable."* (`DESIGN.md` §2.3.)

Concretely, the parts that are not automatic:

- Blocks must declare what they `provides` (capabilities) and `consumes` (slots), and a
  consuming slot declares the fixed address the unchanged application already calls,
  e.g. `address: { host: 127.0.0.1, port: 8001 }`. Without those declarations there is
  nothing for the router to intercept.
- Validation rejects mismatched connections (a `database` slot routed to a UI) rather
  than inferring an intent.
- The magic is real once declared: rebinding swaps a consumer's whole route table live,
  with no application restart or rebuild. The setup before that point is authored.

## 3. Isolation is not free for every kind of segment

ABOUT.md says routing works as if there were no isolation. That holds for the
container-backed path, which is the main one: each instance gets its own network
namespace and its own `127.0.0.1`, and a router sidecar joins that namespace
(`network_mode: service:<consumer>`) to bind the addresses the application expects. Two
instances can bind the same port with no collision.

Two places where it does not hold:

- **Host-mode commands** (`execution: { type: host }`) run directly on the host with no
  network isolation. They must declare their ports and resources, and planning fails if
  two instances claim the same one. `DESIGN.md` is explicit that Switchyard will never
  silently offset a port, because service-to-service URLs are embedded in scripts and
  config. So duplicating a host-mode segment requires its ports to be parameterized
  first — you cannot just run two.
- **The browser**, which lives on the host and has only one shared `localhost`. Reaching
  a specific UI instance needs an explicit identity: a per-tab `X-Switchyard-Route`
  header from a Chromium extension, a distinct `Origin` via a custom domain, or a managed
  Chromium profile launched with `switchyard open`. A request with no valid identity is
  rejected rather than routed to an arbitrary backend. This is the one place where the
  developer has to do something Switchyard-specific.

## 4. Multiple instances, one machine

ABOUT.md implies the instances all run wherever you are working. The implementation has a
`device` field per instance and can register remote SSH devices, but only `local` is
honored — selecting a non-local device is a validation error until the remote execution
cut lands. That is a deliberate choice (never accept a placement field and then ignore
it), not a bug, but it does mean "multiple instances" currently means "multiple instances
on this host".

## 5. Naming

ABOUT.md calls the tool "APM project manager (switchyard)". The repository directory is
`apm-projectrunner`; everything inside it — crates, CLI binary, config directory, YAML
`apiVersion` — uses `switchyard`. Worth settling on one name before a wider release.

## 6. Run actions carry a structured/shell split that may not earn its keep

Not an ABOUT.md deviation — an open design question recorded here because
[user_flow.md](user_flow.md) describes the intended shape rather than the current one.

Today a run action (`crates/switchyard-run-actions/src/lib.rs`) is a seven-field record that
is *either* a `shell:` string *or* a `command:` naming one of `up`, `down`, `plan`, `status`
with `overlays`, `variation`, and `set` arguments. `validate()` mostly exists to reject
both-or-neither. The Web UI can create, edit, and delete the structured kind but not the
shell kind.

The two kinds converge before execution: `OperationSpec::process_command_with` turns a
structured action into an argv for Switchyard's own binary and runs it through the same
`std::process::Command` as a shell string. So the structured form is a typed subset of what
a shell line already expresses.

What the split buys, weighed:

- **Attribution** — the daemon knows a structured action's target deployment, so the
  operation is tagged in the timeline and counts against the heavy-operation limit. Real.
- **Binary resolution** — structured actions resolve the executable via `SWITCHYARD_BIN` or
  `current_exe()`, so `switchyard` need not be on `PATH`. Real, and reproducible by putting
  that directory on `PATH` the way `npm run` prepends `node_modules/.bin`.
- **A browser authoring boundary** — the weakest. `RoutingPanel` in
  `packages/web/src/DeploymentWorkspace.tsx` already lets the browser save a deployment
  definition, where a block's `script`/`container` execution mode names a command that runs
  on the host. Arbitrary execution from the browser is already reachable by a shorter path,
  so the boundary keeps out shell-in-run-actions specifically, not shell in general.

The proposed shape is a flat name-to-command map with `PATH`, `$SWITCHYARD_PROJECT`, and
`$SWITCHYARD_BUNDLE` injected by the runner. It removes `StructuredCommand`,
`OperationSpec::Structured`, `from_script`, and most of `validate()`. Browser authoring is
dropped rather than widened: a one-line script is easier to edit in a file than in a form.

Worth deciding: whether the attribution win justifies keeping a second authoring format, or
whether the runner can recover it from the bundle it exported without parsing the command.
Changing this is a schema change and needs a migration for existing `run-scripts.yaml`
files.

## 7. Vocabulary

ABOUT.md's "segment" is the implementation's **block** (a reusable startup definition,
surfaced in the UI as a *startup profile*), and an "instance of a segment" is an
**instance** (one block + one source + parameters). The docs also use **source** for the
code a block runs from, and **deployment** for the whole topology. If ABOUT.md is meant
to be the front-door document, it may be worth introducing these four words once so the
rest of the docs are readable — or renaming in the code to match how the project is
actually described out loud.
