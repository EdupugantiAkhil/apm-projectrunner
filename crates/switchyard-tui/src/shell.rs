use std::{
    collections::VecDeque,
    error::Error,
    path::{Path, PathBuf},
    sync::Mutex,
};

use appcui::prelude::*;

use crate::{
    dialogs::confirm,
    handoff::{ExitOutcome, OutcomeCell},
    state::ProjectState,
    tabs::{self, code, home},
};
use code::TreeRow;

static STATE_JOBS: Mutex<VecDeque<StateJob>> = Mutex::new(VecDeque::new());

enum StateAction {
    Refresh,
    Register { name: String, path: PathBuf },
    Worktree(code::WorktreeRequest),
    Remove { name: String },
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
- Tab actions use **F-keys** (shown in the bottom command bar). Typing letters
  in a list searches it; **Insert/Space** select list items.

## Code tab

- **F2** — add code (register a local directory or clone a repository).
- **F3** — create a managed worktree from the selected repository.
- **Delete** — safely remove the selected managed entry.
- **Enter** — show full details for the selection.

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
    events = ButtonEvents + WindowEvents + CommandBarEvents + TreeViewEvents<TreeRow> + BackgroundTaskEvents<StateUpdate, TaskResponse> + TimerEvents,
    commands = [Help, Refresh, Quit, Next, AddCode, Worktree, RemoveCode, CodeDetails]
)]
pub(crate) struct SwitchyardShell {
    tabs: Handle<Tab>,
    home: home::Handles,
    code: code::Handles,
    state: ProjectState,
    outcome: OutcomeCell,
    operation_running: bool,
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
            state,
            outcome,
            operation_running: false,
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
        tabs::profiles::add(&mut tab, profiles_index);
        tabs::instances::add(&mut tab, instances_index);
        tabs::connections::add(&mut tab, connections_index);
        tabs::devices::add(&mut tab, devices_index);
        tabs::operations::add(&mut tab, operations_index);
        shell.tabs = shell.add(tab);
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
    }

    fn start_state_job(&mut self, action: StateAction, busy_notice: &str) {
        if self.operation_running {
            self.set_code_notice("Another project operation is already running.");
            return;
        }
        let Ok(mut jobs) = STATE_JOBS.lock() else {
            self.set_code_notice("Could not start the project operation: task queue unavailable.");
            return;
        };
        jobs.push_back(StateJob {
            project_dir: self.state.project_dir.clone(),
            action,
        });
        drop(jobs);
        self.operation_running = true;
        self.set_code_notice(busy_notice);
        self.state_task = BackgroundTask::run(execute_state_job, self.handle());
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

    fn add_code(&mut self) {
        if self.operation_running {
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
        if self.operation_running {
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
        if self.operation_running {
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
        if !self.operation_running
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
        if !self.operation_running
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

impl BackgroundTaskEvents<StateUpdate, TaskResponse> for SwitchyardShell {
    fn on_update(
        &mut self,
        update: StateUpdate,
        _: &BackgroundTask<StateUpdate, TaskResponse>,
    ) -> EventProcessStatus {
        self.state = update.state;
        self.operation_running = false;
        self.apply_state_controls();
        self.set_code_notice(
            update
                .notice
                .as_deref()
                .unwrap_or("Project state refreshed."),
        );
        EventProcessStatus::Processed
    }

    fn on_finish(&mut self, _: &BackgroundTask<StateUpdate, TaskResponse>) -> EventProcessStatus {
        if self.operation_running {
            self.operation_running = false;
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
            // List controls consume Insert/Space for selection and plain
            // letters for their search bar, so actions bind to F-keys.
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
