use appcui::prelude::*;

use crate::state::ProjectState;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Destination {
    Code = 1,
    Profiles = 2,
    Instances = 3,
    Connections = 4,
    Operations = 6,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ChecklistStep {
    pub(crate) done: bool,
    pub(crate) label: &'static str,
    pub(crate) destination: Destination,
    pub(crate) shortcut: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NextAction {
    pub(crate) caption: &'static str,
    pub(crate) destination: Destination,
}

#[derive(Clone, Copy)]
pub(crate) struct Handles {
    pub(crate) header: Handle<Label>,
    pub(crate) checklist: Handle<ListBox>,
    pub(crate) next: Handle<Button>,
    pub(crate) problems: Handle<Label>,
}

pub(crate) fn add(tab: &mut Tab, index: u32, state: &ProjectState) -> Handles {
    let mut header_panel = Panel::new("Project", layout!("l:0,t:0,r:0,h:5"));
    let header = header_panel.add(Label::new(&header_text(state), layout!("l:1,t:1,r:1,h:2")));
    tab.add(index, header_panel);

    let mut checklist_panel = Panel::new("First-run checklist", layout!("l:0,t:5,r:0,h:9"));
    let mut checklist = ListBox::new(layout!("l:1,t:1,r:1,b:1"), listbox::Flags::None);
    fill_checklist(&mut checklist, state);
    checklist.set_enabled(false);
    let checklist = checklist_panel.add(checklist);
    tab.add(index, checklist_panel);

    let action = next_action(state);
    let next = tab.add(index, Button::new(action.caption, layout!("l:2,t:15,w:48")));

    let mut problems_panel = Panel::new("Problems", layout!("l:0,t:18,r:0,b:0"));
    let problems = problems_panel.add(Label::new(
        &problems_text(state),
        layout!("l:1,t:1,r:1,b:1"),
    ));
    tab.add(index, problems_panel);

    Handles {
        header,
        checklist,
        next,
        problems,
    }
}

pub(crate) fn checklist(state: &ProjectState) -> [ChecklistStep; 5] {
    let has_instances = state
        .deployments
        .iter()
        .any(|deployment| !deployment.instances.is_empty());
    let any_running = state.deployments.iter().any(|deployment| {
        deployment.services.iter().any(|service| {
            let status = service.status.to_ascii_lowercase();
            let health = service.health.to_ascii_lowercase();
            health == "healthy" || status.contains("running")
        })
    });
    let any_bindings = state
        .deployments
        .iter()
        .any(|deployment| deployment.binding_count > 0)
        || state
            .connections
            .iter()
            .any(|matrix| matrix.rows.iter().any(|row| row.current_group.is_some()));
    [
        ChecklistStep {
            done: !state.sources.is_empty(),
            label: "Register code",
            destination: Destination::Code,
            shortcut: "Alt+C",
        },
        ChecklistStep {
            done: !state.profiles.is_empty(),
            label: "Pick a startup profile",
            destination: Destination::Profiles,
            shortcut: "Alt+P",
        },
        ChecklistStep {
            done: has_instances,
            label: "Create an instance",
            destination: Destination::Instances,
            shortcut: "Alt+I",
        },
        ChecklistStep {
            done: any_running,
            label: "Start it",
            destination: Destination::Instances,
            shortcut: "Alt+I",
        },
        ChecklistStep {
            done: any_bindings,
            label: "Connect routes",
            destination: Destination::Connections,
            shortcut: "Alt+N",
        },
    ]
}

pub(crate) fn next_action(state: &ProjectState) -> NextAction {
    checklist(state).into_iter().find(|step| !step.done).map_or(
        NextAction {
            caption: "Review operations (Enter)",
            destination: Destination::Operations,
        },
        |step| NextAction {
            caption: match step.destination {
                Destination::Code => "Add your first repository (Enter)",
                Destination::Profiles => "Pick a startup profile (Enter)",
                Destination::Instances if step.label == "Create an instance" => {
                    "Create your first instance (Enter)"
                }
                Destination::Instances => "Start your instance (Enter)",
                Destination::Connections => "Connect routes (Enter)",
                Destination::Operations => "Review operations (Enter)",
            },
            destination: step.destination,
        },
    )
}

pub(crate) fn fill_checklist(list: &mut ListBox, state: &ProjectState) {
    list.clear();
    for step in checklist(state) {
        let status = if step.done { "[done]" } else { "[todo]" };
        list.add(&format!("{status}  {}  — {}", step.label, step.shortcut));
    }
}

pub(crate) fn header_text(state: &ProjectState) -> String {
    let project_name = state
        .project_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Switchyard project");
    let running = state
        .deployments
        .iter()
        .flat_map(|deployment| &deployment.services)
        .filter(|service| service.status.to_ascii_lowercase().contains("running"))
        .count();
    let unhealthy = state
        .deployments
        .iter()
        .flat_map(|deployment| &deployment.services)
        .filter(|service| {
            let status = service.status.to_ascii_lowercase();
            service.health.eq_ignore_ascii_case("unhealthy")
                || ["failed", "dead", "exited"]
                    .iter()
                    .any(|word| status.contains(word))
        })
        .count();
    let unapplied = state
        .deployments
        .iter()
        .filter(|deployment| !deployment.applied)
        .count();
    format!(
        "{project_name} — {}\n{} deployment(s) | {running} running | {unhealthy} unhealthy | {unapplied} unapplied",
        state.project_dir.display(),
        state.deployments.len()
    )
}

pub(crate) fn problems_text(state: &ProjectState) -> String {
    let mut problems = Vec::new();
    for (area, destination, error) in [
        ("Code", "Code tab", state.sources_error.as_deref()),
        ("Devices", "Devices tab", state.devices_error.as_deref()),
        (
            "Instances",
            "Instances tab",
            state.deployments_error.as_deref(),
        ),
        ("Profiles", "Profiles tab", state.profiles_error.as_deref()),
        (
            "Operations",
            "Operations tab",
            state.run_scripts_error.as_deref(),
        ),
        (
            "Connections",
            "Connections tab",
            state.connections_error.as_deref(),
        ),
    ] {
        if let Some(error) = error {
            problems.push(format!("{area}: {error} — fix in the {destination}."));
        }
    }
    problems.extend(
        state
            .profile_source_errors
            .iter()
            .map(|error| format!("Startup profile: {error} — fix in the Profiles tab.")),
    );
    for deployment in &state.deployments {
        problems.extend(deployment.validation_problems.iter().map(|problem| {
            format!(
                "Deployment {}: {problem} — fix in the Instances tab.",
                deployment.name
            )
        }));
    }
    if problems.is_empty() {
        "No problems found. Press F5 whenever project files or runtime state change.".into()
    } else {
        problems.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{DeploymentProjection, InstanceProjection, ServiceProjection};

    fn state() -> ProjectState {
        ProjectState {
            project_dir: "/work/demo".into(),
            ..ProjectState::default()
        }
    }

    #[test]
    fn empty_project_starts_with_code() {
        let state = state();
        assert_eq!(
            checklist(&state).map(|step| step.done),
            [false, false, false, false, false]
        );
        assert_eq!(next_action(&state).destination, Destination::Code);
    }

    #[test]
    fn checklist_and_next_action_advance_from_project_state() {
        let mut state = state();
        state.sources.push(Default::default());
        state.profiles.push(Default::default());
        state.deployments.push(DeploymentProjection {
            instances: vec![InstanceProjection { name: "api".into() }],
            services: vec![ServiceProjection {
                status: "running".into(),
                health: "healthy".into(),
            }],
            ..DeploymentProjection::default()
        });
        assert_eq!(
            checklist(&state).map(|step| step.done),
            [true, true, true, true, false]
        );
        assert_eq!(next_action(&state).destination, Destination::Connections);

        state.deployments[0].binding_count = 1;
        assert_eq!(next_action(&state).destination, Destination::Operations);
    }
}
