use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Wrap},
};
use switchyard_ops::profiles::{ProfileAdapterKind, ProfileOrigin, ProfileTrust};

use super::centered;
use crate::app::{App, BusyKind};

pub(super) fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let diagnostics_height = if app.profile_source_errors.is_empty() {
        0
    } else {
        (app.profile_source_errors.len() as u16 + 2).min(7)
    };
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(diagnostics_height)])
        .split(area);
    if app.profiles.is_empty() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from("No startup profiles are available yet."),
                Line::from(""),
                Line::from("Profiles come from the project deployment definition or from"),
                Line::from("switchyard-profiles.yaml at the root of a registered source."),
                Line::from("Press r to refresh after adding or editing a definition."),
            ])
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Startup profiles "),
            )
            .wrap(Wrap { trim: false }),
            areas[0],
        );
    } else {
        let rows = app.profiles.iter().map(|row| {
            let origin = match &row.origin {
                ProfileOrigin::Project => "project".into(),
                ProfileOrigin::ImportedFromSource { source, commit } => format!(
                    "imported from {source}@{}",
                    commit
                        .as_deref()
                        .map(short_commit)
                        .unwrap_or_else(|| "unknown".into())
                ),
                ProfileOrigin::DiscoveredInSource { source, .. } => format!("found in {source}"),
            };
            let (trust, style) = match row.trust {
                ProfileTrust::Trusted => ("trusted", Style::default().fg(Color::Green)),
                ProfileTrust::Imported => ("imported", Style::default().fg(Color::Cyan)),
                ProfileTrust::Changed => ("changed — review", Style::default().fg(Color::Yellow)),
                ProfileTrust::NotImported => ("not imported", Style::default().fg(Color::DarkGray)),
            };
            let services = row
                .services
                .iter()
                .map(|service| {
                    format!(
                        "{}({})",
                        service.name,
                        match service.adapter_kind {
                            ProfileAdapterKind::Container => "container",
                            ProfileAdapterKind::Script => "script",
                            ProfileAdapterKind::ProcessCompose => "process-compose",
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            Row::new([
                Cell::from(row.name.clone()),
                Cell::from(origin),
                Cell::from(trust).style(style),
                Cell::from(services),
                Cell::from(if row.shadowed {
                    "shadowed by project profile"
                } else {
                    "-"
                }),
            ])
        });
        let mut state = TableState::default();
        state.select(Some(app.profile_selected));
        frame.render_stateful_widget(
            Table::new(
                rows,
                [
                    Constraint::Length(18),
                    Constraint::Length(30),
                    Constraint::Length(19),
                    Constraint::Percentage(35),
                    Constraint::Min(28),
                ],
            )
            .header(
                Row::new(["Name", "Origin", "Trust", "Services", "Precedence"])
                    .style(Style::default().add_modifier(Modifier::BOLD)),
            )
            .row_highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White))
            .highlight_symbol("› ")
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Startup profiles "),
            ),
            areas[0],
            &mut state,
        );
    }
    if diagnostics_height > 0 {
        let lines = app.profile_source_errors.iter().map(|error| {
            Line::from(vec![
                Span::styled(
                    format!("{}: ", error.source),
                    Style::default().fg(Color::Red),
                ),
                Span::raw(error.message.clone()),
            ])
        });
        frame.render_widget(
            Paragraph::new(lines.collect::<Vec<_>>())
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Source diagnostics "),
                )
                .wrap(Wrap { trim: false }),
            areas[1],
        );
    }
}

fn short_commit(commit: &str) -> String {
    commit.chars().take(10).collect()
}

pub(super) fn render_inspector(frame: &mut Frame<'_>, name: &str, lines: &[String], scroll: usize) {
    let area = centered(frame.area(), 92, 30);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Startup profile — {name} "));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);
    let text = lines
        .iter()
        .skip(scroll)
        .cloned()
        .map(Line::from)
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), sections[0]);
    frame.render_widget(Paragraph::new("↑/↓ scroll  Esc close"), sections[1]);
}

pub(super) fn render_review(
    frame: &mut Frame<'_>,
    name: &str,
    source: &str,
    yaml: &str,
    scroll: usize,
    error: Option<&str>,
    busy: Option<BusyKind>,
) {
    let area = centered(frame.area(), 100, 34);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Review import ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .split(inner);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(format!(
                "Review `{name}` discovered in `{source}` before trusting it."
            )),
            Line::from(
                "The definition below is data only; preview does not execute repository content.",
            ),
        ]),
        sections[0],
    );
    let lines = yaml
        .lines()
        .skip(scroll)
        .map(|line| Line::from(line.to_owned()))
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        sections[1],
    );
    let footer = if busy == Some(BusyKind::ProfileImport) {
        Line::from(Span::styled(
            "Importing…",
            Style::default().fg(Color::Yellow),
        ))
    } else if let Some(error) = error {
        Line::from(Span::styled(
            error.to_owned(),
            Style::default().fg(Color::Red),
        ))
    } else {
        Line::from("Enter confirm import  ↑/↓ scroll  Esc cancel")
    };
    frame.render_widget(
        Paragraph::new(footer).wrap(Wrap { trim: false }),
        sections[2],
    );
}

pub(super) fn render_remove_confirm(
    frame: &mut Frame<'_>,
    name: &str,
    error: Option<&str>,
    busy: Option<BusyKind>,
) {
    let area = centered(frame.area(), 72, 9);
    frame.render_widget(Clear, area);
    let mut lines = vec![
        Line::from(format!("Remove imported startup profile `{name}`?")),
        Line::from("The source manifest and project profiles are not changed."),
        Line::from(""),
    ];
    if busy == Some(BusyKind::ProfileRemove) {
        lines.push(Line::from("Removing import…"));
    } else if let Some(error) = error {
        lines.push(Line::from(Span::styled(
            error.to_owned(),
            Style::default().fg(Color::Red),
        )));
    } else {
        lines.push(Line::from("Enter confirm removal  Esc cancel"));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Confirm removal "),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}
