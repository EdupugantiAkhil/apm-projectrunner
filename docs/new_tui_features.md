it should have the following views

1. Sources (git repo)

- maybe we should call them project and project instances as we need to clone them before running

1. Devices (obtained either form global config or even local project level)
2. Instances (the current implementation feels worng)

- we need to have something like runner scripts that can be diclared in the root folder or in the source project root (list all the runner scripts in the ui )
- we will have the option to pick a runner script and a device and it gets executed
- then we can select the routing logic where we can say which instance of the runner the traffic has to get routed to (ie merge localhost, network traffic in that group)

the ui should be very very intutive and beginer friendly and the ai skill in the init should have enougph info to auto configure them on top of tui having the ability to configure everything

i am going to state the main goal of this project again
have multiple instances of the same project running(maybe even parts of it) and we can just select wher the traffic can get routed to
say we have 1 node js service that interacts with 5 python services(all 5 are in the same group) we should be able to deploy different instance of that node service to be able to test core functionality

---

# TUI control-plane implementation plan

Status: accepted — Phase A in progress

The handwritten requirements above are intentionally preserved verbatim. This plan
translates their intent into Switchyard's existing architecture.

## 0. Decision: keep the Ratatui TUI

The handwritten note proposed switching to egui. After review, the plan keeps the
existing Ratatui terminal UI and extends it instead, because:

- The TUI must remain usable over plain SSH to headless LAN devices, which is a real
  Switchyard workflow today. egui has no terminal backend, so a native GUI would end
  that workflow rather than improve it.
- The recently landed clone-authentication work (terminal handoff to git and ephemeral
  SSH credential prompts) depends on owning a real terminal. A native app would need an
  entirely new credential-prompt subsystem to preserve the same no-secret-storage
  guarantees.
- The current TUI is small (~3,300 lines) with working Sources, Devices, and Instances
  views. Extending it to the missing workflows is far cheaper than rebuilding parity in
  a new toolkit while carrying three clients.

If a native desktop client is wanted later, it becomes its own milestone with its own
design review; nothing in this plan may make that harder, but nothing waits on it.

## 1. Product outcome

Extend the TUI so a developer can:

1. Add or clone code repositories and choose a checkout or worktree.
2. Discover, create, inspect, and select a reusable way to run that code.
3. Create multiple independently named instances from the same or different checkouts.
4. Validate and start those instances without hand-editing generated Compose files.
5. See which source-backed instances are running and healthy.
6. Connect a consumer instance to a complete compatible provider group.
7. Change that connection atomically without restarting unrelated instances.
8. Pick a startup profile and a device and run it — with `local` first, and a limited
   but real remote-device execution cut in this plan rather than an indefinite later
   milestone (see Phase D).
9. Use an initialized AI skill to inspect a repository and safely prepare the same
   configuration that can be authored through the UI.

The representative acceptance scenario is the one in the handwritten requirements:
run multiple instances of a Node.js consumer and independently choose which complete
five-service Python group each consumer uses. Applications may continue using fixed
loopback or network addresses; Switchyard performs the routing without source changes.

## 2. User-facing terminology

The persisted architecture keeps its existing precise terms. The UI may use friendlier
labels. The table below is the current proposal; the final naming pass is a product
decision to be made explicitly during Phase A, including whether the handwritten
"project / project instance" naming is adopted for any screen.

| UI label | Architecture term | Meaning |
|---|---|---|
| Switchyard project | Workspace/project directory | Authored deployment, overlays, and project state |
| Code | Sources | Code made available from a local path, repository, or worktree |
| Repository | Repository | The Git repository and its relationship to linked worktrees |
| Checkout | Source path/worktree | The exact code tree selected for an instance |
| Startup profile | Block | A reusable definition that expands into one service or a coordinated suite |
| Instance | Instance | One checkout run through one startup profile with its own parameters |
| Service group | Service group | A complete reusable set of compatible providers |
| Connection | Binding/routes | The selected provider group or routes for a consumer instance |
| Run action | Project run script | A project-level Up, Down, Plan, Status, or smoke-test operation |
| Device | Registered device | A known execution host; `local` plus registered SSH hosts |

Do not rename persisted `source`, `block`, `instance`, `group`, or `binding` fields.

## 3. Execution model

### Startup profiles and run actions are different

A startup profile owns the long-running services inside an instance. It must use the
existing block and execution-adapter model so planning, isolation, health, ownership,
routing, recovery, logs, and cleanup continue to work.

A project run action invokes deployment-level commands or a smoke test. Existing
`.switchyard/run-scripts.yaml` entries remain project operations and must not become a
second instance execution format.

This is a deliberate reframing of the handwritten "runner scripts" idea: anything that
participates in routing, health, and cleanup must be a block, not an arbitrary script.
The cost is slightly more friction than "drop a script in the repo" — a source-local
profile is declared in a manifest, not inferred — and that trade is accepted.

The primary creation flow is:

```text
Add code -> choose checkout -> choose startup profile -> configure instance
         -> validate and preview -> choose device -> run -> connect routes
```

### Source-local startup profiles

The TUI must be able to list startup profiles declared in either:

- the Switchyard project; or
- an explicitly supported manifest in the selected source repository.

Source-local discovery must be declarative and deterministic. It must not execute or
infer arbitrary executable files merely because they look like scripts. A discovered
profile is imported or resolved through the existing block and adapter contracts before
it can run. The format and precedence rules must be added to `DESIGN.md` before this
feature is implemented.

### Devices and placement

Devices are part of the core interaction in the handwritten requirements ("pick a
runner script and a device and it gets executed"), so this plan treats remote execution
as an in-scope milestone with a deliberately limited first cut, not an indefinite
deferral:

- `local` remains the default and always-available execution device.
- Registered SSH devices can be added, checked, inspected, and removed (already
  supported).
- Phase D delivers a **limited remote cut**: container-backed instances started over
  SSH against the remote host's Docker daemon, with the router remaining local and the
  remote services reachable through explicit published addresses. It requires a short
  written design first (Phase A), but the design is scoped to this cut only.
- Anything beyond the limited cut (remote routers, cross-device sidecar routing, remote
  process-adapter instances, drift-tracked remote checkouts) stays a later milestone.
- The UI must always show true placement. A device selection that the runtime cannot
  honor is a validation error, never a silently ignored field.

## 4. Client and architecture direction

- The Ratatui TUI is Switchyard's primary local interactive control plane. `switchyard
  tui [project]` keeps its name.
- Extract application operations and state projection from the view/widget code where
  they are currently entangled, so the TUI, CLI, and daemon share one operations layer.
  This refactor is incremental — extract what each new workflow touches, not a
  big-bang rewrite.
- Use the daemon/API for durable operations where the existing contract supports them.
  Standalone project startup and reconciliation must remain possible and well defined.
- **Decide the React GUI's status in Phase A**, before new workflows land: either it is
  a supported secondary client that must receive the new workflows on a stated
  schedule, or it enters a documented deprecation path. Building every new workflow
  twice by default is not an option.

Changes to supported clients, command naming, or packaging require corresponding
updates to `DESIGN.md`, `IMPLEMENTATION_PLAN.md`, release documentation, and the
support policy.

## 5. Information architecture

### Home

- Show a guided first-run checklist.
- Explain the difference between code, startup profiles, instances, and connections.
- Provide one obvious next action and show validation problems in plain language.
- Summarize running instances, unhealthy services, and unapplied changes without
  turning the screen into a generic statistics dashboard.

### Code (existing Sources view, extended)

- Display repositories as parents with linked checkouts and worktrees beneath them.
- Show path, branch/ref, commit, dirty state, ownership, and availability.
- Register an existing local directory, clone a repository, refresh inspection, create
  a managed worktree, open a directory, and safely remove managed entries.
- Preserve the existing terminal-handoff and ephemeral-credential behavior for Git and
  SSH; never collect passwords or private-key material into Switchyard state.

### Startup profiles (new view)

- List project-local and source-local profiles with their origin clearly identified.
- Show expanded services, execution adapter, command, working directory, mounts,
  capabilities, consumed slots, probes, parameters, lifecycle, and trust status.
- Validate a profile against a selected checkout without starting it.
- Render adapter configuration from JSON Schema instead of hard-coded
  language-specific forms.
- Provide a guided editor capable of configuring the supported execution adapters.

### Instances (existing view, reworked)

- Create an instance by selecting a checkout, startup profile, device, and name, and
  supplying schema-approved parameters.
- Show authored instances before their first run and merge them with observed runtime
  services after apply.
- Display exact source identity, service expansion, placement, state, health,
  resources, active connections, and recent operations.
- Support validate, plan, start, stop while preserving volumes, logs, and safe cleanup.
- Show true placement always; a device the runtime cannot honor fails validation.

### Connections (new view)

- Use the existing typed capabilities, slots, service groups, routes, and bindings.
- Present a route matrix: consumers as rows, their slots and selected provider groups
  as columns. A terminal table is the primary representation, not a fallback.
- List only compatible provider groups for a consumer.
- Preview every old and new provider before applying a complete change.
- Apply a group switch through one validated atomic binding operation.
- Explain fixed `localhost` and network-address interception in user language while
  retaining the router's sidecar/gateway implementation model.
- Never silently connect to an arbitrary available provider.

### Devices (existing view, extended)

- Show `local` and registered SSH devices with connectivity status.
- If global device configuration is added, show origin and effective precedence and
  prevent accidental mutation of the wrong scope.
- After Phase D, show which devices are eligible for the limited remote cut (Docker
  reachable over SSH) and why an ineligible device is ineligible.

### Operations and logs

- Keep project run actions here rather than presenting them as startup profiles.
- Show validation, planning, build, start, readiness, route change, stop, and cleanup
  in one ordered timeline.
- Stream output without blocking the UI and retain actionable exit information.
- Filter logs by deployment, instance, and expanded service.
- Make destructive cleanup visibly different from a normal stop.

## 6. Beginner-friendly interaction requirements

- Empty states teach the next step instead of merely saying that no data exists.
- Forms reveal advanced options progressively.
- Every mutation shows what will change before it happens.
- Validation runs before runtime mutation and attaches errors to the relevant field or
  topology element.
- Use names and explanatory text in addition to colors; do not rely on color alone.
- All workflows are keyboard-driven with discoverable bindings and a help overlay.
- Preserve source changes; never reset, discard, or destructively rewrite a checkout.
- Preserve volumes during ordinary stop operations.
- Never store secrets in deployment YAML, SQLite, logs, or generated previews.
- Errors include the failed action, affected instance/service, concrete reason, and a
  suggested recovery step where one is known.

## 7. Initialized AI skill

`switchyard init` should install a project-local skill that can help a user reach the
same valid desired state as the UI. Expand the skill so an agent knows how to:

1. Inspect the project deployment, overlays, registered sources, and repository layout.
2. Identify existing Dockerfiles, safe startup commands, Compose or Process Compose
   definitions, required variables, fixed ports, health endpoints, and dependencies.
3. Propose or author startup profiles through supported adapters without inventing
   language-specific product concepts.
4. Model a coordinated multi-service suite as one reusable profile where appropriate.
5. Declare provided capabilities, consumed slots, compatible service groups, and
   complete bindings.
6. Validate and plan every authored change before running it.
7. Explain unsafe or unsupported cases rather than bypassing trust, ownership, routing,
   or secret-handling rules — and when a repository cannot be safely configured, say
   so explicitly with the concrete blockers instead of producing a best guess.
8. Use normal stop by default and perform destructive cleanup only with explicit user
   intent.

The skill should include concise examples and point to the authoritative local schemas
and documentation. It must not edit `.switchyard/generated`, embed credentials, or
silently execute newly discovered repository scripts.

## 8. Delivery phases

Each phase ships user-visible value on the existing TUI; there is no big-bang parity
gate because the existing client is retained.

### Phase A — architecture and contracts

- [ ] Update `DESIGN.md`: retained-TUI decision, shared operations layer, and — since
      devices are currently implemented but undocumented — a retroactive device model
      section (registration, scope, connectivity checks, placement rules).
- [ ] Specify the source-local startup-profile manifest format, discovery boundaries,
      precedence, and trust behavior.
- [ ] Write the scoped design for the Phase D limited remote cut (SSH + remote Docker,
      local router, explicit published addresses; ownership, cleanup, and failure
      behavior).
- [ ] Decide and document the React GUI's status: supported secondary client with a
      workflow-parity schedule, or deprecation path.
- [ ] Make the final user-facing terminology pass (including the "project / project
      instance" question from the handwritten notes).
- [ ] Add this work to `IMPLEMENTATION_PLAN.md` without reopening already verified
      routing milestones.

Exit gate: the manifest format, device model, remote-cut design, client policy, and
naming are documented and reviewed.

### Phase B — guided configuration on the existing TUI

- [ ] Implement the Home first-run workflow.
- [ ] Implement the startup-profile library view and schema-driven inspector/editor.
- [ ] Implement explicit source-local profile discovery and import.
- [ ] Implement checkout + startup profile + parameters + device instance creation
      (device choice limited to `local` until Phase D, with honest labeling).
- [ ] Continuously validate drafts and preview expanded services and resources.
- [ ] Keep project run actions discoverable under Operations.
- [ ] Extract the operations/state-projection layer for each workflow this phase
      touches.
- [ ] Add component/state tests and pty-driven smoke coverage for the new views.

Exit gate: a new user can configure and start a supported repository through the TUI
without manually editing generated files.

### Phase C — routing workflow

- [ ] Implement the Connections route matrix.
- [ ] Implement compatible-group selection and complete old/new route preview.
- [ ] Apply route changes through the existing atomic binding operation.
- [ ] Surface route version, transition state, failures, and rollback information.
- [ ] Verify the Node.js consumer and interchangeable five-service Python-group
      scenario end to end from the TUI.

Exit gate: multiple consumer instances can independently select complete compatible
provider groups from the TUI, and switching one consumer does not restart unrelated
instances.

### Phase D — limited remote device execution

Entry gate: the Phase A remote-cut design is approved.

- [ ] Implement SSH + remote-Docker execution for container-backed instances, with the
      router local and remote services reachable via explicit published addresses.
- [ ] Implement remote ownership labeling, lifecycle, health, logs, and cleanup for
      the cut.
- [ ] Validate device eligibility (SSH reachable, Docker reachable, resource claims)
      before start; ineligible selections fail validation with the concrete reason.
- [ ] Expose device placement in instance creation and show true placement everywhere.
- [ ] Verify end to end against a real LAN device.

Exit gate: selecting an eligible remote device causes real, observable, recoverable
execution on that device; everything outside the cut is clearly labeled unsupported.

### Phase E — AI skill and release integration

- [ ] Expand and validate the initialized AI skill per section 7, including the
      explicit cannot-safely-configure failure mode.
- [ ] Apply the approved React GUI decision from Phase A.
- [ ] Update development, TUI, release, upgrade, and support docs.
- [ ] Run workspace tests, Clippy `-D warnings`, formatting, documentation checks,
      pty-driven TUI smoke tests, routing fixtures, and release assembly verification.
- [ ] Update `PROGRESS.md` and `AGENTMISTAKES.md` with implementation and verification
      results as each increment lands.
- [ ] Commit reviewed phase-sized increments throughout.

Exit gate: one clearly documented primary interactive experience ships with full local
workflow coverage, the limited remote cut, and no ambiguous duplicate product model.

## 9. End-to-end acceptance criteria

- [ ] A user can register or clone a repository and select an exact checkout/worktree.
- [ ] The UI lists eligible startup profiles from approved project and source
      manifests.
- [ ] A user can create two instances of the same startup profile from different or
      identical checkouts without naming, port, network, or volume collisions.
- [ ] A coordinated five-service profile duplicates as a complete suite.
- [ ] A profile can expand a subset of a project's services ("parts of it"), and such
      partial instances participate in groups and routing like any other.
- [ ] A Node.js consumer can continue using its fixed addresses while Switchyard
      routes them to the selected Python provider group.
- [ ] Two Node.js consumer instances can independently select different Python groups.
- [ ] Switching one consumer replaces its complete route table atomically and does not
      restart either consumer or unrelated providers.
- [ ] Exact checkout commit/dirty state, health, logs, resources, and active
      connections are visible for every instance.
- [ ] Selecting an eligible remote device runs a container-backed instance on that
      device with visible ownership, logs, health, and working cleanup; ineligible or
      out-of-cut selections fail validation with a concrete reason.
- [ ] Normal stop preserves persistent data; destructive cleanup requires explicit
      confirmation and ownership proof.
- [ ] The UI and initialized AI skill produce definitions accepted by the same
      validator and planner used by the CLI and daemon.
- [ ] The AI skill, when a repository cannot be safely configured, reports the
      concrete blockers instead of authoring a best-guess configuration.
- [ ] All TUI workflows, including the new views, remain fully usable over SSH.
