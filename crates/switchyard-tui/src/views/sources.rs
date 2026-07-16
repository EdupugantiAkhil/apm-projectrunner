use super::centered;
use crate::app::{AddForm, AddSourceMode, AddSourcePanel, App, BusyKind, WorktreeForm};
use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Wrap},
};
use switchyard_state::RegisteredSourceKind;

pub(super) fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let header = Row::new([
        "Name",
        "Type",
        "Repository",
        "Ref / branch",
        "Dirty",
        "Path",
    ])
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
        let ownership = match entry.source.kind {
            RegisteredSourceKind::Managed => "managed",
            RegisteredSourceKind::Unmanaged => "external",
        };
        let source_type = match entry.inspection.linked_worktree {
            Some(true) => format!("worktree/{ownership}"),
            Some(false) => format!("repository/{ownership}"),
            None => format!("directory/{ownership}"),
        };
        let repository = entry
            .source
            .repository_path
            .as_ref()
            .and_then(|path| {
                app.sources
                    .iter()
                    .find(|candidate| {
                        candidate.inspection.linked_worktree == Some(false)
                            && &candidate.source.path == path
                    })
                    .map(|candidate| candidate.source.name.as_str())
            })
            .unwrap_or("-");
        let name = if entry.inspection.linked_worktree == Some(true) {
            format!("  ↳ {}", entry.source.name)
        } else {
            entry.source.name.clone()
        };
        Row::new([
            Cell::from(name),
            Cell::from(source_type),
            Cell::from(repository.to_owned()),
            Cell::from(reference.to_owned()),
            Cell::from(dirty),
            Cell::from(entry.source.path.display().to_string()),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(20),
            Constraint::Length(21),
            Constraint::Length(18),
            Constraint::Length(22),
            Constraint::Length(14),
            Constraint::Min(24),
        ],
    )
    .header(header)
    .row_highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White))
    .highlight_symbol("› ")
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Repositories and worktrees "),
    );
    let mut state = TableState::default();
    if !app.sources.is_empty() {
        state.select(Some(app.selected));
    }
    frame.render_stateful_widget(table, area, &mut state);
}

pub(super) fn render_worktree(
    frame: &mut Frame<'_>,
    app: &App,
    form: &WorktreeForm,
    busy: Option<BusyKind>,
) {
    let area = centered(frame.area(), 84, 12);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Create managed worktree ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let base = app
        .sources
        .iter()
        .find(|source| source.source.name == form.source);
    let base_path = base
        .map(|source| source.source.path.display().to_string())
        .unwrap_or_else(|| "missing".into());
    let base_label = base
        .and_then(|source| source.inspection.branch.as_deref())
        .unwrap_or("detached HEAD");
    let short_commit = form.base_ref.chars().take(10).collect::<String>();
    let mut lines = vec![
        Line::from(format!(
            "Based on: {} ({base_label}@{short_commit})",
            form.source
        )),
        Line::from(Span::styled(
            format!("  {base_path}"),
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        source_field(true, "Worktree and new branch name", &form.name),
        Line::from(Span::styled(
            "  Created under .switchyard/worktrees/<name> and registered automatically.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "  The new branch starts at the selected checkout's exact HEAD commit.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
    ];
    if busy == Some(BusyKind::WorktreeAdd) {
        lines.push(Line::from(Span::styled(
            "Creating worktree…",
            Style::default().fg(Color::Yellow),
        )));
    } else if let Some(error) = form.error.as_deref() {
        lines.push(Line::from(Span::styled(
            error,
            Style::default().fg(Color::Red),
        )));
    } else {
        lines.push(Line::from("Enter create  Esc cancel"));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

pub(super) fn render_add(frame: &mut Frame<'_>, form: &AddForm, busy: Option<BusyKind>) {
    if form.panel == AddSourcePanel::GitOptions {
        render_git_options(frame, form, busy);
        return;
    }
    let area = centered(frame.area(), 86, 19);
    frame.render_widget(Clear, area);
    let block = Block::default().borders(Borders::ALL).title(" Add source ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let location_label = match form.mode {
        AddSourceMode::Local => "Directory",
        AddSourceMode::Git => "Clone address",
    };
    let location_help = match form.mode {
        AddSourceMode::Local => {
            "An existing project directory. Relative paths start from this Switchyard project."
        }
        AddSourceMode::Git => {
            "Examples: git@github.com:org/repo.git, ssh://host/repo.git, or https://host/repo.git"
        }
    };
    let mut lines = vec![
        Line::from("Enter exactly one source location; Switchyard derives a registry name."),
        Line::from(""),
        source_field(
            form.active_field == 0,
            "Source type (Space/←/→)",
            form.mode.label(),
        ),
        Line::from(Span::styled(
            match form.mode {
                AddSourceMode::Local => "  Register files already present on this machine.",
                AddSourceMode::Git => "  Clone a managed copy into .switchyard/clones.",
            },
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        source_field(form.active_field == 1, location_label, &form.location),
        Line::from(Span::styled(
            format!("  {location_help}"),
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(format!("  Derived name: {}", form.inferred_name())),
        Line::from(Span::styled(
            "  Names come from the final directory/repository segment; an available numeric suffix is added if needed.",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    if form.mode == AddSourceMode::Git {
        let git_ref = if form.git_ref.trim().is_empty() {
            "default"
        } else {
            &form.git_ref
        };
        lines.push(Line::from(format!("  Git options: ref {git_ref}")));
        lines.push(Line::from(
            "  Git and SSH choose credentials and display their native terminal prompts.",
        ));
    }
    lines.push(Line::from(""));
    if busy == Some(BusyKind::Add) {
        lines.push(Line::from(Span::styled(
            "Working…",
            Style::default().fg(Color::Yellow),
        )));
    } else if let Some(error) = form.error.as_deref() {
        lines.push(Line::from(Span::styled(
            error.to_owned(),
            Style::default().fg(Color::Red),
        )));
    } else {
        lines.push(Line::from(if form.mode == AddSourceMode::Git {
            "Tab changes focus  Enter review Git options  F2 Git options  Esc cancel"
        } else {
            "Tab changes focus  Enter register  ←/→ switches type  Esc cancel"
        }));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn render_git_options(frame: &mut Frame<'_>, form: &AddForm, busy: Option<BusyKind>) {
    let area = centered(frame.area(), 90, 20);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Git clone options ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let mut lines = vec![
        Line::from(format!("Repository: {}", form.location)),
        Line::from(""),
        source_field(form.active_field == 0, "Git ref (optional)", &form.git_ref),
        Line::from(Span::styled(
            "  Branch or tag to check out. Leave empty to use the remote's default branch.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Native authentication",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "  Switchyard leaves the full-screen UI and runs git clone on this terminal.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "  Git credential helpers, ssh-agent, ~/.ssh/config, automatic key selection, and prompts work unchanged.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "You may be prompted by Git or SSH after pressing Enter.",
            Style::default().fg(Color::Yellow),
        )),
    ];
    if busy == Some(BusyKind::Add) {
        lines.extend([
            Line::from(""),
            Line::from(Span::styled("Cloning…", Style::default().fg(Color::Yellow))),
        ]);
    } else if let Some(error) = form.error.as_deref() {
        lines.push(Line::from(""));
        for (index, message) in error.lines().take(6).enumerate() {
            lines.push(Line::from(Span::styled(
                if index == 0 {
                    format!("Clone failed: {message}")
                } else {
                    message.to_owned()
                },
                Style::default().fg(Color::Red),
            )));
        }
        if error.lines().count() > 6 {
            lines.push(Line::from(Span::styled(
                "…additional Git output omitted",
                Style::default().fg(Color::Red),
            )));
        }
        lines.push(Line::from(Span::styled(
            "Press Enter to retry with native Git/OpenSSH authentication.",
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines.extend([
        Line::from(""),
        Line::from("Enter run native git clone  F2/Esc back"),
    ]);
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn source_field(active: bool, label: &str, value: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("{} {label}: {value}", if active { ">" } else { " " }),
        if active {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        },
    ))
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
