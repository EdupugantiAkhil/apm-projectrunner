use appcui::prelude::*;
use switchyard_ops::{ConnectionRow, RouteStatus, SwitchPreview, execution::OperationSpec};

use crate::{
    shell::{PendingBind, SwitchyardShell},
    state::ProjectState,
};

impl SwitchyardShell {
    pub(crate) fn selected_connection_row(&self) -> Option<ConnectionRowView> {
        self.control(self.connections.list)?.current_item().cloned()
    }

    pub(crate) fn refresh_connection_controls(&mut self) {
        let preferred = self
            .selected_connection_row()
            .map(|row| (row.deployment_index, row.consumer, row.slot));
        let state = self.state.clone();
        let handles = self.connections;
        let empty = project_rows(&state).is_empty();
        if let Some(list) = self.control_mut(handles.list) {
            fill_list(list, &state, preferred);
            list.set_visible(!empty);
        }
        if let Some(label) = self.control_mut(handles.empty) {
            label.set_visible(empty);
        }
        let diagnostics = diagnostics(&state);
        if let Some(notice) = self.control_mut(handles.notice) {
            notice.set_caption(&diagnostics);
        }
    }

    pub(crate) fn switch_connection(&mut self) {
        if self.operation_gate.is_running() {
            self.set_notices("Another project operation is already running.");
            return;
        }
        let Some(view) = self.selected_connection_row() else {
            self.set_notices(
                "Connections need a consumer instance with at least one consumed slot. Create one with F2 on Instances.",
            );
            return;
        };
        let Some((deployment, row)) = source_row(&self.state, &view) else {
            self.set_notices(
                "The selected connection is no longer available. Press F5 to refresh.",
            );
            return;
        };
        if row.compatible_groups.is_empty() {
            self.set_notices(
                "No complete provider group is compatible with this consumer. Fix the deployment definition and press F5.",
            );
            return;
        }
        let deployment_name = deployment.name.clone();
        let bundle = deployment.bundle.clone();
        let consumer = row.consumer.clone();
        let compatible_groups = row.compatible_groups.clone();
        let mut previews = Vec::with_capacity(compatible_groups.len());
        for group in compatible_groups {
            match switchyard_ops::switch_preview(
                &self.state.project_dir,
                &bundle,
                &consumer,
                &group,
            ) {
                Ok(preview) => previews.push(preview),
                Err(error) => {
                    self.set_notices(&format!(
                        "Could not build the atomic switch preview for `{group}`: {error}"
                    ));
                    return;
                }
            }
        }
        let Some(request) = show_switch(&consumer, previews) else {
            return;
        };
        let spec = OperationSpec::bind(bundle, consumer.clone(), request.group.clone());
        let started = self.start_operation(
            format!("bind {consumer} → {}", request.group),
            Some(deployment_name.clone()),
            false,
            spec,
        );
        if started {
            self.pending_bind = Some(PendingBind {
                deployment: deployment_name,
                consumer,
            });
        }
    }

    pub(crate) fn finish_bind(&mut self, succeeded: bool, operation_detail: &str) {
        let Some(bind) = self.pending_bind.take() else {
            return;
        };
        let (statuses, status_error) =
            match switchyard_ops::route_status(&self.state.project_dir, &bind.deployment) {
                Ok(statuses) => (statuses, None),
                Err(error) => (Vec::new(), Some(error)),
            };
        let detail = status_error.map_or_else(
            || operation_detail.to_owned(),
            |error| format!("{operation_detail}\nRoute status lookup failed: {error}"),
        );
        let report = render_result(&bind.consumer, succeeded, &detail, &statuses);
        show_result(&report);
    }
}

pub(crate) const EXPLAINER: &str = "Consumers keep their fixed localhost/network addresses; Switchyard routes them to the selected group.";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConnectionRowView {
    pub(crate) deployment_index: usize,
    pub(crate) consumer: String,
    pub(crate) slot: String,
    pub(crate) group: String,
    pub(crate) route_version: String,
    pub(crate) state: String,
    pub(crate) compatible_groups: Vec<String>,
}

impl ListItem for ConnectionRowView {
    fn columns_count() -> u16 {
        5
    }

    fn column(index: u16) -> Column {
        match index {
            0 => Column::new("Consumer", 22, TextAlignment::Left),
            1 => Column::new("Slot", 18, TextAlignment::Left),
            2 => Column::new("Selected provider group", 27, TextAlignment::Left),
            3 => Column::new("Route version", 21, TextAlignment::Left),
            _ => Column::new("State", 34, TextAlignment::Left),
        }
    }

    fn render_method(&self, column_index: u16) -> Option<listview::RenderMethod<'_>> {
        let text = match column_index {
            0 => &self.consumer,
            1 => &self.slot,
            2 => &self.group,
            3 => &self.route_version,
            4 => &self.state,
            _ => return None,
        };
        Some(listview::RenderMethod::Text(text))
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Handles {
    pub(crate) list: Handle<ListView<ConnectionRowView>>,
    pub(crate) empty: Handle<Label>,
    pub(crate) notice: Handle<Label>,
}

pub(crate) fn add(tab: &mut Tab, index: u32, state: &ProjectState) -> Handles {
    let mut panel = Panel::new("Route matrix", layout!("l:0,t:0,r:0,b:3"));
    let mut list = ListView::new(
        layout!("l:1,t:1,r:1,b:2"),
        // SearchBar is intentionally absent; it breaks global Ctrl bindings.
        listview::Flags::ScrollBars,
    );
    fill_list(&mut list, state, None);
    list.set_visible(!project_rows(state).is_empty());
    let list_handle = panel.add(list);
    let mut empty = Label::new(
        "Connections appear after a consumer instance has at least one consumed service slot.\n\nCreate a consumer with F2 on Instances, then return here and press Enter on a slot to connect it. Switchyard never chooses a provider group for you.",
        layout!("l:3,t:3,r:3,h:7"),
    );
    empty.set_visible(project_rows(state).is_empty());
    let empty_handle = panel.add(empty);
    panel.add(Label::new(EXPLAINER, layout!("l:1,b:0,r:1,h:1")));
    tab.add(index, panel);
    let notice_handle = tab.add(
        index,
        Label::new(&diagnostics(state), layout!("l:1,b:0,r:1,h:2")),
    );
    Handles {
        list: list_handle,
        empty: empty_handle,
        notice: notice_handle,
    }
}

pub(crate) fn project_rows(state: &ProjectState) -> Vec<ConnectionRowView> {
    state
        .deployments
        .iter()
        .enumerate()
        .flat_map(|(deployment_index, deployment)| {
            deployment.connections.rows.iter().map(move |row| {
                let matching = deployment
                    .route_statuses
                    .iter()
                    .filter(|status| status.binding_id == row.consumer)
                    .collect::<Vec<_>>();
                let (route_version, route_state) = route_summary(&matching);
                let unbound = row.current_group.is_none();
                ConnectionRowView {
                    deployment_index,
                    consumer: row.consumer.clone(),
                    slot: row.slot.clone(),
                    group: row
                        .current_group
                        .clone()
                        .unwrap_or_else(|| "not connected — press Enter to fix".into()),
                    route_version,
                    state: if unbound && route_state == "not applied" {
                        "not connected — press Enter to fix".into()
                    } else if unbound {
                        format!("not connected — press Enter to fix; last {route_state}")
                    } else {
                        route_state
                    },
                    compatible_groups: row.compatible_groups.clone(),
                }
            })
        })
        .collect()
}

fn route_summary(statuses: &[&RouteStatus]) -> (String, String) {
    if statuses.is_empty() {
        return ("—".into(), "not applied".into());
    }
    let desired = statuses
        .iter()
        .filter_map(|status| status.desired_version)
        .max();
    let observed = statuses
        .iter()
        .filter_map(|status| status.observed_version)
        .max();
    let version = match (desired, observed) {
        (Some(desired), Some(observed)) if desired != observed => {
            format!("desired v{desired} / observed v{observed}")
        }
        (_, Some(observed)) => format!("v{observed}"),
        (Some(desired), None) => format!("desired v{desired} / observed —"),
        (None, None) => "—".into(),
    };
    if let Some(failed) = statuses
        .iter()
        .find(|status| status.apply_status == "failed")
    {
        let error = failed.last_error_code.as_deref().unwrap_or("unknown error");
        return (
            version,
            format!("failed: {error} · transition {}", failed.transition_state),
        );
    }
    if let Some(applying) = statuses
        .iter()
        .find(|status| status.apply_status == "pending")
    {
        return (
            version,
            format!("applying · transition {}", applying.transition_state),
        );
    }
    let transitions = distinct(
        statuses
            .iter()
            .map(|status| status.transition_state.as_str()),
    );
    (
        version,
        format!("active · transition {}", transitions.join(", ")),
    )
}

fn distinct<'a>(values: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let mut values = values.collect::<Vec<_>>();
    values.sort_unstable();
    values.dedup();
    values
}

pub(crate) fn fill_list(
    list: &mut ListView<ConnectionRowView>,
    state: &ProjectState,
    preferred: Option<(usize, String, String)>,
) {
    list.clear();
    let rows = project_rows(state);
    let selected = preferred
        .as_ref()
        .and_then(|(deployment, consumer, slot)| {
            rows.iter().position(|row| {
                row.deployment_index == *deployment
                    && row.consumer == *consumer
                    && row.slot == *slot
            })
        })
        .or_else(|| (!rows.is_empty()).then_some(0));
    for row in rows {
        list.add(row);
    }
    for _ in 0..selected.unwrap_or(0) {
        OnKeyPressed::on_key_pressed(list, Key::new(KeyCode::Down, KeyModifier::None), '\0');
    }
}

pub(crate) fn diagnostics(state: &ProjectState) -> String {
    state
        .connections_error
        .as_ref()
        .map_or_else(String::new, |error| {
            format!("Some connections could not be loaded: {error}")
        })
}

pub(crate) fn source_row<'a>(
    state: &'a ProjectState,
    view: &ConnectionRowView,
) -> Option<(&'a crate::state::DeploymentProjection, &'a ConnectionRow)> {
    let deployment = state.deployments.get(view.deployment_index)?;
    let row = deployment
        .connections
        .rows
        .iter()
        .find(|row| row.consumer == view.consumer && row.slot == view.slot)?;
    Some((deployment, row))
}

#[derive(Clone)]
struct GroupChoice {
    name: String,
    preview_index: usize,
}

fn group_choices(previews: &[SwitchPreview]) -> Vec<GroupChoice> {
    previews
        .iter()
        .enumerate()
        .map(|(preview_index, preview)| GroupChoice {
            name: preview.new_group.clone(),
            preview_index,
        })
        .collect()
}

impl DropDownListType for GroupChoice {
    fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BindRequest {
    pub(crate) group: String,
}

#[ModalWindow(
    events = ButtonEvents + DropDownListEvents<GroupChoice>,
    response = BindRequest
)]
struct SwitchDialog {
    groups: Handle<DropDownList<GroupChoice>>,
    preview: Handle<TextArea>,
    error: Handle<Label>,
    apply: Handle<Button>,
    cancel: Handle<Button>,
    previews: Vec<SwitchPreview>,
}

impl SwitchDialog {
    fn new(consumer: &str, previews: Vec<SwitchPreview>) -> Self {
        let mut dialog = Self {
            base: ModalWindow::new(
                "Atomic connection switch",
                layout!("a:c,w:92,h:28"),
                window::Flags::None,
            ),
            groups: Handle::None,
            preview: Handle::None,
            error: Handle::None,
            apply: Handle::None,
            cancel: Handle::None,
            previews,
        };
        dialog.add(Label::new(
            &format!("Consumer: {consumer}\nChoose one compatible complete provider group:"),
            layout!("l:2,t:1,r:2,h:2"),
        ));
        let mut groups = DropDownList::new(
            layout!("l:2,t:4,r:2,h:1"),
            dropdownlist::Flags::AllowNoneSelection,
        );
        groups.set_none_string("Choose a compatible group (no automatic selection)");
        for choice in group_choices(&dialog.previews) {
            groups.add(choice);
        }
        dialog.groups = dialog.add(groups);
        dialog.preview = dialog.add(TextArea::new(
            "Select a provider group to see every route in the old → new preview.",
            layout!("l:2,t:6,r:2,b:4"),
            textarea::Flags::ReadOnly | textarea::Flags::ScrollBars,
        ));
        dialog.error = dialog.add(Label::new("", layout!("l:2,b:3,r:2,h:1")));
        dialog.apply = dialog.add(Button::new("&Apply", layout!("x:35%,y:100%,p:b,w:16,h:1")));
        dialog.cancel = dialog.add(Button::new("&Cancel", layout!("x:65%,y:100%,p:b,w:16,h:1")));
        dialog
    }

    fn selected_preview_index(&self) -> Option<usize> {
        self.control(self.groups)
            .and_then(DropDownList::selected_item)
            .map(|choice| choice.preview_index)
    }

    fn update_preview(&mut self) {
        let text = self
            .selected_preview_index()
            .and_then(|index| self.previews.get(index))
            .map_or_else(
                || "Select a provider group to see every route in the old → new preview.".into(),
                render_preview,
            );
        let preview = self.preview;
        if let Some(area) = self.control_mut(preview) {
            area.set_text(&text);
        }
        let error = self.error;
        if let Some(label) = self.control_mut(error) {
            label.set_caption("");
        }
    }
}

impl DropDownListEvents<GroupChoice> for SwitchDialog {
    fn on_selection_changed(
        &mut self,
        handle: Handle<DropDownList<GroupChoice>>,
    ) -> EventProcessStatus {
        if handle == self.groups {
            self.update_preview();
            EventProcessStatus::Processed
        } else {
            EventProcessStatus::Ignored
        }
    }
}

impl ButtonEvents for SwitchDialog {
    fn on_pressed(&mut self, handle: Handle<Button>) -> EventProcessStatus {
        if handle == self.cancel {
            self.exit();
        } else if handle == self.apply {
            let Some(index) = self.selected_preview_index() else {
                let error = self.error;
                if let Some(label) = self.control_mut(error) {
                    label.set_caption("Choose a compatible provider group before applying.");
                }
                return EventProcessStatus::Processed;
            };
            let preview = &self.previews[index];
            if !preview.diagnostics.is_empty() {
                let error = self.error;
                if let Some(label) = self.control_mut(error) {
                    label.set_caption("This preview has validation diagnostics and cannot apply.");
                }
                return EventProcessStatus::Processed;
            }
            self.exit_with(BindRequest {
                group: preview.new_group.clone(),
            });
        } else {
            return EventProcessStatus::Ignored;
        }
        EventProcessStatus::Processed
    }
}

pub(crate) fn show_switch(consumer: &str, previews: Vec<SwitchPreview>) -> Option<BindRequest> {
    SwitchDialog::new(consumer, previews).show()
}

pub(crate) fn render_preview(preview: &SwitchPreview) -> String {
    let routes = if preview.affected_services.is_empty() {
        "  (no route changes)".into()
    } else {
        let mut lines = vec![format!(
            "  {:<24} | {:<28} | {}",
            "Route", "Old provider", "New provider"
        )];
        lines.push(format!("  {}", "-".repeat(78)));
        lines.extend(preview.affected_services.iter().map(|change| {
            format!(
                "  {:<24} | {:<28} | {}",
                change.service,
                change.old_provider.as_deref().unwrap_or("not connected"),
                change.new_provider.as_deref().unwrap_or("not connected"),
            )
        }));
        lines.join("\n")
    };
    let diagnostics = if preview.diagnostics.is_empty() {
        "Validation: compatible — ready for one atomic apply.".into()
    } else {
        format!(
            "Cannot apply:\n{}",
            preview
                .diagnostics
                .iter()
                .map(|item| format!("  {item}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };
    format!(
        "Group: {} → {}\n\nEvery route that will change:\n{}\n\n{}\nThe complete binding is validated and applied atomically; no route-by-route mutation is performed.",
        preview.old_group.as_deref().unwrap_or("not connected"),
        preview.new_group,
        routes,
        diagnostics,
    )
}

pub(crate) fn render_result(
    consumer: &str,
    succeeded: bool,
    operation_detail: &str,
    statuses: &[RouteStatus],
) -> String {
    let matching = statuses
        .iter()
        .filter(|status| status.binding_id == consumer)
        .collect::<Vec<_>>();
    let mut lines = vec![
        if succeeded {
            "Atomic binding operation succeeded.".into()
        } else {
            "Atomic binding operation failed.".into()
        },
        operation_detail.into(),
    ];
    if matching.is_empty() {
        lines.push("Route status: no durable router observation is available yet.".into());
        if !succeeded {
            lines.push("Rollback information: unavailable from route status.".into());
        }
    } else {
        lines.push(String::new());
        lines.push("Router observations:".into());
        for status in matching {
            lines.push(route_detail(status));
        }
    }
    lines.join("\n")
}

fn route_detail(status: &RouteStatus) -> String {
    let rollback = status
        .history
        .iter()
        .rev()
        .find(|entry| entry.status == "rolled_back")
        .map(|entry| {
            format!(
                "rollback recorded at v{} (timestamp {})",
                entry.version, entry.recorded_at
            )
        })
        .or_else(|| {
            status
                .previous_version
                .map(|version| format!("previous version v{version} available for rollback"))
        })
        .unwrap_or_else(|| "no rollback recorded".into());
    format!(
        "  {} — desired {}; observed {}; status {}; transition {}; error {}; {}",
        status.router,
        version(status.desired_version),
        version(status.observed_version),
        status.apply_status,
        status.transition_state,
        status.last_error_code.as_deref().unwrap_or("none"),
        rollback,
    )
}

fn version(value: Option<i64>) -> String {
    value.map_or_else(|| "—".into(), |version| format!("v{version}"))
}

#[ModalWindow(events = ButtonEvents)]
struct ResultDialog {}

impl ResultDialog {
    fn new(text: &str) -> Self {
        let mut dialog = Self {
            base: ModalWindow::new(
                "Connection switch result",
                layout!("a:c,w:88,h:23"),
                window::Flags::None,
            ),
        };
        dialog.add(TextArea::new(
            text,
            layout!("l:1,t:1,r:1,b:3"),
            textarea::Flags::ReadOnly | textarea::Flags::ScrollBars,
        ));
        dialog.add(Button::new("Close", layout!("x:50%,y:100%,p:b,w:14,h:1")));
        dialog
    }
}

impl ButtonEvents for ResultDialog {
    fn on_pressed(&mut self, _: Handle<Button>) -> EventProcessStatus {
        self.exit();
        EventProcessStatus::Processed
    }
}

pub(crate) fn show_result(text: &str) {
    ResultDialog::new(text).show();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{DeploymentProjection, ProjectState};
    use switchyard_ops::{RouteChange, RouteHistoryEntry};

    fn connection() -> ConnectionRow {
        ConnectionRow {
            consumer: "frontend-a".into(),
            slot: "api".into(),
            current_group: None,
            compatible_groups: vec!["feature".into(), "main".into()],
            providers: Vec::new(),
        }
    }

    #[test]
    fn matrix_projection_surfaces_unbound_fix_key_and_route_failure() {
        let failed = RouteStatus {
            router: "sidecar".into(),
            binding_id: "frontend-a".into(),
            desired_version: Some(5),
            observed_version: Some(4),
            previous_version: Some(3),
            apply_status: "failed".into(),
            transition_state: "rolling_back".into(),
            last_error_code: Some("timeout".into()),
            history: Vec::new(),
        };
        let state = ProjectState {
            deployments: vec![DeploymentProjection {
                connections: switchyard_ops::ConnectionMatrix {
                    rows: vec![connection()],
                },
                route_statuses: vec![failed],
                ..Default::default()
            }],
            ..Default::default()
        };
        let rows = project_rows(&state);
        assert_eq!(rows[0].group, "not connected — press Enter to fix");
        assert_eq!(rows[0].route_version, "desired v5 / observed v4");
        assert!(rows[0].state.contains("not connected — press Enter to fix"));
        assert!(rows[0].state.contains("failed: timeout"));
        assert!(rows[0].state.contains("rolling_back"));
    }

    #[test]
    fn switch_display_contains_only_ops_compatible_groups() {
        let row = connection();
        let previews = row
            .compatible_groups
            .iter()
            .map(|group| SwitchPreview {
                consumer: row.consumer.clone(),
                old_group: None,
                new_group: group.clone(),
                old_providers: Vec::new(),
                new_providers: Vec::new(),
                affected_services: Vec::new(),
                diagnostics: Vec::new(),
            })
            .collect::<Vec<_>>();
        let groups = group_choices(&previews);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].name(), "feature");
        assert_eq!(groups[1].name(), "main");
    }

    #[test]
    fn synthetic_switch_preview_renders_every_old_to_new_route() {
        let preview = SwitchPreview {
            consumer: "frontend-a".into(),
            old_group: Some("main".into()),
            new_group: "feature".into(),
            old_providers: Vec::new(),
            new_providers: Vec::new(),
            affected_services: vec![
                RouteChange {
                    service: "api".into(),
                    old_provider: Some("api-main/http".into()),
                    new_provider: Some("api-feature/http".into()),
                },
                RouteChange {
                    service: "db".into(),
                    old_provider: Some("db-main/postgres".into()),
                    new_provider: Some("db-feature/postgres".into()),
                },
            ],
            diagnostics: Vec::new(),
        };
        let rendered = render_preview(&preview);
        assert!(rendered.contains("main → feature"));
        assert!(rendered.contains("api-main/http"));
        assert!(rendered.contains("api-feature/http"));
        assert!(rendered.contains("db-main/postgres"));
        assert!(rendered.contains("db-feature/postgres"));
        assert!(rendered.contains("applied atomically"));

        let status = RouteStatus {
            router: "sidecar".into(),
            binding_id: "frontend-a".into(),
            desired_version: Some(6),
            observed_version: Some(5),
            previous_version: Some(5),
            apply_status: "failed".into(),
            transition_state: "rolled_back".into(),
            last_error_code: Some("activation_failed".into()),
            history: vec![RouteHistoryEntry {
                version: 5,
                status: "rolled_back".into(),
                recorded_at: 42,
            }],
        };
        let result = render_result("frontend-a", false, "exit code 1", &[status]);
        assert!(result.contains("rollback recorded at v5"));
        assert!(result.contains("activation_failed"));
    }
}
