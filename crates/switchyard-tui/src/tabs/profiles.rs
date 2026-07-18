use appcui::prelude::*;
use serde_json::Value;
use switchyard_ops::{
    CreateInstanceRequest, ProfileAdapterKind, ProfileOrigin, ProfileTrust, preview_instance,
};

use crate::{
    dialogs::forms::SchemaFormDialog,
    state::{ProfileProjection, ProjectState},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProfileRowView {
    pub(crate) profile_index: usize,
    pub(crate) group: String,
    pub(crate) name: String,
    pub(crate) adapter: String,
    pub(crate) services: String,
    pub(crate) trust: String,
}

impl ListItem for ProfileRowView {
    fn columns_count() -> u16 {
        4
    }
    fn column(index: u16) -> Column {
        match index {
            0 => Column::new("Name", 22, TextAlignment::Left),
            1 => Column::new("Adapter", 20, TextAlignment::Left),
            2 => Column::new("Services", 10, TextAlignment::Right),
            _ => Column::new("Trust", 20, TextAlignment::Left),
        }
    }
    fn render_method(&self, column_index: u16) -> Option<listview::RenderMethod<'_>> {
        let text = match column_index {
            0 => &self.name,
            1 => &self.adapter,
            2 => &self.services,
            3 => &self.trust,
            _ => return None,
        };
        Some(listview::RenderMethod::Text(text))
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Handles {
    pub(crate) list: Handle<ListView<ProfileRowView>>,
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
    splitter.set_min_width(vsplitter::Panel::Left, 48);
    splitter.set_min_width(vsplitter::Panel::Right, 30);
    let mut left = Panel::new("Startup profile library", layout!("d:f"));
    let mut list = ListView::new(
        layout!("l:1,t:1,r:1,b:1"),
        listview::Flags::ScrollBars | listview::Flags::ShowGroups,
    );
    fill_list(&mut list, &state.profiles, None);
    let list_handle = left.add(list);
    let mut empty = Label::new(
        "A startup profile is a reusable definition of one service or a coordinated suite.\n\nPress F2 to start a project profile, or register a source containing switchyard-profiles.yaml and press F5 to discover it.",
        layout!("l:3,t:3,r:3,h:7"),
    );
    empty.set_visible(state.profiles.is_empty());
    let empty_handle = left.add(empty);
    splitter.add(vsplitter::Panel::Left, left);
    let mut right = Panel::new("Full expansion", layout!("d:f"));
    let detail = state
        .profiles
        .first()
        .map(|p| p.detail.as_str())
        .unwrap_or("Select a startup profile to inspect its full expansion.");
    // TextArea, not Markdown: the AppCUI 0.4.13 Markdown control hangs on
    // language-tagged code fences, and this content is data-driven plain text.
    let detail_handle = right.add(TextArea::new(
        detail,
        layout!("l:1,t:1,r:1,b:1"),
        textarea::Flags::ReadOnly | textarea::Flags::ScrollBars,
    ));
    splitter.add(vsplitter::Panel::Right, right);
    tab.add(index, splitter);
    let notice = profile_diagnostics(state);
    let notice_handle = tab.add(index, Label::new(&notice, layout!("l:1,b:0,r:1,h:2")));
    Handles {
        list: list_handle,
        detail: detail_handle,
        empty: empty_handle,
        notice: notice_handle,
    }
}

pub(crate) fn project_rows(profiles: &[ProfileProjection]) -> Vec<ProfileRowView> {
    profiles
        .iter()
        .enumerate()
        .map(|(profile_index, profile)| {
            let group = match &profile.row.origin {
                ProfileOrigin::Project => "Project".into(),
                ProfileOrigin::DiscoveredInSource { source, .. } => format!("Source: {source}"),
                ProfileOrigin::ImportedFromSource { .. } => "Imported".into(),
            };
            let adapters = profile
                .row
                .services
                .iter()
                .map(|service| adapter_name(service.adapter_kind))
                .collect::<Vec<_>>();
            let adapter = if adapters.windows(2).all(|pair| pair[0] == pair[1]) {
                adapters.first().copied().unwrap_or("none").into()
            } else {
                "mixed".into()
            };
            ProfileRowView {
                profile_index,
                group,
                name: profile.name.clone(),
                adapter,
                services: profile.row.services.len().to_string(),
                trust: trust_name(profile.row.trust).into(),
            }
        })
        .collect()
}

pub(crate) fn fill_list(
    list: &mut ListView<ProfileRowView>,
    profiles: &[ProfileProjection],
    _preferred: Option<usize>,
) {
    list.clear();
    let rows = project_rows(profiles);
    let mut groups = rows.iter().map(|row| row.group.clone()).collect::<Vec<_>>();
    groups.sort_by_key(|name| {
        if name == "Project" {
            (0, name.clone())
        } else if name.starts_with("Source: ") {
            (1, name.clone())
        } else {
            (2, name.clone())
        }
    });
    groups.dedup();
    for name in groups {
        let group = list.add_group(&name);
        list.add_to_group(
            rows.iter()
                .filter(|row| row.group == name)
                .cloned()
                .collect(),
            group,
        );
    }
}

fn adapter_name(kind: ProfileAdapterKind) -> &'static str {
    match kind {
        ProfileAdapterKind::Container => "container",
        ProfileAdapterKind::Script => "script",
        ProfileAdapterKind::ProcessCompose => "process-compose",
    }
}

fn trust_name(trust: ProfileTrust) -> &'static str {
    match trust {
        ProfileTrust::Trusted => "trusted",
        ProfileTrust::Imported => "imported",
        ProfileTrust::Changed => "changed — review",
        ProfileTrust::NotImported => "not imported",
    }
}

pub(crate) fn profile_diagnostics(state: &ProjectState) -> String {
    let mut messages = state.profile_source_errors.clone();
    if let Some(error) = &state.profiles_error {
        messages.push(error.clone());
    }
    messages.join(" | ")
}

#[derive(Clone)]
struct CheckoutChoice {
    index: usize,
    name: String,
    description: String,
}
impl DropDownListType for CheckoutChoice {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.description
    }
}

#[ModalWindow(events = ButtonEvents, response = usize)]
struct CheckoutDialog {
    choices: Handle<DropDownList<CheckoutChoice>>,
    validate: Handle<Button>,
    cancel: Handle<Button>,
}
impl CheckoutDialog {
    fn new(state: &ProjectState) -> Self {
        let mut dialog = Self {
            base: ModalWindow::new(
                "Validate against checkout",
                layout!("a:c,w:72,h:10"),
                window::Flags::None,
            ),
            choices: Handle::None,
            validate: Handle::None,
            cancel: Handle::None,
        };
        dialog.add(Label::new(
            "Checkout (validation only; nothing will be started)",
            layout!("l:2,t:1,r:2,h:1"),
        ));
        let mut choices = DropDownList::new(
            layout!("l:2,t:3,r:2,h:1"),
            dropdownlist::Flags::ShowDescription,
        );
        for (index, source) in state.sources.iter().enumerate() {
            let description = format!(
                "{} · {}",
                source
                    .inspection
                    .identity
                    .commit
                    .as_deref()
                    .map(|v| v.chars().take(10).collect::<String>())
                    .unwrap_or_else(|| "unknown".into()),
                if source
                    .inspection
                    .changes
                    .as_ref()
                    .is_some_and(|c| c.is_dirty())
                {
                    "dirty"
                } else {
                    "clean"
                }
            );
            choices.add(CheckoutChoice {
                index,
                name: source.name.clone(),
                description,
            });
        }
        dialog.choices = dialog.add(choices);
        dialog.validate = dialog.add(Button::new(
            "&Validate",
            layout!("x:35%,y:100%,p:b,w:16,h:1"),
        ));
        dialog.cancel = dialog.add(Button::new("&Cancel", layout!("x:65%,y:100%,p:b,w:16,h:1")));
        dialog
    }
}
impl ButtonEvents for CheckoutDialog {
    fn on_pressed(&mut self, handle: Handle<Button>) -> EventProcessStatus {
        if handle == self.validate {
            if let Some(index) = self
                .control(self.choices)
                .and_then(|c| c.selected_item())
                .map(|c| c.index)
            {
                self.exit_with(index);
            }
        } else if handle == self.cancel {
            self.exit();
        } else {
            return EventProcessStatus::Ignored;
        }
        EventProcessStatus::Processed
    }
}

pub(crate) fn validate_profile(state: &ProjectState, profile: &ProfileProjection) {
    if state.sources.is_empty() {
        dialogs::message(
            "Profile validation",
            "No checkout is available. Add code in the Code tab first.",
        );
        return;
    }
    let Some(source_index) = CheckoutDialog::new(state).show() else {
        return;
    };
    let source = &state.sources[source_index];
    let parameters = profile
        .json
        .get("parameters")
        .and_then(Value::as_object)
        .map(|items| {
            items
                .iter()
                .filter_map(|(name, spec)| {
                    spec.get("default")
                        .and_then(Value::as_str)
                        .map(|value| (name.clone(), value.into()))
                })
                .collect()
        })
        .unwrap_or_default();
    let request = CreateInstanceRequest {
        name: "profile-validation-preview".into(),
        profile: profile.name.clone(),
        profile_origin: profile.row.origin.clone(),
        source: source.name.clone(),
        device: "local".into(),
        parameters,
    };
    let report = match preview_instance(&state.project_dir, &profile.definition, &request) {
        Ok(preview) if preview.diagnostics.is_empty() => format!(
            "# Validation passed\n\n**Profile:** {}\n\n**Checkout:** {}\n\nExpanded services:\n{}",
            profile.name,
            source.name,
            preview
                .expanded_services
                .iter()
                .map(|s| format!("- `{s}`"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        Ok(preview) => format!(
            "# Validation errors\n\n{}",
            preview
                .diagnostics
                .iter()
                .map(|d| format!("- **{}** — {} (`{:?}`)", d.path, d.message, d.code))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        Err(error) => format!("# Validation could not run\n\n- **profile / checkout** — {error}"),
    };
    ReportDialog::new("Profile validation report", &report).show();
}

#[ModalWindow(events = ButtonEvents)]
struct ReportDialog {}
impl ReportDialog {
    fn new(title: &str, report: &str) -> Self {
        let mut d = Self {
            base: ModalWindow::new(title, layout!("a:c,w:84,h:28"), window::Flags::None),
        };
        d.add(Markdown::new(
            report,
            layout!("l:1,t:1,r:1,b:3"),
            markdown::Flags::ScrollBars,
        ));
        d.add(Button::new("Close", layout!("x:50%,y:100%,p:b,w:14,h:1")));
        d
    }
}
impl ButtonEvents for ReportDialog {
    fn on_pressed(&mut self, _: Handle<Button>) -> EventProcessStatus {
        self.exit();
        EventProcessStatus::Processed
    }
}

pub(crate) fn show_editor(profile: Option<&ProfileProjection>) {
    let registry = switchyard_adapters::built_in_registry();
    let schemas = registry.list();
    let (title, adapter_id, initial) = if let Some(profile) = profile {
        let execution = profile
            .json
            .get("services")
            .and_then(Value::as_object)
            .and_then(|s| s.values().next())
            .and_then(|s| s.get("execution"))
            .cloned()
            .unwrap_or(Value::Null);
        let id = match execution.get("type").and_then(Value::as_str) {
            Some("script") => "execution-runner-script",
            Some("processCompose") => "supervisor-process-compose",
            _ => "execution-container",
        };
        (
            format!("Edit {} — adapter configuration", profile.name),
            id.to_owned(),
            execution,
        )
    } else {
        let choices = schemas
            .iter()
            .filter(|metadata| {
                metadata.declaration.id.starts_with("execution-")
                    || metadata.declaration.id.starts_with("supervisor-")
            })
            .map(|metadata| AdapterChoice(metadata.declaration.id.clone()))
            .collect::<Vec<_>>();
        let Some(adapter) = AdapterDialog::new(choices).show() else {
            return;
        };
        (
            format!("New profile — {adapter}"),
            adapter,
            Value::Object(Default::default()),
        )
    };
    let Some(schema) = schemas
        .iter()
        .find(|metadata| metadata.declaration.id == adapter_id)
        .map(|metadata| &metadata.configuration_schema)
    else {
        dialogs::message(
            "Guided editor",
            "The selected adapter did not publish a JSON Schema.",
        );
        return;
    };
    if let Some(values) = SchemaFormDialog::new(&title, schema, &initial).show() {
        let yaml = serde_yaml::to_string(&values)
            .unwrap_or_else(|error| format!("could not render YAML: {error}"));
        ReportDialog::new("Guided editor preview", &format!("# Adapter configuration preview\n\n```\n{yaml}```\n\nSaving is unavailable because switchyard-ops does not yet expose a profile create/update mutation.")).show();
    }
}

#[derive(Clone)]
struct AdapterChoice(String);

impl DropDownListType for AdapterChoice {
    fn name(&self) -> &str {
        &self.0
    }
}

#[ModalWindow(events = ButtonEvents, response = String)]
struct AdapterDialog {
    choices: Handle<DropDownList<AdapterChoice>>,
    next: Handle<Button>,
    cancel: Handle<Button>,
}

impl AdapterDialog {
    fn new(choices: Vec<AdapterChoice>) -> Self {
        let mut dialog = Self {
            base: ModalWindow::new(
                "Choose startup adapter",
                layout!("a:c,w:68,h:10"),
                window::Flags::None,
            ),
            choices: Handle::None,
            next: Handle::None,
            cancel: Handle::None,
        };
        dialog.add(Label::new(
            "The next screen is generated from this adapter's JSON Schema.",
            layout!("l:2,t:1,r:2,h:1"),
        ));
        let mut list = DropDownList::new(layout!("l:2,t:3,r:2,h:1"), dropdownlist::Flags::None);
        for choice in choices {
            list.add(choice);
        }
        dialog.choices = dialog.add(list);
        dialog.next = dialog.add(Button::new("&Next", layout!("x:35%,y:100%,p:b,w:16,h:1")));
        dialog.cancel = dialog.add(Button::new("&Cancel", layout!("x:65%,y:100%,p:b,w:16,h:1")));
        dialog
    }
}

impl ButtonEvents for AdapterDialog {
    fn on_pressed(&mut self, handle: Handle<Button>) -> EventProcessStatus {
        if handle == self.next {
            if let Some(adapter) = self
                .control(self.choices)
                .and_then(|choices| choices.selected_item())
                .map(|choice| choice.0.clone())
            {
                self.exit_with(adapter);
            }
        } else if handle == self.cancel {
            self.exit();
        } else {
            return EventProcessStatus::Ignored;
        }
        EventProcessStatus::Processed
    }
}

pub(crate) fn source_for_import(profile: &ProfileProjection) -> Option<&str> {
    match (&profile.row.origin, profile.row.trust) {
        (ProfileOrigin::DiscoveredInSource { source, .. }, ProfileTrust::NotImported)
        | (ProfileOrigin::ImportedFromSource { source, .. }, ProfileTrust::Changed) => Some(source),
        _ => None,
    }
}

pub(crate) fn import_manifest_text(state: &ProjectState, source: &str) -> Result<String, String> {
    let path = state
        .sources
        .iter()
        .find(|item| item.name == source)
        .ok_or_else(|| format!("source `{source}` is not registered"))?
        .path
        .join("switchyard-profiles.yaml");
    std::fs::read_to_string(&path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))
}

pub(crate) fn confirm_import(profile: &ProfileProjection, source: &str, manifest: &str) -> bool {
    let explanation = format!(
        "Importing `{}` trusts the declarative definition from `{}`.\nNo repository script is inferred or executed. Review the verbatim manifest below.",
        profile.name, source
    );
    ImportDialog::new(&explanation, manifest).show().unwrap_or(false)
}

#[ModalWindow(events = ButtonEvents, response = bool)]
struct ImportDialog {
    import: Handle<Button>,
    cancel: Handle<Button>,
}
impl ImportDialog {
    fn new(explanation: &str, manifest: &str) -> Self {
        let mut d = Self {
            base: ModalWindow::new(
                "Review and trust import",
                layout!("a:c,w:94,h:32"),
                window::Flags::None,
            ),
            import: Handle::None,
            cancel: Handle::None,
        };
        d.add(Label::new(explanation, layout!("l:1,t:1,r:1,h:2")));
        // The manifest is arbitrary repository content; render it through a
        // read-only TextArea so no Markdown construct can affect it.
        d.add(TextArea::new(
            manifest,
            layout!("l:1,t:4,r:1,b:3"),
            textarea::Flags::ReadOnly | textarea::Flags::ScrollBars,
        ));
        d.import = d.add(Button::new("&Import", layout!("x:35%,y:100%,p:b,w:16,h:1")));
        d.cancel = d.add(Button::new("&Refuse", layout!("x:65%,y:100%,p:b,w:16,h:1")));
        let cancel = d.cancel;
        d.request_focus_for_control(cancel);
        d
    }
}
impl ButtonEvents for ImportDialog {
    fn on_pressed(&mut self, h: Handle<Button>) -> EventProcessStatus {
        if h == self.import {
            self.exit_with(true);
        } else if h == self.cancel {
            self.exit_with(false);
        } else {
            return EventProcessStatus::Ignored;
        }
        EventProcessStatus::Processed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use switchyard_ops::{ProfileRow, ProfileService};
    fn projection(name: &str, origin: ProfileOrigin) -> ProfileProjection {
        ProfileProjection {
            name: name.into(),
            row: ProfileRow {
                name: name.into(),
                origin,
                trust: ProfileTrust::Trusted,
                shadowed: false,
                services: vec![ProfileService {
                    name: "web".into(),
                    adapter_kind: ProfileAdapterKind::Container,
                }],
            },
            definition: "deployment.yaml".into(),
            json: Value::Null,
            detail: String::new(),
        }
    }
    #[test]
    fn rows_project_origin_groups_and_columns() {
        let rows = project_rows(&[
            projection("project", ProfileOrigin::Project),
            projection(
                "found",
                ProfileOrigin::DiscoveredInSource {
                    source: "repo".into(),
                    commit: None,
                },
            ),
            projection(
                "saved",
                ProfileOrigin::ImportedFromSource {
                    source: "repo".into(),
                    commit: None,
                },
            ),
        ]);
        assert_eq!(
            rows.iter()
                .map(|r| (&*r.group, &*r.name, &*r.adapter, &*r.services))
                .collect::<Vec<_>>(),
            vec![
                ("Project", "project", "container", "1"),
                ("Source: repo", "found", "container", "1"),
                ("Imported", "saved", "container", "1")
            ]
        );
    }
}
