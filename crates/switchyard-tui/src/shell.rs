use std::{
    collections::VecDeque,
    error::Error,
    path::{Path, PathBuf},
    sync::Mutex,
};

use appcui::prelude::*;

use crate::{
    dialogs::wizard,
    handoff::{ExitOutcome, OutcomeCell},
    state::ProjectState,
    tabs::{self, code, connections, devices, home, instances, operations, profiles},
    tasks::{self, OpCommand, OpUpdate, OperationGate, OperationJob},
};
use code::TreeRow;
use connections::ConnectionRowView;
use devices::DeviceRowView;
use instances::{InstanceCommand, InstanceRowView};
use operations::ScriptRowView;
use profiles::ProfileRowView;
use switchyard_ops::{create_instance, execution::OperationSpec};
use switchyard_state::{DeviceCheckStatus, RegisteredDevice, StateStore};

static STATE_JOBS: Mutex<VecDeque<StateJob>> = Mutex::new(VecDeque::new());

pub(crate) enum StateAction {
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
    /// Whether the finished job held the mutation gate (refreshes do not).
    releases_gate: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingBind {
    pub(crate) deployment: String,
    pub(crate) consumer: String,
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
    let releases_gate = !matches!(job.action, StateAction::Refresh);
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
        releases_gate,
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
            crate::help::TEXT,
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
    pub(crate) tabs: Handle<Tab>,
    pub(crate) home: home::Handles,
    pub(crate) code: code::Handles,
    pub(crate) profiles: profiles::Handles,
    pub(crate) instances: instances::Handles,
    pub(crate) connections: connections::Handles,
    pub(crate) devices: devices::Handles,
    pub(crate) operations: operations::Handles,
    busy_chip: Handle<Label>,
    pub(crate) state: ProjectState,
    pub(crate) outcome: OutcomeCell,
    pub(crate) operation_gate: OperationGate,
    pub(crate) pending_bind: Option<PendingBind>,
    state_task: Handle<BackgroundTask<StateUpdate, TaskResponse>>,
    reopen_code_pending: bool,
    initial_selection_pending: bool,
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
            initial_selection_pending: true,
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

    pub(crate) fn refresh_state(&mut self) {
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

    pub(crate) fn start_state_job(&mut self, action: StateAction, busy_notice: &str) {
        // Refreshes are read-only projections; only mutations hold the gate.
        let holds_gate = !matches!(action, StateAction::Refresh);
        if holds_gate {
            if let Err(error) = self.operation_gate.try_start() {
                self.set_notices(error);
                return;
            }
        }
        let Ok(mut jobs) = STATE_JOBS.lock() else {
            if holds_gate {
                self.operation_gate.finish();
            }
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

    pub(crate) fn set_busy(&mut self, busy: bool) {
        let chip = self.busy_chip;
        if let Some(label) = self.control_mut(chip) {
            label.set_caption(if busy { "BUSY" } else { "ready" });
        }
    }

    pub(crate) fn set_notices(&mut self, text: &str) {
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

    pub(crate) fn start_operation(
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
        if self.initial_selection_pending {
            self.initial_selection_pending = false;
            // ListView cannot move past a group header until layout gives it a
            // non-zero viewport. Refill once after activation so grouped and
            // ordinary lists all have a real current row immediately.
            self.apply_state_controls();
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
        if (self.initial_selection_pending || self.reopen_code_pending)
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

impl BackgroundTaskEvents<StateUpdate, TaskResponse> for SwitchyardShell {
    fn on_update(
        &mut self,
        update: StateUpdate,
        _: &BackgroundTask<StateUpdate, TaskResponse>,
    ) -> EventProcessStatus {
        let log = std::mem::take(&mut self.state.operation_log);
        self.state = update.state;
        self.state.operation_log = log;
        if update.releases_gate {
            self.operation_gate.finish();
        }
        self.set_busy(self.operation_gate.is_running());
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
        operations::apply_operation_update(self, update);
        EventProcessStatus::Processed
    }

    fn on_finish(&mut self, _: &BackgroundTask<OpUpdate, OpCommand>) -> EventProcessStatus {
        operations::finish_operation_task(self);
        EventProcessStatus::Processed
    }

    fn on_query(&mut self, _: OpUpdate, _: &BackgroundTask<OpUpdate, OpCommand>) -> OpCommand {
        OpCommand::Continue
    }
}

impl CommandBarEvents for SwitchyardShell {
    fn on_update_commandbar(&self, commandbar: &mut CommandBar) {
        use switchyardshell::Commands as C;
        commandbar.set(key!("F1"), "Help", C::Help);
        commandbar.set(key!("F5"), "Refresh", C::Refresh);
        commandbar.set(key!("Escape"), "Quit", C::Quit);
        commandbar.set(key!("Ctrl+Q"), "Quit", C::Quit);
        match self.control(self.tabs).and_then(Tab::current_tab) {
            Some(0) => {
                commandbar.set(key!("Enter"), "Next step", C::Next);
            }
            Some(1) => {
                commandbar.set(key!("F2"), "Add", C::AddCode);
                commandbar.set(key!("F3"), "Worktree", C::Worktree);
                commandbar.set(key!("Delete"), "Remove", C::RemoveCode);
                commandbar.set(key!("Enter"), "Details", C::CodeDetails);
            }
            Some(2) => {
                commandbar.set(key!("F2"), "New", C::NewProfile);
                commandbar.set(key!("F3"), "Edit", C::EditProfile);
                commandbar.set(key!("F4"), "Validate", C::ValidateProfile);
                commandbar.set(key!("F6"), "Import", C::ImportProfile);
                commandbar.set(key!("Delete"), "Remove", C::RemoveProfile);
                commandbar.set(key!("Enter"), "Details", C::ProfileDetails);
            }
            Some(3) => {
                commandbar.set(key!("F2"), "New", C::NewInstance);
                commandbar.set(key!("Enter"), "Details", C::InstanceDetails);
                commandbar.set(key!("F7"), "Validate", C::ValidateInstance);
                commandbar.set(key!("F8"), "Plan", C::PlanInstance);
                commandbar.set(key!("F9"), "Start", C::StartInstance);
                commandbar.set(key!("F10"), "Stop", C::StopInstance);
                commandbar.set(key!("Ctrl+Delete"), "Cleanup", C::CleanupInstance);
            }
            Some(4) => {
                commandbar.set(key!("Enter"), "Switch", C::SwitchConnection);
            }
            Some(5) => {
                commandbar.set(key!("F2"), "Add", C::AddDevice);
                commandbar.set(key!("F6"), "Re-check", C::CheckDevice);
                commandbar.set(key!("Delete"), "Remove", C::RemoveDevice);
                commandbar.set(key!("Enter"), "Details", C::DeviceDetails);
            }
            Some(6) => {
                commandbar.set(key!("F2"), "New", C::NewScript);
                commandbar.set(key!("F3"), "Edit", C::EditScript);
                commandbar.set(key!("Delete"), "Delete", C::RemoveScript);
                commandbar.set(key!("Enter"), "Run", C::RunScript);
            }
            _ => {}
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
