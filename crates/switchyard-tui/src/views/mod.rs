mod connections;
mod devices;
mod home;
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
        ActiveView::Home => 0,
        ActiveView::Sources => 1,
        ActiveView::Profiles => 2,
        ActiveView::Devices => 3,
        ActiveView::Instances => 4,
        ActiveView::Connections => 5,
    };
    let tabs = Tabs::new([
        "Home",
        "Sources",
        "Profiles",
        "Devices",
        "Instances",
        "Connections",
    ])
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
        ActiveView::Home => home::render(frame, areas[1], app),
        ActiveView::Sources => sources::render(frame, areas[1], app),
        ActiveView::Profiles => profiles::render(frame, areas[1], app),
        ActiveView::Devices => devices::render(frame, areas[1], app),
        ActiveView::Instances => instances::render(frame, areas[1], app),
        ActiveView::Connections => connections::render(frame, areas[1], app),
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
            ActiveView::Home => {
                "↑/↓ select  Enter open  1–5 jump to checklist step  Tab view  ? help  q quit"
            }
            ActiveView::Sources => {
                "a add repository/path  w new worktree  d remove  r refresh  ↑/↓ select  Tab view  ? help  q quit"
            }
            ActiveView::Profiles => {
                "Enter inspect  i import/re-review  d remove import  r refresh  ↑/↓ select  Tab view  ? help  q quit"
            }
            ActiveView::Devices => "a add  c check  d remove  ↑/↓ select  Tab view  ? help  q quit",
            ActiveView::Instances => {
                "i add instance  u/s/x/p lifecycle  Enter run  n/e/D scripts  PgUp/PgDn output  Tab view  ? help  q quit"
            }
            ActiveView::Connections => {
                "↑/↓ row  ←/→ group draft  Enter preview  Esc cancel  Tab view  ? help  q quit"
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
        Overlay::ConnectionPreview(preview) => connections::render_preview(frame, preview),
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
            "Home",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from("  ↑ / ↓, j / k  select checklist item"),
        Line::from("  Enter         open the selected item's tab"),
        Line::from("  1–5           open that checklist item's tab"),
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
        Line::from("  Enter         run selected preset"),
        Line::from("  n / e / D     new, edit, delete preset"),
        Line::from("  PgUp / PgDn   scroll operation output"),
        Line::from(""),
        Line::from(Span::styled(
            "Connections",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from("  ↑ / ↓, j / k  select consumer slot"),
        Line::from("  ← / →, h / l  draft a compatible complete group"),
        Line::from("  Enter         preview; Enter again applies atomically"),
        Line::from("  Esc           cancel preview and clear its draft"),
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
    use switchyard_adapter_sdk::SourceIdentity;
    use switchyard_ops::instances::InstancePreview;
    use switchyard_ops::profiles::{
        ProfileAdapterKind, ProfileOrigin, ProfileRow, ProfileService, ProfileTrust,
        SourceManifestError,
    };
    use switchyard_ops::projections::{InstanceRow, ServiceRow};
    use switchyard_planner::{Diagnostic, DiagnosticCode};
    use switchyard_sources::{RegisteredSourceInspection, SourceInspection};
    use switchyard_state::{RegisteredSource, RegisteredSourceKind};

    use super::*;
    use crate::app::{
        AddForm, AddSourceMode, AddSourcePanel, DeploymentEntry, DeviceForm, InstanceForm,
        InstanceParameterField, SourceChoice,
    };

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

    #[test]
    fn renders_guided_instance_fields_disabled_reason_and_attached_preview_error() {
        let mut app = App::with_sources(PathBuf::from("/project"), Vec::new());
        app.active_view = ActiveView::Instances;
        app.deployments.push(DeploymentEntry {
            name: "demo".into(),
            bundle: "/project/deployment.yaml".into(),
            state: "not applied".into(),
            services: Vec::new(),
            instances: Vec::new(),
            blocks: vec!["api".into()],
            source_choices: vec![SourceChoice {
                name: "feature-checkout".into(),
                path: "/work/feature".into(),
                declared: true,
                worktree: true,
                repository: Some("/work/repo".into()),
                requested_ref: Some("feature".into()),
            }],
            bindings: Vec::new(),
            connections: switchyard_ops::ConnectionMatrix::default(),
            connections_error: None,
            route_statuses: Vec::new(),
            last_operation: None,
            applied: false,
            consumer_slot_count: 0,
            validation_problems: Vec::new(),
        });
        app.profiles.push(profile_row(
            "source-api",
            ProfileOrigin::ImportedFromSource {
                source: "feature-checkout".into(),
                commit: Some("abc".into()),
            },
            ProfileTrust::Changed,
        ));
        app.overlay = Overlay::Instance(InstanceForm {
            name: "api-main".into(),
            profile_index: 0,
            source_index: 0,
            device_index: 0,
            parameters: vec![InstanceParameterField {
                name: "TOKEN".into(),
                value: String::new(),
                required: true,
            }],
            active_field: 4,
            field_errors: std::collections::BTreeMap::from([(
                "parameter:TOKEN".into(),
                "required block parameter has no value".into(),
            )]),
            preview: Some(InstancePreview {
                draft: String::new(),
                expanded_services: vec!["demo--api-main--web".into()],
                diagnostics: vec![Diagnostic {
                    code: DiagnosticCode::MissingVariable,
                    path: "spec.instances[0].parameters.TOKEN".into(),
                    message: "required block parameter has no value".into(),
                }],
            }),
            error: None,
        });
        let contents = rendered(&app, 120, 38);
        assert!(contents.contains("Startup profile"));
        assert!(contents.contains("source-api"));
        assert!(contents.contains("disabled: changed"));
        assert!(contents.contains("feature-checkout"));
        assert!(contents.contains("Instance name"));
        assert!(contents.contains("Device"));
        assert!(contents.contains("Parameter TOKEN (required)"));
        assert!(contents.contains("TOKEN: required block parameter"));
        assert!(contents.contains("demo--api-main--web"));
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
    fn connections_matrix_renders_unbound_drafts_and_route_states() {
        use switchyard_ops::{ConnectionMatrix, ConnectionRow, ProviderDetail, RouteStatus};

        let mut app = App::with_sources(PathBuf::from("/project"), Vec::new());
        app.active_view = ActiveView::Connections;
        let rows = ["unbound", "draft", "active", "applying", "failed"]
            .into_iter()
            .map(|consumer| ConnectionRow {
                consumer: consumer.into(),
                slot: "catalog".into(),
                current_group: (!matches!(consumer, "unbound" | "draft")).then(|| "main".into()),
                compatible_groups: vec!["feature".into(), "main".into()],
                providers: (!matches!(consumer, "unbound" | "draft"))
                    .then(|| ProviderDetail {
                        instance: "catalog-main".into(),
                        service: "app".into(),
                        health: "healthy".into(),
                    })
                    .into_iter()
                    .collect(),
            })
            .collect();
        let status =
            |binding: &str, apply_status: &str, observed, error: Option<&str>| RouteStatus {
                router: format!("{binding}-router"),
                binding_id: binding.into(),
                desired_version: Some(3),
                observed_version: observed,
                previous_version: Some(2),
                apply_status: apply_status.into(),
                transition_state: "drain".into(),
                last_error_code: error.map(str::to_owned),
                history: Vec::new(),
            };
        let mut deployment = home_deployment(true, true, false);
        deployment.connections = ConnectionMatrix { rows };
        deployment.route_statuses = vec![
            status("active", "active", Some(3), None),
            status("applying", "pending", Some(2), None),
            status("failed", "failed", Some(2), Some("router_timeout")),
        ];
        app.deployments.push(deployment);
        app.connection_drafts
            .insert("draft".into(), "feature".into());
        let contents = rendered(&app, 180, 35);
        assert!(contents.contains("not connected"));
        assert!(contents.contains("feature — pending change"));
        assert!(contents.contains("catalog-main/app healthy"));
        assert!(contents.contains("active v3"));
        assert!(contents.contains("applying"));
        assert!(contents.contains("failed: router_timeout"));
    }

    #[test]
    fn connection_preview_lists_old_and_new_providers() {
        use switchyard_ops::{ProviderDetail, RouteChange, SwitchPreview};

        let mut app = App::with_sources(PathBuf::from("/project"), Vec::new());
        app.active_view = ActiveView::Connections;
        app.overlay = Overlay::ConnectionPreview(SwitchPreview {
            consumer: "backend-1".into(),
            old_group: Some("main".into()),
            new_group: "feature".into(),
            old_providers: vec![ProviderDetail {
                instance: "catalog-main".into(),
                service: "app".into(),
                health: "unknown".into(),
            }],
            new_providers: vec![ProviderDetail {
                instance: "catalog-feature".into(),
                service: "app".into(),
                health: "unknown".into(),
            }],
            affected_services: vec![RouteChange {
                service: "catalog".into(),
                old_provider: Some("catalog-main/app".into()),
                new_provider: Some("catalog-feature/app".into()),
            }],
            diagnostics: Vec::new(),
        });
        let contents = rendered(&app, 130, 38);
        assert!(contents.contains("Old providers"));
        assert!(contents.contains("catalog-main/app"));
        assert!(contents.contains("New providers"));
        assert!(contents.contains("catalog-feature/app"));
        assert!(contents.contains("unrelated instances are not restarted"));
        assert!(contents.contains("Enter confirms and applies"));
    }

    fn source() -> RegisteredSourceInspection {
        RegisteredSourceInspection {
            source: RegisteredSource {
                name: "code".into(),
                kind: RegisteredSourceKind::Unmanaged,
                path: "/work/code".into(),
                repository_path: None,
                requested_ref: None,
                created_at: 1,
                managed_relative_path: None,
            },
            inspection: SourceInspection {
                identity: SourceIdentity {
                    path: "/work/code".into(),
                    repository: None,
                    r#ref: None,
                    commit: None,
                    dirty: None,
                },
                linked_worktree: None,
                branch: None,
                detached: None,
                changes: None,
                ahead: None,
                behind: None,
                unknown_code: None,
            },
        }
    }

    fn home_deployment(applied: bool, running: bool, bound: bool) -> DeploymentEntry {
        DeploymentEntry {
            name: "demo".into(),
            bundle: "/project/deployment.yaml".into(),
            state: if running { "running" } else { "not applied" }.into(),
            services: if running {
                vec![ServiceRow {
                    instance: "api-one".into(),
                    service: "web".into(),
                    status: "running".into(),
                    health: "healthy".into(),
                }]
            } else {
                Vec::new()
            },
            instances: vec![InstanceRow {
                name: "api-one".into(),
                block: "api".into(),
                source: "code".into(),
            }],
            blocks: vec!["api".into()],
            source_choices: Vec::new(),
            bindings: if bound {
                vec![switchyard_ops::BindingRow {
                    consumer: "api-one".into(),
                    group: "providers".into(),
                    compatible_groups: vec!["providers".into()],
                }]
            } else {
                Vec::new()
            },
            connections: switchyard_ops::ConnectionMatrix::default(),
            connections_error: None,
            route_statuses: Vec::new(),
            last_operation: running.then(|| "up succeeded".into()),
            applied,
            consumer_slot_count: 1,
            validation_problems: Vec::new(),
        }
    }

    #[test]
    fn home_fresh_project_renders_all_pending_and_first_next() {
        let app = App::with_sources(PathBuf::from("/project"), Vec::new());
        let contents = rendered(&app, 150, 32);
        assert!(contents.contains("Next: ○ Pending — Register or clone code"));
        assert!(contents.contains("○ Pending — Choose or import a startup profile"));
        assert!(contents.contains("○ Pending — Create an instance"));
        assert!(contents.contains("○ Pending — Start the deployment"));
        assert!(contents.contains("Connect consumers to providers (optional)"));
        assert!(contents.contains("Sources tab → a"));
    }

    #[test]
    fn home_partially_configured_project_mixes_done_and_pending() {
        let mut app = App::with_sources(PathBuf::from("/project"), vec![source()]);
        app.profiles.push(profile_row(
            "api",
            ProfileOrigin::Project,
            ProfileTrust::Trusted,
        ));
        app.deployments.push(home_deployment(false, false, false));
        let contents = rendered(&app, 150, 32);
        assert!(contents.contains("✓ Done — Register or clone code"));
        assert!(contents.contains("✓ Done — Choose or import a startup profile"));
        assert!(contents.contains("✓ Done — Create an instance"));
        assert!(contents.contains("Next: ○ Pending — Start the deployment"));
        assert!(contents.contains("○ Pending — Connect consumers to providers"));
    }

    #[test]
    fn home_fully_running_project_renders_status_and_all_done() {
        let mut app = App::with_sources(PathBuf::from("/project"), vec![source()]);
        app.profiles.push(profile_row(
            "api",
            ProfileOrigin::Project,
            ProfileTrust::Trusted,
        ));
        app.deployments.push(home_deployment(true, true, true));
        app.profile_source_errors.push(SourceManifestError {
            source: "broken".into(),
            message: "manifest needs a profiles map".into(),
        });
        let contents = rendered(&app, 150, 32);
        assert!(contents.contains("✓ Done — Start the deployment"));
        assert!(contents.contains("✓ Done — Connect consumers to providers"));
        assert!(contents.contains("Deployment: demo  |  state: running"));
        assert!(contents.contains("services: 1 running, 0 unhealthy"));
        assert!(contents.contains("Latest operation: up succeeded"));
        assert!(contents.contains("Startup profile manifest (broken)"));
    }

    #[test]
    fn help_overlay_lists_home_jump_bindings() {
        let mut app = App::with_sources(PathBuf::from("/project"), Vec::new());
        app.overlay = Overlay::Help;
        let contents = rendered(&app, 100, 42);
        assert!(contents.contains("Home"));
        assert!(contents.contains("select checklist item"));
        assert!(contents.contains("1–5"));
        assert!(contents.contains("open that checklist item's tab"));
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
