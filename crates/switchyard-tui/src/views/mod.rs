mod devices;
mod instances;
mod profiles;
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
        ActiveView::Profiles => 1,
        ActiveView::Devices => 2,
        ActiveView::Instances => 3,
    };
    let tabs = Tabs::new(["Sources", "Profiles", "Devices", "Instances"])
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
        ActiveView::Profiles => profiles::render(frame, areas[1], app),
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
                BusyKind::ProfileRefresh => "refreshing startup profiles",
                BusyKind::ProfileInspect => "loading profile details",
                BusyKind::ProfileReview => "loading import review",
                BusyKind::ProfileImport => "importing startup profile",
                BusyKind::ProfileRemove => "removing profile import",
                BusyKind::Operation => "running operation",
            }
        )
    } else {
        let keys = match app.active_view {
            ActiveView::Sources => {
                "a add repository/path  w new worktree  d remove  r refresh  ↑/↓ select  Tab view  ? help  q quit"
            }
            ActiveView::Profiles => {
                "Enter inspect  i import/re-review  d remove import  r refresh  ↑/↓ select  Tab view  ? help  q quit"
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
        Overlay::ProfileInspector {
            name,
            lines,
            scroll,
        } => profiles::render_inspector(frame, name, lines, *scroll),
        Overlay::ProfileReview {
            name,
            source,
            yaml,
            scroll,
            error,
        } => profiles::render_review(
            frame,
            name,
            source,
            yaml,
            *scroll,
            error.as_deref(),
            app.busy,
        ),
        Overlay::ConfirmRemoveProfile { name, error } => {
            profiles::render_remove_confirm(frame, name, error.as_deref(), app.busy)
        }
        Overlay::Help => render_help(frame),
        Overlay::None => {}
    }
}

fn render_help(frame: &mut Frame<'_>) {
    let area = centered(frame.area(), 68, 38);
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
            "Startup profiles",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from("  ↑ / ↓, j / k  select profile"),
        Line::from("  Enter         inspect full profile definition"),
        Line::from("  i             review and explicitly import/re-import"),
        Line::from("  d             confirm removal of an import"),
        Line::from("  r             refresh project and source profiles"),
        Line::from("  Esc           close inspector/review without changes"),
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
    use switchyard_ops::profiles::{
        ProfileAdapterKind, ProfileOrigin, ProfileRow, ProfileService, ProfileTrust,
        SourceManifestError,
    };

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

    fn profile_row(name: &str, origin: ProfileOrigin, trust: ProfileTrust) -> ProfileRow {
        ProfileRow {
            name: name.into(),
            origin,
            trust,
            shadowed: false,
            services: vec![ProfileService {
                name: "api".into(),
                adapter_kind: ProfileAdapterKind::Container,
            }],
        }
    }

    fn rendered(app: &App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn profiles_render_all_trust_words_and_shadow_marker() {
        let mut app = App::with_sources(PathBuf::from("/project"), Vec::new());
        app.active_view = ActiveView::Profiles;
        app.profiles = vec![
            profile_row("project", ProfileOrigin::Project, ProfileTrust::Trusted),
            profile_row(
                "imported",
                ProfileOrigin::ImportedFromSource {
                    source: "main".into(),
                    commit: Some("1234567890abcdef".into()),
                },
                ProfileTrust::Imported,
            ),
            profile_row(
                "changed",
                ProfileOrigin::ImportedFromSource {
                    source: "feature".into(),
                    commit: Some("abcdef".into()),
                },
                ProfileTrust::Changed,
            ),
            profile_row(
                "found",
                ProfileOrigin::DiscoveredInSource {
                    source: "other".into(),
                    commit: Some("fedcba987654".into()),
                },
                ProfileTrust::NotImported,
            ),
        ];
        app.profiles[1].shadowed = true;
        let contents = rendered(&app, 180, 30);
        for expected in [
            "trusted",
            "imported",
            "changed — review",
            "not imported",
            "shadowed by project profile",
            "api(container)",
            "imported from main@1234567890",
            "found in other",
        ] {
            assert!(contents.contains(expected), "missing `{expected}`");
        }
    }

    #[test]
    fn profiles_empty_state_teaches_manifest_and_refresh_key() {
        let mut app = App::with_sources(PathBuf::from("/project"), Vec::new());
        app.active_view = ActiveView::Profiles;
        let contents = rendered(&app, 120, 24);
        assert!(contents.contains("project deployment definition"));
        assert!(contents.contains("switchyard-profiles.yaml"));
        assert!(contents.contains("Press r to refresh"));
    }

    #[test]
    fn profiles_render_source_manifest_diagnostics() {
        let mut app = App::with_sources(PathBuf::from("/project"), Vec::new());
        app.active_view = ActiveView::Profiles;
        app.profile_source_errors = vec![SourceManifestError {
            source: "broken-checkout".into(),
            message: "invalid profile manifest: expected a map".into(),
        }];
        let contents = rendered(&app, 130, 24);
        assert!(contents.contains("Source diagnostics"));
        assert!(contents.contains("broken-checkout"));
        assert!(contents.contains("expected a map"));
    }

    #[test]
    fn profile_review_and_remove_popups_name_explicit_confirmation() {
        let mut app = App::with_sources(PathBuf::from("/project"), Vec::new());
        app.active_view = ActiveView::Profiles;
        app.overlay = Overlay::ProfileReview {
            name: "api".into(),
            source: "feature".into(),
            yaml: "services:\n  api:\n    execution:\n      type: container".into(),
            scroll: 0,
            error: None,
        };
        let review = rendered(&app, 120, 38);
        assert!(review.contains("Review import"));
        assert!(review.contains("Enter confirm import"));
        assert!(review.contains("Esc cancel"));

        app.overlay = Overlay::ConfirmRemoveProfile {
            name: "api".into(),
            error: None,
        };
        let remove = rendered(&app, 100, 24);
        assert!(remove.contains("Enter confirm removal"));
        assert!(remove.contains("source manifest and project profiles are not changed"));
    }
}
