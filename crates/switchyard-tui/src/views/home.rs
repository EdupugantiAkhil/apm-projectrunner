use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::app::App;
use switchyard_ops::profiles::ProfileOrigin;

struct ChecklistItem<'a> {
    label: &'a str,
    explanation: &'a str,
    action: &'a str,
    done: bool,
}

pub(super) fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(13),
            Constraint::Length(7),
            Constraint::Min(6),
        ])
        .split(area);

    render_checklist(frame, areas[0], app);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from("Code is a registered source checkout containing the files you run."),
            Line::from("A startup profile is a reusable block that defines one service or a coordinated suite."),
            Line::from("An instance runs one checkout through one startup profile with its own parameters."),
            Line::from("A connection is a binding or route from a consumer instance to a provider service group."),
        ])
        .block(Block::default().borders(Borders::ALL).title(" Concepts "))
        .wrap(Wrap { trim: false }),
        areas[1],
    );
    render_status(frame, areas[2], app);
}

fn render_checklist(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let deployment = app.current_deployment();
    let items = [
        ChecklistItem {
            label: "Register or clone code",
            explanation: "Make at least one source checkout available to the project.",
            action: "Sources tab → a",
            done: !app.sources.is_empty(),
        },
        ChecklistItem {
            label: "Choose or import a startup profile",
            explanation: "Select a reusable definition for the services an instance runs.",
            action: "Profiles tab → i",
            done: app.profiles.iter().any(|profile| {
                matches!(
                    profile.origin,
                    ProfileOrigin::Project | ProfileOrigin::ImportedFromSource { .. }
                )
            }),
        },
        ChecklistItem {
            label: "Create an instance",
            explanation: "Combine a checkout, startup profile, device, name, and parameters.",
            action: "Instances tab → i",
            done: deployment.is_some_and(|entry| !entry.instances.is_empty()),
        },
        ChecklistItem {
            label: "Start the deployment",
            explanation: "Apply the definition and start its long-running services.",
            action: "Instances tab → u",
            done: deployment.is_some_and(|entry| entry.applied),
        },
        ChecklistItem {
            label: if deployment.is_none_or(|entry| entry.consumer_slot_count == 0) {
                "Connect consumers to providers (optional)"
            } else {
                "Connect consumers to providers"
            },
            explanation: "Choose a complete compatible provider group for a consumer.",
            action: "Instances tab → b",
            done: deployment.is_some_and(|entry| !entry.bindings.is_empty()),
        },
    ];
    let first_pending = items.iter().position(|item| !item.done);
    let mut lines = Vec::with_capacity(items.len() * 2);
    for (index, item) in items.iter().enumerate() {
        let state = if item.done { "✓ Done" } else { "○ Pending" };
        let next = if first_pending == Some(index) {
            "Next: "
        } else {
            ""
        };
        let style = if app.home_selected == index || first_pending == Some(index) {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![Span::styled(
            format!("{}. {next}{state} — {}", index + 1, item.label),
            style,
        )]));
        lines.push(Line::from(format!(
            "   {}  [{}]",
            item.explanation, item.action
        )));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" First-run checklist "),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_status(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let mut lines = Vec::new();
    if let Some(deployment) = app.current_deployment() {
        let unhealthy = deployment
            .services
            .iter()
            .filter(|service| {
                service.health == "unhealthy"
                    || ["unhealthy", "exited", "dead", "failed"]
                        .iter()
                        .any(|bad| service.status.to_ascii_lowercase().contains(bad))
            })
            .count();
        let running = deployment
            .services
            .iter()
            .filter(|service| {
                let status = service.status.to_ascii_lowercase();
                !status.contains("unhealthy")
                    && (status.contains("running") || service.health == "healthy")
            })
            .count();
        lines.push(Line::from(format!(
            "Deployment: {}  |  state: {}  |  services: {running} running, {unhealthy} unhealthy",
            deployment.name, deployment.state
        )));
        lines.push(Line::from(format!(
            "Latest operation: {}",
            deployment.last_operation.as_deref().unwrap_or("none yet")
        )));
        for problem in &deployment.validation_problems {
            lines.push(Line::from(format!("Validation problem: {problem}")));
        }
    } else {
        lines.push(Line::from(
            "Deployment: none found  |  state: not applied  |  services: 0 running, 0 unhealthy",
        ));
        lines.push(Line::from(
            "Validation problem: Create or add a deployment definition before starting services.",
        ));
    }
    for diagnostic in &app.profile_source_errors {
        lines.push(Line::from(format!(
            "Startup profile manifest ({}): {}",
            diagnostic.source, diagnostic.message
        )));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" Status "))
            .wrap(Wrap { trim: false }),
        area,
    );
}
