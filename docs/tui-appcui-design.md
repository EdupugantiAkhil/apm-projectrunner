# Switchyard TUI rewrite on AppCUI-rs — layout and interaction design

Status: accepted for implementation (2026-07-18). Supersedes the "keep Ratatui"
decision in `docs/new_tui_features.md` §0 by explicit product decision: the TUI is
being rewritten on [AppCUI-rs](https://github.com/gdt050579/AppCUI-rs) (`appcui`
crate, pinned `=0.4.13`). Everything else in `docs/new_tui_features.md` — product
outcome, terminology, execution model, beginner-friendly requirements — still
applies and is not restated here.

A full offline copy of the AppCUI repository (docs book under `docs/`, runnable
examples under `examples/`) is available at `.codex-refs/appcui/`. The crate
source is vendored in `~/.cargo/registry/src/*/appcui-0.4.13/`.

## 1. Application shell

- **Single-window app**: `App::new().single_window().command_bar().build()`.
  One fullscreen window (`type: panel`, no close decoration confusion) containing a
  `Tab` control docked to fill the client area.
- **Seven tabs**, in workflow order, each with an Alt-hotkey in the caption:

  | # | Tab caption | Maps to |
  |---|---|---|
  | 1 | `&Home` | guided checklist + project summary |
  | 2 | `&Code` | sources: repositories, checkouts, worktrees |
  | 3 | `&Profiles` | startup profile library |
  | 4 | `&Instances` | instance list + creation wizard |
  | 5 | Co&`nnections` | route matrix |
  | 6 | `&Devices` | local + SSH devices, eligibility |
  | 7 | `&Operations` | run actions, timeline, logs |

- **Navigation**: `Alt+letter` jumps to a tab; `Ctrl+Tab`/`Ctrl+Shift+Tab` cycle.
  Within a tab, standard focus traversal (`Tab`/arrows) between panes.
- **Command bar** (bottom line) is the discoverability surface: it always shows the
  actions valid for the focused pane (e.g. `F5 Refresh | Ins Add | Enter Details |
  F9 Start`). Every action reachable by mouse must have a key shown here.
- **Help**: `F1` opens a modal window with a scrollable keybinding + concept
  reference (uses the `Markdown` control).
- **Quit**: `Esc` on the shell (with confirmation if an operation is running),
  or `Ctrl+Q` anywhere.

### State model

- One `ProjectState` struct owns everything projected from `switchyard-ops`
  (sources, profiles, deployments/instances, devices, route matrix, run scripts).
  It is refreshed as a whole (`F5` and after every mutation) — views never load
  data ad hoc.
- Views receive `&ProjectState` and rebuild their controls from it; they keep only
  selection/scroll state of their own.
- All mutations go through `switchyard-ops`; the TUI contains **no** business
  logic, only orchestration + presentation.

### Long-running work

- Every operation that can block (start/stop/plan/validate, device checks, git
  fetch/inspection) runs on an AppCUI `BackgroundTask<OpUpdate, OpCommand>`
  (see `.codex-refs/appcui/examples/background_task/`). The UI stays responsive;
  output lines stream into the Operations tab; a status chip in the shell shows
  "busy" state. Only one mutating operation runs at a time (same rule as today).

### Terminal handoff (interactive git auth)

AppCUI clears its global singleton when `App::run()` returns, so a new `App` can
be built in the same process. The crate keeps its existing public entry:

```text
pub fn run(project_dir) -> loop {
    outcome = run_app(project_dir)   // build App, run to exit
    match outcome {
        Exit               => break,
        CloneHandoff(req)  => run git on the real terminal (existing
                              execute_interactive_clone semantics, SIGINT-safe),
                              then loop to rebuild the App with fresh state
    }
}
```

The handoff request is passed out of the app via a shared cell written by the Code
tab before it closes the single window. Credential behavior is unchanged: no
secrets ever enter Switchyard state; the terminal is fully restored before git
runs (AppCUI restores it when `run()` returns).

## 2. Per-tab layout

Conventions used by every tab:

- **Master/detail**: a `VSplitter` with the collection on the left (~60%) and a
  read-only detail panel on the right, updating on selection change.
- **Empty states teach**: when a list is empty, the pane shows a short explanation
  of what the thing is and the exact key to create one — never a bare "no data".
- **Forms are modal windows** with `OK`/`Cancel` buttons, field-attached
  validation messages (red label under the offending field, plus text — never
  color alone), and progressive disclosure: an `Advanced ▸` toggle reveals rare
  options.
- **Every mutation previews**: the confirm dialog states exactly what will change
  before anything runs. Destructive cleanup uses a visually distinct dialog
  (warning icon, action verb spelled out, default button = Cancel).

### Home

Vertical stack (single `Panel`):
1. Project header: name, path, deployment count, running/unhealthy counts,
   unapplied-change indicator (words + count, not color only).
2. **First-run checklist** (`ListBox`, non-interactive checkmarks): Register code →
   Pick a startup profile → Create an instance → Start it → Connect routes. Each
   line shows done/todo and the key that jumps to the right tab.
3. **One obvious next action**: a single focused button ("Add your first
   repository (Enter)") computed from the checklist.
4. Problems panel: current validation problems in plain language, each with the
   affected object and the tab that fixes it.

### Code

- Left: `TreeView` — repositories as parents; their checkouts and managed
  worktrees as children. Columns: name, branch/ref, short commit, dirty (`✗
  dirty` / `clean`), availability.
- Right: detail panel for the selection — full path, remote, ownership, last
  inspection, linked instances.
- Actions (command bar): `Ins` add (modal with two modes: register local
  directory / clone repository), `W` new worktree, `F5` re-inspect, `Del` remove
  managed entry (safe-remove preview), `Enter` details.
- Cloning a private repo triggers the terminal-handoff flow above.

### Profiles

- Left: `ListView`, grouped by origin (**Project** / **Source: <repo>** /
  **Imported**), columns: name, adapter, services, trust status.
- Right: inspector — expanded services, adapter, command, workdir, mounts,
  capabilities, consumed slots, probes, parameters, lifecycle, trust.
- Actions: `V` validate against a chosen checkout (checkout picker modal →
  validation report modal), `I` import a source-local profile (trust prompt shows
  the manifest verbatim), `E` guided editor (schema-driven form rendered from the
  adapter's JSON Schema — field types map to TextField/NumericSelector/
  CheckBox/DropDownList; unknown schema → read-only YAML view, never a hand-rolled
  form), `Ins` new profile.

### Instances

- Left: `ListView` — name, profile, checkout (short commit + dirty), device
  (true placement, always), state, health. Authored-but-never-run instances are
  listed with state `not started`.
- Right: detail — exact source identity, expanded services with per-service
  state/health/resources, active connections, recent operations.
- **Create wizard** (modal, one step per screen, `Back`/`Next`):
  1. Checkout (tree picker, shows commit/dirty)
  2. Startup profile (only ones valid for that checkout)
  3. Device (eligible devices selectable; ineligible ones visible but disabled
     with the concrete reason inline)
  4. Name + parameters (schema-approved fields, defaults pre-filled)
  5. Preview: expanded services, ports, volumes → `Create`
- Actions: `F7` validate, `F8` plan (preview modal), `F9` start, `F10` stop
  (preserves volumes), `Ctrl+Del` destructive cleanup (distinct confirm),
  `Ins` wizard.

### Connections

- Main area: the **route matrix** as a `ListView` table — consumer instances as
  rows; columns: consumer, slot, selected provider group, route version, state.
- `Enter` on a row → switch dialog: DropDownList of *compatible* groups only;
  below it a two-column old→new preview of every route that will change.
  `Apply` performs the single atomic binding operation; result (including
  rollback info on failure) shown in a report modal.
- A short fixed explainer line under the matrix: "Consumers keep their fixed
  localhost/network addresses; Switchyard routes them to the selected group."
- Never auto-selects a provider; unbound slots show as `not connected` with the
  key to fix it.

### Devices

- `ListView`: name, kind (`local`/`ssh`), address, connectivity, eligibility for
  remote execution (`eligible` / `ineligible: <concrete reason>`), origin/scope
  if global config exists.
- Actions: `Ins` add SSH device (form + connectivity check before save), `C`
  re-check, `Del` remove (blocked with reason if instances are placed on it),
  `Enter` details (last check output).

### Operations

- Top: run actions (`.switchyard/run-scripts.yaml`) as a `ListView` — these are
  project operations, deliberately kept out of Profiles. `Enter` runs one
  (confirm first), `Ins`/`E`/`Del` manage entries (shell-notice flow preserved).
- Bottom (HSplitter): **timeline + log pane** — one ordered timeline of
  validation, planning, build, start, readiness, route changes, stop, cleanup;
  streaming output appended live from the background task; exit status retained.
  Filter field narrows by deployment / instance / service. Destructive operations
  are labeled `DESTRUCTIVE` in the timeline.

## 3. Module layout (crate `switchyard-tui`)

```text
src/
  lib.rs          – public run(project_dir): outer handoff loop (unchanged API)
  shell.rs        – single window, Tab host, command bar, help modal, busy chip
  state.rs        – ProjectState projection + refresh (wraps switchyard-ops)
  handoff.rs      – CloneHandoff plumbing + interactive git execution
  tasks.rs        – BackgroundTask wiring (OpUpdate/OpCommand types)
  tabs/
    home.rs  code.rs  profiles.rs  instances.rs
    connections.rs  devices.rs  operations.rs
  dialogs/
    forms.rs      – shared form helpers, schema-driven field rendering
    confirm.rs    – preview/confirm + destructive-confirm dialogs
    wizard.rs     – instance creation wizard
```

Rules: tabs never call `switchyard-state`/`planner` directly — only through
`state.rs`/`switchyard-ops`. No persisted field renames. No secrets in any
rendered preview or log.

## 4. Testing

- Unit tests per tab for state→row projection (pure functions, no terminal).
- AppCUI debug/event-script tests where practical
  (`.codex-refs/appcui/docs/chapter-2/debug_scenarious.md`).
- The existing pty-driven smoke tests are rewritten to drive the new binary
  (see `scripts/`, and the ESC-coalescing note in AGENTMISTAKES/memory: send
  ESC as a lone byte only when intended; AppCUI parses CSI sequences itself).
- All views must remain fully usable over SSH (termios backend is the default on
  Linux; no mouse-only interactions).

## 5. Delivery parts (each = one Codex brief, one reviewed commit)

1. **Shell**: appcui dependency, outer handoff loop, single window + tabs +
   command bar + help modal, ProjectState refresh plumbing, full Home tab,
   placeholder panes for the other tabs. Ratatui code removed (git history keeps
   it); binary works from this part on.
2. **Code tab** incl. add/clone/worktree/remove dialogs + terminal handoff.
3. **Profiles tab** incl. discovery, import + trust, schema-driven editor.
4. **Instances tab** incl. wizard, validate/plan/start/stop, background streaming.
5. **Connections tab**: route matrix, switch preview, atomic apply.
6. **Devices + Operations tabs**: eligibility, run actions, timeline/logs.
7. **Hardening**: pty smoke tests for every tab, docs (`docs/tui.md`), final
   polish pass.

Work happens on branch `tui-appcui-rewrite`; merge to `main` when part 7 passes
the full verification suite (workspace tests, clippy `-D warnings`, fmt, rustdoc,
pty smoke).
