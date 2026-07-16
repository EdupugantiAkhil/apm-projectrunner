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

The top bar switches between **Sources**, **Devices**, and **Instances**. Use `Tab` or
Right to advance, Shift-Tab or Left to go back, press `?` for the complete in-app key
reference, and quit with `q` or Ctrl-C.

## Sources

The Sources table shows each project-registered source's name, managed/unmanaged kind,
path, current Git branch or requested ref, and live dirty state. Up/Down or `j`/`k`
changes the selected row.

- `a` opens a form for registering an existing local path or cloning a Git URL into
  `.switchyard/clones`. A clone may include a branch, tag, or other Git ref. Clone work
  runs in the background and its errors remain in the form.
- `d` confirms removal. Unmanaged sources are only deregistered; managed sources are
  deleted through Switchyard's ownership and dirty-state safety checks, then
  deregistered. Dirty managed sources are refused.
- `r` refreshes source and Git observations.
- `Esc` closes a form or confirmation without changing state.

All source records use the same project-local `.switchyard/state.sqlite3` registry as
the CLI and daemon source commands.

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
service table shows persisted container observations and their runtime/health state
when available. An applied deployment with no observed resources is shown as
`stopped`; this includes reconciliation with an `observed_resources_missing`
diagnostic after a normal down or cleanup, rather than presenting it as unknown.

- `u` starts the selected deployment, `s` refreshes status, `p` prints its plan, and
  `x` opens a y/n confirmation before stopping it. The TUI runs all four operations in
  the background.
- The output pane receives stdout and stderr while an operation runs, then displays
  its exit code. `PageUp` and `PageDown` scroll its retained output.
- If a project contains multiple definitions, `[` and `]` select the deployment.
- `i` adds another instance to the selected authored definition. The form selects an
  existing reusable block and either a deployment source or project-registered source.
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
