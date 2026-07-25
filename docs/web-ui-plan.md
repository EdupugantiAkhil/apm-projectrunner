# Web UI implementation plan

Work to bring the React Web UI (`packages/web`) to the point where a project can be set
up, authored, and operated without dropping to the TUI.

This is **not** a port of the TUI. The TUI (`crates/switchyard-tui`) was surveyed to find
capabilities worth having in the browser, but several of its workflows exist because a
terminal shows one pane at a time or because a TTY is available — those are re-scoped or
dropped here rather than reproduced. See "Deliberate divergences" below.

## Status

**Complete.** Parts 1 through 13 shipped, plus follow-up Parts 11a–11c, each verified and
committed separately per `CLAUDE.md`. Per-part detail is in `PROGRESS.md`; that file, not
the checkboxes here, is the authoritative record.

One item is deliberately not a checkbox: **Part 9's shell-authoring item** was never in
scope, so it is recorded there as "Not built" rather than as an unticked box, which would
imply unfinished work. See "Not in scope".

Part 13's security review was completed after that part had already merged, and found two
real issues that are now fixed. Reviewing before merge, as the plan asked, would have
caught them first.

Follow-ups noted during implementation but out of this plan's scope:

- Part 12's Home signal loader issues three requests per deployment, one of which
  (`validateDeployment`) runs the planner, and it re-fires after every command. Correct,
  but costly on a project with many deployments.
- Startup profiles are blocks, so Part 11c joins the profile library by exact
  `(name, deployment)`. A block with no library entry is marked unlisted rather than given
  inferred origin or trust.

## What this plan is not

An earlier survey framed this work as a frontend backlog. Most of the highest-value items
are in fact **backend work**: `crates/switchyard-daemon/src/server.rs:1277-1307` is the
complete control-plane route table, and it contains no profile, run-action, device
eligibility, or operation-list endpoints. The TUI reaches `switchyard-ops`
(`profiles.rs`, `run_scripts.rs`, `devices.rs`) in-process; the browser cannot.

Each part below therefore states its API prerequisite explicitly. A part whose API does
not exist yet cannot be started as a frontend task.

## Deliberate divergences from the TUI

Three TUI workflows are re-scoped rather than reproduced:

- **Terminal handoff is out of scope.** The TUI re-exec mechanism
  (`crates/switchyard-tui/src/handoff.rs:65-111`) is an implementation detail of a
  terminal client, not a capability to reproduce. The browser collects credentials
  through its own UI.
- **The five-step instance wizard becomes one progressive form.** The linear modal
  exists because a TUI shows one pane at a time. `docs/gui.md:25-32` already commits to
  extending the schema-driven dashboard rather than introducing a dashboard-only model.
- **Shell run-action authoring stays out of the browser.** Listing and running are in
  scope; create/edit of shell actions is not. See Part 9.

## Sequencing constraints

- **Profile save is blocked in every client.** The shared operations layer does not
  expose the profile mutation (`crates/switchyard-tui/src/tabs/profiles.rs:511-565`), so
  the TUI's schema-generated editor produces a preview only. The web profile *library*
  (Part 3) can ship without it; a web profile *editor* cannot, in either client. Treat
  the ops-layer mutation as a separate prerequisite, not part of this plan.
- Part 4 (instance authoring) depends on Part 3 (profile library) and Part 5 (device
  eligibility) for its inputs.
- Part 7 (initial bindings) is independent of Parts 3-5 and can run in parallel.

---

## Part 1 — Unmanaged source deregistration

**API status:** exists. `DELETE /api/v1/sources/{name}` is already routed at
`crates/switchyard-daemon/src/server.rs:1299`.

Frontend-only, under an hour.

- [x] Add `deregisterSource(name)` to `packages/web/src/api.ts` (`registerSource` at
      `api.ts:153` is the model; note this is *not* `removeWorktree` at `api.ts:157`).
- [x] Render Remove for unmanaged sources at `packages/web/src/App.tsx:164`, which
      currently gates on `source.source.kind === 'managed'`.
- [x] Use distinct confirmation copy per kind: managed says the directory is deleted,
      unmanaged says only the registration is forgotten and files are untouched.
- [x] Test: unmanaged Remove calls `deregisterSource`, managed still calls
      `removeWorktree` with its dirty-state guard.

**Done when:** an unmanaged source can be deregistered from the browser with its
directory intact.

---

## Part 2 — Operations list endpoint

**API status:** missing. Only `GET /api/v1/operations/{id}` exists
(`server.rs:1281`), which is why `App.tsx:195-196` can show only operations started in
the current browser session.

This one endpoint unblocks three separate gap-document items (§9 timeline, §11 filtering,
§12 per-instance recent operations). Build it before any of them.

- [x] Add `GET /api/v1/operations` returning durable operation records from SQLite,
      with query filters for deployment, instance, kind, and status, plus a cursor.
- [x] Include a destructive-operation marker on each record (`cleanup`, `down`) so the
      timeline can flag them without the client hardcoding a kind list.
- [x] Document the endpoint in `docs/control-plane-api.md` alongside the existing
      operations rows.
- [x] Type the response in `packages/web/src/api.ts` and add a client method.
- [x] Replace the session-only Operations view (`App.tsx:195-196`) with the durable list.

**Done when:** the Operations view survives a browser reload and shows operations started
by the CLI and TUI.

---

## Part 3 — Startup-profile library and trust workflow

**API status:** missing entirely. `switchyard-ops/src/profiles.rs` is reachable only
in-process. The Block Library (`DeploymentBuilder.tsx:30-31`) lists adapter declarations
and their JSON Schemas, which is a different thing.

- [x] Add profile endpoints: list discovered project-local and source-local profiles with
      origin and trust state; read one profile's expanded definition; validate a profile
      against a named checkout; import (and re-import) after manifest review; remove an
      imported profile.
- [x] Mirror the TUI's trust semantics exactly — a source-local profile is untrusted
      until its manifest is reviewed, and changed content requires review again
      (`crates/switchyard-tui/src/tabs/profiles.rs:75-103`).
- [x] Document the endpoints in `docs/control-plane-api.md`.
- [x] Build a Profiles view: origin and trust badges, manifest review before import,
      re-import on change, remove-imported, and validate-against-checkout with the
      expansion report (`profiles.rs:427-485`).
- [x] Do **not** build a profile editor. Blocked on the ops-layer mutation — see
      Sequencing constraints.

**Done when:** a user can discover, review, import, validate, and remove startup profiles
in the browser without touching the TUI.

---

## Part 4 — Device eligibility and placement visibility

**API status:** partial. `/api/v1/devices` exists (`server.rs:1300-1302`) but carries no
eligibility field; `DeviceStatus` in `packages/web/src/api.ts:84` is only
`never | ok | unreachable | auth-failed`, which conflates SSH reachability with runtime
eligibility.

- [x] Extend the device payload with an eligibility verdict and its reason, kept separate
      from SSH check status (`crates/switchyard-tui/src/tabs/devices.rs:194-236`).
- [x] Include the implicit `local` device in the listing, or document that clients
      synthesize it — pick one and apply it consistently.
- [x] Return the instances currently placed on a device so removal can be placement-aware
      (`devices.rs:66-102`).
- [x] Widen the `DeviceStatus` type and render reachability and eligibility as two
      distinct columns in `DevicesView`.
- [x] Show placed instances in the device removal dialog and block removal when non-empty.
- [x] Surface authored and observed placement on instance cards
      (`App.tsx:137-144`; TUI reference `instances.rs:386-490`).

**Done when:** a user can tell why a reachable device is ineligible, and cannot remove a
device that instances are placed on.

---

## Part 5 — Guided instance authoring

**API status:** depends on Parts 3 and 4.

Re-scoped: keep the five inputs from the TUI wizard, drop the five-step modal. One
progressively-revealed form with live validation, consistent with the schema-driven
dashboard the GUI already uses (`SchemaForm.tsx`).

The current builder (`DeploymentBuilder.tsx:7-21`) creates a block directly from an
execution adapter, and the generated instance has empty parameters and no device
placement (`DeploymentBuilder.tsx:24-27`).

- [x] Add "Add instance to existing deployment" as an entry point — today the builder
      only creates whole deployments.
- [x] Checkout/worktree selector, filtered to registered sources.
- [x] Trusted-profile selector, filtered to profiles valid for the chosen checkout
      (`crates/switchyard-tui/src/dialogs/wizard.rs:101-139`).
- [x] Device selector restricted to eligible devices, showing the ineligibility reason
      inline for the rest.
- [x] Render profile-defined parameters through `SchemaForm`, not free-text.
- [x] Live expansion preview — services, ports, volumes — before the instance is
      appended (`wizard.rs:327-395`).
- [x] Field-level errors for profile, device, and parameter problems, not one global
      validation banner.

**Done when:** an instance with a profile, parameters, and device placement can be
authored without hand-editing YAML.

---

## Part 6 — Initial connection authoring for unbound consumers

**API status:** exists. The bind command is routed via `/api/v1/commands/{kind}`
(`server.rs:1280`) and `DeploymentWorkspace.tsx` already drives it.

Highest value-per-line item in the plan. The group selector at
`DeploymentWorkspace.tsx:36` renders only when an existing binding is truthy, so an
authored consumer with required slots but no initial binding has no graphical path.

- [x] Derive consumers with consumed slots from the authored definition, including those
      with no binding (`crates/switchyard-tui/src/tabs/connections.rs:162-223`).
- [x] Render unbound consumers as such rather than omitting them.
- [x] Allow selecting a compatible complete group for an unbound consumer, reusing the
      existing compatibility filter and preview.
- [x] Reuse the existing `ChangePreview` transition flow; first-bind has no old routes to
      show, so handle the empty old-provider column deliberately.

**Done when:** a freshly authored deployment's consumer can get its first binding without
editing YAML.

---

## Part 7 — Authored connection view while stopped

**API status:** exists — the authored definition is readable via
`/api/v1/deployments/{deployment}/definition` (`server.rs:1293`).

Re-scoped: do **not** restore the live patch bay while stopped. It renders *runtime*
topology from the applied snapshot, and overloading it with a second data source will
make both harder to reason about. `App.tsx:141-145` hides it deliberately.

- [x] Build a separate authored/desired-state connection view driven by the definition.
- [x] Show it in place of the runtime patch bay when stopped, with copy that names which
      of the two the user is looking at.
- [x] Allow editing desired connections offline; they take effect on the next `Up`.
- [x] Keep the existing stopped-state callout and `Run Up` affordance.

**Done when:** connections can be authored before the first `Up` and while stopped,
without the user mistaking desired state for observed state.

---

## Part 8 — Connection transition and rollback details

**API status:** exists. `GET /api/v1/deployments/{deployment}/routes`
(`server.rs:1285`) already returns history; `packages/web/src/api.ts:96-100` types it as
`unknown[]` and nothing renders it.

- [x] Type `RouteState.history` properly against the actual response.
- [x] Show desired versus observed version separately — the active-routes table at
      `App.tsx:143-145` currently collapses them into one column.
- [x] Add explicit transition state and previous version.
- [x] Render rollback history (`crates/switchyard-tui/src/tabs/connections.rs:526-586`).
- [x] Show a post-switch success/failure report after an apply.

**Done when:** a failed route switch is diagnosable from the browser.

---

## Part 9 — Project run actions (partial scope)

**API status:** missing. `switchyard-ops/src/run_scripts.rs` is in-process only.

Re-scoped on risk. A browser page is a much wider attack surface than a terminal, and
`run_scripts.rs:141` writes actions to a project file on disk. **Authoring arbitrary
shell actions from the browser is out of scope.**

In scope for the web:

- [x] Add endpoints to list project run actions and run one, distinguishing structured
      from shell actions.
- [x] Create/edit/delete of **structured** actions.
- [x] Run an existing **shell** action, gated on the existing project-local
      acknowledgement (`run_scripts.rs:150`, `run_scripts.rs:154`).
- [x] Confirmation preview before any execution, matching TUI behavior.

Explicitly out of scope for the web:

- **Not built:** create or edit shell actions from the browser — CLI/TUI only. The UI
  states this where a user would expect the button, so the absence reads as a decision
  (`RunActionsView.tsx`, the `profile-boundary` note).

**Done when:** existing run actions are usable from the browser and the shell-authoring
boundary is visible rather than implied.

---

## Part 10 — Rich operation and log filtering

**API status:** depends on Part 2.

- [x] Free-text filter over deployment, instance, service, operation label, and output
      lines (`crates/switchyard-tui/src/tabs/operations.rs:337-365`).
- [x] Instance and service log filtering in the event drawer, which today filters by
      deployment only (`App.tsx:127-130`).
- [x] Destructive-operation markers in the timeline, from the Part 2 field.
- [x] Keep the existing copy-as-plain-text action working against the filtered set.

**Done when:** a specific service's output can be found without scrolling the whole drawer.

---

## Part 11 — Per-instance inspector

**API status:** depends on Part 2 (recent operations) and Part 4 (placement).

Instance cards (`App.tsx:137-144`) show runtime state, source identity, logs, and Open.
The TUI assembles considerably more (`crates/switchyard-tui/src/tabs/instances.rs:386-493`).

- [x] Startup profile per instance.
- [x] Authored and observed device placement.
- [x] Expanded service inventory grouped under the instance.
- [x] Per-service state, health, and resource placement.
- [x] The instance's active connections in one place.
- [x] Instance-scoped recent operations.
- [x] Fold in the existing node inspector in `DeploymentWorkspace.tsx` rather than
      building a second inspector beside it.

**Done when:** one view answers what an instance is running, where, from what, and how it
is connected.

---

## Part 12 — Project Home and onboarding

**API status:** depends on Parts 3, 4, and 6 for its inputs.

Deliberately last: a checklist is only useful once the actions it links to exist. The TUI
Home tab tracks code registration, profile selection, instance creation, startup, and
route connection, and recommends the next unfinished action
(`crates/switchyard-tui/src/tabs/home.rs:41-56`, `:66-139`, `:186-230`).

- [x] Setup-progress checklist across source, profile, instance, startup, connection.
- [x] Next-recommended-action affordance linking into the relevant view.
- [x] Project-wide problem summary aggregating source, profile, deployment, device,
      operation, and connection problems (`home.rs:186-230`).
- [x] Add Home to the top-level views at `App.tsx:94-121` and make it the default landing
      view for a project with no deployments.

**Done when:** a newly registered empty project gives the user an obvious first step.

---

## Part 13 — Git clone with in-browser credentials

**API status:** missing.

Re-scoped. Terminal handoff and TUI re-exec are not the target. The web UI can register
an existing local directory and create a worktree (`App.tsx:157-166`) but cannot clone.

- [x] Attempt a non-interactive clone first, using ambient credential helpers and SSH
      agent, exactly as the TUI's underlying Git invocation does.
- [x] On auth failure, prompt in-browser and feed the response to Git through a one-shot
      `GIT_ASKPASS` helper. Credentials pass through memory only; nothing is persisted.
      Reuse the existing loopback-only credential posture described in `docs/gui.md`.
- [x] Treat unknown-host-key as an explicit UI approval step showing the fingerprint —
      not a passthrough TTY prompt.
- [x] Stream clone progress as a normal operation so it appears in the timeline.
- [x] Security-review this part specifically before merge; it is the only part that moves
      credential material through the browser. Reviewed after merge rather than before.
      Two findings were confirmed by experiment and fixed — credentials travelling in
      cleartext to a remote `http://` host, and order-dependent SSH host-key pinning. See
      the review entry in `PROGRESS.md`.

**Done when:** a private repository can be cloned and registered from the browser.

---

## Not in scope

- **Multi-project registry and switcher.** Neither client has it today
  (`docs/tui.md:3-9`, `docs/gui.md:10-23`). It is a product feature to design on its own
  terms, not something the Web UI is behind on.
- **Profile editing in any client.** Blocked on the ops-layer mutation.
- **Shell run-action authoring in the browser.** See Part 9.
- **Terminal handoff.** See Part 13.

## Suggested order

Ship in this order; the dependency notes above are the reason, not the numbering.

1. Part 1 — unmanaged deregistration (quick win, no API work)
2. Part 6 — initial connection authoring (high value, API already exists)
3. Part 2 — operations list endpoint (unblocks Parts 10 and 11)
4. Part 3 — profile library
5. Part 4 — device eligibility
6. Part 5 — guided instance authoring (needs 3 and 4)
7. Part 7 — authored connection view
8. Part 8 — transition and rollback details
9. Part 9 — run actions, partial scope
10. Part 10 — filtering (needs 2)
11. Part 11 — per-instance inspector (needs 2 and 4)
12. Part 12 — Home and onboarding (needs 3, 4, 6)
13. Part 13 — clone with in-browser credentials

Per `CLAUDE.md`, each part is its own brief, verified and committed separately before the
next one starts. Record progress in `PROGRESS.md`.
