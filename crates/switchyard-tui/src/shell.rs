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
    tabs::{self, code, home, instances, operations, profiles},
    tasks::{self, OpCommand, OpUpdate, OperationGate, OperationJob},
};
use code::TreeRow;
use instances::InstanceRowView;
use profiles::ProfileRowView;
use switchyard_ops::{create_instance, execution::OperationSpec, run_scripts::StructuredCommand};

static STATE_JOBS: Mutex<VecDeque<StateJob>> = Mutex::new(VecDeque::new());

enum StateAction {
    Refresh,
    Register { name: String, path: PathBuf },
    Worktree(code::WorktreeRequest),
    Remove { name: String },
    ImportProfile { source: String, name: String },
    RemoveProfile { name: String },
    CreateInstance(wizard::CreateWizardResult),
}

struct StateJob {
    project_dir: PathBuf,
    action: StateAction,
}

struct StateUpdate {
    state: ProjectState,
    notice: Option<String>,
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
    };
    connector.notify(StateUpdate {
        state: ProjectState::load(&job.project_dir),
        notice,
    });
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
    events = ButtonEvents + WindowEvents + CommandBarEvents + TreeViewEvents<TreeRow> + ListViewEvents<ProfileRowView> + ListViewEvents<InstanceRowView> + BackgroundTaskEvents<StateUpdate, TaskResponse> + BackgroundTaskEvents<OpUpdate, OpCommand> + TimerEvents,
    commands = [Help, Refresh, Quit, Next, AddCode, Worktree, RemoveCode, CodeDetails, NewProfile, EditProfile, ValidateProfile, ImportProfile, RemoveProfile, ProfileDetails, NewInstance, InstanceDetails, ValidateInstance, PlanInstance, StartInstance, StopInstance, CleanupInstance]
)]
pub(crate) struct SwitchyardShell {
    tabs: Handle<Tab>,
    home: home::Handles,
    code: code::Handles,
    profiles: profiles::Handles,
    instances: instances::Handles,
    operations: operations::Handles,
    busy_chip: Handle<Label>,
    state: ProjectState,
    outcome: OutcomeCell,
    operation_gate: OperationGate,
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
            operations: operations::Handles { log: Handle::None },
            busy_chip: Handle::None,
            state,
            outcome,
            operation_gate: OperationGate::default(),
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
        tabs::connections::add(&mut tab, connections_index);
        tabs::devices::add(&mut tab, devices_index);
        shell.operations =
            tabs::operations::add(&mut tab, operations_index, &shell.state.operation_log);
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

    fn refresh_operation_log(&mut self) {
        let text = self.state.operation_log.render();
        let log = self.operations.log;
        if let Some(area) = self.control_mut(log) {
            area.set_text(if text.is_empty() {
                "No operations have run in this session. Use F7–F10 from Instances."
            } else {
                &text
            });
        }
        self.update_instance_detail();
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
    ) {
        if let Err(error) = self.operation_gate.try_start() {
            self.set_notices(error);
            return;
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
            }
            Err(error) => {
                self.operation_gate.finish();
                self.set_busy(false);
                self.set_notices(&format!("Could not start operation: {error}"));
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
        self.set_code_notice(
            update
                .notice
                .as_deref()
                .unwrap_or("Project state refreshed."),
        );
        if let Some(notice) = update.notice.as_deref() {
            self.set_profile_notice(notice);
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
                if exit_code == 0 {
                    self.refresh_state();
                }
            }
            OpUpdate::Failed(error) => {
                self.state.operation_log.append(format!("ERROR: {error}"));
                self.state
                    .operation_log
                    .finish(OperationOutcome::Failed(error));
                self.operation_gate.finish();
                self.set_busy(false);
                self.set_notices(
                    "Operation failed. Review Operations output for the verbatim error.",
                );
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
