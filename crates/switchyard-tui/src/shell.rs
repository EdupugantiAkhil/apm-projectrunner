use std::{
    collections::VecDeque,
    error::Error,
    path::{Path, PathBuf},
    sync::Mutex,
};

use appcui::prelude::*;

use crate::{
    dialogs::{confirm, wizard},
    handoff::{ExitOutcome, OutcomeCell},
    state::{OperationOutcome, ProjectState},
    tabs::{self, code, connections, devices, home, instances, operations, profiles},
    tasks::{self, OpCommand, OpUpdate, OperationGate, OperationJob},
};
use code::TreeRow;
use connections::ConnectionRowView;
use devices::DeviceRowView;
use instances::InstanceRowView;
use operations::ScriptRowView;
use profiles::ProfileRowView;
use switchyard_ops::{create_instance, execution::OperationSpec, run_scripts::StructuredCommand};
use switchyard_state::{DeviceCheckStatus, RegisteredDevice, StateStore};

static STATE_JOBS: Mutex<VecDeque<StateJob>> = Mutex::new(VecDeque::new());

enum StateAction {
    Refresh,
    Register { name: String, path: PathBuf },
    Worktree(code::WorktreeRequest),
    Remove { name: String },
    ImportProfile { source: String, name: String },
    RemoveProfile { name: String },
    CreateInstance(wizard::CreateWizardResult),
    ProbeDevice(RegisteredDevice),
    CheckDevice(String),
    RemoveDevice(String),
}

struct StateJob {
    project_dir: PathBuf,
    action: StateAction,
}

struct StateUpdate {
    state: ProjectState,
    notice: Option<String>,
    device_probe: Option<RegisteredDevice>,
}

#[derive(Clone, Debug)]
struct PendingBind {
    deployment: String,
    consumer: String,
}

#[derive(Clone, Copy)]
enum TaskResponse {
    Continue,
}

fn execute_state_job(connector: &BackgroundTaskConector<StateUpdate, TaskResponse>) {
    let job = STATE_JOBS.lock().ok().and_then(|mut jobs| jobs.pop_front());
    let Some(job) = job else {
        return;
    };
    let state = ProjectState {
        project_dir: job.project_dir.clone(),
        ..ProjectState::default()
    };
    let mut device_probe = None;
    let notice = match job.action {
        StateAction::Refresh => None,
        StateAction::Register { name, path } => Some(
            state
                .register_local_source(&name, &path)
                .map(|()| "Source registered successfully.".into())
                .unwrap_or_else(|error| format!("Add failed: {error}")),
        ),
        StateAction::Worktree(request) => Some(
            state
                .create_worktree(
                    &request.source,
                    &request.base_ref,
                    &request.branch,
                    &request.name,
                    request.path.as_deref(),
                )
                .map(|()| "Managed worktree created successfully.".into())
                .unwrap_or_else(|error| format!("Worktree failed: {error}")),
        ),
        StateAction::Remove { name } => Some(
            state
                .remove_source(&name)
                .map(|()| "Source removed from Switchyard.".into())
                .unwrap_or_else(|error| format!("Safe removal refused: {error}")),
        ),
        StateAction::ImportProfile { source, name } => Some(
            switchyard_ops::import_source_profile(&job.project_dir, &source, &name)
                .map(|_| format!("Startup profile `{name}` imported and trusted."))
                .unwrap_or_else(|error| format!("Profile import failed: {error}")),
        ),
        StateAction::RemoveProfile { name } => Some(
            switchyard_ops::remove_imported_profile(&job.project_dir, &name)
                .map(|()| format!("Imported startup profile `{name}` removed."))
                .unwrap_or_else(|error| format!("Profile removal failed: {error}")),
        ),
        StateAction::CreateInstance(result) => Some(
            create_instance(&job.project_dir, &result.definition, &result.request)
                .map(|created| {
                    format!(
                        "Instance `{}` created with {} expanded service(s). Press F9 to start it.",
                        created.name,
                        created.expanded_services.len()
                    )
                })
                .unwrap_or_else(|error| format!("Instance creation failed: {error}")),
        ),
        StateAction::ProbeDevice(mut device) => {
            let (status, detail) = switchyard_ops::devices::check_device_eligibility(&device);
            let checked_at = current_unix_millis();
            device.created_at = checked_at;
            device.last_checked_at = Some(checked_at);
            device.last_check_status = status;
            device.last_check_detail = Some(detail.clone());
            device_probe = Some(device);
            Some(format!("Device check completed: {detail}"))
        }
        StateAction::CheckDevice(name) => Some(
            (|| {
                let (store, _) =
                    StateStore::open(job.project_dir.join(".switchyard/state.sqlite3"))
                        .map_err(|error| error.to_string())?;
                let device = store
                    .device(&name)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("device `{name}` is not registered"))?;
                let (status, detail) = switchyard_ops::devices::check_device_eligibility(&device);
                store
                    .record_device_check(&name, current_unix_millis(), status, Some(&detail))
                    .map_err(|error| error.to_string())?;
                Ok::<_, String>(format!("Device check completed: {detail}"))
            })()
            .unwrap_or_else(|error| format!("Device check failed: {error}")),
        ),
        StateAction::RemoveDevice(name) => Some(
            StateStore::open(job.project_dir.join(".switchyard/state.sqlite3"))
                .map_err(|error| error.to_string())
                .and_then(|(store, _)| {
                    store
                        .deregister_device(&name)
                        .map_err(|error| error.to_string())
                })
                .map(|()| {
                    format!("SSH device `{name}` removed. SSH configuration was not changed.")
                })
                .unwrap_or_else(|error| format!("Device removal failed: {error}")),
        ),
    };
    connector.notify(StateUpdate {
        state: ProjectState::load(&job.project_dir),
        notice,
        device_probe,
    });
}

fn current_unix_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

const HELP: &str = r#"# Switchyard help

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

#[ModalWindow(events = ButtonEvents)]
struct HelpDialog {}

impl HelpDialog {
    fn new() -> Self {
        let mut dialog = Self {
            base: ModalWindow::new(
                "Switchyard help",
                layout!("a:c,w:78,h:30"),
                window::Flags::None,
            ),
        };
        dialog.add(Markdown::new(
            HELP,
            layout!("l:1,t:1,r:1,b:3"),
            markdown::Flags::ScrollBars,
        ));
        dialog.add(Button::new("Close", layout!("x:50%,y:100%,p:b,w:14,h:1")));
        dialog
    }
}

impl ButtonEvents for HelpDialog {
    fn on_pressed(&mut self, _: Handle<Button>) -> EventProcessStatus {
        self.exit();
        EventProcessStatus::Processed
    }
}

#[Window(
    events = ButtonEvents + WindowEvents + CommandBarEvents + TreeViewEvents<TreeRow> + ListViewEvents<ProfileRowView> + ListViewEvents<InstanceRowView> + ListViewEvents<ConnectionRowView> + ListViewEvents<DeviceRowView> + ListViewEvents<ScriptRowView> + TextFieldEvents + BackgroundTaskEvents<StateUpdate, TaskResponse> + BackgroundTaskEvents<OpUpdate, OpCommand> + TimerEvents,
    commands = [Help, Refresh, Quit, Next, AddCode, Worktree, RemoveCode, CodeDetails, NewProfile, EditProfile, ValidateProfile, ImportProfile, RemoveProfile, ProfileDetails, NewInstance, InstanceDetails, ValidateInstance, PlanInstance, StartInstance, StopInstance, CleanupInstance, SwitchConnection, AddDevice, CheckDevice, RemoveDevice, DeviceDetails, NewScript, EditScript, RemoveScript, RunScript]
)]
pub(crate) struct SwitchyardShell {
    tabs: Handle<Tab>,
    home: home::Handles,
    code: code::Handles,
    profiles: profiles::Handles,
    instances: instances::Handles,
    connections: connections::Handles,
    devices: devices::Handles,
    operations: operations::Handles,
    busy_chip: Handle<Label>,
    state: ProjectState,
    outcome: OutcomeCell,
    operation_gate: OperationGate,
    pending_bind: Option<PendingBind>,
    state_task: Handle<BackgroundTask<StateUpdate, TaskResponse>>,
    reopen_code_pending: bool,
}

impl SwitchyardShell {
    fn new(project_dir: &Path, outcome: OutcomeCell) -> Self {
        let state = ProjectState::load(project_dir);
        let restart = outcome.restart_context();
        let title = format!("Switchyard — {}", project_dir.display());
        let mut shell = Self {
            base: Window::with_type(
                &title,
                layout!("d:f"),
                window::Flags::NoCloseButton,
                window::Type::Panel,
                window::Background::Normal,
            ),
            tabs: Handle::None,
            home: home::Handles {
                header: Handle::None,
                checklist: Handle::None,
                next: Handle::None,
                problems: Handle::None,
            },
            code: code::Handles {
                tree: Handle::None,
                detail: Handle::None,
                empty: Handle::None,
                notice: Handle::None,
            },
            profiles: profiles::Handles {
                list: Handle::None,
                detail: Handle::None,
                empty: Handle::None,
                notice: Handle::None,
            },
            instances: instances::Handles {
                list: Handle::None,
                detail: Handle::None,
                empty: Handle::None,
                notice: Handle::None,
            },
            connections: connections::Handles {
                list: Handle::None,
                empty: Handle::None,
                notice: Handle::None,
            },
            devices: devices::Handles {
                list: Handle::None,
                detail: Handle::None,
                notice: Handle::None,
            },
            operations: operations::Handles {
                list: Handle::None,
                empty: Handle::None,
                filter: Handle::None,
                log: Handle::None,
                notice: Handle::None,
            },
            busy_chip: Handle::None,
            state,
            outcome,
            operation_gate: OperationGate::default(),
            pending_bind: None,
            state_task: Handle::None,
            reopen_code_pending: restart.reopen_code,
        };

        let mut tab = Tab::with_type(layout!("d:f"), tab::Flags::TabsBar, tab::Type::OnTop);
        tab.set_tab_width(14);
        let home_index = tab.add_tab("&Home");
        let code_index = tab.add_tab("&Code");
        let profiles_index = tab.add_tab("&Profiles");
        let instances_index = tab.add_tab("&Instances");
        let connections_index = tab.add_tab("Co&nnections");
        let devices_index = tab.add_tab("&Devices");
        let operations_index = tab.add_tab("&Operations");

        shell.home = home::add(&mut tab, home_index, &shell.state);
        shell.code = code::add(
            &mut tab,
            code_index,
            &shell.state,
            restart.code_notice.as_deref(),
        );
        shell.profiles = tabs::profiles::add(&mut tab, profiles_index, &shell.state);
        shell.instances = tabs::instances::add(&mut tab, instances_index, &shell.state);
        shell.connections = tabs::connections::add(&mut tab, connections_index, &shell.state);
        shell.devices = tabs::devices::add(&mut tab, devices_index, &shell.state);
        shell.operations = tabs::operations::add(&mut tab, operations_index, &shell.state);
        shell.tabs = shell.add(tab);
        shell.busy_chip = shell.add(Label::new("ready", layout!("r:2,t:0,w:8,h:1")));
        if !restart.reopen_code {
            let next = shell.home.next;
            shell.request_focus_for_control(next);
        }
        shell
    }

    /// Deferred restore of the Code tab after a clone handoff restart: tab focus
    /// changes are only honored once the runtime processes the live window.
    fn restore_code_tab(&mut self) {
        let tabs = self.tabs;
        if let Some(tab) = self.control_mut(tabs) {
            tab.set_current_tab(1);
        }
        let tree = self.code.tree;
        self.request_focus_for_control(tree);
    }

    fn refresh_state(&mut self) {
        self.start_state_job(StateAction::Refresh, "Refreshing project state…");
    }

    fn apply_state_controls(&mut self) {
        let header_text = home::header_text(&self.state);
        let problems_text = home::problems_text(&self.state);
        let next_caption = home::next_action(&self.state).caption;
        let state = self.state.clone();
        let home = self.home;
        if let Some(header) = self.control_mut(home.header) {
            header.set_caption(&header_text);
        }
        if let Some(checklist) = self.control_mut(home.checklist) {
            home::fill_checklist(checklist, &state);
        }
        if let Some(next) = self.control_mut(home.next) {
            next.set_caption(next_caption);
        }
        if let Some(problems) = self.control_mut(home.problems) {
            problems.set_caption(&problems_text);
        }
        self.refresh_code_controls();
        self.refresh_profile_controls();
        self.refresh_instance_controls();
        self.refresh_connection_controls();
        self.refresh_device_controls();
        self.refresh_script_controls();
    }

    fn start_state_job(&mut self, action: StateAction, busy_notice: &str) {
        if let Err(error) = self.operation_gate.try_start() {
            self.set_notices(error);
            return;
        }
        let Ok(mut jobs) = STATE_JOBS.lock() else {
            self.operation_gate.finish();
            self.set_notices("Could not start the project operation: task queue unavailable.");
            return;
        };
        jobs.push_back(StateJob {
            project_dir: self.state.project_dir.clone(),
            action,
        });
        drop(jobs);
        self.set_busy(true);
        self.set_notices(busy_notice);
        self.state_task = BackgroundTask::run(execute_state_job, self.handle());
    }

    fn set_busy(&mut self, busy: bool) {
        let chip = self.busy_chip;
        if let Some(label) = self.control_mut(chip) {
            label.set_caption(if busy { "BUSY" } else { "ready" });
        }
    }

    fn set_notices(&mut self, text: &str) {
        self.set_code_notice(text);
        self.set_profile_notice(text);
        let notice = self.instances.notice;
        if let Some(label) = self.control_mut(notice) {
            label.set_caption(text);
        }
        let notice = self.connections.notice;
        if let Some(label) = self.control_mut(notice) {
            label.set_caption(text);
        }
        let notice = self.devices.notice;
        if let Some(label) = self.control_mut(notice) {
            label.set_caption(text);
        }
        let notice = self.operations.notice;
        if let Some(label) = self.control_mut(notice) {
            label.set_caption(text);
        }
    }

    fn selected_source_index(&self) -> Option<usize> {
        self.control(self.code.tree)?
            .current_item()
            .map(|item| item.value().source_index)
    }

    fn refresh_code_controls(&mut self) {
        let selected = self.selected_source_index().unwrap_or(0);
        let sources = self.state.sources.clone();
        let handles = self.code;
        if let Some(tree) = self.control_mut(handles.tree) {
            code::fill_tree(tree, &sources, Some(selected));
        }
        if let Some(empty) = self.control_mut(handles.empty) {
            empty.set_visible(sources.is_empty());
        }
        self.update_code_detail();
    }

    fn update_code_detail(&mut self) {
        let text = self
            .selected_source_index()
            .and_then(|index| self.state.sources.get(index))
            .map_or_else(
                || "Select a source to inspect its identity and ownership.".into(),
                |source| code::detail_text(&self.state, source),
            );
        let detail = self.code.detail;
        if let Some(label) = self.control_mut(detail) {
            label.set_caption(&text);
        }
    }

    fn set_code_notice(&mut self, text: &str) {
        let notice = self.code.notice;
        if let Some(label) = self.control_mut(notice) {
            label.set_caption(text);
        }
    }

    fn selected_profile_index(&self) -> Option<usize> {
        self.control(self.profiles.list)?
            .current_item()
            .map(|row| row.profile_index)
    }

    fn refresh_profile_controls(&mut self) {
        let selected = self.selected_profile_index().unwrap_or(0);
        let profiles = self.state.profiles.clone();
        let handles = self.profiles;
        if let Some(list) = self.control_mut(handles.list) {
            profiles::fill_list(list, &profiles, Some(selected));
        }
        if let Some(empty) = self.control_mut(handles.empty) {
            empty.set_visible(profiles.is_empty());
        }
        let diagnostics = profiles::profile_diagnostics(&self.state);
        if let Some(notice) = self.control_mut(handles.notice) {
            notice.set_caption(&diagnostics);
        }
        self.update_profile_detail();
    }

    fn update_profile_detail(&mut self) {
        let text = self
            .selected_profile_index()
            .and_then(|index| self.state.profiles.get(index))
            .map(|p| p.detail.clone())
            .unwrap_or_else(|| "Select a startup profile to inspect its full expansion.".into());
        let detail = self.profiles.detail;
        if let Some(area) = self.control_mut(detail) {
            area.set_text(&text);
        }
    }

    fn set_profile_notice(&mut self, text: &str) {
        let notice = self.profiles.notice;
        if let Some(label) = self.control_mut(notice) {
            label.set_caption(text);
        }
    }

    fn selected_profile(&self) -> Option<&crate::state::ProfileProjection> {
        self.selected_profile_index()
            .and_then(|index| self.state.profiles.get(index))
    }

    fn validate_profile(&self) {
        if let Some(profile) = self.selected_profile() {
            profiles::validate_profile(&self.state, profile);
        } else {
            dialogs::message("Profile validation", "Select a startup profile first.");
        }
    }

    fn edit_profile(&self, new: bool) {
        profiles::show_editor(if new { None } else { self.selected_profile() });
    }

    fn import_profile(&mut self) {
        if self.operation_gate.is_running() {
            self.set_profile_notice("Another project operation is already running.");
            return;
        }
        let Some(profile) = self.selected_profile().cloned() else {
            self.set_profile_notice("Select a source-local profile first.");
            return;
        };
        let Some(source) = profiles::source_for_import(&profile).map(str::to_owned) else {
            self.set_profile_notice("F6 applies only to a new or changed source-local profile.");
            return;
        };
        let manifest = match profiles::import_manifest_text(&self.state, &source) {
            Ok(text) => text,
            Err(error) => {
                self.set_profile_notice(&error);
                return;
            }
        };
        if profiles::confirm_import(&profile, &source, &manifest) {
            self.start_state_job(
                StateAction::ImportProfile {
                    source,
                    name: profile.name,
                },
                "Importing reviewed startup profile…",
            );
        }
    }

    fn remove_profile(&mut self) {
        if self.operation_gate.is_running() {
            self.set_profile_notice("Another project operation is already running.");
            return;
        }
        let Some(profile) = self.selected_profile().cloned() else {
            self.set_profile_notice("Select an imported profile first.");
            return;
        };
        if !matches!(
            profile.row.origin,
            switchyard_ops::ProfileOrigin::ImportedFromSource { .. }
        ) {
            self.set_profile_notice("Only imported profiles can be removed here.");
            return;
        }
        let preview = format!(
            "Remove imported startup profile `{}`?\n\nThe source manifest and project profiles will not be changed.",
            profile.name
        );
        if confirm::safe_remove(&preview) {
            self.start_state_job(
                StateAction::RemoveProfile { name: profile.name },
                "Removing imported profile…",
            );
        }
    }

    fn show_profile_details(&self) {
        if let Some(profile) = self.selected_profile() {
            appcui::dialogs::message("Startup profile details", &profile.detail);
        } else {
            appcui::dialogs::message("Startup profile details", "No startup profile is selected.");
        }
    }

    fn add_code(&mut self) {
        if self.operation_gate.is_running() {
            self.set_code_notice("Another project operation is already running.");
            return;
        }
        let Some(request) = code::AddDialog::new(&self.state.project_dir).show() else {
            return;
        };
        match request {
            code::AddRequest::Local { name, path } => {
                self.start_state_job(StateAction::Register { name, path }, "Registering source…");
            }
            code::AddRequest::Clone(request) => {
                self.outcome.request_clone(request);
                self.close();
            }
        }
    }

    fn create_worktree(&mut self) {
        if self.operation_gate.is_running() {
            self.set_code_notice("Another project operation is already running.");
            return;
        }
        let selected = self
            .selected_source_index()
            .and_then(|index| self.state.sources.get(index));
        let Some(dialog) = code::WorktreeDialog::new(&self.state.sources, selected) else {
            self.set_code_notice(
                "Worktree creation requires a Git source with a known HEAD commit.",
            );
            return;
        };
        let Some(request) = dialog.show() else {
            return;
        };
        self.start_state_job(StateAction::Worktree(request), "Creating managed worktree…");
    }

    fn remove_code(&mut self) {
        if self.operation_gate.is_running() {
            self.set_code_notice("Another project operation is already running.");
            return;
        }
        let Some(source) = self
            .selected_source_index()
            .and_then(|index| self.state.sources.get(index))
            .cloned()
        else {
            self.set_code_notice("Select a source before removing it.");
            return;
        };
        let preview = match source.ownership {
            switchyard_state::RegisteredSourceKind::Managed => format!(
                "Remove and deregister `{}`?\n\nManaged path to delete: {}\nSwitchyard will refuse if ownership, containment, or clean-state checks fail.",
                source.name,
                source.path.display(),
            ),
            switchyard_state::RegisteredSourceKind::Unmanaged => format!(
                "Deregister `{}`?\n\nThe unmanaged directory will NOT be deleted:\n{}",
                source.name,
                source.path.display(),
            ),
        };
        if !confirm::safe_remove(&preview) {
            return;
        }
        self.start_state_job(
            StateAction::Remove { name: source.name },
            "Checking and removing source…",
        );
    }

    fn show_code_details(&self) {
        if let Some(source) = self
            .selected_source_index()
            .and_then(|index| self.state.sources.get(index))
        {
            appcui::dialogs::message("Code details", &code::detail_text(&self.state, source));
        } else {
            appcui::dialogs::message(
                "Code details",
                "No source is selected. Press F2 to add code.",
            );
        }
    }

    fn selected_instance_row(&self) -> Option<InstanceRowView> {
        self.control(self.instances.list)?.current_item().cloned()
    }

    fn selected_instance_deployment(&self) -> Option<&crate::state::DeploymentProjection> {
        let row = self.selected_instance_row()?;
        instances::deployment_for_row(&self.state, &row)
    }

    fn refresh_instance_controls(&mut self) {
        let preferred = self
            .selected_instance_row()
            .map(|row| (row.deployment_index, row.instance_index));
        let state = self.state.clone();
        let handles = self.instances;
        if let Some(list) = self.control_mut(handles.list) {
            instances::fill_list(list, &state, preferred);
        }
        if let Some(empty) = self.control_mut(handles.empty) {
            empty.set_visible(instances::project_rows(&state).is_empty());
        }
        self.update_instance_detail();
    }

    fn update_instance_detail(&mut self) {
        let text = self.selected_instance_row().map_or_else(
            || "Select an instance to inspect its exact source identity, services, connections, and recent operations.".into(),
            |row| instances::detail_text(&self.state, &row),
        );
        let detail = self.instances.detail;
        if let Some(area) = self.control_mut(detail) {
            area.set_text(&text);
        }
    }

    fn selected_connection_row(&self) -> Option<ConnectionRowView> {
        self.control(self.connections.list)?.current_item().cloned()
    }

    fn refresh_connection_controls(&mut self) {
        let preferred = self
            .selected_connection_row()
            .map(|row| (row.deployment_index, row.consumer, row.slot));
        let state = self.state.clone();
        let handles = self.connections;
        if let Some(list) = self.control_mut(handles.list) {
            connections::fill_list(list, &state, preferred);
        }
        if let Some(empty) = self.control_mut(handles.empty) {
            empty.set_visible(connections::project_rows(&state).is_empty());
        }
        let diagnostics = connections::diagnostics(&state);
        if let Some(notice) = self.control_mut(handles.notice) {
            notice.set_caption(&diagnostics);
        }
    }

    fn switch_connection(&mut self) {
        if self.operation_gate.is_running() {
            self.set_notices("Another project operation is already running.");
            return;
        }
        let Some(view) = self.selected_connection_row() else {
            self.set_notices(
                "Connections need a consumer instance with at least one consumed slot. Create one with F2 on Instances.",
            );
            return;
        };
        let Some((deployment, row)) = connections::source_row(&self.state, &view) else {
            self.set_notices(
                "The selected connection is no longer available. Press F5 to refresh.",
            );
            return;
        };
        if row.compatible_groups.is_empty() {
            self.set_notices(
                "No complete provider group is compatible with this consumer. Fix the deployment definition and press F5.",
            );
            return;
        }
        let deployment_name = deployment.name.clone();
        let bundle = deployment.bundle.clone();
        let consumer = row.consumer.clone();
        let compatible_groups = row.compatible_groups.clone();
        let mut previews = Vec::with_capacity(compatible_groups.len());
        for group in compatible_groups {
            match switchyard_ops::switch_preview(
                &self.state.project_dir,
                &bundle,
                &consumer,
                &group,
            ) {
                Ok(preview) => previews.push(preview),
                Err(error) => {
                    self.set_notices(&format!(
                        "Could not build the atomic switch preview for `{group}`: {error}"
                    ));
                    return;
                }
            }
        }
        let Some(request) = connections::show_switch(&consumer, previews) else {
            return;
        };
        let spec = OperationSpec::bind(bundle, consumer.clone(), request.group.clone());
        let started = self.start_operation(
            format!("bind {consumer} → {}", request.group),
            Some(deployment_name.clone()),
            false,
            spec,
        );
        if started {
            self.pending_bind = Some(PendingBind {
                deployment: deployment_name,
                consumer,
            });
        }
    }

    fn finish_bind(&mut self, succeeded: bool, operation_detail: &str) {
        let Some(bind) = self.pending_bind.take() else {
            return;
        };
        let (statuses, status_error) =
            match switchyard_ops::route_status(&self.state.project_dir, &bind.deployment) {
                Ok(statuses) => (statuses, None),
                Err(error) => (Vec::new(), Some(error)),
            };
        let detail = status_error.map_or_else(
            || operation_detail.to_owned(),
            |error| format!("{operation_detail}\nRoute status lookup failed: {error}"),
        );
        let report = connections::render_result(&bind.consumer, succeeded, &detail, &statuses);
        connections::show_result(&report);
    }

    fn refresh_operation_log(&mut self) {
        let filter = self
            .control(self.operations.filter)
            .map_or("", TextField::text)
            .to_owned();
        let operation_log = self.state.operation_log.clone();
        let log = self.operations.log;
        if let Some(list) = self.control_mut(log) {
            let selected = if list.count() == 0 || list.index().saturating_add(1) >= list.count() {
                Some(usize::MAX)
            } else {
                Some(list.index())
            };
            operations::fill_timeline(list, &operation_log, &filter, selected);
        }
        self.update_instance_detail();
    }

    fn selected_device_row(&self) -> Option<devices::DeviceRowView> {
        self.control(self.devices.list)?.current_item().cloned()
    }

    fn refresh_device_controls(&mut self) {
        let preferred = self.selected_device_row().and_then(|row| row.device_index);
        let state = self.state.clone();
        let list_handle = self.devices.list;
        if let Some(list) = self.control_mut(list_handle) {
            devices::fill_list(list, &state, Some(preferred));
        }
        self.update_device_detail();
    }

    fn update_device_detail(&mut self) {
        let text = self.selected_device_row().map_or_else(
            || "Select a device to inspect its last check output.".into(),
            |row| devices::detail_text(&self.state, &row),
        );
        let detail = self.devices.detail;
        if let Some(area) = self.control_mut(detail) {
            area.set_text(&text);
        }
    }

    fn add_device(&mut self) {
        if self.operation_gate.is_running() {
            self.set_notices("Another project operation is already running.");
            return;
        }
        if let Some(device) = devices::DeviceDialog::new().show() {
            self.start_state_job(
                StateAction::ProbeDevice(device),
                "Checking SSH connectivity and Docker eligibility before save…",
            );
        }
    }

    fn check_device(&mut self) {
        let Some(row) = self.selected_device_row() else {
            return;
        };
        let Some(index) = row.device_index else {
            self.set_notices("The local device is always available and needs no SSH check.");
            return;
        };
        let name = self.state.devices[index].name.clone();
        self.start_state_job(
            StateAction::CheckDevice(name),
            "Re-checking SSH connectivity and Docker eligibility…",
        );
    }

    fn remove_device(&mut self) {
        if self.operation_gate.is_running() {
            self.set_notices("Another project operation is already running.");
            return;
        }
        let Some(row) = self.selected_device_row() else {
            return;
        };
        let Some(_) = row.device_index else {
            self.set_notices("The implicit local device cannot be removed.");
            return;
        };
        let placements = self
            .state
            .deployments
            .iter()
            .flat_map(|deployment| {
                deployment
                    .instances
                    .iter()
                    .filter(|instance| instance.device == row.name)
                    .map(move |instance| format!("{} / {}", deployment.name, instance.name))
            })
            .collect::<Vec<_>>();
        if !placements.is_empty() {
            self.set_notices(&format!("Cannot remove `{}`: instances are placed on it: {}. Move or remove those instances first.", row.name, placements.join(", ")));
            return;
        }
        if confirm::safe_remove(&format!(
            "Remove SSH device registration `{}`?\n\nSSH keys, agent state, and SSH configuration will not be changed.",
            row.name
        )) {
            self.start_state_job(
                StateAction::RemoveDevice(row.name),
                "Removing device registration…",
            );
        }
    }

    fn show_device_details(&self) {
        if let Some(row) = self.selected_device_row() {
            appcui::dialogs::message("Device details", &devices::detail_text(&self.state, &row));
        }
    }

    fn selected_script_index(&self) -> Option<usize> {
        self.control(self.operations.list)?
            .current_item()
            .map(|row| row.script_index)
    }

    fn refresh_script_controls(&mut self) {
        let selected = self.selected_script_index();
        let scripts = self.state.run_scripts.clone();
        let handles = self.operations;
        if let Some(list) = self.control_mut(handles.list) {
            operations::fill_list(list, &scripts, selected);
        }
        if let Some(empty) = self.control_mut(handles.empty) {
            empty.set_visible(scripts.is_empty());
        }
        let error = self.state.run_scripts_error.clone().unwrap_or_default();
        if let Some(notice) = self.control_mut(handles.notice) {
            notice.set_caption(&error);
        }
    }

    fn edit_script(&mut self, new: bool) {
        if self.operation_gate.is_running() {
            self.set_notices("Another project operation is already running.");
            return;
        }
        let edit_index = (!new).then(|| self.selected_script_index()).flatten();
        if !new && edit_index.is_none() {
            self.set_notices("Select a project run action to edit.");
            return;
        }
        let existing = edit_index.and_then(|index| self.state.run_scripts.get(index));
        let reserved_names = self
            .state
            .run_scripts
            .iter()
            .enumerate()
            .filter(|(index, _)| Some(*index) != edit_index)
            .map(|(_, script)| script.name.clone())
            .collect();
        let Some(script) = operations::ScriptDialog::new(existing, reserved_names).show() else {
            return;
        };
        if self
            .state
            .run_scripts
            .iter()
            .enumerate()
            .any(|(index, item)| Some(index) != edit_index && item.name == script.name)
        {
            self.set_notices(&format!(
                "Run action name `{}` already exists.",
                script.name
            ));
            return;
        }
        let mut scripts = self.state.run_scripts.clone();
        if let Some(index) = edit_index {
            scripts[index] = script;
        } else {
            scripts.push(script);
        }
        match switchyard_ops::run_scripts::save(&self.state.project_dir, &scripts) {
            Ok(()) => {
                self.state.run_scripts = scripts;
                self.state.run_scripts_error = None;
                self.refresh_script_controls();
                self.set_notices("Project run actions saved.");
            }
            Err(error) => self.set_notices(&format!("Could not save run actions: {error}")),
        }
    }

    fn remove_script(&mut self) {
        if self.operation_gate.is_running() {
            self.set_notices("Another project operation is already running.");
            return;
        }
        let Some(index) = self.selected_script_index() else {
            self.set_notices("Select a project run action to delete.");
            return;
        };
        let name = self.state.run_scripts[index].name.clone();
        if !confirm::safe_remove(&format!(
            "Delete project run action `{name}`?\n\nThis edits .switchyard/run-scripts.yaml. It does not remove a startup profile or stop services."
        )) {
            return;
        }
        let mut scripts = self.state.run_scripts.clone();
        scripts.remove(index);
        match switchyard_ops::run_scripts::save(&self.state.project_dir, &scripts) {
            Ok(()) => {
                self.state.run_scripts = scripts;
                self.refresh_script_controls();
                self.set_notices("Project run action deleted.");
            }
            Err(error) => self.set_notices(&format!("Could not delete run action: {error}")),
        }
    }

    fn run_script(&mut self) {
        if self.operation_gate.is_running() {
            self.set_notices("Another project operation is already running.");
            return;
        }
        let Some(index) = self.selected_script_index() else {
            self.set_notices("Select a project run action to run.");
            return;
        };
        let script = self.state.run_scripts[index].clone();
        let deployment = self
            .selected_instance_deployment()
            .or_else(|| self.state.deployments.first());
        if script.command.is_some() && deployment.is_none() {
            self.set_notices("A structured run action requires a deployment definition.");
            return;
        }
        let bundle = deployment.map_or_else(
            || self.state.project_dir.join("deployment.yaml"),
            |item| item.bundle.clone(),
        );
        let deployment_name = deployment.map(|item| item.name.clone());
        let Ok(spec) = OperationSpec::from_script(&script, bundle) else {
            self.set_notices("The selected run action is invalid. Press F3 to edit it.");
            return;
        };
        let preview = format!(
            "Run project action `{}`?\n\n{}\n\nTarget deployment: {}\nOutput will stream into the ordered Operations timeline.",
            script.name,
            script.description.as_deref().unwrap_or("No description."),
            deployment_name
                .as_deref()
                .unwrap_or("project shell context")
        );
        if !dialogs::validate("Confirm project run action", &preview) {
            return;
        }
        if matches!(spec, OperationSpec::Shell(_))
            && !switchyard_ops::run_scripts::shell_notice_acknowledged(&self.state.project_dir)
        {
            let warning = "This project run action executes a shell command with your user permissions from the project directory. Review .switchyard/run-scripts.yaml before continuing. Acknowledgement is remembered for this project.";
            if !dialogs::validate("Shell command warning", warning) {
                return;
            }
            if let Err(error) =
                switchyard_ops::run_scripts::acknowledge_shell_notice(&self.state.project_dir)
            {
                self.set_notices(&format!(
                    "Could not remember shell warning acknowledgement: {error}"
                ));
                return;
            }
        }
        self.start_operation(script.name, deployment_name, false, spec);
    }

    fn create_instance(&mut self) {
        if self.operation_gate.is_running() {
            self.set_notices("Another project operation is already running.");
            return;
        }
        let deployment = self
            .selected_instance_row()
            .map_or(0, |row| row.deployment_index);
        if let Some(result) = wizard::show(&self.state, deployment) {
            self.start_state_job(
                StateAction::CreateInstance(result),
                "Creating the reviewed authored instance…",
            );
        }
    }

    fn show_instance_details(&self) {
        if let Some(row) = self.selected_instance_row() {
            appcui::dialogs::message(
                "Instance details",
                &instances::detail_text(&self.state, &row),
            );
        } else {
            appcui::dialogs::message(
                "Instance details",
                "No instance is selected. Press F2 to create one.",
            );
        }
    }

    fn run_instance_command(&mut self, command: InstanceCommand) {
        if self.operation_gate.is_running() {
            self.set_notices("Another project operation is already running.");
            return;
        }
        let Some(deployment) = self.selected_instance_deployment().cloned() else {
            self.set_notices("Select an instance first.");
            return;
        };
        let problems = if deployment.validation_problems.is_empty() {
            "none".into()
        } else {
            deployment.validation_problems.join("\n")
        };
        let (title, action, explanation, spec, destructive) = match command {
            InstanceCommand::Validate => (
                "Validate deployment",
                "&Validate",
                format!(
                    "Validate {} without starting services.\n\nDefinition: {}\nCurrent projected problems:\n{}",
                    deployment.name,
                    deployment.bundle.display(),
                    problems
                ),
                cli_shell_spec("validate", &deployment.bundle, false),
                false,
            ),
            InstanceCommand::Plan => (
                "Plan preview",
                "&Plan",
                format!(
                    "Generate and validate the complete runtime plan for {}. No services will start.\n\nDefinition: {}\nAuthored instances: {}\nProjected services: {}\nCurrent projected problems:\n{}\n\nOutput will stream to Operations.",
                    deployment.name,
                    deployment.bundle.display(),
                    deployment.instances.len(),
                    deployment.services.len(),
                    problems
                ),
                OperationSpec::direct(StructuredCommand::Plan, deployment.bundle.clone()),
                false,
            ),
            InstanceCommand::Start => (
                "Start deployment",
                "&Start",
                format!(
                    "Plan and start deployment {} from {}. This affects every authored instance in that deployment.\n\nTrue placements remain those shown in the Instances list. Output will stream to Operations.",
                    deployment.name,
                    deployment.bundle.display()
                ),
                OperationSpec::direct(StructuredCommand::Up, deployment.bundle.clone()),
                false,
            ),
            InstanceCommand::Stop => (
                "Stop deployment",
                "&Stop",
                format!(
                    "Stop deployment {} and its services.\n\nNamed volumes WILL BE PRESERVED. Use Ctrl+Delete only when you explicitly want owned volumes deleted.",
                    deployment.name
                ),
                OperationSpec::direct(StructuredCommand::Down, deployment.bundle.clone()),
                false,
            ),
        };
        if instances::confirm_operation(title, &explanation, action) {
            self.start_operation(title.to_owned(), Some(deployment.name), destructive, spec);
        }
    }

    fn cleanup_instance(&mut self) {
        if self.operation_gate.is_running() {
            self.set_notices("Another project operation is already running.");
            return;
        }
        let Some(deployment) = self.selected_instance_deployment().cloned() else {
            self.set_notices("Select an instance first.");
            return;
        };
        let preview = format!(
            "CLEAN UP deployment `{}`?\n\nThis stops every service and PERMANENTLY DELETES Switchyard-owned named volumes for this deployment. Volume data cannot be recovered by Switchyard.\n\nDefinition files and unmanaged source directories are not deleted.",
            deployment.name
        );
        if confirm::destructive_cleanup(&preview) {
            self.start_operation(
                "cleanup".into(),
                Some(deployment.name),
                true,
                cli_shell_spec("cleanup", &deployment.bundle, true),
            );
        }
    }

    fn start_operation(
        &mut self,
        label: String,
        deployment: Option<String>,
        destructive: bool,
        spec: OperationSpec,
    ) -> bool {
        if let Err(error) = self.operation_gate.try_start() {
            self.set_notices(error);
            return false;
        }
        let job = OperationJob {
            project_dir: self.state.project_dir.clone(),
            label,
            deployment,
            destructive,
            spec,
        };
        match tasks::start(job, self.handle()) {
            Ok(_) => {
                self.set_busy(true);
                self.set_notices("Operation started; output is streaming to Operations.");
                true
            }
            Err(error) => {
                self.operation_gate.finish();
                self.set_busy(false);
                self.set_notices(&format!("Could not start operation: {error}"));
                false
            }
        }
    }

    fn navigate_to_next_action(&mut self) {
        if self
            .control(self.tabs)
            .and_then(Tab::current_tab)
            .is_none_or(|index| index != 0)
        {
            return;
        }
        let destination = home::next_action(&self.state).destination as usize;
        let tabs = self.tabs;
        if let Some(tab) = self.control_mut(tabs) {
            tab.set_current_tab(destination);
        }
    }

    fn try_quit(&mut self) {
        if !self.operation_gate.is_running()
            || dialogs::validate(
                "Operation in progress",
                "An operation is still running. Quit Switchyard anyway?",
            )
        {
            self.close();
        }
    }
}

#[derive(Clone, Copy)]
enum InstanceCommand {
    Validate,
    Plan,
    Start,
    Stop,
}

fn cli_shell_spec(command: &str, bundle: &Path, confirmed: bool) -> OperationSpec {
    let executable = std::env::var_os("SWITCHYARD_BIN")
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok())
        .unwrap_or_else(|| PathBuf::from("switchyard"));
    let mut shell = format!(
        "exec {} {} {}",
        shell_quote(&executable.to_string_lossy()),
        shell_quote(command),
        shell_quote(&bundle.to_string_lossy()),
    );
    if confirmed {
        shell.push_str(" --yes");
    }
    OperationSpec::Shell(shell)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

impl ButtonEvents for SwitchyardShell {
    fn on_pressed(&mut self, handle: Handle<Button>) -> EventProcessStatus {
        if handle == self.home.next {
            self.navigate_to_next_action();
            EventProcessStatus::Processed
        } else {
            EventProcessStatus::Ignored
        }
    }
}

impl TimerEvents for SwitchyardShell {
    fn on_update(&mut self, _ticks: u64) -> EventProcessStatus {
        if let Some(timer) = self.timer() {
            timer.stop();
        }
        if self.reopen_code_pending {
            self.reopen_code_pending = false;
            self.restore_code_tab();
        }
        EventProcessStatus::Processed
    }
}

impl WindowEvents for SwitchyardShell {
    fn on_activate(&mut self) {
        if self.reopen_code_pending
            && let Some(timer) = self.timer()
            && !timer.is_running()
        {
            timer.start(std::time::Duration::from_millis(30));
        }
    }

    fn on_cancel(&mut self) -> ActionRequest {
        if !self.operation_gate.is_running()
            || dialogs::validate(
                "Operation in progress",
                "An operation is still running. Quit Switchyard anyway?",
            )
        {
            ActionRequest::Allow
        } else {
            ActionRequest::Deny
        }
    }
}

impl TreeViewEvents<code::TreeRow> for SwitchyardShell {
    fn on_current_item_changed(
        &mut self,
        handle: Handle<TreeView<code::TreeRow>>,
        _: Handle<treeview::Item<code::TreeRow>>,
    ) -> EventProcessStatus {
        if handle == self.code.tree {
            self.update_code_detail();
            EventProcessStatus::Processed
        } else {
            EventProcessStatus::Ignored
        }
    }

    fn on_item_action(
        &mut self,
        handle: Handle<TreeView<code::TreeRow>>,
        _: Handle<treeview::Item<code::TreeRow>>,
    ) -> EventProcessStatus {
        if handle == self.code.tree {
            self.show_code_details();
            EventProcessStatus::Processed
        } else {
            EventProcessStatus::Ignored
        }
    }
}

impl ListViewEvents<ProfileRowView> for SwitchyardShell {
    fn on_current_item_changed(
        &mut self,
        handle: Handle<ListView<ProfileRowView>>,
    ) -> EventProcessStatus {
        if handle == self.profiles.list {
            self.update_profile_detail();
            EventProcessStatus::Processed
        } else {
            EventProcessStatus::Ignored
        }
    }
    fn on_item_action(
        &mut self,
        handle: Handle<ListView<ProfileRowView>>,
        _: usize,
    ) -> EventProcessStatus {
        if handle == self.profiles.list {
            self.show_profile_details();
            EventProcessStatus::Processed
        } else {
            EventProcessStatus::Ignored
        }
    }
}

impl ListViewEvents<InstanceRowView> for SwitchyardShell {
    fn on_current_item_changed(
        &mut self,
        handle: Handle<ListView<InstanceRowView>>,
    ) -> EventProcessStatus {
        if handle == self.instances.list {
            self.update_instance_detail();
            EventProcessStatus::Processed
        } else {
            EventProcessStatus::Ignored
        }
    }

    fn on_item_action(
        &mut self,
        handle: Handle<ListView<InstanceRowView>>,
        _: usize,
    ) -> EventProcessStatus {
        if handle == self.instances.list {
            self.show_instance_details();
            EventProcessStatus::Processed
        } else {
            EventProcessStatus::Ignored
        }
    }
}

impl ListViewEvents<ConnectionRowView> for SwitchyardShell {
    fn on_item_action(
        &mut self,
        handle: Handle<ListView<ConnectionRowView>>,
        _: usize,
    ) -> EventProcessStatus {
        if handle == self.connections.list {
            self.switch_connection();
            EventProcessStatus::Processed
        } else {
            EventProcessStatus::Ignored
        }
    }
}

impl ListViewEvents<DeviceRowView> for SwitchyardShell {
    fn on_current_item_changed(
        &mut self,
        handle: Handle<ListView<DeviceRowView>>,
    ) -> EventProcessStatus {
        if handle == self.devices.list {
            self.update_device_detail();
            EventProcessStatus::Processed
        } else {
            EventProcessStatus::Ignored
        }
    }
    fn on_item_action(
        &mut self,
        handle: Handle<ListView<DeviceRowView>>,
        _: usize,
    ) -> EventProcessStatus {
        if handle == self.devices.list {
            self.show_device_details();
            EventProcessStatus::Processed
        } else {
            EventProcessStatus::Ignored
        }
    }
}

impl ListViewEvents<ScriptRowView> for SwitchyardShell {
    fn on_item_action(
        &mut self,
        handle: Handle<ListView<ScriptRowView>>,
        _: usize,
    ) -> EventProcessStatus {
        if handle == self.operations.list {
            self.run_script();
            EventProcessStatus::Processed
        } else {
            EventProcessStatus::Ignored
        }
    }
}

impl TextFieldEvents for SwitchyardShell {
    fn on_text_changed(&mut self, handle: Handle<TextField>) -> EventProcessStatus {
        if handle == self.operations.filter {
            self.refresh_operation_log();
            EventProcessStatus::Processed
        } else {
            EventProcessStatus::Ignored
        }
    }
}

impl BackgroundTaskEvents<StateUpdate, TaskResponse> for SwitchyardShell {
    fn on_update(
        &mut self,
        update: StateUpdate,
        _: &BackgroundTask<StateUpdate, TaskResponse>,
    ) -> EventProcessStatus {
        let log = std::mem::take(&mut self.state.operation_log);
        self.state = update.state;
        self.state.operation_log = log;
        self.operation_gate.finish();
        self.set_busy(false);
        self.apply_state_controls();
        self.set_notices(
            update
                .notice
                .as_deref()
                .unwrap_or("Project state refreshed."),
        );
        if let Some(device) = update.device_probe {
            let detail = device
                .last_check_detail
                .as_deref()
                .unwrap_or("check returned no detail");
            let eligible = device.last_check_status == DeviceCheckStatus::Eligible;
            let prompt = if eligible {
                format!(
                    "Check result for `{}`:\n\n{}\n\nSave this SSH device?",
                    device.name, detail
                )
            } else {
                format!(
                    "Check result for `{}`:\n\n{}\n\nThis device is currently ineligible. Save anyway? The failed result will remain visible and F6 can re-check it later.",
                    device.name, detail
                )
            };
            if dialogs::validate(
                if eligible {
                    "Device eligible"
                } else {
                    "Save ineligible device?"
                },
                &prompt,
            ) {
                let saved =
                    StateStore::open(self.state.project_dir.join(".switchyard/state.sqlite3"))
                        .map_err(|error| error.to_string())
                        .and_then(|(store, _)| {
                            store
                                .register_device(&device)
                                .map_err(|error| error.to_string())
                        });
                match saved {
                    Ok(()) => {
                        let project_dir = self.state.project_dir.clone();
                        let log = std::mem::take(&mut self.state.operation_log);
                        self.state = ProjectState::load(&project_dir);
                        self.state.operation_log = log;
                        self.apply_state_controls();
                        self.set_notices(&format!(
                            "SSH device `{}` saved with its check result.",
                            device.name
                        ));
                    }
                    Err(error) => self.set_notices(&format!("Device save failed: {error}")),
                }
            }
        }
        EventProcessStatus::Processed
    }

    fn on_finish(&mut self, _: &BackgroundTask<StateUpdate, TaskResponse>) -> EventProcessStatus {
        if self.operation_gate.is_running() {
            self.operation_gate.finish();
            self.set_busy(false);
            self.set_code_notice("Project operation ended without a result.");
        }
        EventProcessStatus::Processed
    }

    fn on_query(
        &mut self,
        _: StateUpdate,
        _: &BackgroundTask<StateUpdate, TaskResponse>,
    ) -> TaskResponse {
        TaskResponse::Continue
    }
}

impl BackgroundTaskEvents<OpUpdate, OpCommand> for SwitchyardShell {
    fn on_update(
        &mut self,
        update: OpUpdate,
        _: &BackgroundTask<OpUpdate, OpCommand>,
    ) -> EventProcessStatus {
        match update {
            OpUpdate::Started {
                label,
                deployment,
                destructive,
            } => {
                self.state
                    .operation_log
                    .start(label, deployment, destructive);
            }
            OpUpdate::Output(line) => self.state.operation_log.append(line),
            OpUpdate::Finished(exit_code) => {
                self.state
                    .operation_log
                    .finish(OperationOutcome::Finished(exit_code));
                self.operation_gate.finish();
                self.set_busy(false);
                self.set_notices(if exit_code == 0 {
                    "Operation completed successfully; refreshing project observations…"
                } else {
                    "Operation finished with a non-zero exit code. Review Operations output."
                });
                let was_bind = self.pending_bind.is_some();
                self.finish_bind(
                    exit_code == 0,
                    &format!("Process completed with exit code {exit_code}."),
                );
                if exit_code == 0 || was_bind {
                    self.refresh_state();
                }
            }
            OpUpdate::Failed(error) => {
                self.state.operation_log.append(format!("ERROR: {error}"));
                self.state
                    .operation_log
                    .finish(OperationOutcome::Failed(error.clone()));
                self.operation_gate.finish();
                self.set_busy(false);
                self.set_notices(
                    "Operation failed. Review Operations output for the verbatim error.",
                );
                let was_bind = self.pending_bind.is_some();
                self.finish_bind(false, &format!("Background execution failed: {error}"));
                if was_bind {
                    self.refresh_state();
                }
            }
        }
        self.refresh_operation_log();
        EventProcessStatus::Processed
    }

    fn on_finish(&mut self, _: &BackgroundTask<OpUpdate, OpCommand>) -> EventProcessStatus {
        if self.state.operation_log.last_is_running() {
            self.state.operation_log.finish(OperationOutcome::Failed(
                "background operation stopped unexpectedly".into(),
            ));
            self.operation_gate.finish();
            self.set_busy(false);
            self.set_notices("Background operation stopped unexpectedly.");
            let was_bind = self.pending_bind.is_some();
            self.finish_bind(false, "Background operation stopped unexpectedly.");
            if was_bind {
                self.refresh_state();
            }
            self.refresh_operation_log();
        }
        EventProcessStatus::Processed
    }

    fn on_query(&mut self, _: OpUpdate, _: &BackgroundTask<OpUpdate, OpCommand>) -> OpCommand {
        OpCommand::Continue
    }
}

impl CommandBarEvents for SwitchyardShell {
    fn on_update_commandbar(&self, commandbar: &mut CommandBar) {
        commandbar.set(key!("F1"), "Help", switchyardshell::Commands::Help);
        commandbar.set(key!("F5"), "Refresh", switchyardshell::Commands::Refresh);
        commandbar.set(key!("Escape"), "Quit", switchyardshell::Commands::Quit);
        commandbar.set(key!("Ctrl+Q"), "Quit", switchyardshell::Commands::Quit);
        if self
            .control(self.tabs)
            .and_then(Tab::current_tab)
            .is_some_and(|index| index == 0)
        {
            commandbar.set(key!("Enter"), "Next step", switchyardshell::Commands::Next);
        } else if self
            .control(self.tabs)
            .and_then(Tab::current_tab)
            .is_some_and(|index| index == 1)
        {
            // List controls reserve selection keys; tab actions stay on the
            // standard F-key/Delete/Enter scheme.
            commandbar.set(key!("F2"), "Add", switchyardshell::Commands::AddCode);
            commandbar.set(key!("F3"), "Worktree", switchyardshell::Commands::Worktree);
            commandbar.set(
                key!("Delete"),
                "Remove",
                switchyardshell::Commands::RemoveCode,
            );
            commandbar.set(
                key!("Enter"),
                "Details",
                switchyardshell::Commands::CodeDetails,
            );
        } else if self
            .control(self.tabs)
            .and_then(Tab::current_tab)
            .is_some_and(|index| index == 2)
        {
            commandbar.set(key!("F2"), "New", switchyardshell::Commands::NewProfile);
            commandbar.set(key!("F3"), "Edit", switchyardshell::Commands::EditProfile);
            commandbar.set(
                key!("F4"),
                "Validate",
                switchyardshell::Commands::ValidateProfile,
            );
            commandbar.set(
                key!("F6"),
                "Import",
                switchyardshell::Commands::ImportProfile,
            );
            commandbar.set(
                key!("Delete"),
                "Remove",
                switchyardshell::Commands::RemoveProfile,
            );
            commandbar.set(
                key!("Enter"),
                "Details",
                switchyardshell::Commands::ProfileDetails,
            );
        } else if self
            .control(self.tabs)
            .and_then(Tab::current_tab)
            .is_some_and(|index| index == 3)
        {
            commandbar.set(key!("F2"), "New", switchyardshell::Commands::NewInstance);
            commandbar.set(
                key!("Enter"),
                "Details",
                switchyardshell::Commands::InstanceDetails,
            );
            commandbar.set(
                key!("F7"),
                "Validate",
                switchyardshell::Commands::ValidateInstance,
            );
            commandbar.set(key!("F8"), "Plan", switchyardshell::Commands::PlanInstance);
            commandbar.set(
                key!("F9"),
                "Start",
                switchyardshell::Commands::StartInstance,
            );
            commandbar.set(key!("F10"), "Stop", switchyardshell::Commands::StopInstance);
            commandbar.set(
                key!("Ctrl+Delete"),
                "Cleanup",
                switchyardshell::Commands::CleanupInstance,
            );
        } else if self
            .control(self.tabs)
            .and_then(Tab::current_tab)
            .is_some_and(|index| index == 4)
        {
            commandbar.set(
                key!("Enter"),
                "Switch",
                switchyardshell::Commands::SwitchConnection,
            );
        } else if self
            .control(self.tabs)
            .and_then(Tab::current_tab)
            .is_some_and(|index| index == 5)
        {
            commandbar.set(key!("F2"), "Add", switchyardshell::Commands::AddDevice);
            commandbar.set(
                key!("F6"),
                "Re-check",
                switchyardshell::Commands::CheckDevice,
            );
            commandbar.set(
                key!("Delete"),
                "Remove",
                switchyardshell::Commands::RemoveDevice,
            );
            commandbar.set(
                key!("Enter"),
                "Details",
                switchyardshell::Commands::DeviceDetails,
            );
        } else if self
            .control(self.tabs)
            .and_then(Tab::current_tab)
            .is_some_and(|index| index == 6)
        {
            commandbar.set(key!("F2"), "New", switchyardshell::Commands::NewScript);
            commandbar.set(key!("F3"), "Edit", switchyardshell::Commands::EditScript);
            commandbar.set(
                key!("Delete"),
                "Delete",
                switchyardshell::Commands::RemoveScript,
            );
            commandbar.set(key!("Enter"), "Run", switchyardshell::Commands::RunScript);
        }
    }

    fn on_event(&mut self, command_id: switchyardshell::Commands) {
        match command_id {
            switchyardshell::Commands::Help => {
                HelpDialog::new().show();
            }
            switchyardshell::Commands::Refresh => self.refresh_state(),
            switchyardshell::Commands::Quit => self.try_quit(),
            switchyardshell::Commands::Next => self.navigate_to_next_action(),
            switchyardshell::Commands::AddCode => self.add_code(),
            switchyardshell::Commands::Worktree => self.create_worktree(),
            switchyardshell::Commands::RemoveCode => self.remove_code(),
            switchyardshell::Commands::CodeDetails => self.show_code_details(),
            switchyardshell::Commands::NewProfile => self.edit_profile(true),
            switchyardshell::Commands::EditProfile => self.edit_profile(false),
            switchyardshell::Commands::ValidateProfile => self.validate_profile(),
            switchyardshell::Commands::ImportProfile => self.import_profile(),
            switchyardshell::Commands::RemoveProfile => self.remove_profile(),
            switchyardshell::Commands::ProfileDetails => self.show_profile_details(),
            switchyardshell::Commands::NewInstance => self.create_instance(),
            switchyardshell::Commands::InstanceDetails => self.show_instance_details(),
            switchyardshell::Commands::ValidateInstance => {
                self.run_instance_command(InstanceCommand::Validate)
            }
            switchyardshell::Commands::PlanInstance => {
                self.run_instance_command(InstanceCommand::Plan)
            }
            switchyardshell::Commands::StartInstance => {
                self.run_instance_command(InstanceCommand::Start)
            }
            switchyardshell::Commands::StopInstance => {
                self.run_instance_command(InstanceCommand::Stop)
            }
            switchyardshell::Commands::CleanupInstance => self.cleanup_instance(),
            switchyardshell::Commands::SwitchConnection => self.switch_connection(),
            switchyardshell::Commands::AddDevice => self.add_device(),
            switchyardshell::Commands::CheckDevice => self.check_device(),
            switchyardshell::Commands::RemoveDevice => self.remove_device(),
            switchyardshell::Commands::DeviceDetails => self.show_device_details(),
            switchyardshell::Commands::NewScript => self.edit_script(true),
            switchyardshell::Commands::EditScript => self.edit_script(false),
            switchyardshell::Commands::RemoveScript => self.remove_script(),
            switchyardshell::Commands::RunScript => self.run_script(),
        }
    }
}

pub(crate) fn run_app(
    project_dir: &Path,
    outcome: OutcomeCell,
) -> Result<ExitOutcome, Box<dyn Error>> {
    let mut app = App::new().single_window().command_bar().build()?;
    app.add_window(SwitchyardShell::new(project_dir, outcome.clone()));
    app.run();
    Ok(outcome.take())
}
