pub(crate) const TEXT: &str = r#"# Switchyard help

## Global keys

- **Alt+H / C / P / I / N / D / O** — open Home, Code, Profiles, Instances, Connections, Devices, or Operations.
- **Ctrl+Tab / Ctrl+Shift+Tab** — move to the next or previous tab.
- **F5** — refresh every project projection.
- **F1** — open this help.
- **Esc** or **Ctrl+Q** — quit (with confirmation while an operation is running).
- **Tab / Shift+Tab / arrows** — move focus within the current tab.
- Tab actions use **F-keys** (shown in the bottom command bar). Lists deliberately
  have no implicit search bar; **Insert/Space** remain reserved by list selection.

## Code tab

- **F2** — add code (register a local directory or clone a repository).
- **F3** — create a managed worktree from the selected repository.
- **Delete** — safely remove the selected managed entry.
- **Enter** — show full details for the selection.

## Profiles tab

- **F2 / F3** — open the shared JSON-Schema-driven new/edit form.
- **F4** — validate against a selected checkout without starting anything.
- **F6** — review the verbatim source manifest, then import/re-check trust.
- **Delete** — confirm removal of an imported profile.
- **Enter** — show full expansion details.

## Instances tab

- **F2** — create an authored instance with the five-step checkout/profile/device/parameter/preview wizard.
- **F7 / F8** — validate or plan without starting services.
- **F9 / F10** — start or stop the selected instance's deployment; normal stop preserves named volumes.
- **Ctrl+Delete** — destructive cleanup after a distinct confirmation; owned named volumes are deleted.
- **Enter** — inspect source identity, true placement, services, connections, and recent operations.

## Connections tab

- **Enter** — choose from compatible complete groups, review every old→new route, then apply one atomic binding operation.
- Unbound slots remain **not connected** until you explicitly choose and apply a provider group.

## Devices tab

- **F2** — enter an SSH device, then run its connectivity and Docker eligibility check before deciding whether to save it.
- **F6** — re-check the selected SSH device in the background.
- **Delete / Enter** — safely remove an unused registration or inspect the retained check output. The implicit local device cannot be removed.

## Operations tab

- **F2 / F3 / Delete** — manage project run actions from `.switchyard/run-scripts.yaml`; they are not startup profiles.
- **Enter** — confirm and run the selected action. Shell actions require a one-time per-project warning acknowledgement.
- The bottom timeline streams ordered output. Its explicit filter matches deployment, instance, or service text. Moving selection upward pauses follow; return to the last row to resume.

## Concepts

- **Code** — code made available from a local path, repository, or worktree.
- **Startup profile** — a reusable definition that expands into one service or a coordinated suite.
- **Instance** — one checkout run through one startup profile with its own parameters.
- **Connection** — the selected provider group or routes for a consumer instance.
"#;
