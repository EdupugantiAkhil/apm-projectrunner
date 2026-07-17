# Switchyard terminal UI

Create a project and launch its full-screen control plane without switching to separate
management commands:

```sh
switchyard init
cd <project>
switchyard tui .

# Existing projects are supported too:
switchyard tui
switchyard tui path/to/project
```

The TUI lands on **Home**. The top bar order is **Home**, **Sources**, **Profiles**,
**Devices**, then **Instances**. Use `Tab` or Right to advance, Shift-Tab or Left to go
back, press `?` for the complete in-app key reference, and quit with `q` or Ctrl-C.

## Home

Home is a projection-driven first-run guide; it does not save separate checklist state.
Each item becomes **Done** when the real project state contains the corresponding source,
startup profile, instance, applied deployment, or binding. The first unfinished item is
labeled **Next**, and the connection step is labeled optional when the deployment declares
no consumer slots. Every row explains the concept and names the exact destination key.

Use Up/Down or `j`/`k` to select a checklist item and Enter to open its relevant tab.
Number keys `1` through `5` jump directly from Home to the matching checklist destination.
The concepts panel distinguishes code, startup profiles, instances, and connections. The
status panel stays deliberately compact: deployment and reconciled state, running and
unhealthy service counts, latest operation, plain-language validation problems, and any
source startup-profile manifest diagnostics.

## Sources

The Sources table separates repositories, linked worktrees, and ordinary directories.
Linked worktrees are indented beneath the source conceptually and show their registered
parent repository; each row also distinguishes Switchyard-managed paths from external
paths. The remaining columns show the path, current Git branch or requested ref, and
live dirty state. Up/Down or `j`/`k` changes the selected row.

- `a` opens a two-mode dialog. Choose **Local path** or **Git clone**, then enter exactly
  one directory or clone address. Switchyard derives the registry name from its final
  path component and adds a numeric suffix if that name is already used.
- In Git mode, Enter always opens a separate options and authentication review before
  cloning (`F2` opens the same popup directly). A clone may select a branch/tag ref.
  Enter in that popup temporarily leaves the alternate-screen TUI and runs the real
  `git clone` attached to the controlling terminal.
- Native Git/OpenSSH authentication is preserved without an intermediate Switchyard
  credential form: credential helpers, `ssh-agent`, `~/.ssh/config`, automatic identity
  selection, host confirmation, and password/key-passphrase prompts behave exactly as
  they do for `git clone` in the shell. On failure or Ctrl-C, the terminal shows Git's
  output and waits for Enter before restoring the TUI.
- Bracketed paste is enabled, so pasting a path or URL updates only the focused field and
  trailing terminal newlines are discarded.
- `d` confirms removal. Unmanaged sources are only deregistered; managed sources are
  deleted through Switchyard's ownership and dirty-state safety checks, then
  deregistered. Dirty managed sources are refused.
- `r` refreshes source and Git observations.
- `w` creates a managed linked worktree from the selected repository or worktree. Enter
  only a unique checkout name; Switchyard creates a branch with that name at the
  selected checkout's exact current commit, creates the checkout under
  `.switchyard/worktrees`, registers it, and makes it immediately available to the
  instance form. A non-Git row or checkout whose HEAD cannot be resolved is refused.
- `Esc` closes a form or confirmation without changing state.

All source records use the same project-local `.switchyard/state.sqlite3` registry as
the CLI and daemon source commands.

## Startup profiles

The Profiles view lists reusable startup definitions from the selected project
deployment and from registered source checkouts. A source advertises profiles only in
`switchyard-profiles.yaml` at its checkout root; Switchyard does not search for likely
scripts or execute repository content while discovering or previewing that file.

Each row names its origin, trust state, expanded services and execution adapters. A
project profile is **trusted**. A discovered source profile is **not imported** until it
has been reviewed explicitly. Import records the source name, current source commit,
and a deterministic content hash in project state. If the source definition later
differs from that hash, the row says **changed — review** and must be reviewed and
imported again before it can run. A project profile with the same name takes precedence;
the source row remains visible with **shadowed by project profile** so that precedence is
not hidden. An invalid manifest is reported against its source in the diagnostics panel
without hiding valid project or source profiles.

- Up/Down or `j`/`k` selects a profile. `r` refreshes the project definition, imported
  state, and registered-source manifests in the background.
- Enter opens a scrollable inspector with origin, trust, commit and content-hash status,
  followed by each service's adapter, image or command, working directory, provided
  capabilities, consumed slots and probe, plus profile parameters. `Esc` closes it.
- `i` opens a review of the fully expanded block body as YAML for a discovered profile,
  or for an imported profile whose content changed. The review is read-only and causes
  no state change until Enter explicitly confirms import; `Esc` cancels. Pressing `i`
  on a project profile or an unchanged import reports that no action is needed.
- `d` opens an Enter confirmation before removing an imported record. It never edits
  the source manifest or project deployment. Project profiles cannot be removed from
  this view, and a discovered-but-unimported row has nothing to remove.

Imported records live in `.switchyard/state.sqlite3`. Import makes a reviewed definition
eligible for normal planning; it does not start a service. Startup profiles remain
separate from project run actions in `.switchyard/run-scripts.yaml`.

## Devices

The Devices table is an interactive selector for registered SSH targets. Up/Down or
`j`/`k` changes the selected device.

- `a` registers a name, SSH user, host, port, and optional identity-file path. Existing
  SSH agent and configuration behavior is preserved; Switchyard never stores passwords
  or private-key material.
- `c` checks the selected device in the background and persists its status and detail.
- `d` confirms removal of the registry entry without touching SSH keys or configuration.

Device registrations currently prove and record SSH connectivity. The instance form
lists `local` and registered devices so placement intent is visible, but only `local`
runs today. Selecting a registered remote device produces a planner error in the draft
preview; Switchyard never silently falls back to local execution.

## Instances

The Instances view combines authored deployment definitions with the durable
`.switchyard/state.sqlite3` deployment and operation records. Its header shows the
selected deployment, reconciled state, latest operation, and definition path. The
service table merges each persisted container observation with its authored instance,
block, and source instead of showing duplicate authored/runtime rows. The standalone
TUI reconciles generated manifests and labeled Docker resources when it loads and after
lifecycle operations. An applied deployment with no observed resources is shown as
`stopped`; this includes reconciliation with an `observed_resources_missing`
diagnostic after a normal down or cleanup, rather than presenting it as unknown.

- `u` starts the selected deployment, `s` refreshes status, `p` prints its plan, and
  `x` opens a y/n confirmation before stopping it. The TUI runs all four operations in
  the background.
- The output pane receives stdout and stderr while an operation runs, then displays
  its exit code. `PageUp` and `PageDown` scroll its retained output.
- If a project contains multiple definitions, `[` and `]` select the deployment.
- `i` opens guided instance creation. In order, choose a **startup profile**, checkout,
  instance name, device, and each parameter declared by the profile. Required
  parameters are labeled, defaults are prefilled, and undeclared parameters cannot be
  added. Project profiles and unchanged imported profiles are runnable; discovered or
  changed profiles remain visible but disabled with a prompt to review/import them in
  the Profiles view first. A startup profile owns the instance's long-running service
  commands and remains separate from project-level run scripts.
- On the last field, Enter plans an in-memory draft and shows the expanded service names
  plus validation diagnostics beside the relevant name, checkout, device, or parameter
  field. This preview does not change the deployment or runtime. When validation passes,
  a second Enter writes the definition. Imported source-local profiles are materialized
  under `spec.blocks` the first time they are used, and a registered checkout is added
  to `spec.sources` when needed. The targeted edit preserves the surrounding authored
  YAML and is fully planned before atomic replacement. Press `u` afterwards to plan and
  start the updated deployment.
- Instances persist `device: local` today. Registered remote device names may be
  selected for an honest compatibility preview, but remote placement is not yet
  supported and cannot be saved as a valid deployment.
- `b` opens the pairing selector. Up/Down chooses a consumer and Left/Right or Space
  chooses a complete provider group. Incompatible groups are omitted. Enter previews
  the old/new choice in the form and applies it through Switchyard's live, validated
  `bind` operation.
- Up/Down or `j`/`k` selects a run script. Enter runs it; `n`, `e`, and `D` create,
  edit, and confirm deletion respectively.

Structured actions use the same typed argument construction as the direct keys. The
spawned `switchyard` CLI automatically delegates compatible operations to a running
project daemon, while presets with overlays, a variation, or `set` values retain the
CLI's one-shot behavior because the current daemon command contract cannot carry
those options.

## Project run scripts

Run scripts are stored in `.switchyard/run-scripts.yaml`. The file is project-local
and safe to commit. Newly initialized projects exclude it from the otherwise ignored
`.switchyard` runtime directory; older projects may need the same gitignore exception
or `git add -f`. Its root is a YAML list; every item has a unique `name`, an optional
`description`, and exactly one of the following forms:

```yaml
- name: up with dev overlay
  description: Start the development topology
  command: up                 # up, down, plan, or status
  overlays:
    - overlays/dev.yaml
  variation: fast             # optional
  set:                        # optional KEY=VALUE strings
    - API_PORT=9000

- name: smoke test
  description: Exercise the local fixture
  shell: ./scripts/smoke.sh
```

`overlays`, `variation`, and `set` belong only to structured entries. Paths and shell
commands run with the project directory as their working directory. A malformed file
is reported in the scripts pane and does not crash the TUI.

Shell entries execute arbitrary commands through the user's `$SHELL` (or `/bin/sh` if
unset). The TUI displays a one-time project warning before the first shell entry runs.
Review `shell:` entries carefully before running a scripts file received from
someone else; committing the YAML does not make those commands trusted.
