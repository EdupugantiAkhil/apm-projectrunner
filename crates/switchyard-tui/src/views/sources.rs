use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Wrap},
};
use switchyard_state::RegisteredSourceKind;

use super::centered;
use crate::app::{AddForm, App, BusyKind};

pub(super) fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let header = Row::new(["Name", "Kind", "Path", "Ref / branch", "Dirty"])
        .style(Style::default().add_modifier(Modifier::BOLD))
        .bottom_margin(1);
    let rows = app.sources.iter().map(|entry| {
        let inspection = &entry.inspection;
        let reference = inspection
            .branch
            .as_deref()
            .or(inspection.identity.r#ref.as_deref())
            .unwrap_or("-");
        let dirty = match inspection.changes.as_ref() {
            Some(changes) if changes.is_dirty() => format!(
                "yes ({}/{}/{})",
                changes.staged, changes.unstaged, changes.untracked
            ),
            Some(_) => "no".into(),
            None => inspection
                .unknown_code
                .as_deref()
                .unwrap_or("unknown")
                .into(),
        };
        Row::new([
            Cell::from(entry.source.name.clone()),
            Cell::from(match entry.source.kind {
                RegisteredSourceKind::Managed => "managed",
                RegisteredSourceKind::Unmanaged => "unmanaged",
            }),
            Cell::from(entry.source.path.display().to_string()),
            Cell::from(reference.to_owned()),
            Cell::from(dirty),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(20),
            Constraint::Length(11),
            Constraint::Percentage(45),
            Constraint::Length(22),
            Constraint::Min(14),
        ],
    )
    .header(header)
    .row_highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White))
    .highlight_symbol("› ")
    .block(Block::default().borders(Borders::ALL).title(" Sources "));
    let mut state = TableState::default();
    if !app.sources.is_empty() {
        state.select(Some(app.selected));
    }
    frame.render_stateful_widget(table, area, &mut state);
}

pub(super) fn render_add(frame: &mut Frame<'_>, form: &AddForm, busy: Option<BusyKind>) {
    let area = centered(frame.area(), 74, 18);
    frame.render_widget(Clear, area);
    let inner = Block::default()
        .borders(Borders::ALL)
        .title(" Add source ")
        .inner(area);
    frame.render_widget(
        Block::default().borders(Borders::ALL).title(" Add source "),
        area,
    );
    let fields = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Min(1),
        ])
        .split(inner);
    frame.render_widget(
        Paragraph::new("Choose one: local path, or git URL with an optional ref."),
        fields[0],
    );
    let values = [
        ("Name", &form.name),
        ("Local path", &form.local_path),
        ("Git URL", &form.git_url),
        ("Git ref", &form.git_ref),
    ];
    for (index, (label, value)) in values.into_iter().enumerate() {
        let marker = if form.active_field == index { ">" } else { " " };
        let style = if form.active_field == index {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };
        frame.render_widget(
            Paragraph::new(format!("{marker} {label}: {value}"))
                .style(style)
                .wrap(Wrap { trim: false }),
            fields[index + 1],
        );
    }
    let message = if busy == Some(BusyKind::Add) {
        Line::from(Span::styled("Working…", Style::default().fg(Color::Yellow)))
    } else if let Some(error) = form.error.as_deref() {
        Line::from(Span::styled(error, Style::default().fg(Color::Red)))
    } else {
        Line::from("Tab/Shift-Tab fields  Enter add  Esc cancel")
    };
    frame.render_widget(
        Paragraph::new(message).wrap(Wrap { trim: false }),
        fields[5],
    );
}

pub(super) fn render_confirm(
    frame: &mut Frame<'_>,
    name: &str,
    error: Option<&str>,
    busy: Option<BusyKind>,
) {
    let area = centered(frame.area(), 68, 9);
    frame.render_widget(Clear, area);
    let mut lines = vec![
        Line::from(format!("Remove and deregister source `{name}`?")),
        Line::from("Managed files are deleted only after ownership and dirty checks."),
        Line::from(""),
    ];
    if busy == Some(BusyKind::Remove) {
        lines.push(Line::from(Span::styled(
            "Removing…",
            Style::default().fg(Color::Yellow),
        )));
    } else if let Some(error) = error {
        lines.push(Line::from(Span::styled(
            error,
            Style::default().fg(Color::Red),
        )));
        lines.push(Line::from(
            "Fix the source state, then press y to retry; n cancels.",
        ));
    } else {
        lines.push(Line::from("y remove  n/Esc cancel"));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" Confirm "))
            .wrap(Wrap { trim: false }),
        area,
    );
}
