mod devices;
mod instances;
mod sources;

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Tabs, Wrap},
};

use crate::app::{ActiveView, App, BusyKind, Overlay};

pub(crate) fn render(frame: &mut Frame<'_>, app: &App) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(2),
        ])
        .split(frame.area());
    let selected = match app.active_view {
        ActiveView::Sources => 0,
        ActiveView::Devices => 1,
        ActiveView::Instances => 2,
    };
    let tabs = Tabs::new(["Sources", "Devices", "Instances"])
        .select(selected)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Switchyard — {} ", app.project_dir.display())),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(tabs, areas[0]);
    match app.active_view {
        ActiveView::Sources => sources::render(frame, areas[1], app),
        ActiveView::Devices => devices::render(frame, areas[1], app),
        ActiveView::Instances => instances::render(frame, areas[1], app),
    }
    let footer = if let Some(kind) = app.busy {
        let spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"][app.spinner_tick % 10];
        format!(
            "{spinner} {}…  q quit  ? help",
            match kind {
                BusyKind::Add => "adding source",
                BusyKind::WorktreeAdd => "creating worktree",
                BusyKind::Remove => "removing source",
                BusyKind::Refresh => "refreshing sources",
                BusyKind::DeviceAdd => "adding device",
                BusyKind::DeviceRemove => "removing device",
                BusyKind::DeviceCheck => "checking device",
                BusyKind::Operation => "running operation",
            }
        )
    } else {
        let keys = match app.active_view {
            ActiveView::Sources => {
                "a add repository/path  w new worktree  d remove  r refresh  ↑/↓ select  Tab view  ? help  q quit"
            }
            ActiveView::Devices => "a add  c check  d remove  ↑/↓ select  Tab view  ? help  q quit",
            ActiveView::Instances => {
                "i add instance  b pair  u/s/x/p lifecycle  Enter run  n/e/D scripts  PgUp/PgDn output  Tab view  ? help  q quit"
            }
        };
        app.status
            .as_ref()
            .map_or_else(|| keys.into(), |status| format!("{status}  |  {keys}"))
    };
    frame.render_widget(Paragraph::new(footer), areas[2]);

    match &app.overlay {
        Overlay::Add(form) => sources::render_add(frame, form, app.busy),
        Overlay::Worktree(form) => sources::render_worktree(frame, app, form, app.busy),
        Overlay::Device(form) => devices::render_add(frame, form, app.busy),
        Overlay::ConfirmRemoveDevice { name, error } => {
            devices::render_confirm(frame, name, error.as_deref(), app.busy)
        }
        Overlay::ConfirmRemove { name, error } => {
            sources::render_confirm(frame, name, error.as_deref(), app.busy)
        }
        Overlay::Script(form) => instances::render_script_form(frame, form),
        Overlay::Instance(form) => instances::render_instance_form(frame, app, form),
        Overlay::Pair(form) => instances::render_pair_form(frame, app, form),
        Overlay::ConfirmDown { deployment } => instances::render_confirm(
            frame,
            " Confirm down ",
            &format!("Stop deployment `{deployment}`? Named volumes will be preserved."),
            None,
        ),
        Overlay::ConfirmDeleteScript { name, error, .. } => instances::render_confirm(
            frame,
            " Delete run script ",
            &format!("Delete run script `{name}`?"),
            error.as_deref(),
        ),
        Overlay::ShellNotice { name, .. } => instances::render_confirm(
            frame,
            " Shell script warning ",
            &format!("`{name}` executes an arbitrary command using your shell in this project."),
            None,
        ),
        Overlay::Help => render_help(frame),
        Overlay::None => {}
    }
}

fn render_help(frame: &mut Frame<'_>) {
    let area = centered(frame.area(), 68, 29);
    frame.render_widget(Clear, area);
    let lines = vec![
        Line::from(Span::styled(
            "Global",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from("  Tab / ← / →   switch view"),
        Line::from("  ?             toggle help"),
        Line::from("  q / Ctrl-C    quit"),
        Line::from(""),
        Line::from(Span::styled(
            "Sources",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from("  ↑ / ↓, j / k  select source"),
        Line::from("  a             add one local path or Git clone address"),
        Line::from("  w             branch from selected checkout into a worktree"),
        Line::from("  Enter / F2    review ref; Git auth runs in terminal"),
        Line::from("  d             remove/deregister selected source"),
        Line::from("  r             refresh live Git state"),
        Line::from("  Esc           close a dialog"),
        Line::from(""),
        Line::from(Span::styled(
            "Devices",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from("  ↑ / ↓, j / k  select device"),
        Line::from("  a / c / d     add, check SSH, remove"),
        Line::from(""),
        Line::from(Span::styled(
            "Instances",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from("  u / s / x / p up, status, confirmed down, plan"),
        Line::from("  i             add instance (startup profile/worktree selectors)"),
        Line::from("  b             select and apply a provider-group pairing"),
        Line::from("  Enter         run selected preset"),
        Line::from("  n / e / D     new, edit, delete preset"),
        Line::from("  PgUp / PgDn   scroll operation output"),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" Help "))
            .wrap(Wrap { trim: false }),
        area,
    );
}

pub(crate) fn centered(
    area: ratatui::layout::Rect,
    width: u16,
    height: u16,
) -> ratatui::layout::Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Length(height.min(area.height)),
            Constraint::Percentage(50),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Length(width.min(area.width)),
            Constraint::Percentage(50),
        ])
        .split(vertical[1])[1]
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::app::{AddForm, AddSourceMode, AddSourcePanel, DeviceForm};

    #[test]
    fn renders_inline_add_error_with_test_backend() {
        let backend = TestBackend::new(90, 28);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::with_sources(PathBuf::from("/project"), Vec::new());
        app.overlay = Overlay::Add(AddForm {
            error: Some("enter an existing local directory".into()),
            ..AddForm::default()
        });
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let contents = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(contents.contains("enter an existing local directory"));
        assert!(contents.contains("Enter exactly one source location"));
    }

    #[test]
    fn renders_native_git_authentication_handoff_popup() {
        let backend = TestBackend::new(110, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::with_sources(PathBuf::from("/project"), Vec::new());
        app.overlay = Overlay::Add(AddForm {
            mode: AddSourceMode::Git,
            location: "git@github.com:team/project.git".into(),
            panel: AddSourcePanel::GitOptions,
            ..AddForm::default()
        });
        let Overlay::Add(form) = &mut app.overlay else {
            panic!("add source form closed")
        };
        form.error = Some("Permission denied (publickey).\nCheck the selected key.".into());
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let contents = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(contents.contains("Git clone options"));
        assert!(contents.contains("Native authentication"));
        assert!(contents.contains("automatic key selection"));
        assert!(contents.contains("You may be prompted by Git or SSH"));
        assert!(contents.contains("Check the selected key"));
    }

    #[test]
    fn renders_removal_guard_error_in_confirmation() {
        let backend = TestBackend::new(90, 28);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::with_sources(PathBuf::from("/project"), Vec::new());
        app.overlay = Overlay::ConfirmRemove {
            name: "feature".into(),
            error: Some("source has 1 staged, 0 unstaged, and 0 untracked path(s)".into()),
        };
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let contents = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(contents.contains("source has 1 staged"));
    }

    #[test]
    fn renders_devices_tab_and_inline_device_error() {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::with_sources(PathBuf::from("/project"), Vec::new());
        app.active_view = ActiveView::Devices;
        app.overlay = Overlay::Device(DeviceForm {
            error: Some("host cannot be empty".into()),
            ..DeviceForm::default()
        });
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let contents = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(contents.contains("Sources"));
        assert!(contents.contains("Devices"));
        assert!(contents.contains("Instances"));
        assert!(contents.contains("host cannot be empty"));
        assert!(contents.contains("Identity file"));
    }

    #[test]
    fn renders_new_script_modal_and_down_confirmation() {
        let backend = TestBackend::new(110, 34);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::with_sources(PathBuf::from("/project"), Vec::new());
        app.active_view = ActiveView::Instances;
        app.overlay = Overlay::Script(crate::app::ScriptForm::default());
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let contents = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(contents.contains("New run script"));
        assert!(contents.contains("Command (Space cycles)"));

        app.overlay = Overlay::ConfirmDown {
            deployment: "demo".into(),
        };
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let contents = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(contents.contains("Stop deployment `demo`"));
    }
}
