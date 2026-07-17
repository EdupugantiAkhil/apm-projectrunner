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

The top bar switches between **Sources**, **Profiles**, **Devices**, and **Instances**. Use `Tab` or
Right to advance, Shift-Tab or Left to go back, press `?` for the complete in-app key
reference, and quit with `q` or Ctrl-C.

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

Device registrations currently prove and record SSH connectivity. Switchyard's runtime
and router are local-development components, so the instance form accurately shows the
runtime device as `local`; it does not pretend that a registered SSH target can host one
instance of a distributed deployment.

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
- `i` adds another instance to the selected authored definition. The form presents an
  existing reusable block as a **startup profile**, then selects either a deployment
  checkout or project-registered repository/worktree. A startup profile owns the
  instance's long-running service commands; it is separate from the project-level run
  scripts shown below.
  A registered source is added to `spec.sources` automatically. The complete deployment
  is planned before an atomic save, and the targeted insertion preserves the rest of
  the YAML, including scaffold comments. Press `u` afterwards to apply the change.
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
