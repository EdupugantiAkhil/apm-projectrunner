use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Row, Table, TableState, Wrap},
};

use super::centered;
use crate::app::{App, InstanceForm, PairForm, ScriptForm, ScriptMode};

pub(super) fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Percentage(28),
            Constraint::Percentage(18),
            Constraint::Percentage(22),
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
    let authored_rows = app.current_deployment().into_iter().flat_map(|deployment| {
        deployment.instances.iter().map(|instance| {
            Row::new([
                instance.name.clone(),
                instance.block.clone(),
                instance.source.clone(),
                deployment.state.clone(),
                "-".into(),
            ])
        })
    });
    let service_rows = authored_rows.chain(services.iter().map(|service| {
        Row::new([
            service.instance.clone(),
            service.service.clone(),
            "runtime resource".into(),
            service.status.clone(),
            service.health.clone(),
        ])
    }));
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
                "Block / service",
                "Source / kind",
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

    let bindings = app
        .current_deployment()
        .map(|deployment| deployment.bindings.as_slice())
        .unwrap_or_default();
    let binding_rows = bindings.iter().map(|binding| {
        Row::new([
            binding.consumer.clone(),
            binding.group.clone(),
            binding.compatible_groups.join(", "),
        ])
    });
    let mut binding_state = TableState::default();
    if !bindings.is_empty() {
        binding_state.select(Some(app.binding_selected.min(bindings.len() - 1)));
    }
    frame.render_stateful_widget(
        Table::new(
            binding_rows,
            [
                Constraint::Percentage(28),
                Constraint::Percentage(28),
                Constraint::Min(24),
            ],
        )
        .header(
            Row::new(["Consumer instance", "Paired group", "Compatible groups"])
                .style(Style::default().add_modifier(Modifier::BOLD)),
        )
        .row_highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White))
        .highlight_symbol("› ")
        .block(Block::default().borders(Borders::ALL).title(" Pairings ")),
        areas[2],
        &mut binding_state,
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
        areas[3],
        &mut script_state,
    );

    let height = areas[4].height.saturating_sub(2) as usize;
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
        areas[4],
    );
}

pub(super) fn render_instance_form(frame: &mut Frame<'_>, app: &App, form: &InstanceForm) {
    let area = centered(frame.area(), 82, 16);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Add instance ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some((block_name, source)) = app.instance_selection() else {
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
    let mut lines = vec![
        Line::from("Create another instance from a deployment block and source."),
        Line::from(""),
        field(form.active_field == 0, "Name", &form.name),
        field(form.active_field == 1, "Block (←/→/Space)", block_name),
        field(form.active_field == 2, "Source (←/→/Space)", &source.name),
        Line::from(format!(
            "  Source path: {} ({source_kind})",
            source.path.display()
        )),
        Line::from("  Runtime device: local (distributed placement is not yet supported)"),
        Line::from(""),
    ];
    if let Some(error) = &form.error {
        lines.push(Line::from(Span::styled(
            error.clone(),
            Style::default().fg(Color::Red),
        )));
    }
    lines.push(Line::from(
        "Tab/Shift-Tab fields  Enter validate and add  Esc cancel",
    ));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

pub(super) fn render_pair_form(frame: &mut Frame<'_>, app: &App, form: &PairForm) {
    let area = centered(frame.area(), 84, 16);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Pair instances ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some((binding, selected_group)) = app.pair_selection() else {
        frame.render_widget(
            Paragraph::new("No compatible provider-group pairing is available."),
            inner,
        );
        return;
    };
    let changed = binding.group != selected_group;
    let mut lines = vec![
        Line::from("Choose a consumer and a complete compatible provider group."),
        Line::from(""),
        field(true, "Consumer (↑/↓)", &binding.consumer),
        field(true, "Provider group (←/→/Space)", selected_group),
        Line::from(""),
        Line::from(format!(
            "Preview: {}  {}  →  {}",
            binding.consumer, binding.group, selected_group
        )),
        Line::from(format!(
            "{} compatible choice(s); incompatible groups are omitted.",
            binding.compatible_groups.len()
        )),
        Line::from(""),
    ];
    if let Some(error) = &form.error {
        lines.push(Line::from(Span::styled(
            error.clone(),
            Style::default().fg(Color::Red),
        )));
    }
    lines.push(Line::from(if changed {
        "Enter/y apply live pairing  n/Esc cancel"
    } else {
        "Select a different group, or Enter/y reapply  n/Esc cancel"
    }));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
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
