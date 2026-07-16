# Switchyard terminal UI

Launch the full-screen project UI from a project directory, or pass one explicitly:

```sh
switchyard tui
switchyard tui path/to/project
```

The top bar switches between **Sources** and the placeholder **Instances** view. Use
`Tab`, Left, or Right to change views; press `?` for the complete in-app key reference,
and quit with `q` or Ctrl-C.

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
