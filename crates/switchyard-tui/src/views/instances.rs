use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Row, Table, TableState, Wrap},
};

use super::centered;
use crate::app::{App, InstanceForm, ScriptForm, ScriptMode};

pub(super) fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Percentage(38),
            Constraint::Percentage(27),
            Constraint::Min(7),
        ])
        .split(area);
    let header = if let Some(deployment) = app.current_deployment() {
        let operation = deployment
            .last_operation
            .as_deref()
            .map_or(String::new(), |value| format!("  last: {value}"));
        format!(
            " Deployment: {}  state: {}{operation}  definition: {} ",
            deployment.name,
            deployment.state,
            deployment.bundle.display()
        )
    } else {
        " No deployment definition or state found ".into()
    };
    frame.render_widget(
        Paragraph::new(header).block(Block::default().borders(Borders::ALL).title(" Instances ")),
        areas[0],
    );

    let service_rows = app
        .current_deployment()
        .map(instance_service_rows)
        .unwrap_or_default()
        .into_iter()
        .map(Row::new);
    frame.render_widget(
        Table::new(
            service_rows,
            [
                Constraint::Percentage(20),
                Constraint::Percentage(22),
                Constraint::Percentage(24),
                Constraint::Percentage(20),
                Constraint::Percentage(14),
            ],
        )
        .header(
            Row::new([
                "Instance",
                "Startup profile / service",
                "Checkout / source",
                "Status",
                "Health",
            ])
            .style(Style::default().add_modifier(Modifier::BOLD)),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Instances and services "),
        ),
        areas[1],
    );

    let script_rows = app.scripts.iter().map(|script| {
        let kind = script.command.map_or("shell", |command| command.as_str());
        Row::new([
            script.name.clone(),
            kind.into(),
            script.description.clone().unwrap_or_default(),
        ])
    });
    let mut script_state = TableState::default();
    if !app.scripts.is_empty() {
        script_state.select(Some(app.script_selected));
    }
    let scripts_title = app.scripts_error.as_ref().map_or_else(
        || " Run scripts ".into(),
        |error| format!(" Run scripts — {error} "),
    );
    frame.render_stateful_widget(
        Table::new(
            script_rows,
            [
                Constraint::Length(24),
                Constraint::Length(10),
                Constraint::Min(20),
            ],
        )
        .header(
            Row::new(["Name", "Kind", "Description"])
                .style(Style::default().add_modifier(Modifier::BOLD)),
        )
        .row_highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White))
        .highlight_symbol("› ")
        .block(Block::default().borders(Borders::ALL).title(scripts_title)),
        areas[2],
        &mut script_state,
    );

    let height = areas[3].height.saturating_sub(2) as usize;
    let end = app.output.len().saturating_sub(app.output_scroll);
    let start = end.saturating_sub(height);
    let lines = app.output[start..end]
        .iter()
        .map(|line| Line::from(line.as_str()))
        .collect::<Vec<_>>();
    let title = app.last_exit_code.map_or_else(
        || " Output ".into(),
        |code| format!(" Output — exit code {code} "),
    );
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: false }),
        areas[3],
    );
}

fn instance_service_rows(deployment: &crate::app::DeploymentEntry) -> Vec<[String; 5]> {
    if deployment.services.is_empty() {
        return deployment
            .instances
            .iter()
            .map(|instance| {
                [
                    instance.name.clone(),
                    instance.block.clone(),
                    instance.source.clone(),
                    deployment.state.clone(),
                    "-".into(),
                ]
            })
            .collect();
    }
    deployment
        .services
        .iter()
        .map(|service| {
            let authored = deployment
                .instances
                .iter()
                .find(|instance| instance.name == service.instance);
            [
                service.instance.clone(),
                authored.map_or_else(
                    || service.service.clone(),
                    |instance| format!("{} / {}", instance.block, service.service),
                ),
                authored.map_or_else(|| "-".into(), |instance| instance.source.clone()),
                service.status.clone(),
                service.health.clone(),
            ]
        })
        .collect()
}

pub(super) fn render_instance_form(frame: &mut Frame<'_>, app: &App, form: &InstanceForm) {
    let height = (18 + form.parameters.len() as u16 + form.preview.as_ref().map_or(0, |_| 6))
        .min(frame.area().height.saturating_sub(2));
    let area = centered(frame.area(), 92, height);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Add instance ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some((profile, source, device)) = app.instance_selection() else {
        frame.render_widget(
            Paragraph::new("No block/source selection is available."),
            inner,
        );
        return;
    };
    let source_kind = if source.declared {
        "declared"
    } else {
        "registered; will be added to deployment"
    };
    let profile_state = match profile.trust {
        switchyard_ops::profiles::ProfileTrust::Trusted => "project profile".to_owned(),
        switchyard_ops::profiles::ProfileTrust::Imported if !profile.shadowed => {
            "imported and unchanged".to_owned()
        }
        switchyard_ops::profiles::ProfileTrust::Imported => {
            "disabled: shadowed by a project profile".to_owned()
        }
        switchyard_ops::profiles::ProfileTrust::Changed => {
            "disabled: changed; review/import it in Profiles first".to_owned()
        }
        switchyard_ops::profiles::ProfileTrust::NotImported => {
            "disabled: not imported; review/import it in Profiles first".to_owned()
        }
    };
    let mut lines = vec![
        Line::from("Create an instance from a reusable startup profile and code checkout."),
        Line::from(""),
        field(
            form.active_field == 0,
            "Startup profile (←/→/Space)",
            &profile.name,
        ),
        Line::from(format!("    {profile_state}")),
        field(
            form.active_field == 1,
            "Checkout/worktree (←/→/Space)",
            &source.name,
        ),
        Line::from(format!(
            "  Checkout path: {} ({source_kind})",
            source.path.display()
        )),
        field(form.active_field == 2, "Instance name", &form.name),
        field(form.active_field == 3, "Device (←/→/Space)", device),
    ];
    if let Some(error) = form.field_errors.get("name") {
        lines.push(error_line("Name", error));
    }
    if let Some(error) = form.field_errors.get("profile") {
        lines.push(error_line("Startup profile", error));
    }
    if let Some(error) = form.field_errors.get("source") {
        lines.push(error_line("Checkout", error));
    }
    if let Some(error) = form.field_errors.get("device") {
        lines.push(error_line("Device", error));
    }
    for (index, parameter) in form.parameters.iter().enumerate() {
        let required = if parameter.required {
            " (required)"
        } else {
            ""
        };
        lines.push(field(
            form.active_field == index + 4,
            &format!("Parameter {}{required}", parameter.name),
            &parameter.value,
        ));
        if let Some(error) = form
            .field_errors
            .get(&format!("parameter:{}", parameter.name))
        {
            lines.push(error_line(&parameter.name, error));
        }
    }
    lines.push(Line::from(""));
    if let Some(preview) = &form.preview {
        lines.push(Line::from(Span::styled(
            "Preview — no files have been changed",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(format!(
            "  Expanded services: {}",
            if preview.expanded_services.is_empty() {
                "none".into()
            } else {
                preview.expanded_services.join(", ")
            }
        )));
        if preview.diagnostics.is_empty() {
            lines.push(Line::from(
                "  Validation passed. Press Enter again to write.",
            ));
        } else {
            lines.push(Line::from(Span::styled(
                format!(
                    "  {} validation diagnostic(s); fix the fields above.",
                    preview.diagnostics.len()
                ),
                Style::default().fg(Color::Red),
            )));
        }
    }
    if let Some(error) = &form.error {
        lines.push(Line::from(Span::styled(
            error.clone(),
            Style::default().fg(Color::Red),
        )));
    }
    lines.push(Line::from(
        "Tab/Shift-Tab or arrows move  Enter advances/previews/confirms  Esc cancel",
    ));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn error_line(label: &str, error: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("    {label}: {error}"),
        Style::default().fg(Color::Red),
    ))
}

pub(super) fn render_script_form(frame: &mut Frame<'_>, form: &ScriptForm) {
    let area = centered(frame.area(), 88, 22);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(if form.edit_index.is_some() {
            " Edit run script "
        } else {
            " New run script "
        });
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let mut lines = vec![
        field(form.active_field == 0, "Name", &form.name),
        field(form.active_field == 1, "Description", &form.description),
        field(
            form.active_field == 2,
            "Type (Space toggles)",
            if form.mode == ScriptMode::Structured {
                "structured"
            } else {
                "shell"
            },
        ),
    ];
    match form.mode {
        ScriptMode::Structured => lines.extend([
            field(
                form.active_field == 3,
                "Command (Space cycles)",
                form.command.as_str(),
            ),
            field(
                form.active_field == 4,
                "Overlays (comma-separated)",
                &form.overlays,
            ),
            field(form.active_field == 5, "Variation", &form.variation),
            field(
                form.active_field == 6,
                "Set (comma-separated KEY=VALUE)",
                &form.set,
            ),
        ]),
        ScriptMode::Shell => {
            lines.push(field(form.active_field == 3, "Shell command", &form.shell))
        }
    }
    lines.push(Line::from(""));
    if let Some(error) = &form.error {
        lines.push(Line::from(Span::styled(
            error,
            Style::default().fg(Color::Red),
        )));
    }
    lines.push(Line::from(
        "Tab/Shift-Tab fields  Space changes choices  Enter save  Esc cancel",
    ));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn field(active: bool, label: &str, value: &str) -> Line<'static> {
    let marker = if active { ">" } else { " " };
    Line::from(Span::styled(
        format!("{marker} {label}: {value}"),
        if active {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        },
    ))
}

pub(super) fn render_confirm(
    frame: &mut Frame<'_>,
    title: &str,
    prompt: &str,
    error: Option<&str>,
) {
    let area = centered(frame.area(), 72, 9);
    frame.render_widget(Clear, area);
    let mut lines = vec![Line::from(prompt.to_owned()), Line::from("")];
    if let Some(error) = error {
        lines.push(Line::from(Span::styled(
            error,
            Style::default().fg(Color::Red),
        )));
    }
    lines.push(Line::from("y confirm  n/Esc cancel"));
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: false }),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use switchyard_ops::projections::{DeploymentEntry, InstanceRow, ServiceRow};

    #[test]
    fn runtime_services_merge_with_authored_instance_context() {
        let deployment = DeploymentEntry {
            name: "demo".into(),
            bundle: PathBuf::from("deployment.yaml"),
            state: "running".into(),
            services: vec![ServiceRow {
                instance: "web".into(),
                service: "server".into(),
                status: "running".into(),
                health: "healthy".into(),
            }],
            instances: vec![InstanceRow {
                name: "web".into(),
                block: "web".into(),
                source: "project".into(),
            }],
            blocks: Vec::new(),
            source_choices: Vec::new(),
            bindings: Vec::new(),
            connections: switchyard_ops::ConnectionMatrix::default(),
            connections_error: None,
            route_statuses: Vec::new(),
            last_operation: None,
            applied: true,
            consumer_slot_count: 0,
            validation_problems: Vec::new(),
        };
        assert_eq!(
            instance_service_rows(&deployment),
            [[
                String::from("web"),
                String::from("web / server"),
                String::from("project"),
                String::from("running"),
                String::from("healthy"),
            ]]
        );
    }
}
