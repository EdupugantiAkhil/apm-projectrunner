use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Clear, Paragraph, Row, Table, TableState, Wrap},
};

use super::centered;
use crate::app::App;
use switchyard_ops::connections::{RouteStatus, SwitchPreview};

pub(super) fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(8),
            Constraint::Length(7),
        ])
        .split(area);
    frame.render_widget(
        Paragraph::new("Apps keep their fixed localhost/network addresses; Switchyard routes those addresses to the selected provider group.")
            .block(Block::default().borders(Borders::ALL).title(" Connections "))
            .wrap(Wrap { trim: true }),
        areas[0],
    );
    let rows = app.current_connection_rows();
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new("A connection selects one complete provider group for a consumer's service slots. Declare groups and bindings in the deployment definition to make connections available.")
                .block(Block::default().borders(Borders::ALL).title(" No connections declared "))
                .wrap(Wrap { trim: true }),
            areas[1],
        );
    } else {
        let table_rows = rows.iter().map(|row| {
            let draft = app.connection_drafts.get(&row.consumer);
            let group = draft
                .map(|value| format!("{value} — pending change"))
                .or_else(|| row.current_group.clone())
                .unwrap_or_else(|| "not connected".into());
            let providers = if row.providers.is_empty() {
                "—".into()
            } else {
                row.providers
                    .iter()
                    .map(|provider| {
                        format!(
                            "{}/{} {}",
                            provider.instance, provider.service, provider.health
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            Row::new([
                row.consumer.clone(),
                row.slot.clone(),
                group,
                providers,
                route_words(app, &row.consumer),
            ])
        });
        let mut state = TableState::default();
        state.select(Some(app.connection_selected.min(rows.len() - 1)));
        frame.render_stateful_widget(
            Table::new(
                table_rows,
                [
                    Constraint::Percentage(17),
                    Constraint::Percentage(15),
                    Constraint::Percentage(22),
                    Constraint::Percentage(31),
                    Constraint::Percentage(15),
                ],
            )
            .header(
                Row::new([
                    "Consumer instance",
                    "Slot",
                    "Connected group",
                    "Providers",
                    "Route status",
                ])
                .style(Style::default().add_modifier(Modifier::BOLD)),
            )
            .row_highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White))
            .highlight_symbol("› ")
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Route matrix "),
            ),
            areas[1],
            &mut state,
        );
    }
    let details = rows.get(app.connection_selected).map_or_else(
        || {
            app.current_deployment()
                .and_then(|deployment| deployment.connections_error.clone())
                .map_or_else(
                    || {
                        "Compatible groups appear here after consumers, slots, and groups are declared."
                            .into()
                    },
                    |error| format!("connections could not be loaded: {error}"),
                )
        },
        |row| {
            let compatible = if row.compatible_groups.is_empty() {
                "none (definition is incomplete or incompatible)".into()
            } else {
                row.compatible_groups.join(", ")
            };
            let route = matching_statuses(app, &row.consumer)
                .max_by_key(|status| match status.apply_status.as_str() {
                    "failed" => 3,
                    "pending" => 2,
                    _ => 1,
                })
                .map(route_detail)
                .unwrap_or_else(|| "No route version has been observed yet.".into());
            format!("Compatible complete groups: {compatible}\n{route}\n←/→ or h/l chooses a draft; Enter previews. Nothing applies until Enter is pressed again in the preview.")
        },
    );
    frame.render_widget(
        Paragraph::new(details)
            .block(Block::default().borders(Borders::ALL).title(" Selection "))
            .wrap(Wrap { trim: true }),
        areas[2],
    );
}

fn matching_statuses<'a>(app: &'a App, consumer: &str) -> impl Iterator<Item = &'a RouteStatus> {
    app.current_deployment()
        .into_iter()
        .flat_map(|deployment| deployment.route_statuses.iter())
        .filter(move |status| status.binding_id == consumer)
}

fn route_words(app: &App, consumer: &str) -> String {
    let statuses = matching_statuses(app, consumer).collect::<Vec<_>>();
    if let Some(failed) = statuses
        .iter()
        .find(|status| status.apply_status == "failed")
    {
        return format!(
            "failed: {}",
            failed.last_error_code.as_deref().unwrap_or("unknown")
        );
    }
    if statuses
        .iter()
        .any(|status| status.apply_status == "pending")
    {
        return "applying".into();
    }
    statuses
        .iter()
        .filter_map(|status| status.observed_version)
        .max()
        .map_or_else(
            || "not applied".into(),
            |version| format!("active v{version}"),
        )
}

fn route_detail(status: &RouteStatus) -> String {
    let rollback = status.previous_version.map_or_else(String::new, |version| {
        let recorded = status
            .history
            .iter()
            .rev()
            .find(|item| item.status == "rolled_back");
        recorded.map_or_else(
            || format!(" Previous version: v{version}."),
            |item| {
                format!(
                    " Rolled back via v{} at timestamp {} (previous v{version}).",
                    item.version, item.recorded_at
                )
            },
        )
    });
    format!(
        "Desired: {}  observed: {}  status: {}  transition: {}.{}{rollback}",
        status
            .desired_version
            .map_or_else(|| "—".into(), |value| format!("v{value}")),
        status
            .observed_version
            .map_or_else(|| "—".into(), |value| format!("v{value}")),
        status.apply_status,
        status.transition_state,
        status
            .last_error_code
            .as_deref()
            .map_or_else(String::new, |code| format!(" Error: {code}.")),
    )
}

pub(super) fn render_preview(frame: &mut Frame<'_>, preview: &SwitchPreview) {
    let area = centered(frame.area(), 92, 30);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Atomic connection switch preview ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let provider_lines = |providers: &[switchyard_ops::connections::ProviderDetail]| {
        if providers.is_empty() {
            "  (none)".into()
        } else {
            providers
                .iter()
                .map(|item| format!("  {}/{}", item.instance, item.service))
                .collect::<Vec<_>>()
                .join("\n")
        }
    };
    let affected = if preview.affected_services.is_empty() {
        "  (no route changes)".into()
    } else {
        preview
            .affected_services
            .iter()
            .map(|item| {
                format!(
                    "  {}: {} → {}",
                    item.service,
                    item.old_provider.as_deref().unwrap_or("none"),
                    item.new_provider.as_deref().unwrap_or("none")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let diagnostics = if preview.diagnostics.is_empty() {
        String::new()
    } else {
        format!(
            "\nCannot apply:\n{}",
            preview
                .diagnostics
                .iter()
                .map(|item| format!("  {item}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };
    let text = format!(
        "Consumer: {}\nOld group: {}\nOld providers:\n{}\n\nNew group: {}\nNew providers:\n{}\n\nServices whose routes change:\n{}\n\nThis complete change is applied atomically; unrelated instances are not restarted.{}\n\nEnter confirms and applies  Esc cancels and clears the draft",
        preview.consumer,
        preview.old_group.as_deref().unwrap_or("not connected"),
        provider_lines(&preview.old_providers),
        preview.new_group,
        provider_lines(&preview.new_providers),
        affected,
        diagnostics,
    );
    frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), inner);
}
