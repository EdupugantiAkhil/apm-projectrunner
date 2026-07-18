use std::path::{Path, PathBuf};

use appcui::prelude::*;
use switchyard_ops::{execution::OperationSpec, run_scripts::StructuredCommand};

use crate::{
    dialogs::{confirm, wizard},
    shell::{StateAction, SwitchyardShell},
    state::{DeploymentProjection, ProjectState},
};

impl SwitchyardShell {
    pub(crate) fn selected_instance_row(&self) -> Option<InstanceRowView> {
        self.control(self.instances.list)?.current_item().cloned()
    }

    pub(crate) fn selected_instance_deployment(&self) -> Option<&DeploymentProjection> {
        let row = self.selected_instance_row()?;
        deployment_for_row(&self.state, &row)
    }

    pub(crate) fn refresh_instance_controls(&mut self) {
        let preferred = self
            .selected_instance_row()
            .map(|row| (row.deployment_index, row.instance_index));
        let state = self.state.clone();
        let handles = self.instances;
        let empty = project_rows(&state).is_empty();
        if let Some(list) = self.control_mut(handles.list) {
            fill_list(list, &state, preferred);
            list.set_visible(!empty);
        }
        if let Some(label) = self.control_mut(handles.empty) {
            label.set_visible(empty);
        }
        self.update_instance_detail();
    }

    pub(crate) fn update_instance_detail(&mut self) {
        let text = self.selected_instance_row().map_or_else(
            || "Select an instance to inspect its exact source identity, services, connections, and recent operations.".into(),
            |row| detail_text(&self.state, &row),
        );
        let detail = self.instances.detail;
        if let Some(area) = self.control_mut(detail) {
            area.set_text(&text);
        }
    }

    pub(crate) fn create_instance(&mut self) {
        if self.operation_gate.is_running() {
            self.set_notices("Another project operation is already running.");
            return;
        }
        let deployment = self
            .selected_instance_row()
            .map_or(0, |row| row.deployment_index);
        if let Some(result) = wizard::show(&self.state, deployment) {
            self.start_state_job(
                StateAction::CreateInstance(result),
                "Creating the reviewed authored instance…",
            );
        }
    }

    pub(crate) fn show_instance_details(&self) {
        if let Some(row) = self.selected_instance_row() {
            appcui::dialogs::message("Instance details", &detail_text(&self.state, &row));
        } else {
            appcui::dialogs::message(
                "Instance details",
                "No instance is selected. Press F2 to create one.",
            );
        }
    }

    pub(crate) fn run_instance_command(&mut self, command: InstanceCommand) {
        if self.operation_gate.is_running() {
            self.set_notices("Another project operation is already running.");
            return;
        }
        let Some(deployment) = self.selected_instance_deployment().cloned() else {
            self.set_notices("Select an instance first.");
            return;
        };
        let problems = if deployment.validation_problems.is_empty() {
            "none".into()
        } else {
            deployment.validation_problems.join("\n")
        };
        let (title, action, explanation, spec, destructive) = match command {
            InstanceCommand::Validate => (
                "Validate deployment",
                "&Validate",
                format!(
                    "Validate {} without starting services.\n\nDefinition: {}\nCurrent projected problems:\n{}",
                    deployment.name,
                    deployment.bundle.display(),
                    problems
                ),
                cli_shell_spec("validate", &deployment.bundle, false),
                false,
            ),
            InstanceCommand::Plan => (
                "Plan preview",
                "&Plan",
                format!(
                    "Generate and validate the complete runtime plan for {}. No services will start.\n\nDefinition: {}\nAuthored instances: {}\nProjected services: {}\nCurrent projected problems:\n{}\n\nOutput will stream to Operations.",
                    deployment.name,
                    deployment.bundle.display(),
                    deployment.instances.len(),
                    deployment.services.len(),
                    problems
                ),
                OperationSpec::direct(StructuredCommand::Plan, deployment.bundle.clone()),
                false,
            ),
            InstanceCommand::Start => (
                "Start deployment",
                "&Start",
                format!(
                    "Plan and start deployment {} from {}. This affects every authored instance in that deployment.\n\nTrue placements remain those shown in the Instances list. Output will stream to Operations.",
                    deployment.name,
                    deployment.bundle.display()
                ),
                OperationSpec::direct(StructuredCommand::Up, deployment.bundle.clone()),
                false,
            ),
            InstanceCommand::Stop => (
                "Stop deployment",
                "&Stop",
                format!(
                    "Stop deployment {} and its services.\n\nNamed volumes WILL BE PRESERVED. Use Ctrl+Delete only when you explicitly want owned volumes deleted.",
                    deployment.name
                ),
                OperationSpec::direct(StructuredCommand::Down, deployment.bundle.clone()),
                false,
            ),
        };
        if confirm_operation(title, &explanation, action) {
            self.start_operation(title.to_owned(), Some(deployment.name), destructive, spec);
        }
    }

    pub(crate) fn cleanup_instance(&mut self) {
        if self.operation_gate.is_running() {
            self.set_notices("Another project operation is already running.");
            return;
        }
        let Some(deployment) = self.selected_instance_deployment().cloned() else {
            self.set_notices("Select an instance first.");
            return;
        };
        let preview = format!(
            "CLEAN UP deployment `{}`?\n\nThis stops every service and PERMANENTLY DELETES Switchyard-owned named volumes for this deployment. Volume data cannot be recovered by Switchyard.\n\nDefinition files and unmanaged source directories are not deleted.",
            deployment.name
        );
        if confirm::destructive_cleanup(&preview) {
            self.start_operation(
                "cleanup".into(),
                Some(deployment.name),
                true,
                cli_shell_spec("cleanup", &deployment.bundle, true),
            );
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum InstanceCommand {
    Validate,
    Plan,
    Start,
    Stop,
}

fn cli_shell_spec(command: &str, bundle: &Path, confirmed: bool) -> OperationSpec {
    let executable = std::env::var_os("SWITCHYARD_BIN")
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok())
        .unwrap_or_else(|| PathBuf::from("switchyard"));
    let mut shell = format!(
        "exec {} {} {}",
        shell_quote(&executable.to_string_lossy()),
        shell_quote(command),
        shell_quote(&bundle.to_string_lossy()),
    );
    if confirmed {
        shell.push_str(" --yes");
    }
    OperationSpec::Shell(shell)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InstanceRowView {
    pub(crate) deployment_index: usize,
    pub(crate) instance_index: usize,
    pub(crate) name: String,
    pub(crate) profile: String,
    pub(crate) checkout: String,
    pub(crate) device: String,
    pub(crate) state: String,
    pub(crate) health: String,
}

impl ListItem for InstanceRowView {
    fn columns_count() -> u16 {
        6
    }

    fn column(index: u16) -> Column {
        match index {
            0 => Column::new("Name", 20, TextAlignment::Left),
            1 => Column::new("Profile", 18, TextAlignment::Left),
            2 => Column::new("Checkout", 22, TextAlignment::Left),
            3 => Column::new("Device", 14, TextAlignment::Left),
            4 => Column::new("State", 16, TextAlignment::Left),
            _ => Column::new("Health", 12, TextAlignment::Left),
        }
    }

    fn render_method(&self, column_index: u16) -> Option<listview::RenderMethod<'_>> {
        let text = match column_index {
            0 => &self.name,
            1 => &self.profile,
            2 => &self.checkout,
            3 => &self.device,
            4 => &self.state,
            5 => &self.health,
            _ => return None,
        };
        Some(listview::RenderMethod::Text(text))
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Handles {
    pub(crate) list: Handle<ListView<InstanceRowView>>,
    pub(crate) detail: Handle<TextArea>,
    pub(crate) empty: Handle<Label>,
    pub(crate) notice: Handle<Label>,
}

pub(crate) fn add(tab: &mut Tab, index: u32, state: &ProjectState) -> Handles {
    let mut splitter = VSplitter::new(
        0.60,
        layout!("l:0,t:0,r:0,b:3"),
        vsplitter::ResizeBehavior::PreserveAspectRatio,
    );
    splitter.set_min_width(vsplitter::Panel::Left, 58);
    splitter.set_min_width(vsplitter::Panel::Right, 34);

    let mut left = Panel::new("Instances", layout!("d:f"));
    let mut list = ListView::new(
        layout!("l:1,t:1,r:1,b:1"),
        // SearchBar is intentionally absent; it breaks global Ctrl bindings.
        listview::Flags::ScrollBars,
    );
    fill_list(&mut list, state, None);
    list.set_visible(!project_rows(state).is_empty());
    let list_handle = left.add(list);
    let mut empty = Label::new(
        "An instance combines a code checkout, startup profile, true execution device, and parameters.\n\nPress F2 to create your first instance. Nothing starts until you review the preview and press F9.",
        layout!("l:3,t:3,r:3,h:7"),
    );
    empty.set_visible(project_rows(state).is_empty());
    let empty_handle = left.add(empty);
    splitter.add(vsplitter::Panel::Left, left);

    let mut right = Panel::new("Instance details", layout!("d:f"));
    let first = project_rows(state).first().cloned();
    let detail = first.as_ref().map_or_else(
        || "Select an instance to inspect its exact source identity, services, connections, and recent operations.".into(),
        |row| detail_text(state, row),
    );
    let detail_handle = right.add(TextArea::new(
        &detail,
        layout!("l:1,t:1,r:1,b:1"),
        textarea::Flags::ReadOnly | textarea::Flags::ScrollBars,
    ));
    splitter.add(vsplitter::Panel::Right, right);
    tab.add(index, splitter);
    let notice_handle = tab.add(index, Label::new("", layout!("l:1,b:0,r:1,h:2")));
    Handles {
        list: list_handle,
        detail: detail_handle,
        empty: empty_handle,
        notice: notice_handle,
    }
}

pub(crate) fn project_rows(state: &ProjectState) -> Vec<InstanceRowView> {
    let mut rows = Vec::new();
    for (deployment_index, deployment) in state.deployments.iter().enumerate() {
        for (instance_index, instance) in deployment.instances.iter().enumerate() {
            let source = state
                .sources
                .iter()
                .find(|source| source.name == instance.source);
            let commit = source
                .and_then(|source| source.inspection.identity.commit.as_deref())
                .map(|commit| commit.chars().take(10).collect::<String>())
                .unwrap_or_else(|| "unknown".into());
            let dirty = source
                .and_then(|source| source.inspection.changes.as_ref())
                .is_some_and(|changes| changes.is_dirty());
            let services = deployment
                .services
                .iter()
                .filter(|service| service.instance == instance.name)
                .collect::<Vec<_>>();
            let state_label = if services.is_empty() && !deployment.applied {
                "not started".into()
            } else if services.is_empty() {
                deployment.state.clone()
            } else {
                aggregate(
                    services.iter().map(|service| service.status.as_str()),
                    &deployment.state,
                )
            };
            let health = if services.is_empty() {
                "-".into()
            } else {
                aggregate(services.iter().map(|service| service.health.as_str()), "-")
            };
            let device = if services.is_empty() {
                instance.device.clone()
            } else {
                aggregate(
                    services.iter().map(|service| service.device.as_str()),
                    &instance.device,
                )
            };
            rows.push(InstanceRowView {
                deployment_index,
                instance_index,
                name: instance.name.clone(),
                profile: instance.profile.clone(),
                checkout: format!("{commit} {}", if dirty { "✗ dirty" } else { "clean" }),
                device,
                state: state_label,
                health,
            });
        }
    }
    rows
}

fn aggregate<'a>(values: impl Iterator<Item = &'a str>, fallback: &str) -> String {
    let mut values = values.filter(|value| !value.is_empty()).collect::<Vec<_>>();
    values.sort_unstable();
    values.dedup();
    match values.as_slice() {
        [] => fallback.into(),
        [one] => (*one).into(),
        many => many.join(", "),
    }
}

pub(crate) fn fill_list(
    list: &mut ListView<InstanceRowView>,
    state: &ProjectState,
    preferred: Option<(usize, usize)>,
) {
    list.clear();
    let rows = project_rows(state);
    let selected = preferred
        .and_then(|preferred| {
            rows.iter()
                .position(|row| (row.deployment_index, row.instance_index) == preferred)
        })
        .or_else(|| (!rows.is_empty()).then_some(0));
    for row in rows {
        list.add(row);
    }
    for _ in 0..selected.unwrap_or(0) {
        OnKeyPressed::on_key_pressed(list, Key::new(KeyCode::Down, KeyModifier::None), '\0');
    }
}

pub(crate) fn detail_text(state: &ProjectState, row: &InstanceRowView) -> String {
    let Some(deployment) = state.deployments.get(row.deployment_index) else {
        return "The selected deployment is no longer available.".into();
    };
    let Some(instance) = deployment.instances.get(row.instance_index) else {
        return "The selected instance is no longer available.".into();
    };
    let source = state
        .sources
        .iter()
        .find(|source| source.name == instance.source);
    let identity = source.map_or_else(
        || {
            deployment
                .source_choices
                .iter()
                .find(|source| source.name == instance.source)
                .map_or_else(
                    || format!("{} (source identity unavailable)", instance.source),
                    |source| {
                        format!(
                            "{}\n  path: {}\n  requested ref: {}\n  commit: unavailable until inspection\n  dirty: unknown",
                            source.name,
                            source.path.display(),
                            source.requested_ref.as_deref().unwrap_or("not pinned")
                        )
                    },
                )
        },
        |source| {
            format!(
                "{}\n  path: {}\n  ref: {}\n  commit: {}\n  dirty: {}",
                source.name,
                source.path.display(),
                source
                    .inspection
                    .identity
                    .r#ref
                    .as_deref()
                    .unwrap_or("unknown"),
                source
                    .inspection
                    .identity
                    .commit
                    .as_deref()
                    .unwrap_or("unknown"),
                source
                    .inspection
                    .changes
                    .as_ref()
                    .map_or("unknown", |changes| if changes.is_dirty() {
                        "yes"
                    } else {
                        "no"
                    }),
            )
        },
    );
    let services = deployment
        .services
        .iter()
        .filter(|service| service.instance == instance.name)
        .map(|service| {
            format!(
                "  {} — state: {}; health: {}; resource placement: {}",
                service.service, service.status, service.health, service.device
            )
        })
        .collect::<Vec<_>>();
    let connections = deployment
        .connections
        .rows
        .iter()
        .filter(|connection| connection.consumer == instance.name)
        .map(|connection| {
            format!(
                "  {} -> {}",
                connection.slot,
                connection
                    .current_group
                    .as_deref()
                    .unwrap_or("not connected")
            )
        })
        .collect::<Vec<_>>();
    let mut recent = state
        .operation_log
        .entries()
        .iter()
        .rev()
        .filter(|entry| {
            entry.deployment.as_deref() == Some(&deployment.name)
                || entry.lines.iter().any(|line| line.contains(&instance.name))
        })
        .take(5)
        .map(|entry| format!("  {} — {:?}", entry.label, entry.outcome))
        .collect::<Vec<_>>();
    if let Some(operation) = &deployment.last_operation {
        recent.push(format!("  durable last operation: {operation}"));
    }
    format!(
        "Deployment: {}\nProfile: {}\nTrue placement: {}\n\nSource identity:\n{}\n\nExpanded services:\n{}\n\nActive connections:\n{}\n\nRecent operations:\n{}",
        deployment.name,
        instance.profile,
        row.device,
        identity,
        or_none(services),
        or_none(connections),
        or_none(recent),
    )
}

fn or_none(lines: Vec<String>) -> String {
    if lines.is_empty() {
        "  none".into()
    } else {
        lines.join("\n")
    }
}

pub(crate) fn deployment_for_row<'a>(
    state: &'a ProjectState,
    row: &InstanceRowView,
) -> Option<&'a DeploymentProjection> {
    state.deployments.get(row.deployment_index)
}

#[ModalWindow(events = ButtonEvents + WindowEvents, response = bool)]
struct OperationPreview {
    run: Handle<Button>,
    cancel: Handle<Button>,
}

impl WindowEvents for OperationPreview {
    // Enter deterministically confirms a non-destructive preview regardless of
    // which control holds focus; destructive dialogs deliberately do NOT get
    // this and keep Cancel as their focused default.
    fn on_accept(&mut self) {
        self.exit_with(true);
    }
}

impl OperationPreview {
    fn new(title: &str, text: &str, action: &str) -> Self {
        let mut dialog = Self {
            base: ModalWindow::new(title, layout!("a:c,w:84,h:22"), window::Flags::None),
            run: Handle::None,
            cancel: Handle::None,
        };
        dialog.add(TextArea::new(
            text,
            layout!("l:1,t:1,r:1,b:3"),
            textarea::Flags::ReadOnly | textarea::Flags::ScrollBars,
        ));
        dialog.run = dialog.add(Button::new(action, layout!("x:35%,y:100%,p:b,w:16,h:1")));
        dialog.cancel = dialog.add(Button::new("&Cancel", layout!("x:65%,y:100%,p:b,w:16,h:1")));
        // Initial focus is otherwise nondeterministic (differs after the
        // handoff re-exec); Enter must always mean the non-destructive action.
        let run = dialog.run;
        dialog.request_focus_for_control(run);
        dialog
    }
}

impl ButtonEvents for OperationPreview {
    fn on_pressed(&mut self, handle: Handle<Button>) -> EventProcessStatus {
        if handle == self.run {
            self.exit_with(true);
        } else if handle == self.cancel {
            self.exit_with(false);
        } else {
            return EventProcessStatus::Ignored;
        }
        EventProcessStatus::Processed
    }
}

pub(crate) fn confirm_operation(title: &str, text: &str, action: &str) -> bool {
    OperationPreview::new(title, text, action)
        .show()
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{DeploymentProjection, InstanceProjection, ServiceProjection};

    #[test]
    fn authored_instance_without_runtime_rows_is_not_started_and_keeps_placement() {
        let state = ProjectState {
            deployments: vec![DeploymentProjection {
                name: "demo".into(),
                bundle: "deployment.yaml".into(),
                state: "not applied".into(),
                instances: vec![InstanceProjection {
                    name: "api".into(),
                    profile: "web".into(),
                    source: "checkout".into(),
                    device: "builder".into(),
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let rows = project_rows(&state);
        assert_eq!(rows[0].state, "not started");
        assert_eq!(rows[0].device, "builder");
    }

    #[test]
    fn runtime_service_state_health_and_true_device_are_projected() {
        let state = ProjectState {
            deployments: vec![DeploymentProjection {
                name: "demo".into(),
                bundle: "deployment.yaml".into(),
                applied: true,
                instances: vec![InstanceProjection {
                    name: "api".into(),
                    profile: "web".into(),
                    source: "checkout".into(),
                    device: "builder".into(),
                }],
                services: vec![ServiceProjection {
                    instance: "api".into(),
                    service: "server".into(),
                    device: "builder".into(),
                    status: "running".into(),
                    health: "healthy".into(),
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let rows = project_rows(&state);
        assert_eq!(
            (rows[0].state.as_str(), rows[0].health.as_str()),
            ("running", "healthy")
        );
        assert_eq!(rows[0].device, "builder");
    }
}
