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
**Devices**, **Instances**, then **Connections**. Use `Tab` to advance and Shift-Tab to
go back. Left/Right also switch views except in Connections, where they select a group.
Press `?` for the complete in-app key reference, and quit with `q` or Ctrl-C. In a
project with multiple deployment definitions, `[` and `]` select the deployment from
the Instances and Connections views.

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

## Profiles

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

The Devices table is an interactive selector for the always-available local machine and
registered SSH targets. The implicit local device appears first as `this device`, with
`-` for its non-applicable SSH fields; it maps to the `local` placement used by
deployment definitions and needs no connectivity check. For registered targets, the
eligibility column says `eligible for remote container execution (docker <version>)`,
`no docker over SSH: <reason>`, or `unchecked`. Up/Down or `j`/`k` changes the selected
device.

- `a` registers a name, SSH user, host, port, and optional identity-file path. Existing
  SSH agent and configuration behavior is preserved; Switchyard never stores passwords
  or private-key material.
- `c` checks the selected registered device in the background. It first probes SSH with
  batch authentication, then runs `docker version` through Docker's native SSH
  transport, and persists the outcome and server version or concrete failure.
  Identity-file paths may be used, but passwords and private-key contents are never
  stored.
- `d` confirms removal of the selected registered device without touching SSH keys or
  configuration. The implicit `this device` entry cannot be checked or removed.

Eligibility covers the deliberately limited remote-execution cut: container instances
that act only as providers and publish every provided capability port. The local
Switchyard router reaches them through the device host and published address; remote
routers, remote consumers, process adapters, and cross-device sidecars are unsupported.
The registered host must therefore resolve and be reachable from containers. Prefer a
LAN IP; `localhost` names the container itself, and mDNS names are often unavailable in
container DNS. An eligibility check proves SSH and Docker access, not that a particular
profile satisfies these planner-enforced workload boundaries.

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
  instance name, device, and each parameter declared by the profile. The device selector
  labels remote entries as eligible, ineligible, or unchecked from their persisted
  check. Any entry may be selected so the plan preview can report the exact placement
  incompatibility; selection never silently falls back to local. Required
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
- When a remote device is selected, the form reminds you that services must publish
  their provided ports and the startup profile must contain only container-backed
  providers. Valid remote instances persist the registered device name. The instances
  and services table shows `local` or that device name in its Placement column. If a
  status refresh cannot reach a device, its rows say `device unreachable: <detail>` and
  retain the prior observations instead of claiming that the resources are missing.
- Up/Down or `j`/`k` selects a run script. Enter runs it; `n`, `e`, and `D` create,
  edit, and confirm deletion respectively.

Structured actions use the same typed argument construction as the direct keys. The
spawned `switchyard` CLI automatically delegates compatible operations to a running
project daemon, while presets with overlays, a variation, or `set` values retain the
CLI's one-shot behavior because the current daemon command contract cannot carry
those options.

## Connections

Connections are edited in their own route-matrix view, with one row for each consumer
instance and consumed service slot. The columns show the consumer, slot, connected
group, every provider in that group with observed health when available, and route
state in words such as `active v3`, `applying`, or `failed: router_timeout`. An unbound
consumer says `not connected`; Switchyard never chooses a group merely because one is
available.

Applications continue calling their fixed `localhost` or network addresses. Switchyard
intercepts those addresses through its sidecar or host-gateway routers and sends them
to the complete provider group selected for that consumer. A backend has one downstream
group at a time; if two callers need the same backend code connected to different
groups, run two backend instances.

- Up/Down or `j`/`k` selects a consumer-slot row. Left/Right or `h`/`l` cycles only
  groups that the planner proves are complete and compatible. The choice is marked
  `pending change` and remains an in-memory draft.
- Enter opens a no-mutation preview containing the full old and new provider lists and
  each service route that changes. The preview states that the complete route table is
  applied atomically and unrelated instances are not restarted.
- A second explicit Enter starts the existing validated `switchyard bind` operation.
  `Esc` cancels the preview and clears that consumer's draft.
- After the operation, the view reloads desired and observed route versions, transition
  state, error codes, and recent timestamped activation history. When state records show
  a prior version or a rolled-back history entry, the selection details explain the
  rollback.
- If a project contains multiple definitions, `[` and `]` select the deployment and
  reset the selected connection row.

Groups and bindings are authored in the deployment definition. If no consumers with
slots are declared, the empty state explains those concepts rather than inventing a
connection.

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
