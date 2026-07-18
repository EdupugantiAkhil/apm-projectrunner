use appcui::prelude::*;
use switchyard_ops::{RunScript, run_scripts::StructuredCommand};

use crate::state::{OperationLog, OperationOutcome, ProjectState};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ScriptRowView {
    pub(crate) script_index: usize,
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) action: String,
    pub(crate) description: String,
}

impl ListItem for ScriptRowView {
    fn columns_count() -> u16 {
        4
    }
    fn column(index: u16) -> Column {
        match index {
            0 => Column::new("Run action", 22, TextAlignment::Left),
            1 => Column::new("Kind", 12, TextAlignment::Left),
            2 => Column::new("Command", 18, TextAlignment::Left),
            _ => Column::new("Description", 50, TextAlignment::Left),
        }
    }
    fn render_method(&self, column_index: u16) -> Option<listview::RenderMethod<'_>> {
        let value = match column_index {
            0 => &self.name,
            1 => &self.kind,
            2 => &self.action,
            3 => &self.description,
            _ => return None,
        };
        Some(listview::RenderMethod::Text(value))
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Handles {
    pub(crate) list: Handle<ListView<ScriptRowView>>,
    pub(crate) empty: Handle<Label>,
    pub(crate) filter: Handle<TextField>,
    pub(crate) log: Handle<ListBox>,
    pub(crate) notice: Handle<Label>,
}

pub(crate) fn add(tab: &mut Tab, index: u32, state: &ProjectState) -> Handles {
    let mut splitter = HSplitter::new(
        0.42,
        layout!("l:0,t:0,r:0,b:3"),
        hsplitter::ResizeBehavior::PreserveAspectRatio,
    );
    splitter.set_min_height(hsplitter::Panel::Top, 8);
    splitter.set_min_height(hsplitter::Panel::Bottom, 10);
    let mut top = Panel::new(
        "Project run actions — .switchyard/run-scripts.yaml",
        layout!("d:f"),
    );
    let mut list = ListView::new(layout!("l:1,t:1,r:1,b:1"), listview::Flags::ScrollBars);
    fill_list(&mut list, &state.run_scripts, None);
    let list = top.add(list);
    let mut empty = Label::new(
        "Run actions are explicit project operations, not startup profiles.\n\nPress F2 to create the first action. Enter always previews and confirms before running.",
        layout!("l:3,t:3,r:3,h:5"),
    );
    empty.set_visible(state.run_scripts.is_empty());
    let empty = top.add(empty);
    splitter.add(hsplitter::Panel::Top, top);

    let mut bottom = Panel::new("Ordered timeline + streaming output", layout!("d:f"));
    bottom.add(Label::new(
        "Filter deployment / instance / service",
        layout!("l:1,t:1,w:38,h:1"),
    ));
    let filter = bottom.add(TextField::new(
        "",
        layout!("l:40,t:1,r:1,h:1"),
        textfield::Flags::None,
    ));
    let mut output = ListBox::new(layout!("l:1,t:3,r:1,b:1"), listbox::Flags::ScrollBars);
    fill_timeline(&mut output, &state.operation_log, "", Some(usize::MAX));
    let log = bottom.add(output);
    splitter.add(hsplitter::Panel::Bottom, bottom);
    tab.add(index, splitter);
    let notice = tab.add(
        index,
        Label::new(
            state.run_scripts_error.as_deref().unwrap_or(""),
            layout!("l:1,b:0,r:1,h:2"),
        ),
    );
    Handles {
        list,
        empty,
        filter,
        log,
        notice,
    }
}

pub(crate) fn script_rows(scripts: &[RunScript]) -> Vec<ScriptRowView> {
    scripts
        .iter()
        .enumerate()
        .map(|(index, script)| ScriptRowView {
            script_index: index,
            name: script.name.clone(),
            kind: if script.shell.is_some() {
                "shell"
            } else {
                "structured"
            }
            .into(),
            action: script
                .command
                .map_or_else(|| "shell".into(), |command| command.as_str().into()),
            description: script.description.clone().unwrap_or_else(|| "—".into()),
        })
        .collect()
}

pub(crate) fn fill_list(
    list: &mut ListView<ScriptRowView>,
    scripts: &[RunScript],
    _preferred: Option<usize>,
) {
    list.clear();
    for row in script_rows(scripts) {
        list.add(row);
    }
}

pub(crate) fn render_timeline(log: &OperationLog, filter: &str) -> String {
    let needle = filter.trim().to_ascii_lowercase();
    let mut rendered = Vec::new();
    for (index, entry) in log.entries().iter().enumerate() {
        let searchable = std::iter::once(entry.label.as_str())
            .chain(entry.deployment.as_deref())
            .chain(entry.lines.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join("\n")
            .to_ascii_lowercase();
        if !needle.is_empty() && !searchable.contains(&needle) {
            continue;
        }
        let destructive = if entry.destructive {
            " DESTRUCTIVE"
        } else {
            ""
        };
        let deployment = entry
            .deployment
            .as_deref()
            .map_or_else(String::new, |name| format!(" — deployment {name}"));
        let outcome = match &entry.outcome {
            OperationOutcome::Running => "RUNNING".into(),
            OperationOutcome::Finished(code) => format!("EXIT {code}"),
            OperationOutcome::Failed(error) => format!("FAILED: {error}"),
        };
        rendered.push(format!(
            "{:03} | {}{}{} | {}",
            index + 1,
            entry.label,
            destructive,
            deployment,
            outcome
        ));
        rendered.extend(entry.lines.iter().map(|line| format!("      {line}")));
    }
    rendered.join("\n")
}

pub(crate) fn fill_timeline(
    list: &mut ListBox,
    log: &OperationLog,
    filter: &str,
    selected: Option<usize>,
) {
    let rendered = render_timeline(log, filter);
    list.clear();
    list.set_empty_message(if filter.trim().is_empty() {
        "No operations have run in this session. Use Enter here or F7–F10 from Instances."
    } else {
        "No timeline entries match this filter."
    });
    for line in rendered.lines() {
        list.add(line);
    }
    if let Some(index) = selected.filter(|_| list.count() > 0) {
        list.set_index(index.min(list.count() - 1));
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScriptMode {
    Structured,
    Shell,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ScriptForm {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) mode: ScriptMode,
    pub(crate) command: StructuredCommand,
    pub(crate) overlays: String,
    pub(crate) variation: String,
    pub(crate) set: String,
    pub(crate) shell: String,
}

impl Default for ScriptForm {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            mode: ScriptMode::Structured,
            command: StructuredCommand::Up,
            overlays: String::new(),
            variation: String::new(),
            set: String::new(),
            shell: String::new(),
        }
    }
}

impl ScriptForm {
    pub(crate) fn from_script(script: &RunScript) -> Self {
        Self {
            name: script.name.clone(),
            description: script.description.clone().unwrap_or_default(),
            mode: if script.shell.is_some() {
                ScriptMode::Shell
            } else {
                ScriptMode::Structured
            },
            command: script.command.unwrap_or(StructuredCommand::Up),
            overlays: script.overlays.join(", "),
            variation: script.variation.clone().unwrap_or_default(),
            set: script.set.join(", "),
            shell: script.shell.clone().unwrap_or_default(),
        }
    }
    pub(crate) fn script(&self) -> Result<RunScript, String> {
        let nonempty = |value: &str| (!value.trim().is_empty()).then(|| value.trim().to_owned());
        let split = |value: &str| {
            value
                .split(',')
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
                .collect()
        };
        let mut script = RunScript {
            name: self.name.trim().into(),
            description: nonempty(&self.description),
            command: None,
            overlays: Vec::new(),
            variation: None,
            set: Vec::new(),
            shell: None,
        };
        match self.mode {
            ScriptMode::Structured => {
                script.command = Some(self.command);
                script.overlays = split(&self.overlays);
                script.variation = nonempty(&self.variation);
                script.set = split(&self.set);
            }
            ScriptMode::Shell => script.shell = nonempty(&self.shell),
        }
        script.validate()?;
        Ok(script)
    }
}

#[ModalWindow(events = ButtonEvents + ComboBoxEvents + WindowEvents, response = RunScript)]
pub(crate) struct ScriptDialog {
    name: Handle<TextField>,
    description: Handle<TextField>,
    mode: Handle<ComboBox>,
    command: Handle<ComboBox>,
    overlays: Handle<TextField>,
    variation: Handle<TextField>,
    set: Handle<TextField>,
    shell: Handle<TextField>,
    structured_labels: Vec<Handle<Label>>,
    error: Handle<Label>,
    save: Handle<Button>,
    cancel: Handle<Button>,
    reserved_names: Vec<String>,
}

impl ScriptDialog {
    pub(crate) fn new(script: Option<&RunScript>, reserved_names: Vec<String>) -> Self {
        let form = script.map_or_else(ScriptForm::default, ScriptForm::from_script);
        let mut dialog = Self {
            base: ModalWindow::new(
                if script.is_some() {
                    "Edit project run action"
                } else {
                    "New project run action"
                },
                layout!("a:c,w:82,h:29"),
                window::Flags::None,
            ),
            name: Handle::None,
            description: Handle::None,
            mode: Handle::None,
            command: Handle::None,
            overlays: Handle::None,
            variation: Handle::None,
            set: Handle::None,
            shell: Handle::None,
            structured_labels: Vec::new(),
            error: Handle::None,
            save: Handle::None,
            cancel: Handle::None,
            reserved_names,
        };
        dialog.add(Label::new("Name", layout!("l:2,t:1,w:18,h:1")));
        dialog.name = dialog.add(TextField::new(
            &form.name,
            layout!("l:22,t:1,r:2,h:1"),
            textfield::Flags::None,
        ));
        dialog.add(Label::new("Description", layout!("l:2,t:4,w:18,h:1")));
        dialog.description = dialog.add(TextField::new(
            &form.description,
            layout!("l:22,t:4,r:2,h:1"),
            textfield::Flags::None,
        ));
        dialog.add(Label::new("Mode", layout!("l:2,t:7,w:18,h:1")));
        let mut mode = ComboBox::new(layout!("l:22,t:7,r:2,h:1"), combobox::Flags::None);
        mode.add("Structured Switchyard command");
        mode.add("Shell command");
        mode.set_index(u32::from(form.mode == ScriptMode::Shell));
        dialog.mode = dialog.add(mode);
        let command_label = dialog.add(Label::new("Command", layout!("l:2,t:10,w:18,h:1")));
        dialog.structured_labels.push(command_label);
        let mut command = ComboBox::new(layout!("l:22,t:10,r:2,h:1"), combobox::Flags::None);
        for value in ["up", "down", "plan", "status"] {
            command.add(value);
        }
        command.set_index(match form.command {
            StructuredCommand::Up => 0,
            StructuredCommand::Down => 1,
            StructuredCommand::Plan => 2,
            StructuredCommand::Status => 3,
        });
        dialog.command = dialog.add(command);
        let overlays_label = dialog.add(Label::new(
            "Overlays (comma-separated)",
            layout!("l:2,t:13,w:20,h:1"),
        ));
        let variation_label = dialog.add(Label::new("Variation", layout!("l:2,t:16,w:20,h:1")));
        let set_label = dialog.add(Label::new(
            "Set (KEY=VALUE, comma)",
            layout!("l:2,t:19,w:20,h:1"),
        ));
        dialog
            .structured_labels
            .extend([overlays_label, variation_label, set_label]);
        dialog.overlays = dialog.add(TextField::new(
            &form.overlays,
            layout!("l:22,t:13,r:2,h:1"),
            textfield::Flags::None,
        ));
        dialog.variation = dialog.add(TextField::new(
            &form.variation,
            layout!("l:22,t:16,r:2,h:1"),
            textfield::Flags::None,
        ));
        dialog.set = dialog.add(TextField::new(
            &form.set,
            layout!("l:22,t:19,r:2,h:1"),
            textfield::Flags::None,
        ));
        let shell_label = dialog.add(Label::new("Shell command", layout!("l:2,t:10,w:18,h:1")));
        dialog.structured_labels.push(shell_label);
        dialog.shell = dialog.add(TextField::new(
            &form.shell,
            layout!("l:22,t:10,r:2,h:1"),
            textfield::Flags::None,
        ));
        dialog.error = dialog.add(Label::new("", layout!("l:22,t:22,r:2,h:2")));
        dialog.save = dialog.add(Button::new("&Save", layout!("x:40%,y:100%,p:b,w:14,h:1")));
        dialog.cancel = dialog.add(Button::new("&Cancel", layout!("x:62%,y:100%,p:b,w:14,h:1")));
        dialog.update_mode();
        dialog
    }
    fn is_shell(&self) -> bool {
        self.control(self.mode).and_then(ComboBox::index) == Some(1)
    }
    fn update_mode(&mut self) {
        let shell = self.is_shell();
        let command = self.command;
        if let Some(control) = self.control_mut(command) {
            control.set_visible(!shell);
        }
        for handle in [self.overlays, self.variation, self.set] {
            if let Some(control) = self.control_mut(handle) {
                control.set_visible(!shell);
            }
        }
        let shell_handle = self.shell;
        if let Some(control) = self.control_mut(shell_handle) {
            control.set_visible(shell);
        }
        for (index, handle) in self.structured_labels.clone().into_iter().enumerate() {
            if let Some(label) = self.control_mut(handle) {
                label.set_visible(if index == 4 { shell } else { !shell });
            }
        }
    }
    fn form(&self) -> ScriptForm {
        let text = |handle| self.control(handle).map_or("", TextField::text).to_owned();
        let command = match self
            .control(self.command)
            .and_then(ComboBox::index)
            .unwrap_or(0)
        {
            1 => StructuredCommand::Down,
            2 => StructuredCommand::Plan,
            3 => StructuredCommand::Status,
            _ => StructuredCommand::Up,
        };
        ScriptForm {
            name: text(self.name),
            description: text(self.description),
            mode: if self.is_shell() {
                ScriptMode::Shell
            } else {
                ScriptMode::Structured
            },
            command,
            overlays: text(self.overlays),
            variation: text(self.variation),
            set: text(self.set),
            shell: text(self.shell),
        }
    }
    fn submit(&mut self) {
        match self.form().script() {
            Ok(script) if !self.reserved_names.contains(&script.name) => self.exit_with(script),
            Ok(script) => {
                let error_handle = self.error;
                if let Some(label) = self.control_mut(error_handle) {
                    label.set_caption(&format!(
                        "Validation: run action name `{}` already exists",
                        script.name
                    ));
                }
            }
            Err(error) => {
                let error_handle = self.error;
                if let Some(label) = self.control_mut(error_handle) {
                    label.set_caption(&format!("Validation: {error}"));
                }
            }
        }
    }
}
impl ComboBoxEvents for ScriptDialog {
    fn on_selection_changed(&mut self, handle: Handle<ComboBox>) -> EventProcessStatus {
        if handle == self.mode {
            self.update_mode();
            EventProcessStatus::Processed
        } else {
            EventProcessStatus::Ignored
        }
    }
}
impl ButtonEvents for ScriptDialog {
    fn on_pressed(&mut self, handle: Handle<Button>) -> EventProcessStatus {
        if handle == self.save {
            self.submit();
        } else if handle == self.cancel {
            self.exit();
        } else {
            return EventProcessStatus::Ignored;
        }
        EventProcessStatus::Processed
    }
}
impl WindowEvents for ScriptDialog {
    fn on_accept(&mut self) {
        self.submit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{OperationLog, OperationOutcome};

    #[test]
    fn run_script_form_uses_ops_validation() {
        let mut form = ScriptForm {
            name: "smoke".into(),
            set: "PORT=9000, broken".into(),
            ..Default::default()
        };
        assert!(form.script().unwrap_err().contains("KEY=VALUE"));
        form.set = "PORT=9000".into();
        assert_eq!(form.script().unwrap().command, Some(StructuredCommand::Up));
        form.mode = ScriptMode::Shell;
        form.shell = "true".into();
        assert!(form.script().unwrap().command.is_none());
    }

    #[test]
    fn timeline_filter_matches_deployment_instance_and_service_output() {
        let mut log = OperationLog::default();
        log.start("readiness", Some("demo".into()), false);
        log.append("instance frontend service api ready");
        log.finish(OperationOutcome::Finished(0));
        log.start("cleanup", Some("other".into()), true);
        log.finish(OperationOutcome::Finished(0));
        assert!(render_timeline(&log, "frontend").contains("readiness"));
        assert!(!render_timeline(&log, "frontend").contains("cleanup"));
        assert!(render_timeline(&log, "other").contains("DESTRUCTIVE"));
    }
}
