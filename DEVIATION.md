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

## 6. Vocabulary

ABOUT.md's "segment" is the implementation's **block** (a reusable startup definition,
surfaced in the UI as a *startup profile*), and an "instance of a segment" is an
**instance** (one block + one source + parameters). The docs also use **source** for the
code a block runs from, and **deployment** for the whole topology. If ABOUT.md is meant
to be the front-door document, it may be worth introducing these four words once so the
rest of the docs are readable — or renaming in the code to match how the project is
actually described out loud.
