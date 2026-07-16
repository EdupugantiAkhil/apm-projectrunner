use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Row, Table, TableState, Wrap},
};

use super::centered;
use crate::app::{App, ScriptForm, ScriptMode};

pub(super) fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Percentage(35),
            Constraint::Percentage(25),
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

    let services = app
        .current_deployment()
        .map(|deployment| deployment.services.as_slice())
        .unwrap_or_default();
    let service_rows = services.iter().map(|service| {
        Row::new([
            service.instance.clone(),
            service.service.clone(),
            service.status.clone(),
            service.health.clone(),
        ])
    });
    frame.render_widget(
        Table::new(
            service_rows,
            [
                Constraint::Percentage(25),
                Constraint::Percentage(35),
                Constraint::Percentage(25),
                Constraint::Percentage(15),
            ],
        )
        .header(
            Row::new(["Instance", "Service / resource", "Status", "Health"])
                .style(Style::default().add_modifier(Modifier::BOLD)),
        )
        .block(Block::default().borders(Borders::ALL).title(" Services ")),
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
