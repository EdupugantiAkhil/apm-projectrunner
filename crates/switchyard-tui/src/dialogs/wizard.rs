use std::collections::BTreeMap;

use appcui::prelude::*;
use switchyard_ops::{
    CreateInstanceRequest, InstancePreview, ProfileTrust, load_profile_block, preview_instance,
};
use switchyard_state::DeviceCheckStatus;

use crate::state::ProjectState;

#[derive(Clone, Debug)]
pub(crate) struct CreateWizardResult {
    pub(crate) definition: std::path::PathBuf,
    pub(crate) request: CreateInstanceRequest,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct WizardDraft {
    deployment: usize,
    checkout: usize,
    profile: usize,
    device: usize,
    name: String,
    parameters: BTreeMap<String, String>,
    field_errors: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WizardStep {
    Checkout,
    Profile,
    Device,
    Identity,
    Preview,
}

fn validate_step(
    step: WizardStep,
    draft: &WizardDraft,
    required: &[String],
    device_eligible: bool,
) -> BTreeMap<String, String> {
    let mut errors = BTreeMap::new();
    match step {
        WizardStep::Checkout => {}
        WizardStep::Profile => {}
        WizardStep::Device if !device_eligible => {
            errors.insert("device".into(), "the selected device is ineligible".into());
        }
        WizardStep::Identity => {
            if draft.name.trim().is_empty() {
                errors.insert("name".into(), "instance name is required".into());
            }
            for name in required {
                if draft
                    .parameters
                    .get(name)
                    .is_none_or(|value| value.trim().is_empty())
                {
                    errors.insert(
                        format!("parameter:{name}"),
                        format!("parameter `{name}` is required"),
                    );
                }
            }
        }
        _ => {}
    }
    errors
}

pub(crate) fn show(state: &ProjectState, deployment: usize) -> Option<CreateWizardResult> {
    if state.deployments.is_empty() {
        dialogs::message(
            "Create instance",
            "No deployment definition is available. Add deployment.yaml before creating an instance.",
        );
        return None;
    }
    if state.deployments[deployment].source_choices.is_empty() {
        dialogs::message(
            "Create instance",
            "No checkout is available. Press F2 in Code to register or clone one first.",
        );
        return None;
    }
    let mut draft = WizardDraft {
        deployment,
        ..WizardDraft::default()
    };
    let mut step = WizardStep::Checkout;
    loop {
        match step {
            WizardStep::Checkout => match CheckoutStep::new(state, &draft).show()? {
                Nav::Next(index) => {
                    draft.checkout = index;
                    step = WizardStep::Profile;
                }
                Nav::Back => return None,
            },
            WizardStep::Profile => {
                let profiles = valid_profiles(state, &draft);
                if profiles.is_empty() {
                    dialogs::message(
                        "Create instance — profile",
                        "No trusted startup profile is valid for this checkout. Validate or import one in Profiles first.",
                    );
                    step = WizardStep::Checkout;
                    continue;
                }
                match ChoiceStep::new(
                    "Create instance — 2/5 Profile",
                    &format!(
                        "Only trusted/imported startup profiles are selectable for this checkout.{}",
                        draft
                            .field_errors
                            .get("profile")
                            .map_or_else(String::new, |error| format!("\nProfile: {error}"))
                    ),
                    profiles
                        .iter()
                        .map(|index| state.profiles[*index].name.clone())
                        .collect(),
                    profiles
                        .iter()
                        .position(|index| *index == draft.profile)
                        .unwrap_or(0),
                )
                .show()?
                {
                    Nav::Next(index) => {
                        let selected = profiles[index];
                        if draft.profile != selected || draft.parameters.is_empty() {
                            draft.profile = selected;
                            reset_parameter_defaults(state, &mut draft);
                        }
                        step = WizardStep::Device;
                    }
                    Nav::Back => step = WizardStep::Checkout,
                }
            }
            WizardStep::Device => match DeviceStep::new(
                state,
                draft.device,
                draft.field_errors.get("device").map(String::as_str),
            )
            .show()?
            {
                Nav::Next(index) => {
                    draft.device = index;
                    step = WizardStep::Identity;
                }
                Nav::Back => step = WizardStep::Profile,
            },
            WizardStep::Identity => {
                let parameters = parameter_specs(state, &draft);
                match IdentityStep::new(&draft, &parameters).show()? {
                    IdentityNav::Next { name, values } => {
                        draft.name = name;
                        draft.parameters = values;
                        draft.field_errors.clear();
                        step = WizardStep::Preview;
                    }
                    IdentityNav::Back { name, values } => {
                        draft.name = name;
                        draft.parameters = values;
                        step = WizardStep::Device;
                    }
                }
            }
            WizardStep::Preview => {
                let (definition, request) = request(state, &draft)?;
                let preview = match preview_instance(&state.project_dir, &definition, &request) {
                    Ok(preview) => preview,
                    Err(error) => {
                        dialogs::error("Instance preview failed", &error.to_string());
                        step = WizardStep::Identity;
                        continue;
                    }
                };
                draft.field_errors = diagnostic_fields(&preview);
                let text = preview_text(state, &draft, &preview);
                match PreviewStep::new(&text, preview.diagnostics.is_empty()).show()? {
                    Nav::Next(_) => {
                        return Some(CreateWizardResult {
                            definition,
                            request,
                        });
                    }
                    Nav::Back => step = WizardStep::Identity,
                }
            }
        }
    }
}

fn diagnostic_fields(preview: &InstancePreview) -> BTreeMap<String, String> {
    let mut fields = BTreeMap::new();
    for diagnostic in &preview.diagnostics {
        let field = if diagnostic.path.ends_with(".name") {
            Some("name".into())
        } else if diagnostic.path.ends_with(".block") {
            Some("profile".into())
        } else if diagnostic.path.ends_with(".source") {
            Some("source".into())
        } else if diagnostic.path.ends_with(".device")
            || diagnostic.message.starts_with("remote ")
            || diagnostic.message.starts_with("Remote ")
        {
            Some("device".into())
        } else {
            diagnostic
                .path
                .split(".parameters.")
                .nth(1)
                .map(|name| format!("parameter:{name}"))
        };
        if let Some(field) = field {
            fields.insert(field, diagnostic.message.clone());
        }
    }
    fields
}

fn valid_profiles(state: &ProjectState, draft: &WizardDraft) -> Vec<usize> {
    let definition = &state.deployments[draft.deployment].bundle;
    state
        .profiles
        .iter()
        .enumerate()
        .filter(|(_, profile)| {
            matches!(
                profile.row.trust,
                ProfileTrust::Trusted | ProfileTrust::Imported
            )
        })
        .filter(|(_, profile)| {
            load_profile_block(
                &state.project_dir,
                definition,
                &profile.name,
                &profile.row.origin,
            )
            .is_ok()
        })
        .map(|(index, _)| index)
        .collect()
}

#[derive(Clone, Debug)]
struct ParameterSpec {
    name: String,
    required: bool,
    default: String,
}

fn parameter_specs(state: &ProjectState, draft: &WizardDraft) -> Vec<ParameterSpec> {
    let profile = &state.profiles[draft.profile];
    load_profile_block(
        &state.project_dir,
        &state.deployments[draft.deployment].bundle,
        &profile.name,
        &profile.row.origin,
    )
    .map(|block| {
        block
            .parameters
            .into_iter()
            .map(|(name, parameter)| ParameterSpec {
                name,
                required: parameter.required,
                default: parameter.default.unwrap_or_default(),
            })
            .collect()
    })
    .unwrap_or_default()
}

fn reset_parameter_defaults(state: &ProjectState, draft: &mut WizardDraft) {
    draft.parameters = parameter_specs(state, draft)
        .into_iter()
        .map(|parameter| (parameter.name, parameter.default))
        .collect();
}

fn device(state: &ProjectState, index: usize) -> (String, bool, String) {
    if index == 0 {
        return ("local".into(), true, "eligible for local execution".into());
    }
    let Some(device) = state.devices.get(index - 1) else {
        return (
            "unknown".into(),
            false,
            "device is no longer registered".into(),
        );
    };
    let eligible = device.device.last_check_status == DeviceCheckStatus::Eligible;
    let reason = switchyard_ops::devices::eligibility_label(&device.device);
    (device.name.clone(), eligible, reason)
}

fn request(
    state: &ProjectState,
    draft: &WizardDraft,
) -> Option<(std::path::PathBuf, CreateInstanceRequest)> {
    let profile = state.profiles.get(draft.profile)?;
    let source = state
        .deployments
        .get(draft.deployment)?
        .source_choices
        .get(draft.checkout)?;
    let (device, _, _) = device(state, draft.device);
    let definition = state.deployments.get(draft.deployment)?.bundle.clone();
    Some((
        definition,
        CreateInstanceRequest {
            name: draft.name.trim().into(),
            profile: profile.name.clone(),
            profile_origin: profile.row.origin.clone(),
            source: source.name.clone(),
            device,
            parameters: draft.parameters.clone(),
        },
    ))
}

fn preview_text(state: &ProjectState, draft: &WizardDraft, preview: &InstancePreview) -> String {
    let profile = &state.profiles[draft.profile];
    let block = load_profile_block(
        &state.project_dir,
        &state.deployments[draft.deployment].bundle,
        &profile.name,
        &profile.row.origin,
    );
    let topology = block.map_or_else(
        |error| format!("Could not expand ports/volumes: {error}"),
        |block| {
            block
                .services
                .iter()
                .map(|(name, service)| {
                    let ports = if service.publish.is_empty() {
                        "none".into()
                    } else {
                        service
                            .publish
                            .iter()
                            .map(u16::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    };
                    let volumes = if service.volumes.is_empty() {
                        "none".into()
                    } else {
                        service
                            .volumes
                            .iter()
                            .map(|volume| {
                                format!(
                                    "{} -> {}{}",
                                    volume.name,
                                    volume.target.display(),
                                    if volume.read_only { " (read-only)" } else { "" }
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(", ")
                    };
                    format!("  {name}\n    ports: {ports}\n    volumes: {volumes}")
                })
                .collect::<Vec<_>>()
                .join("\n")
        },
    );
    let diagnostics = if preview.diagnostics.is_empty() {
        "Validation passed.".into()
    } else {
        preview
            .diagnostics
            .iter()
            .map(|diagnostic| format!("{}: {}", diagnostic.path, diagnostic.message))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "No files have changed. Create will append this authored instance.\n\nExpanded runtime services:\n{}\n\nService ports and volumes:\n{}\n\n{}",
        preview
            .expanded_services
            .iter()
            .map(|service| format!("  {service}"))
            .collect::<Vec<_>>()
            .join("\n"),
        topology,
        diagnostics,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Nav {
    Next(usize),
    Back,
}

#[derive(Clone)]
struct Choice(String);
impl DropDownListType for Choice {
    fn name(&self) -> &str {
        &self.0
    }
}

#[ModalWindow(events = ButtonEvents, response = Nav)]
struct ChoiceStep {
    choices: Handle<DropDownList<Choice>>,
    next: Handle<Button>,
    back: Handle<Button>,
}

impl ChoiceStep {
    fn new(title: &str, explanation: &str, values: Vec<String>, selected: usize) -> Self {
        let mut dialog = Self {
            base: ModalWindow::new(title, layout!("a:c,w:78,h:11"), window::Flags::None),
            choices: Handle::None,
            next: Handle::None,
            back: Handle::None,
        };
        dialog.add(Label::new(explanation, layout!("l:2,t:1,r:2,h:2")));
        let mut choices = DropDownList::new(layout!("l:2,t:4,r:2,h:1"), dropdownlist::Flags::None);
        for value in values {
            choices.add(Choice(value));
        }
        choices.set_index(selected as u32);
        dialog.choices = dialog.add(choices);
        dialog.back = dialog.add(Button::new("&Back", layout!("x:35%,y:100%,p:b,w:16,h:1")));
        dialog.next = dialog.add(Button::new("&Next", layout!("x:65%,y:100%,p:b,w:16,h:1")));
        dialog
    }
}

impl ButtonEvents for ChoiceStep {
    fn on_pressed(&mut self, handle: Handle<Button>) -> EventProcessStatus {
        if handle == self.back {
            self.exit_with(Nav::Back);
        } else if handle == self.next {
            if let Some(index) = self
                .control(self.choices)
                .and_then(DropDownList::index)
                .map(|index| index as usize)
            {
                self.exit_with(Nav::Next(index));
            }
        } else {
            return EventProcessStatus::Ignored;
        }
        EventProcessStatus::Processed
    }
}

#[derive(Clone)]
struct CheckoutRow {
    index: usize,
    name: String,
    commit: String,
    dirty: String,
}
impl ListItem for CheckoutRow {
    fn columns_count() -> u16 {
        3
    }
    fn column(index: u16) -> Column {
        match index {
            0 => Column::new("Checkout / worktree", 34, TextAlignment::Left),
            1 => Column::new("Commit", 14, TextAlignment::Left),
            _ => Column::new("Working tree", 14, TextAlignment::Left),
        }
    }
    fn render_method(&self, index: u16) -> Option<listview::RenderMethod<'_>> {
        Some(listview::RenderMethod::Text(match index {
            0 => &self.name,
            1 => &self.commit,
            2 => &self.dirty,
            _ => return None,
        }))
    }
}

#[ModalWindow(events = ButtonEvents, response = Nav)]
struct CheckoutStep {
    tree: Handle<TreeView<CheckoutRow>>,
    next: Handle<Button>,
    cancel: Handle<Button>,
}
impl CheckoutStep {
    fn new(state: &ProjectState, draft: &WizardDraft) -> Self {
        let choices = &state.deployments[draft.deployment].source_choices;
        let mut rows = choices
            .iter()
            .enumerate()
            .map(|(index, choice)| {
                let inspected = state
                    .sources
                    .iter()
                    .find(|source| source.name == choice.name);
                let commit = inspected
                    .and_then(|source| source.inspection.identity.commit.as_deref())
                    .map(|value| value.chars().take(10).collect::<String>())
                    .unwrap_or_else(|| "unknown".into());
                let dirty = inspected
                    .and_then(|source| source.inspection.changes.as_ref())
                    .is_some_and(|changes| changes.is_dirty());
                CheckoutRow {
                    index,
                    name: choice.name.clone(),
                    commit,
                    dirty: if dirty {
                        "✗ dirty".into()
                    } else {
                        "clean".into()
                    },
                }
            })
            .collect::<Vec<_>>();
        rows.sort_by_key(|row| choices[row.index].worktree);
        let mut dialog = Self {
            base: ModalWindow::new(
                "Create instance — 1/5 Checkout",
                layout!("a:c,w:82,h:22"),
                window::Flags::None,
            ),
            tree: Handle::None,
            next: Handle::None,
            cancel: Handle::None,
        };
        let explanation = draft.field_errors.get("source").map_or_else(
            || "Choose the exact checkout/worktree identity for this instance.".into(),
            |error| format!("Choose the exact checkout/worktree identity. Checkout: {error}"),
        );
        dialog.add(Label::new(&explanation, layout!("l:2,t:1,r:2,h:1")));
        let mut tree = TreeView::new(layout!("l:2,t:3,r:2,b:3"), treeview::Flags::ScrollBars);
        let mut handles = BTreeMap::<usize, Handle<treeview::Item<CheckoutRow>>>::new();
        let mut preferred_handle = None;
        tree.add_batch(|tree| {
            for row in rows {
                let row_index = row.index;
                let choice = &choices[row.index];
                let parent = choice
                    .repository
                    .as_ref()
                    .and_then(|repository| {
                        choices
                            .iter()
                            .position(|candidate| &candidate.path == repository)
                    })
                    .and_then(|index| handles.get(&index).copied());
                let handle = if let Some(parent) = parent {
                    tree.add_to_parent(row, parent)
                } else {
                    tree.add_item(treeview::Item::expandable(row, false))
                };
                if row_index == draft.checkout {
                    preferred_handle = Some(handle);
                }
                handles.insert(row_index, handle);
            }
        });
        if let Some(handle) = preferred_handle {
            tree.move_cursor_to(handle);
        }
        dialog.tree = dialog.add(tree);
        dialog.cancel = dialog.add(Button::new("&Cancel", layout!("x:35%,y:100%,p:b,w:16,h:1")));
        dialog.next = dialog.add(Button::new("&Next", layout!("x:65%,y:100%,p:b,w:16,h:1")));
        dialog
    }
}

impl ButtonEvents for CheckoutStep {
    fn on_pressed(&mut self, handle: Handle<Button>) -> EventProcessStatus {
        if handle == self.cancel {
            self.exit_with(Nav::Back);
        } else if handle == self.next {
            if let Some(index) = self
                .control(self.tree)
                .and_then(TreeView::current_item)
                .map(|item| item.value().index)
            {
                self.exit_with(Nav::Next(index));
            }
        } else {
            return EventProcessStatus::Ignored;
        }
        EventProcessStatus::Processed
    }
}

#[ModalWindow(events = ButtonEvents, response = Nav)]
struct DeviceStep {
    choices: Handle<DropDownList<Choice>>,
    error: Handle<Label>,
    next: Handle<Button>,
    back: Handle<Button>,
    eligible: Vec<bool>,
}

impl DeviceStep {
    fn new(state: &ProjectState, selected: usize, field_error: Option<&str>) -> Self {
        let devices = (0..=state.devices.len())
            .map(|index| device(state, index))
            .collect::<Vec<_>>();
        let mut dialog = Self {
            base: ModalWindow::new(
                "Create instance — 3/5 Device",
                layout!("a:c,w:88,h:13"),
                window::Flags::None,
            ),
            choices: Handle::None,
            error: Handle::None,
            next: Handle::None,
            back: Handle::None,
            eligible: devices.iter().map(|(_, eligible, _)| *eligible).collect(),
        };
        dialog.add(Label::new("True placement is always explicit. Ineligible devices remain visible with the ops reason.", layout!("l:2,t:1,r:2,h:2")));
        let mut choices = DropDownList::new(layout!("l:2,t:4,r:2,h:1"), dropdownlist::Flags::None);
        for (name, eligible, reason) in devices {
            choices.add(Choice(format!(
                "{name} — {}{reason}",
                if eligible { "" } else { "DISABLED — " }
            )));
        }
        choices.set_index(selected.min(dialog.eligible.len().saturating_sub(1)) as u32);
        dialog.choices = dialog.add(choices);
        dialog.error = dialog.add(Label::new(
            field_error.map_or("", |error| error),
            layout!("l:2,t:6,r:2,h:2"),
        ));
        dialog.back = dialog.add(Button::new("&Back", layout!("x:35%,y:100%,p:b,w:16,h:1")));
        dialog.next = dialog.add(Button::new("&Next", layout!("x:65%,y:100%,p:b,w:16,h:1")));
        dialog
    }
}

impl ButtonEvents for DeviceStep {
    fn on_pressed(&mut self, handle: Handle<Button>) -> EventProcessStatus {
        if handle == self.back {
            self.exit_with(Nav::Back);
        } else if handle == self.next {
            let index = self
                .control(self.choices)
                .and_then(DropDownList::index)
                .map_or(0, |index| index as usize);
            if self.eligible.get(index).copied().unwrap_or(false) {
                self.exit_with(Nav::Next(index));
            } else {
                let error = self.error;
                if let Some(label) = self.control_mut(error) {
                    label.set_caption("Device: this placement is disabled; select an eligible device. The concrete reason is shown above.");
                }
            }
        } else {
            return EventProcessStatus::Ignored;
        }
        EventProcessStatus::Processed
    }
}

enum IdentityNav {
    Next {
        name: String,
        values: BTreeMap<String, String>,
    },
    Back {
        name: String,
        values: BTreeMap<String, String>,
    },
}

#[ModalWindow(events = ButtonEvents, response = IdentityNav)]
struct IdentityStep {
    name: Handle<TextField>,
    fields: Vec<(ParameterSpec, Handle<TextField>, Handle<Label>)>,
    name_error: Handle<Label>,
    next: Handle<Button>,
    back: Handle<Button>,
}

impl IdentityStep {
    fn new(draft: &WizardDraft, specs: &[ParameterSpec]) -> Self {
        let height = (14 + specs.len() as u32 * 2).min(34);
        let mut dialog = Self {
            base: ModalWindow::new(
                "Create instance — 4/5 Name + parameters",
                Layout::aligned(Alignment::Center, 88, height),
                window::Flags::None,
            ),
            name: Handle::None,
            fields: Vec::new(),
            name_error: Handle::None,
            next: Handle::None,
            back: Handle::None,
        };
        dialog.add(Label::new("Instance name", layout!("l:2,t:1,w:27,h:1")));
        dialog.name = dialog.add(TextField::new(
            &draft.name,
            layout!("l:30,t:1,r:2,h:1"),
            textfield::Flags::None,
        ));
        dialog.name_error = dialog.add(Label::new(
            draft.field_errors.get("name").map_or("", String::as_str),
            layout!("l:30,t:2,r:2,h:1"),
        ));
        let mut y = 4;
        for spec in specs {
            dialog.add(Label::new(
                &format!(
                    "{}{}",
                    spec.name,
                    if spec.required { " (required)" } else { "" }
                ),
                Layout::absolute(2, y, 27, 1),
            ));
            let value = draft.parameters.get(&spec.name).unwrap_or(&spec.default);
            let field = dialog.add(TextField::new(
                value,
                Layout::absolute(30, y, 54, 1),
                textfield::Flags::None,
            ));
            let error = dialog.add(Label::new(
                draft
                    .field_errors
                    .get(&format!("parameter:{}", spec.name))
                    .map_or("", String::as_str),
                Layout::absolute(30, y + 1, 54, 1),
            ));
            dialog.fields.push((spec.clone(), field, error));
            y += 2;
        }
        dialog.back = dialog.add(Button::new("&Back", layout!("x:35%,y:100%,p:b,w:16,h:1")));
        dialog.next = dialog.add(Button::new("&Next", layout!("x:65%,y:100%,p:b,w:16,h:1")));
        dialog
    }

    fn values(&self) -> (String, BTreeMap<String, String>) {
        let name = self
            .control(self.name)
            .map(|field| field.text().to_owned())
            .unwrap_or_default();
        let values = self
            .fields
            .iter()
            .map(|(spec, field, _)| {
                (
                    spec.name.clone(),
                    self.control(*field)
                        .map(|field| field.text().to_owned())
                        .unwrap_or_default(),
                )
            })
            .collect();
        (name, values)
    }
}

impl ButtonEvents for IdentityStep {
    fn on_pressed(&mut self, handle: Handle<Button>) -> EventProcessStatus {
        let (name, values) = self.values();
        if handle == self.back {
            self.exit_with(IdentityNav::Back { name, values });
        } else if handle == self.next {
            let draft = WizardDraft {
                name: name.clone(),
                parameters: values.clone(),
                ..Default::default()
            };
            let required = self
                .fields
                .iter()
                .filter(|(spec, _, _)| spec.required)
                .map(|(spec, _, _)| spec.name.clone())
                .collect::<Vec<_>>();
            let errors = validate_step(WizardStep::Identity, &draft, &required, true);
            let name_error = self.name_error;
            if let Some(label) = self.control_mut(name_error) {
                label.set_caption(errors.get("name").map_or("", String::as_str));
            }
            let error_updates = self
                .fields
                .iter()
                .map(|(spec, _, handle)| (spec.name.clone(), *handle))
                .collect::<Vec<_>>();
            for (parameter, handle) in error_updates {
                if let Some(label) = self.control_mut(handle) {
                    label.set_caption(
                        errors
                            .get(&format!("parameter:{parameter}"))
                            .map_or("", String::as_str),
                    );
                }
            }
            if errors.is_empty() {
                self.exit_with(IdentityNav::Next { name, values });
            }
        } else {
            return EventProcessStatus::Ignored;
        }
        EventProcessStatus::Processed
    }
}

#[ModalWindow(events = ButtonEvents, response = Nav)]
struct PreviewStep {
    create: Handle<Button>,
    back: Handle<Button>,
}
impl PreviewStep {
    fn new(text: &str, valid: bool) -> Self {
        let mut dialog = Self {
            base: ModalWindow::new(
                "Create instance — 5/5 Preview",
                layout!("a:c,w:92,h:30"),
                window::Flags::None,
            ),
            create: Handle::None,
            back: Handle::None,
        };
        dialog.add(TextArea::new(
            text,
            layout!("l:1,t:1,r:1,b:3"),
            textarea::Flags::ReadOnly | textarea::Flags::ScrollBars,
        ));
        dialog.back = dialog.add(Button::new("&Back", layout!("x:35%,y:100%,p:b,w:16,h:1")));
        let mut create = Button::new("&Create", layout!("x:65%,y:100%,p:b,w:16,h:1"));
        create.set_enabled(valid);
        dialog.create = dialog.add(create);
        dialog
    }
}
impl ButtonEvents for PreviewStep {
    fn on_pressed(&mut self, handle: Handle<Button>) -> EventProcessStatus {
        if handle == self.back {
            self.exit_with(Nav::Back);
        } else if handle == self.create {
            self.exit_with(Nav::Next(0));
        } else {
            return EventProcessStatus::Ignored;
        }
        EventProcessStatus::Processed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_step_attaches_name_and_required_parameter_errors() {
        let draft = WizardDraft {
            parameters: BTreeMap::from([("PORT".into(), String::new())]),
            ..Default::default()
        };
        let errors = validate_step(WizardStep::Identity, &draft, &["PORT".into()], true);
        assert_eq!(errors["name"], "instance name is required");
        assert_eq!(errors["parameter:PORT"], "parameter `PORT` is required");
    }

    #[test]
    fn ineligible_device_blocks_the_device_step() {
        let errors = validate_step(WizardStep::Device, &WizardDraft::default(), &[], false);
        assert_eq!(errors["device"], "the selected device is ineligible");
    }
}
