use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Row, Table, TableState, Wrap},
};
use std::time::{SystemTime, UNIX_EPOCH};

use super::centered;
use crate::app::{App, BusyKind, DeviceForm};

pub(super) fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let local = Row::new(["this device", "-", "-", "-", "-"]);
    let registered = app.devices.iter().map(|device| {
        Row::new([
            device.name.clone(),
            format!("{}@{}:{}", device.user, device.host, device.port),
            switchyard_ops::devices::eligibility_label(device),
            device
                .last_checked_at
                .map_or_else(|| "never".into(), relative_time),
            device.identity_file.as_ref().map_or_else(
                || "SSH agent/config".into(),
                |path| path.display().to_string(),
            ),
        ])
    });
    let rows = std::iter::once(local).chain(registered);
    let mut state = TableState::default();
    state.select(Some(app.device_selected));
    let detail = if app.device_selected == 0 {
        "This device is the implicit `local` execution device. It is always available and does not require an SSH eligibility check."
    } else {
        app.devices
            .get(app.device_selected - 1)
            .and_then(|device| device.last_check_detail.as_deref())
            .unwrap_or("Eligibility checks SSH and Docker access for the limited remote-container cut. Prefer a LAN IP reachable from containers; localhost and mDNS are usually unsuitable.")
    };
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(6), Constraint::Length(4)])
        .split(area);
    frame.render_stateful_widget(
        Table::new(
            rows,
            [
                Constraint::Length(18),
                Constraint::Percentage(35),
                Constraint::Percentage(34),
                Constraint::Length(16),
                Constraint::Min(18),
            ],
        )
        .header(
            Row::new([
                "Name",
                "SSH target",
                "Eligibility",
                "Last checked",
                "Identity",
            ])
            .style(Style::default().add_modifier(Modifier::BOLD)),
        )
        .row_highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White))
        .highlight_symbol("› ")
        .block(Block::default().borders(Borders::ALL).title(" Devices ")),
        areas[0],
        &mut state,
    );
    frame.render_widget(
        Paragraph::new(detail)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Check detail "),
            )
            .wrap(Wrap { trim: false }),
        areas[1],
    );
}

fn relative_time(timestamp: i64) -> String {
    let now: i128 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i128::MAX);
    let elapsed_seconds = (now - i128::from(timestamp)).max(0) / 1_000;
    match elapsed_seconds {
        0..=59 => format!("{elapsed_seconds}s ago"),
        60..=3_599 => format!("{}m ago", elapsed_seconds / 60),
        3_600..=86_399 => format!("{}h ago", elapsed_seconds / 3_600),
        _ => format!("{}d ago", elapsed_seconds / 86_400),
    }
}

pub(super) fn render_add(frame: &mut Frame<'_>, form: &DeviceForm, busy: Option<BusyKind>) {
    let area = centered(frame.area(), 76, 18);
    frame.render_widget(Clear, area);
    let block = Block::default().borders(Borders::ALL).title(" Add device ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let values = [
        ("Name", &form.name),
        ("SSH user", &form.user),
        ("Host", &form.host),
        ("Port", &form.port),
        ("Identity file (optional)", &form.identity_file),
    ];
    let mut lines = vec![Line::from(
        "Uses existing SSH keys or agent; passwords and key material are never stored.",
    )];
    for (index, (label, value)) in values.into_iter().enumerate() {
        lines.push(Line::from(Span::styled(
            format!(
                "{} {label}: {value}",
                if form.active_field == index { ">" } else { " " }
            ),
            if form.active_field == index {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            },
        )));
    }
    lines.push(Line::from(""));
    if busy == Some(BusyKind::DeviceAdd) {
        lines.push(Line::from(Span::styled(
            "Adding device…",
            Style::default().fg(Color::Yellow),
        )));
    } else if let Some(error) = &form.error {
        lines.push(Line::from(Span::styled(
            error.clone(),
            Style::default().fg(Color::Red),
        )));
    }
    lines.push(Line::from("Tab/Shift-Tab fields  Enter add  Esc cancel"));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
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
        Line::from(format!("Remove device registration `{name}`?")),
        Line::from("SSH keys and configuration will not be changed."),
        Line::from(""),
    ];
    if busy == Some(BusyKind::DeviceRemove) {
        lines.push(Line::from(Span::styled(
            "Removing…",
            Style::default().fg(Color::Yellow),
        )));
    } else if let Some(error) = error {
        lines.push(Line::from(Span::styled(
            error.to_owned(),
            Style::default().fg(Color::Red),
        )));
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
