use std::{error::Error, path::Path};

use appcui::prelude::*;

use crate::{
    handoff::{ExitOutcome, OutcomeCell},
    state::ProjectState,
    tabs::{self, home},
};

const HELP: &str = r#"# Switchyard help

## Global keys

- **Alt+H / C / P / I / N / D / O** — open Home, Code, Profiles, Instances, Connections, Devices, or Operations.
- **Ctrl+Tab / Ctrl+Shift+Tab** — move to the next or previous tab.
- **F5** — refresh every project projection.
- **F1** — open this help.
- **Esc** or **Ctrl+Q** — quit (with confirmation while an operation is running).
- **Tab / Shift+Tab / arrows** — move focus within the current tab.

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
    events = ButtonEvents + WindowEvents + CommandBarEvents,
    commands = [Help, Refresh, Quit, Next]
)]
pub(crate) struct SwitchyardShell {
    tabs: Handle<Tab>,
    home: home::Handles,
    state: ProjectState,
    outcome: OutcomeCell,
    operation_running: bool,
}

impl SwitchyardShell {
    fn new(project_dir: &Path, outcome: OutcomeCell) -> Self {
        let state = ProjectState::load(project_dir);
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
            state,
            outcome,
            operation_running: false,
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
        tabs::code::add(&mut tab, code_index);
        tabs::profiles::add(&mut tab, profiles_index);
        tabs::instances::add(&mut tab, instances_index);
        tabs::connections::add(&mut tab, connections_index);
        tabs::devices::add(&mut tab, devices_index);
        tabs::operations::add(&mut tab, operations_index);
        shell.tabs = shell.add(tab);
        let next = shell.home.next;
        shell.request_focus_for_control(next);
        shell
    }

    fn refresh_state(&mut self) {
        self.state.refresh();
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

impl WindowEvents for SwitchyardShell {
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
        }
    }
}

pub(crate) fn run_app(project_dir: &Path) -> Result<ExitOutcome, Box<dyn Error>> {
    let outcome = OutcomeCell::default();
    let mut app = App::new().single_window().command_bar().build()?;
    app.add_window(SwitchyardShell::new(project_dir, outcome.clone()));
    app.run();
    Ok(outcome.take())
}
