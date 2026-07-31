# User flow

How a person sets up and uses Switchyard through the Web UI, from an empty folder to a
group they can open in the browser by name.

Read [ABOUT.md](ABOUT.md) first for *why* the project exists. This document is the *how*.
Where the current implementation does not yet match the intent, this file says so inline
and points at [DEVIATION.md](DEVIATION.md) rather than describing something that does not
work yet.

The vocabulary used below is defined in the [Glossary](#glossary) at the end. Two words are
worth knowing before you start, because the UI uses them everywhere:

- A **startup profile** is a reusable recipe for starting one part of your project.
- An **instance** is one checkout of code, run through one startup profile.

---

## The flow at a glance

```text
1.  switchyard init          →  a Switchyard project folder exists
2.  switchyard daemon run    →  the background service is running (usually already is)
3.  switchyard gui           →  opens the browser window at the running service
4.  Sources                  →  clone repositories, add worktrees
5.  Startup profiles         →  the recipe for starting each part
6.  Run actions              →  project-level scripts (Up, Down, smoke tests)
7.  Instances                →  live copies of each part, optionally on another device
8.  Service groups           →  a named set of instances
9.  Connections              →  point each consumer at a group
10. Addresses                →  open a group, or one instance, by name
```

Steps 4 through 10 all happen in the browser. Only steps 1 to 3 are CLI, and step 2 is
normally a one-time or machine-startup concern rather than something you type each session.

---

## Step 1 — Initialize the project folder (CLI)

Switchyard needs a project folder to hold your authored deployment and its local state.
There are two ways in, depending on whether the folder already exists.

**A new project folder:**

```bash
switchyard init                      # prompts for a name and directory
switchyard init ./my-project --name my-project
```

This scaffolds `deployment.yaml`, an `overlays/dev.yaml`, a `README.md`, a `.gitignore`,
and a project skill under `.agents/`. It refuses to overwrite existing scaffold files
unless you pass `--force`. The project name must be a lowercase DNS label (it ends up in
hostnames, which is why the rule is strict).

**An existing code folder you want to adopt:**

```bash
switchyard project register path/to/code --name my-project
```

Registration preserves everything already in the folder, creates project-local state and
an empty `deployments/` directory, and registers the folder itself as the first source.
Running it again on the same folder is safe.

After either command you have a `.switchyard/` directory holding project state, and an
authored `deployment.yaml` that is valid but empty of real work.

## Step 2 — The background service

The **daemon** is the background service that owns everything: it holds project state, runs
operations, streams events, and serves the API the Web UI talks to. Instances keep running
and operations keep progressing whether or not any UI is open. It is the long-lived thing;
the Web UI is just a window onto it.

```bash
switchyard daemon run       # run the service (foreground)
switchyard daemon status    # is it running? pid, API version, active operations
switchyard daemon stop      # ask it to shut down
```

The intended posture is that the service is **already running** by the time you want a UI —
started at login or by your machine's service manager, so opening the UI never means
"start Switchyard". Registering it with launchd or systemd is left to you today; Switchyard
does not yet ship a service unit or a login item.

Service output is appended to `.switchyard/daemon.log`.

> **Today's behaviour:** `switchyard gui` still auto-starts the service if it is not running,
> as a convenience. That is a fallback, not the design — the responsibility split is that
> the service is a service and `gui` only opens a window onto it. `switchyard gui` should not
> be understood as the way to start Switchyard.

## Step 3 — Open the Web UI window (CLI)

```bash
switchyard gui
```

The GUI is **one window across every project**, not one window per project. It finds the
running service, prints its local URL, and tries to open your browser (`open` on macOS,
`xdg-open` on Linux). If the opener fails, the command still succeeds — copy the printed
URL. Closing the browser window closes nothing else: the service keeps running and your
instances stay up.

Inside the window, you pick which project you are working on and switch between them without
relaunching anything. A project you initialized in step 1 appears in that list; you do not
open a separate UI per folder, and you do not pass a path on the command line to choose one.

The URL looks like `http://127.0.0.1:<port>/gui/#token=<credential>`. The credential is in
the URL *fragment*, so it is never sent in the HTTP request and never lands in server access
logs. The page strips it from the address bar immediately and keeps it in JavaScript memory
only.

> **Not built yet.** This is the intended shape, and the rest of this document is written
> against it. Today both the service and the GUI are scoped to a single project: the daemon
> is bound to one workspace root, `switchyard gui [project]` takes a path and opens that
> project's daemon, and there is no project registry or in-window switcher in any client.
> One browser session is currently one project. Multi-project support means a project
> registry, a service that can hold more than one workspace, and a switcher in the window —
> it is listed as "Not in scope" in [docs/web-ui-plan.md](docs/web-ui-plan.md), which is a
> statement about that plan's boundary rather than a decision against the feature. Until it
> lands, run `switchyard gui path/to/code` per project.

Everything below happens in that browser tab, scoped to the project you have selected. The
left rail is the view switcher (**home, deployments, sources, devices, profiles, run actions,
operations, block library**); arrow keys move between views.

**Home** is the landing view for a project with no deployments. It shows a setup checklist
across source → profile → instance → startup → connection, recommends the next unfinished
action, and links straight into the view that performs it. If you follow Home's
recommendation each time, it walks you through steps 4 to 9 in order.

## Step 4 — Add code: repositories and worktrees

Go to **sources**. There are three ways to give Switchyard code:

**Clone a Git repository.** Enter a name and a repository URL (optionally a ref and an SSH
identity file). Switchyard first tries a non-interactive clone using your existing Git
credential helper and SSH agent — usually that just works and nothing is asked of you.

If it does not:

- *HTTPS auth needed* — the UI shows a username/password-or-token form. Those credentials
  are used for exactly one retry attempt, pass through memory only, and are never written to
  disk. Plain `http://` URLs to a remote host are refused, because Git would send the
  credentials unencrypted.
- *Unknown SSH host key* — the UI shows the fingerprint and asks you to approve it
  explicitly. Verify it through a trusted channel first. It is a deliberate approval step,
  not a passthrough terminal prompt.

Clone progress streams into the operations timeline like any other operation.

**Register an existing local directory** as an *unmanaged* source. Switchyard records where
it is and nothing else; it never modifies or deletes the files.

**Create a worktree** from a registered repository. Choose the repository, a ref, and an
optional name. This is a *managed* source: Switchyard created the directory and can remove
it. This is the step that makes "several branches alive at once" possible — one worktree per
branch, each a separate checkout on disk, all backed by one clone.

Removal is kind-aware, and the confirmation copy tells you which you are doing:

| Source kind | Remove means |
| --- | --- |
| Managed (worktree) | The worktree directory is deleted from disk. A dirty worktree requires a second explicit confirmation. |
| Unmanaged (registered) | Only the registration is forgotten. Your files are untouched. |

## Step 5 — Startup profiles: how each part gets started

A **startup profile** is the reusable definition of how one part of your project starts —
its command or container, its services, its ports, its volumes, and the parameters you can
vary per instance. Internally this is a *block*; the UI calls it a startup profile.

Go to **profiles**. Switchyard discovers profiles from two places, and the difference matters:

- **Project-local profiles** live in your Switchyard project. They are shared across every
  instance in the project, regardless of which repository or branch the instance runs.
- **Source-local profiles** live inside a source — that is, inside the Git worktree itself.
  They travel with the branch, so a branch can change how it starts itself.

Because a source-local profile is code from a repository, it is **untrusted until you review
its manifest**. The Profiles view shows origin and trust badges, and requires you to read the
manifest before importing. If the profile's content changes later, it needs review again. This
is the same trust model the TUI uses, deliberately mirrored.

From this view you can: list discovered profiles, read one's expanded definition, validate a
profile against a specific checkout (with an expansion report showing what it would produce),
import or re-import after review, and remove an imported profile.

> **Not available:** editing a profile in the browser. Profile *save* is blocked in every
> client — the shared operations layer does not expose the mutation — so authoring or editing
> a profile means editing its YAML file directly. This is a known gap, not an oversight of the
> Web UI. See "Sequencing constraints" in [docs/web-ui-plan.md](docs/web-ui-plan.md).

## Step 6 — Run actions: project-level scripts

**Run actions** are saved shell commands you run often against the project — a smoke test, a
lifecycle shortcut, whatever you would otherwise retype. They are shared at the project level,
not per instance.

They follow the `package.json` `scripts` model: a flat name-to-command map, and the runner
sets up the environment so the commands stay short.

```yaml
scripts:
  dev-up: switchyard up $SWITCHYARD_BUNDLE --with overlays/dev.yaml --set LOG_LEVEL=debug
  smoke: ./scripts/smoke.sh --target feature-test
  status: switchyard status $SWITCHYARD_BUNDLE
```

Like `npm run`, the convenience is the environment rather than the schema. The runner puts
Switchyard's own binary directory on `PATH`, so `switchyard` resolves without being installed
globally, and exports `$SWITCHYARD_PROJECT` and `$SWITCHYARD_BUNDLE` for the selected
deployment. Everything else is an ordinary command in an ordinary shell, run from the project
directory with your permissions.

**Authoring is not in the browser, and does not need to be.** A run action is one line of
shell in one file — editing that file directly is less work than filling in a form, and a
web page writing executable commands to disk is not a trade worth making for a format this
small. Add and edit them in your editor, the CLI, or the TUI.

Go to **run actions** to see the list and run one. Every action shows a confirmation preview
with the exact command before it executes, and the first shell execution in a project also
requires a one-time acknowledgement that arbitrary commands run with your user permissions.
Execution appears in the operations timeline like any other operation.

> **Not built this way yet.** Today the file is `.switchyard/run-scripts.yaml` and each entry
> is a seven-field record that is *either* a `shell:` string *or* a `command:` naming one of
> `up`, `down`, `plan`, `status` with `overlays`, `variation`, and `set` arguments. The browser
> can create, edit, and delete the structured kind but not the shell kind. The flat-map model
> above collapses that split: structured actions are already assembled into an argv for
> Switchyard's own binary and run through the same `std::process::Command` as shell ones, so
> they are a typed subset of what a shell string already expresses. Moving to it is a schema
> change with a migration for existing `run-scripts.yaml` files. See the open question in
> [DEVIATION.md](DEVIATION.md).

## Step 7 — Create instances

An **instance** is one checkout run through one startup profile with its own parameters and
its own device placement. This is the thing that actually runs. Several instances can share
one profile; several instances can share one repository via different worktrees.

Use **+ New deployment** in the left rail to create a deployment, or **Add instance** on an
existing deployment. Either opens one progressively-revealed form (not a multi-step wizard)
with live validation:

1. **Checkout** — which registered source this instance runs from.
2. **Startup profile** — filtered to trusted profiles that are valid for that checkout.
3. **Device** — where it runs. Eligible devices are selectable; ineligible ones are shown
   with the reason inline rather than hidden.
4. **Parameters** — rendered from the profile's own JSON Schema as real form fields, not
   free text.
5. **Live expansion preview** — the services, ports, and volumes this instance will produce,
   shown before you append it.

Errors are attached to the field that caused them.

Instances are authored first and started later. Saving the form updates the deployment
definition; nothing runs until you use **Up**.

### Devices, including external ones

Go to **devices** to register an execution host. A device is `local` plus any registered SSH
hosts. Each device shows two separate columns, because they answer different questions:

- **Reachability** — can Switchyard reach it over SSH at all (`never | ok | unreachable |
  auth-failed`)?
- **Eligibility** — can it actually run instances (Docker present and usable, and so on)?

A device that is reachable can still be ineligible, and the UI tells you why. You cannot
remove a device that instances are placed on; the removal dialog lists them.

> **Current limitation:** a registered SSH device with Docker can run a container-backed
> provider, and local consumers are routed to its published address without application
> changes. Remote consumers, remote routers, and cross-device sidecars are not supported yet.
> Selecting an unsupported placement is a validation error rather than a silent fallback —
> deliberately, so a placement field is never accepted and then ignored. See
> [DEVIATION.md](DEVIATION.md) §4.

### Starting instances

From a deployment, use the command row: **Validate**, **Plan**, **Up**, **Status**, **Logs**,
**Open**, **Down**, **Cleanup**.

- **Up** warns first if any source worktree is dirty, and asks you to acknowledge.
- **Down** and **Cleanup** require you to type the deployment name to confirm.
- Everything becomes an operation in the **operations** view, which is durable — it survives a
  browser reload and shows operations started by the CLI and TUI too. Destructive operations
  are flagged in the timeline.

The **events & logs** drawer at the bottom filters by deployment and by free text across
instance, service, operation label, and output lines, with a copy-as-plain-text action over
the filtered set.

## Step 8 — Service groups

A **service group** is a named, reusable set of instances that belong together. It is just a
list:

```yaml
groups:
  ai-main:
    instances: [ai-main-ingest, ai-main-analysis, ai-main-reports]

  ai-feature:
    extends: ai-main
    instances: [ai-feature-analysis]
```

Nothing classifies the members. Instances are all the same kind of thing — one checkout run
through one startup profile — so a group does not sort them into backends and databases. It
names which ones are in this combination, and that is all.

Switchyard works out the rest. Every startup profile declares what it `provides` (its
capabilities) and what it `consumes` (its slots). When a consumer needs its `database` slot
filled, Switchyard looks through the group for the member that provides `database`. You do
not restate a relationship the profiles already declare.

`extends:` inherits a group and overrides only what differs. Above, `ai-feature` is `ai-main`
with a different analysis instance. The override is matched by capability: `ai-feature-analysis`
provides the same capability as `ai-main-analysis`, so it replaces it rather than joining it.

`instances:` is always a list. There is no second form, because there is nothing a mapping
would resolve — see below.

### Address collisions, and who wins

A group is a shared address space. Membership is not policed by capability: nothing stops two
members providing the same one, because a capability name is a label for wiring a consumer's
slot to a provider, not a category that decides who may belong.

What matters is whether two members would answer at the same address. When a consumer's slot
has more than one candidate in the group, Switchyard **warns and routes to the first candidate
in the list**:

```text
warning: `database` slot on backend-1 has two candidates in group `dual-write`:
db-main and db-replica; routing to db-main, the first listed
```

Order in `instances:` is therefore meaningful when — and only when — there is a collision.
Nothing is rejected, because the topology still runs and the warning tells you what was
chosen. Reorder the list to change the winner, or remove the member you did not mean to
include.

### Temporarily disabling a member

Use `disabled:` to exclude an instance from one group without stopping it or losing its
priority position:

```yaml
groups:
  feature-test:
    instances: [ui-1, backend-1, backend-canary, db-new]
    disabled: [backend-canary]
```

The group ignores disabled members for routing, health, address resolution, and collision
warnings. Removing the name from `disabled:` restores the member at its original position.
Other groups using a receiver-only instance are unaffected. A disabled sender does not count
as belonging to the group. `disabled:` is local to the named group and is not inherited
through `extends:`. Naming an instance that is not a resolved member is a validation error.

This matters most for capabilities nothing inside the deployment consumes. Two UIs in one
group have no consumer slot pointing at either, so they never collide; each is reachable by
its own address (step 10). Constraining that would rule out an ordinary comparison — the same
backend and database, two UI branches — for no benefit.

**An app that genuinely talks to two databases already calls two addresses** — `:5432` and
`:5433`, or two hostnames — so its profile declares two slots with distinct names, and each is
filled by its own provider. That needs no disambiguation and produces no warning:

```yaml
groups:
  dual-write:
    instances: [backend-1, db-primary, db-replica]   # provide `primary` and `replica`
```

Switchyard cannot route a call the application never makes. If your code only ever opens
`localhost:5432`, it has one database channel, and no amount of configuration creates a second
one.

A real address conflict — one instance declaring two slots at the same `127.0.0.1:5432` — is
still a listener conflict and still fails planning. That is a different thing from two members
of a group offering the same capability, and it remains an error.

Groups are authored in the deployment definition. The Web UI resolves and displays them — the
patch bay shows each group with its members — and both connection views only offer you groups
that provide every capability a given consumer requires.

A group may also carry its own custom local address, so opening one name gives you that whole
combination. That is step 10.

## Step 9 — Connections: point each consumer at a group

A **connection** (internally a *binding*) points one consumer instance at one service group.
This is the switch you flip when you want to test a different combination.

The UI shows one of two views depending on whether the deployment is running, and it labels
which one you are looking at so you never mistake desired state for observed state:

- **Stopped → Desired connections (authored state).** A table of every consumer with consumed
  slots, including consumers that have never been bound. Pick a group per consumer and save;
  the change takes effect on the next **Up**. This is how a freshly authored deployment gets
  its first connection without editing YAML.
- **Running → the live patch bay.** Consumers, provider groups, and the routes actually in
  effect, rendered from the applied snapshot.

Changing a live connection is a deliberate, previewed transition:

1. Choose a compatible group. Incompatible groups are omitted and counted, not silently dropped.
2. A **preview** shows the complete old-provider → new-provider table per slot. If there was no
   previous group, the old column says so explicitly instead of showing blanks.
3. Choose what happens to existing connections: **Close**, **Drain** (with a timeout), or **Pin**.
4. Apply. The whole route table is replaced as one operation — partial group application is
   invalid by design.
5. A **switch report** afterwards says whether it succeeded, and the routes view shows desired
   versus observed version, transition state, previous version, and rollback history. A failed
   switch is diagnosable from the browser.

Applications are not restarted and nothing is rebuilt. Only the router sidecar reloads.

**What is automatic and what is not.** Once slots and addresses are declared, switching is
genuinely magic — the running application never finds out. But the declarations themselves are
authored: a profile must state what it `provides` and what it `consumes`, and each consumed slot
declares the fixed address the unchanged application already calls
(`address: { host: 127.0.0.1, port: 8001 }`). Without that, there is nothing for the router to
intercept. Mismatched connections are rejected rather than guessed at. See
[DEVIATION.md](DEVIATION.md) §2.

## Step 10 — Addresses: open a group or an instance by name

Anything addressable carries an `address:`. There is one rule, not a separate mechanism per
kind: a group has an address, an instance has an address, and the field sits on the thing it
names.

### Group address — "open this combination"

A group carries its own address. Opening it gives you that whole combination, which is the
"open a group by its address" idea from [ABOUT.md](ABOUT.md):

```yaml
groups:
  feature-test:
    address: feature-test.comparison.localhost
    instances: [ui-1, backend-1, db-new]

  regression:
    address: regression.comparison.localhost
    instances: [ui-2, backend-2, db-new]
```

The address belongs to the group, so it names the combination rather than one part of it. Two
fields, and `instances` is the same member list from step 8 — nothing extra to keep in sync.

No member is the entry point. The address reaches the group, and which member answers is a
routing decision like any other: by subdomain
(`backend.feature-test.comparison.localhost`), by path, or by whichever slot the requester
asks for. That matches how the rest of Switchyard works — a consumer asks for the capability
it wants and the group answers; nothing is designated the front door.

Opening the bare name in a browser sends one request, so it needs a default. Switchyard
resolves it to the member providing the UI capability. If a group has no UI member, or more
than one, the bare name is an error listing what it could have meant rather than a guess.

`feature-test` and `regression` above differ only in the backend. That is exactly the
comparison you want when working out whether a bug is in the UI or the backend, and it is one
click apart in the browser.

### Instance address — "show me this one instance"

An instance can carry an address too, declared where the instance is:

```yaml
instances:
  - { name: ui-a, block: react-ui, source: monorepo, address: ui-a.comparison.localhost }
  - { name: backend-1, block: java-backend, source: monorepo-main }
```

Opening it reaches that instance, and whatever that instance is connected to (step 9) is what
it talks to. Use it when you want a memorable name for one UI rather than a named topology.

Declaring the address on the instance means it cannot dangle: delete the instance and its
address goes with it, with no separate block holding a reference to something that no longer
exists. Instances without an address are simply not reachable by name — that is the default,
and most instances stay that way.

`.localhost` is the safe default for both kinds; LAN exposure is optional and off by default.

### The one backend, one group rule

One rule constrains how combinations may overlap:

**Two groups cannot route through one backend instance to different downstream groups.** If
`feature-test` reaches `backend-1` expecting it to talk to `feature-services`, and another
group reaches the same `backend-1` expecting `main-services`, planning fails and tells you to
duplicate the backend instance — the copies may point at the same source. One backend process
cannot infer per-request downstream context, so Switchyard refuses rather than picking one.

In practice: if two combinations differ *below* the backend, they need two backend instances,
not one backend and two addresses. A group's own binding is checked for the same agreement,
so an address can never quietly contradict the connection you authored in step 9.

In the Web UI, group and instance addresses and `managedProfiles` are edited through the
**Routing** panel's definition editor. Every save validates the complete deployment first, so
the invariant above surfaces before anything runs, and you can chain a Plan or a Plan-and-Up
onto the save.

> **Not built this way yet.** Neither field exists in this shape. What is implemented is
> `uiRoutes` (`crates/switchyard-planner/src/model.rs:316`): entries keyed by UI carrying
> `origin`, `backend`, and `downstreamGroup`. That is a group address in all but location — it
> names the combination and the planner enforces the rule above against it
> (`lib.rs:1216-1287`) — but it hangs off the UI rather than the group, and its domain is
> declared separately as a `custom_domain` destination on a `hostRouter` listener, so one
> address means editing two places that must agree. Instance-level addresses are described in
> `DESIGN.md` as a separate `ingress:` block and are not implemented anywhere in the crates;
> the shape above drops that block and puts `address:` on the instance instead.
>
> Reaching **any** member of a group by one address is the largest piece of new work here.
> Today a `custom_domain` destination maps to exactly one provider, so serving a whole group
> from one name means the host router resolving a member per request — router work in
> `router-pingora`, not only a schema change. It also needs checking against browser identity
> below: an `Origin` of `feature-test.comparison.localhost` is what currently identifies which
> combination a request belongs to, so a domain that reaches several members must still
> identify the group unambiguously. See [DEVIATION.md](DEVIATION.md) §1a.

### Reaching a specific instance from the browser

This is the one place where you have to do something Switchyard-specific. Every other segment
gets its own network namespace and its own `127.0.0.1`, so two instances can bind the same port
with no collision. The browser cannot: it lives on your host and has exactly one shared
`localhost`.

So a browser request needs an explicit identity, by one of three means:

1. A per-tab `X-Switchyard-Route` header from the Chromium extension.
2. A distinct `Origin`, which you get for free by opening the instance's custom domain.
3. A managed Chromium profile launched with `switchyard open`.

A browser request with none of these is **rejected**, not routed to whichever backend happens to
be available. That is intentional: silently picking a backend would make an experiment
untrustworthy. See [docs/browser-routing.md](docs/browser-routing.md).

---

## What you cannot do from the browser

Stated plainly so their absence reads as a decision:

- **Edit or create a startup profile** — blocked in every client, not just the web one.
- **Create or edit run actions** — they are one line of shell in one file; edit it directly.
  The browser lists and runs them.
- **Switch between multiple projects** — intended, but not built. No client has a project
  registry yet and the service binds one workspace, so one browser session is one project
  today. See step 3.
- **Hand off to a terminal** — the browser collects what it needs through its own UI instead.

---

## Glossary

Terms you will meet in the UI, the docs, and the YAML. Where the UI label and the internal
field name differ, both are given — the persisted YAML keeps the internal name.

| Term | Also called | What it means |
| --- | --- | --- |
| **Daemon** | project service, background service | The long-lived process that owns project state, runs operations, and serves the API. It is what Switchyard *is*; the Web UI and TUI are clients of it. Expected to be running before you open any UI. |
| **Switchyard project** | workspace, project directory | The folder holding your authored deployment, overlays, and project-local state under `.switchyard/`. Created by `switchyard init` or `switchyard project register`. |
| **Source** | code | Code made available to Switchyard: a local path, a cloned repository, or a worktree. |
| **Managed source** | worktree | A directory Switchyard created (a Git worktree). Switchyard may delete it. |
| **Unmanaged source** | registered path | A directory you already had. Switchyard only remembers where it is; removing it forgets the registration and leaves files untouched. |
| **Repository** | — | A Git repository and its relationship to the worktrees linked from it. |
| **Checkout** | source path | The exact code tree an instance runs from — usually one worktree, i.e. one branch. |
| **Worktree** | — | A Git feature giving one repository several checked-out branches in separate directories at once. This is what makes several branches alive simultaneously. |
| **Startup profile** | **block** | A reusable definition of how a part of the project starts. Expands into one service or a coordinated suite. `block` is the YAML field name. |
| **Project-local profile** | — | A startup profile stored in the Switchyard project, shared by every instance. |
| **Source-local profile** | — | A startup profile stored inside a source (the Git worktree), so it travels with the branch. Untrusted until its manifest is reviewed. |
| **Trust / manifest review** | — | The gate on source-local profiles. Reading a profile's manifest marks it trusted; changed content requires review again. |
| **Instance** | — | One checkout + one startup profile + its parameters + its device. The thing that actually runs. |
| **Segment** | — | ABOUT.md's informal word for "a part of the project" (the UI, the backend, the database). In the implementation this is a startup profile, and "an instance of a segment" is an instance. |
| **Service** | — | One concrete process or container that a startup profile expands into. One instance may produce several. |
| **Deployment** | — | The whole authored topology: sources, instances, parameters, groups, connections, addresses, and route overrides. |
| **Overlay** | — | A YAML file layered onto the deployment to vary it (`overlays/dev.yaml`) without duplicating the whole definition. |
| **Provides / capability** | `provides` | What a startup profile offers to others, e.g. `database`, `ai-ingest`. |
| **Consumes / slot** | `consumes` | A dependency a startup profile needs, named by capability, with the fixed address the unchanged application already calls. |
| **Address (slot)** | `address` | The host and port the unchanged application already uses, e.g. `127.0.0.1:8001`. It is the consumer's contract, not the provider's real location. |
| **Service group** | **group** | A named, reusable set of instances that belong together, listed in `instances:`. May `extends:` another group, override what differs, or temporarily exclude members with `disabled:`. |
| **Connection** | **binding** | The selected provider group for one consumer instance. Replaced as a complete set, never slot by slot. |
| **Route** | `routes` | A direct per-slot connection authored without a group. The low-level form a binding resolves into. |
| **Transition** | — | What happens to existing network connections during a switch: **Close** (drop them), **Drain** (let them finish, with a timeout), **Pin** (keep them on the old provider while new ones use the new one). |
| **Desired vs observed** | authored vs runtime | Desired is what you authored; observed is what is actually running. The UI keeps them in separate views and labels which you are looking at. |
| **Group address** | `address` on a group; today `uiRoutes` | A group's own custom local name. Opening it reaches that whole combination; no member is a designated entry point. |
| **Instance address** | `address` on an instance | A stable custom local name for one instance, with no combination implied. Optional; most instances have none. |
| **Host router** | `hostRouter` | The native host process serving custom domains, TLS, and browser-facing traffic, mapping each domain to the instance behind it. |
| **Device** | — | A host that can run instances: the implicit `local`, plus registered SSH hosts. |
| **Reachability** | device status | Whether Switchyard can reach a device over SSH: `never`, `ok`, `unreachable`, `auth-failed`. |
| **Eligibility** | — | Whether a device can actually run instances. Separate from reachability — a device can be reachable and still ineligible. |
| **Run action** | project run script, script | A saved shell command for the project — a lifecycle shortcut or a smoke test — in a flat name-to-command map, like `package.json` `scripts`. Authored in a file; listed and run from the browser. |
| **Operation** | — | Any tracked unit of work (clone, validate, plan, up, bind, down, cleanup), with streamed events and a durable record in the operations timeline. |
| **Router / sidecar** | — | The Rust proxy that makes the fixed addresses resolve to the selected providers. A sidecar shares a consumer's network namespace; a native host router handles custom domains, TLS, and browser traffic. |
| **Block library** | — | The Web UI view listing available execution adapters and their JSON Schemas. Not the same thing as the startup-profile library. |

---

*This document describes the flow as it works today. Where it differs from the intent stated
in [ABOUT.md](ABOUT.md), the difference is called out inline and recorded in
[DEVIATION.md](DEVIATION.md).*
