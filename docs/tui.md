# Switchyard terminal UI

Launch the keyboard-first AppCUI control plane from a Switchyard project:

```sh
switchyard tui
# Or point it at another project:
switchyard tui path/to/project
```

The TUI uses one project snapshot across seven workflow-ordered tabs:

- **Home** summarizes the project, lists current problems, and points to the next
  unfinished first-run step.
- **Code** registers existing directories, clones repositories, creates managed
  worktrees, and inspects source identity and ownership.
- **Profiles** discovers, reviews, imports, validates, and inspects reusable startup
  profiles.
- **Instances** creates instances and validates, plans, starts, stops, or cleans up
  their deployment.
- **Connections** shows consumer slots and atomically switches each slot to a compatible,
  complete provider group. Switchyard never chooses a group implicitly.
- **Devices** manages the local device and SSH targets, including connectivity and
  remote-container eligibility checks.
- **Operations** manages project run actions and shows the ordered, filterable timeline
  and streaming output for work started anywhere in the TUI.

Use `Alt+H/C/P/I/N/D/O` to open a tab, or `Ctrl+Tab` and `Ctrl+Shift+Tab` to cycle.
`Tab`, `Shift+Tab`, and arrows move within the active tab.

## Keys and actions

The bottom command bar always shows actions valid for the active tab:

- `F1` opens help; `F5` refreshes the complete project snapshot.
- `F2` adds or creates; `F3`, `F4`, and `F6` provide tab-specific secondary actions.
- On Instances, `F7` validates, `F8` plans, `F9` starts, and `F10` stops while preserving
  named volumes. `Ctrl+Delete` is the separately confirmed destructive cleanup.
- `Enter` opens details or the primary preview; `Delete` removes after a safety review.
- `Esc` leaves a dialog or quits from the shell. `Ctrl+Q` quits from anywhere.

AppCUI reserves `Insert`, `Space`, `Ctrl+Space`, `Shift+arrows`, and plain letters while
a list or tree has focus. Switchyard therefore puts application actions only on
F-keys, `Delete`, and `Enter`. Lists intentionally have no implicit SearchBar; the
Operations log uses an explicit text filter instead.

## First run

Follow the checklist on Home:

1. Open Code (`Alt+C`), press `F2`, and register a checkout or clone a repository.
2. Open Profiles (`Alt+P`). Select a project profile, or review a discovered source
   profile with `F6` before importing it.
3. Open Instances (`Alt+I`) and press `F2`. Choose the checkout, a valid startup
   profile, an eligible device, a name, and profile parameters; then review the expanded
   service preview and create the instance.
4. With the instance selected, use `F7` to validate, `F8` to inspect the plan, and `F9`
   to start its deployment. Output streams to Operations.
5. If the instance consumes a service slot, open Connections (`Alt+N`), press `Enter`,
   choose a compatible provider group, review every old-to-new route, and apply the
   atomic switch.

Lists and trees select their first row automatically, so a one-item project is ready for
its F-key actions without an initial arrow press.

## Cloning and terminal handoff

Code's `F2` dialog can register a local directory or clone a Git address. A clone may
need native credential helpers, SSH agents, host confirmation, or an interactive
password/passphrase prompt. Switchyard therefore exits the alternate-screen UI, restores
the real terminal, and runs Git directly without collecting credentials itself.

After Git finishes, Switchyard **re-execs the current process** rather than constructing a
second AppCUI application in-process. The restarted TUI returns to Code and displays the
clone result. Failed or interrupted clones pause for Enter before the same restart so the
Git error remains readable.

## Profiles, run actions, and devices

A **startup profile** defines the long-running services for an instance. Profiles found
in a source remain untrusted until their manifest is reviewed and imported, and changed
content must be reviewed again.

A **run action** is a project operation stored in `.switchyard/run-scripts.yaml`; it is
not a startup profile and never becomes part of an instance. Operations can be structured
Switchyard commands or reviewed shell commands. `Enter` always shows a confirmation, and
the first shell action also requires a project-local warning acknowledgement.

The implicit local device is always eligible. An SSH device becomes eligible for the
supported remote cut only when SSH works and Docker is available remotely. Remote
instances must be container-backed providers whose provided ports are published; remote
consumers, process adapters, cross-device sidecars, and remote routers are unsupported.
The planner still checks a selected profile's exact workload constraints.

## PTY smoke suite

Install the test-only Python dependency and run the real-terminal smoke suite locally:

```sh
python3 -m pip install pyte
python3 scripts/tui-smoke.py
```

The script builds `switchyard` offline, creates a temporary initialized project, and
drives AppCUI through a 35x120 PTY. Set `SWITCHYARD_BIN=/path/to/switchyard` to test an
existing binary. If `pyte` is missing, the script reports a skip and exits successfully.
The sandbox used for normal repository checks cannot provide the required PTY/socket
behavior, so run this suite on a local terminal.

For layout decisions, reserved-key findings, and the clone re-exec rationale, see
[the AppCUI TUI design](tui-appcui-design.md).
