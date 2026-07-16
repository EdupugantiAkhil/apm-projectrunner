use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde::Deserialize;
use switchyard_sources::RegisteredSourceInspection;
use switchyard_state::{
    OwnedResourceObservation, RegisteredSourceKind, StateStore, StoredDeployment,
};

use crate::{
    execution::{self, OperationEvent, OperationSpec},
    run_scripts::{self, RunScript, StructuredCommand},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ActiveView {
    Sources,
    Instances,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct AddForm {
    pub(crate) name: String,
    pub(crate) local_path: String,
    pub(crate) git_url: String,
    pub(crate) git_ref: String,
    pub(crate) active_field: usize,
    pub(crate) error: Option<String>,
}

impl AddForm {
    fn active_value_mut(&mut self) -> &mut String {
        match self.active_field {
            0 => &mut self.name,
            1 => &mut self.local_path,
            2 => &mut self.git_url,
            _ => &mut self.git_ref,
        }
    }
    pub(crate) fn validate(&self) -> Result<AddRequest, String> {
        let name = self.name.trim();
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err("name may contain only ASCII letters, digits, '.', '-', and '_'".into());
        }
        let local_path = self.local_path.trim();
        let git_url = self.git_url.trim();
        match (local_path.is_empty(), git_url.is_empty()) {
            (true, true) => Err("enter either a local path or a git URL".into()),
            (false, false) => Err("enter a local path or a git URL, not both".into()),
            (false, true) if !self.git_ref.trim().is_empty() => {
                Err("git ref is only valid with a git URL".into())
            }
            (false, true) => Ok(AddRequest::Local {
                name: name.into(),
                path: PathBuf::from(local_path),
            }),
            (true, false) if git_url.starts_with('-') => {
                Err("git URL may not start with '-'".into())
            }
            (true, false) => Ok(AddRequest::Clone {
                name: name.into(),
                url: git_url.into(),
                git_ref: nonempty(self.git_ref.trim()),
            }),
        }
    }
}

fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AddRequest {
    Local {
        name: String,
        path: PathBuf,
    },
    Clone {
        name: String,
        url: String,
        git_ref: Option<String>,
    },
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
    pub(crate) active_field: usize,
    pub(crate) edit_index: Option<usize>,
    pub(crate) error: Option<String>,
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
            active_field: 0,
            edit_index: None,
            error: None,
        }
    }
}

impl ScriptForm {
    fn from_script(script: &RunScript, index: usize) -> Self {
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
            active_field: 0,
            edit_index: Some(index),
            error: None,
        }
    }
    fn field_count(&self) -> usize {
        if self.mode == ScriptMode::Structured {
            7
        } else {
            4
        }
    }
    fn active_value_mut(&mut self) -> Option<&mut String> {
        match (self.mode, self.active_field) {
            (_, 0) => Some(&mut self.name),
            (_, 1) => Some(&mut self.description),
            (ScriptMode::Structured, 4) => Some(&mut self.overlays),
            (ScriptMode::Structured, 5) => Some(&mut self.variation),
            (ScriptMode::Structured, 6) => Some(&mut self.set),
            (ScriptMode::Shell, 3) => Some(&mut self.shell),
            _ => None,
        }
    }
    fn script(&self) -> Result<RunScript, String> {
        let split = |value: &str| {
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        let mut script = RunScript {
            name: self.name.trim().into(),
            description: nonempty(self.description.trim()),
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
                script.variation = nonempty(self.variation.trim());
                script.set = split(&self.set);
            }
            ScriptMode::Shell => script.shell = nonempty(self.shell.trim()),
        }
        script.validate()?;
        Ok(script)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ServiceRow {
    pub(crate) instance: String,
    pub(crate) service: String,
    pub(crate) status: String,
    pub(crate) health: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DeploymentEntry {
    pub(crate) name: String,
    pub(crate) bundle: PathBuf,
    pub(crate) state: String,
    pub(crate) services: Vec<ServiceRow>,
    pub(crate) last_operation: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Overlay {
    None,
    Add(AddForm),
    ConfirmRemove {
        name: String,
        error: Option<String>,
    },
    Script(ScriptForm),
    ConfirmDown {
        deployment: String,
    },
    ConfirmDeleteScript {
        index: usize,
        name: String,
        error: Option<String>,
    },
    ShellNotice {
        name: String,
        spec: OperationSpec,
    },
    Help,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BusyKind {
    Add,
    Remove,
    Refresh,
    Operation,
}

enum TaskResult {
    Sources(Result<Vec<RegisteredSourceInspection>, String>, BusyKind),
    Operation(OperationEvent),
}

pub(crate) struct App {
    pub(crate) project_dir: PathBuf,
    pub(crate) active_view: ActiveView,
    pub(crate) sources: Vec<RegisteredSourceInspection>,
    pub(crate) selected: usize,
    pub(crate) deployments: Vec<DeploymentEntry>,
    pub(crate) deployment_selected: usize,
    pub(crate) scripts: Vec<RunScript>,
    pub(crate) script_selected: usize,
    pub(crate) scripts_error: Option<String>,
    pub(crate) output: Vec<String>,
    pub(crate) output_scroll: usize,
    pub(crate) last_exit_code: Option<i32>,
    pub(crate) overlay: Overlay,
    pub(crate) busy: Option<BusyKind>,
    pub(crate) spinner_tick: usize,
    pub(crate) status: Option<String>,
    shell_notice_shown: bool,
    quit: bool,
    task: Option<Receiver<TaskResult>>,
}

impl App {
    pub(crate) fn load(project_dir: PathBuf) -> Result<Self, Box<dyn Error>> {
        let sources = list_sources(&project_dir)?;
        let deployments = list_deployments(&project_dir).map_err(io_error)?;
        let (scripts, scripts_error) = run_scripts::load(&project_dir);
        Ok(Self::with_data(
            project_dir,
            sources,
            deployments,
            scripts,
            scripts_error,
        ))
    }

    #[cfg(test)]
    pub(crate) fn with_sources(
        project_dir: PathBuf,
        sources: Vec<RegisteredSourceInspection>,
    ) -> Self {
        Self::with_data(project_dir, sources, Vec::new(), Vec::new(), None)
    }

    fn with_data(
        project_dir: PathBuf,
        sources: Vec<RegisteredSourceInspection>,
        deployments: Vec<DeploymentEntry>,
        scripts: Vec<RunScript>,
        scripts_error: Option<String>,
    ) -> Self {
        let shell_notice_shown = run_scripts::shell_notice_acknowledged(&project_dir);
        Self {
            project_dir,
            active_view: ActiveView::Sources,
            sources,
            selected: 0,
            deployments,
            deployment_selected: 0,
            scripts,
            script_selected: 0,
            scripts_error,
            output: Vec::new(),
            output_scroll: 0,
            last_exit_code: None,
            overlay: Overlay::None,
            busy: None,
            spinner_tick: 0,
            status: None,
            shell_notice_shown,
            quit: false,
            task: None,
        }
    }

    pub(crate) const fn should_quit(&self) -> bool {
        self.quit
    }
    pub(crate) fn current_deployment(&self) -> Option<&DeploymentEntry> {
        self.deployments.get(self.deployment_selected)
    }

    pub(crate) fn handle_event(&mut self, event: Event) {
        if let Event::Key(key) = event {
            if key.kind == crossterm::event::KeyEventKind::Press {
                self.handle_key(key);
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.quit = true;
            return;
        }
        match &mut self.overlay {
            Overlay::Add(form) => match Self::handle_add_key(form, key) {
                Some(FormAction::Close) => self.overlay = Overlay::None,
                Some(FormAction::Submit) if self.busy.is_none() => match form.validate() {
                    Ok(request) => self.start_add(request),
                    Err(error) => form.error = Some(error),
                },
                _ => {}
            },
            Overlay::ConfirmRemove { .. } => self.handle_remove_confirm(key),
            Overlay::Script(form) => match Self::handle_script_key(form, key) {
                Some(FormAction::Close) => self.overlay = Overlay::None,
                Some(FormAction::Submit) => self.submit_script(),
                _ => {}
            },
            Overlay::ConfirmDown { .. } => self.handle_down_confirm(key),
            Overlay::ConfirmDeleteScript { .. } => self.handle_delete_confirm(key),
            Overlay::ShellNotice { .. } => self.handle_shell_notice(key),
            Overlay::Help => {
                if matches!(
                    key.code,
                    KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')
                ) {
                    self.overlay = Overlay::None;
                }
            }
            Overlay::None => self.handle_view_key(key),
        }
    }

    fn handle_view_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => self.overlay = Overlay::Help,
            KeyCode::Tab | KeyCode::Right | KeyCode::Left => {
                self.active_view = if self.active_view == ActiveView::Sources {
                    ActiveView::Instances
                } else {
                    ActiveView::Sources
                }
            }
            KeyCode::Down | KeyCode::Char('j') if self.active_view == ActiveView::Sources => {
                if !self.sources.is_empty() {
                    self.selected = (self.selected + 1).min(self.sources.len() - 1);
                }
            }
            KeyCode::Up | KeyCode::Char('k') if self.active_view == ActiveView::Sources => {
                self.selected = self.selected.saturating_sub(1)
            }
            KeyCode::Char('a')
                if self.active_view == ActiveView::Sources && self.busy.is_none() =>
            {
                self.overlay = Overlay::Add(AddForm::default())
            }
            KeyCode::Char('d')
                if self.active_view == ActiveView::Sources && self.busy.is_none() =>
            {
                if let Some(source) = self.sources.get(self.selected) {
                    self.overlay = Overlay::ConfirmRemove {
                        name: source.source.name.clone(),
                        error: None,
                    };
                }
            }
            KeyCode::Char('r')
                if self.active_view == ActiveView::Sources && self.busy.is_none() =>
            {
                self.start_refresh(BusyKind::Refresh)
            }
            KeyCode::Down | KeyCode::Char('j') if self.active_view == ActiveView::Instances => {
                if !self.scripts.is_empty() {
                    self.script_selected = (self.script_selected + 1).min(self.scripts.len() - 1);
                }
            }
            KeyCode::Up | KeyCode::Char('k') if self.active_view == ActiveView::Instances => {
                self.script_selected = self.script_selected.saturating_sub(1)
            }
            KeyCode::Char('[') if self.active_view == ActiveView::Instances => {
                self.deployment_selected = self.deployment_selected.saturating_sub(1)
            }
            KeyCode::Char(']') if self.active_view == ActiveView::Instances => {
                if !self.deployments.is_empty() {
                    self.deployment_selected =
                        (self.deployment_selected + 1).min(self.deployments.len() - 1);
                }
            }
            KeyCode::PageUp if self.active_view == ActiveView::Instances => {
                self.output_scroll = self
                    .output_scroll
                    .saturating_add(8)
                    .min(self.output.len().saturating_sub(1))
            }
            KeyCode::PageDown if self.active_view == ActiveView::Instances => {
                self.output_scroll = self.output_scroll.saturating_sub(8)
            }
            KeyCode::Char('n')
                if self.active_view == ActiveView::Instances && self.busy.is_none() =>
            {
                self.overlay = Overlay::Script(ScriptForm::default())
            }
            KeyCode::Char('e')
                if self.active_view == ActiveView::Instances && self.busy.is_none() =>
            {
                if let Some(script) = self.scripts.get(self.script_selected) {
                    self.overlay =
                        Overlay::Script(ScriptForm::from_script(script, self.script_selected));
                }
            }
            KeyCode::Char('D')
                if self.active_view == ActiveView::Instances && self.busy.is_none() =>
            {
                if let Some(script) = self.scripts.get(self.script_selected) {
                    self.overlay = Overlay::ConfirmDeleteScript {
                        index: self.script_selected,
                        name: script.name.clone(),
                        error: None,
                    };
                }
            }
            KeyCode::Enter if self.active_view == ActiveView::Instances && self.busy.is_none() => {
                self.run_selected_script()
            }
            KeyCode::Char('u')
                if self.active_view == ActiveView::Instances && self.busy.is_none() =>
            {
                self.start_direct(StructuredCommand::Up)
            }
            KeyCode::Char('s')
                if self.active_view == ActiveView::Instances && self.busy.is_none() =>
            {
                self.start_direct(StructuredCommand::Status)
            }
            KeyCode::Char('p')
                if self.active_view == ActiveView::Instances && self.busy.is_none() =>
            {
                self.start_direct(StructuredCommand::Plan)
            }
            KeyCode::Char('x')
                if self.active_view == ActiveView::Instances && self.busy.is_none() =>
            {
                if let Some(deployment) = self.current_deployment() {
                    self.overlay = Overlay::ConfirmDown {
                        deployment: deployment.name.clone(),
                    };
                }
            }
            _ => {}
        }
    }

    fn handle_add_key(form: &mut AddForm, key: KeyEvent) -> Option<FormAction> {
        match key.code {
            KeyCode::Esc => return Some(FormAction::Close),
            KeyCode::Tab | KeyCode::Down => form.active_field = (form.active_field + 1) % 4,
            KeyCode::BackTab | KeyCode::Up => form.active_field = (form.active_field + 3) % 4,
            KeyCode::Backspace => {
                form.active_value_mut().pop();
                form.error = None;
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                form.active_value_mut().push(character);
                form.error = None;
            }
            KeyCode::Enter => return Some(FormAction::Submit),
            _ => {}
        }
        None
    }

    fn handle_script_key(form: &mut ScriptForm, key: KeyEvent) -> Option<FormAction> {
        match key.code {
            KeyCode::Esc => return Some(FormAction::Close),
            KeyCode::Tab | KeyCode::Down => {
                form.active_field = (form.active_field + 1) % form.field_count()
            }
            KeyCode::BackTab | KeyCode::Up => {
                form.active_field =
                    (form.active_field + form.field_count() - 1) % form.field_count()
            }
            KeyCode::Backspace => {
                if let Some(value) = form.active_value_mut() {
                    value.pop();
                    form.error = None;
                }
            }
            KeyCode::Char(' ') if form.active_field == 2 => {
                form.mode = if form.mode == ScriptMode::Structured {
                    ScriptMode::Shell
                } else {
                    ScriptMode::Structured
                };
                form.active_field = 2;
                form.error = None;
            }
            KeyCode::Char(' ') if form.mode == ScriptMode::Structured && form.active_field == 3 => {
                form.command = form.command.next();
                form.error = None;
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                if let Some(value) = form.active_value_mut() {
                    value.push(character);
                    form.error = None;
                }
            }
            KeyCode::Enter => return Some(FormAction::Submit),
            _ => {}
        }
        None
    }

    fn submit_script(&mut self) {
        let (script, edit_index) = match &self.overlay {
            Overlay::Script(form) => match form.script() {
                Ok(script) => (script, form.edit_index),
                Err(error) => {
                    if let Overlay::Script(form) = &mut self.overlay {
                        form.error = Some(error);
                    }
                    return;
                }
            },
            _ => return,
        };
        if self
            .scripts
            .iter()
            .enumerate()
            .any(|(index, existing)| Some(index) != edit_index && existing.name == script.name)
        {
            if let Overlay::Script(form) = &mut self.overlay {
                form.error = Some(format!("script name `{}` already exists", script.name));
            }
            return;
        }
        let mut scripts = self.scripts.clone();
        let selected = if let Some(index) = edit_index {
            scripts[index] = script;
            index
        } else {
            scripts.push(script);
            scripts.len() - 1
        };
        match run_scripts::save(&self.project_dir, &scripts) {
            Ok(()) => {
                self.scripts = scripts;
                self.script_selected = selected;
                self.scripts_error = None;
                self.overlay = Overlay::None;
                self.status = Some("run scripts saved".into());
            }
            Err(error) => {
                if let Overlay::Script(form) = &mut self.overlay {
                    form.error = Some(format!("could not save scripts: {error}"));
                }
            }
        }
    }

    fn handle_remove_confirm(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('n') | KeyCode::Esc => self.overlay = Overlay::None,
            KeyCode::Char('y') if self.busy.is_none() => {
                let name = match &self.overlay {
                    Overlay::ConfirmRemove { name, .. } => name.clone(),
                    _ => return,
                };
                self.start_remove(name);
            }
            _ => {}
        }
    }
    fn handle_down_confirm(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('n') | KeyCode::Esc => self.overlay = Overlay::None,
            KeyCode::Char('y') if self.busy.is_none() => {
                self.overlay = Overlay::None;
                self.start_direct(StructuredCommand::Down);
            }
            _ => {}
        }
    }
    fn handle_delete_confirm(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('n') | KeyCode::Esc => self.overlay = Overlay::None,
            KeyCode::Char('y') => {
                let index = match self.overlay {
                    Overlay::ConfirmDeleteScript { index, .. } => index,
                    _ => return,
                };
                let mut scripts = self.scripts.clone();
                if index < scripts.len() {
                    scripts.remove(index);
                }
                match run_scripts::save(&self.project_dir, &scripts) {
                    Ok(()) => {
                        self.scripts = scripts;
                        self.script_selected = self
                            .script_selected
                            .min(self.scripts.len().saturating_sub(1));
                        self.overlay = Overlay::None;
                        self.status = Some("run script deleted".into());
                    }
                    Err(error) => {
                        if let Overlay::ConfirmDeleteScript { error: slot, .. } = &mut self.overlay
                        {
                            *slot = Some(error);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    fn handle_shell_notice(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('n') | KeyCode::Esc => self.overlay = Overlay::None,
            KeyCode::Char('y') => {
                let (name, spec) = match &self.overlay {
                    Overlay::ShellNotice { name, spec } => (name.clone(), spec.clone()),
                    _ => return,
                };
                self.shell_notice_shown = true;
                if let Err(error) = run_scripts::acknowledge_shell_notice(&self.project_dir) {
                    self.status = Some(format!("could not remember shell warning: {error}"));
                }
                self.overlay = Overlay::None;
                self.start_operation(name, spec);
            }
            _ => {}
        }
    }

    fn run_selected_script(&mut self) {
        let Some(script) = self.scripts.get(self.script_selected).cloned() else {
            return;
        };
        let bundle = self
            .current_deployment()
            .map(|entry| entry.bundle.clone())
            .unwrap_or_else(|| self.project_dir.join("deployment.yaml"));
        if script.command.is_some() && self.current_deployment().is_none() {
            self.status = Some("error: no deployment definition found".into());
            return;
        }
        match OperationSpec::from_script(&script, bundle) {
            Ok(spec @ OperationSpec::Shell(_)) if !self.shell_notice_shown => {
                self.overlay = Overlay::ShellNotice {
                    name: script.name,
                    spec,
                }
            }
            Ok(spec) => self.start_operation(script.name, spec),
            Err(error) => self.status = Some(format!("error: {error}")),
        }
    }
    fn start_direct(&mut self, command: StructuredCommand) {
        let Some(deployment) = self.current_deployment() else {
            self.status = Some("error: no deployment definition found".into());
            return;
        };
        let label = command.as_str().to_owned();
        let spec = OperationSpec::direct(command, deployment.bundle.clone());
        self.start_operation(label, spec);
    }
    fn start_operation(&mut self, label: String, spec: OperationSpec) {
        self.busy = Some(BusyKind::Operation);
        self.output.clear();
        self.output.push(format!("running {label}…"));
        self.output_scroll = 0;
        self.last_exit_code = None;
        let root = self.project_dir.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let (event_sender, event_receiver) = mpsc::channel();
            let forward = thread::spawn(move || {
                while let Ok(event) = event_receiver.recv() {
                    if sender.send(TaskResult::Operation(event)).is_err() {
                        break;
                    }
                }
            });
            execution::run(&root, spec, &event_sender);
            drop(event_sender);
            let _ = forward.join();
        });
        self.task = Some(receiver);
    }

    pub(crate) fn tick(&mut self) {
        self.spinner_tick = self.spinner_tick.wrapping_add(1);
        loop {
            let result = self
                .task
                .as_ref()
                .and_then(|receiver| match receiver.try_recv() {
                    Ok(result) => Some(result),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => Some(TaskResult::Operation(
                        OperationEvent::Failed("background operation stopped unexpectedly".into()),
                    )),
                });
            let Some(result) = result else { break };
            match result {
                TaskResult::Sources(result, kind) => {
                    self.task = None;
                    self.busy = None;
                    match result {
                        Ok(sources) => {
                            self.sources = sources;
                            self.selected = self.selected.min(self.sources.len().saturating_sub(1));
                            self.overlay = Overlay::None;
                            self.status = Some(
                                match kind {
                                    BusyKind::Add => "source added",
                                    BusyKind::Remove => "source removed",
                                    _ => "sources refreshed",
                                }
                                .into(),
                            );
                        }
                        Err(error) => match (&mut self.overlay, kind) {
                            (Overlay::Add(form), BusyKind::Add) => form.error = Some(error),
                            (Overlay::ConfirmRemove { error: slot, .. }, BusyKind::Remove) => {
                                *slot = Some(error)
                            }
                            _ => self.status = Some(format!("error: {error}")),
                        },
                    }
                    break;
                }
                TaskResult::Operation(OperationEvent::Output(line)) => {
                    self.output.push(line);
                    if self.output_scroll > 0 {
                        self.output_scroll += 1;
                    }
                }
                TaskResult::Operation(OperationEvent::Finished { exit_code }) => {
                    self.output
                        .push(format!("operation completed with exit code {exit_code}"));
                    self.last_exit_code = Some(exit_code);
                    self.busy = None;
                    self.task = None;
                    self.refresh_deployments();
                    break;
                }
                TaskResult::Operation(OperationEvent::Failed(error)) => {
                    self.output.push(format!("error: {error}"));
                    self.last_exit_code = Some(1);
                    self.busy = None;
                    self.task = None;
                    self.refresh_deployments();
                    break;
                }
            }
        }
    }

    fn refresh_deployments(&mut self) {
        match list_deployments(&self.project_dir) {
            Ok(entries) => {
                self.deployments = entries;
                self.deployment_selected = self
                    .deployment_selected
                    .min(self.deployments.len().saturating_sub(1));
            }
            Err(error) => self.status = Some(format!("state refresh failed: {error}")),
        }
    }
    fn start_add(&mut self, request: AddRequest) {
        self.busy = Some(BusyKind::Add);
        self.spawn_sources(BusyKind::Add, move |root| {
            let store = open_store(&root)?;
            let manager = switchyard_sources::SourceManager::new(&root);
            match request {
                AddRequest::Local { name, path } => {
                    let path = if path.is_absolute() {
                        path
                    } else {
                        root.join(path)
                    };
                    manager
                        .register_unmanaged(&store, &name, &path)
                        .map_err(|error| error.to_string())?;
                }
                AddRequest::Clone { name, url, git_ref } => {
                    manager
                        .create_clone_from_url(&store, &url, &name, git_ref.as_deref())
                        .map_err(|error| error.to_string())?;
                }
            }
            manager.list(&store).map_err(|error| error.to_string())
        });
    }
    fn start_remove(&mut self, name: String) {
        self.busy = Some(BusyKind::Remove);
        self.spawn_sources(BusyKind::Remove, move |root| {
            let store = open_store(&root)?;
            let manager = switchyard_sources::SourceManager::new(&root);
            let source = store
                .source(&name)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("source `{name}` is not registered"))?;
            if source.kind == RegisteredSourceKind::Managed {
                manager
                    .remove(&store, &name, false)
                    .map_err(|error| error.to_string())?;
            }
            manager
                .deregister(&store, &name)
                .map_err(|error| error.to_string())?;
            manager.list(&store).map_err(|error| error.to_string())
        });
    }
    fn start_refresh(&mut self, kind: BusyKind) {
        self.busy = Some(kind);
        self.spawn_sources(kind, |root| {
            list_sources(&root).map_err(|error| error.to_string())
        });
    }
    fn spawn_sources(
        &mut self,
        kind: BusyKind,
        operation: impl FnOnce(PathBuf) -> Result<Vec<RegisteredSourceInspection>, String>
        + Send
        + 'static,
    ) {
        let root = self.project_dir.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(TaskResult::Sources(operation(root), kind));
        });
        self.task = Some(receiver);
    }
}

#[derive(Clone, Copy)]
enum FormAction {
    Close,
    Submit,
}
fn io_error(error: String) -> std::io::Error {
    std::io::Error::other(error)
}
fn open_store(root: &Path) -> Result<StateStore, String> {
    StateStore::open(root.join(".switchyard/state.sqlite3"))
        .map(|value| value.0)
        .map_err(|error| error.to_string())
}
fn list_sources(root: &Path) -> Result<Vec<RegisteredSourceInspection>, Box<dyn Error>> {
    let store = StateStore::open(root.join(".switchyard/state.sqlite3"))?.0;
    Ok(switchyard_sources::SourceManager::new(root).list(&store)?)
}

#[derive(Deserialize)]
struct DefinitionHeader {
    kind: String,
    metadata: DefinitionMetadata,
}
#[derive(Deserialize)]
struct DefinitionMetadata {
    name: String,
}

fn definition_paths(root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let primary = root.join("deployment.yaml");
    if primary.is_file() {
        paths.push(primary);
    }
    for directory in [root.to_path_buf(), root.join("deployments")] {
        if let Ok(entries) = fs::read_dir(directory) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file()
                    && matches!(
                        path.extension().and_then(|value| value.to_str()),
                        Some("yaml" | "yml")
                    )
                    && !paths.contains(&path)
                {
                    paths.push(path);
                }
            }
        }
    }
    paths.sort();
    paths
}

fn list_deployments(root: &Path) -> Result<Vec<DeploymentEntry>, String> {
    let store = open_store(root)?;
    let stored = store.deployments().map_err(|error| error.to_string())?;
    let mut definitions = BTreeMap::new();
    for path in definition_paths(root) {
        if let Ok(contents) = fs::read_to_string(&path) {
            if let Ok(header) = serde_yaml::from_str::<DefinitionHeader>(&contents) {
                if header.kind == "Deployment" {
                    definitions.entry(header.metadata.name).or_insert(path);
                }
            }
        }
    }
    let mut names = stored
        .iter()
        .map(|entry| entry.deployment.clone())
        .collect::<BTreeSet<_>>();
    names.extend(definitions.keys().cloned());
    let by_name = stored
        .into_iter()
        .map(|entry| (entry.deployment.clone(), entry))
        .collect::<BTreeMap<_, _>>();
    names
        .into_iter()
        .map(|name| {
            let record = by_name.get(&name);
            let bundle = definitions
                .get(&name)
                .cloned()
                .unwrap_or_else(|| root.join("deployments").join(format!("{name}.yaml")));
            let resources = store
                .active_resources(&name)
                .map_err(|error| error.to_string())?;
            let manifest = load_manifest_services(root, &name);
            Ok(deployment_entry(
                name, bundle, record, &resources, &manifest,
            ))
        })
        .collect()
}

#[derive(Deserialize)]
struct ServiceManifest {
    #[serde(default)]
    services: Vec<ManifestService>,
}

#[derive(Deserialize)]
struct ManifestService {
    instance: String,
    component: String,
    service: String,
}

fn load_manifest_services(root: &Path, deployment: &str) -> Vec<ManifestService> {
    let path = root
        .join(".switchyard/generated")
        .join(deployment)
        .join("manifest.json");
    fs::read_to_string(path)
        .ok()
        .and_then(|contents| serde_yaml::from_str::<ServiceManifest>(&contents).ok())
        .map_or_else(Vec::new, |manifest| manifest.services)
}

fn deployment_entry(
    name: String,
    bundle: PathBuf,
    stored: Option<&StoredDeployment>,
    resources: &[OwnedResourceObservation],
    manifest: &[ManifestService],
) -> DeploymentEntry {
    let operation = stored.and_then(|entry| entry.last_operation.as_ref());
    let active_operation =
        operation.filter(|operation| matches!(operation.status.as_str(), "pending" | "running"));
    let state = if let Some(operation) = active_operation {
        format!("{} ({})", operation.status, operation.kind)
    } else if resources.is_empty()
        && stored
            .and_then(|entry| entry.definition_hash.as_ref())
            .is_some()
    {
        "stopped".into()
    } else if resources.is_empty() {
        "not applied".into()
    } else if resources.iter().any(|resource| {
        resource.state.as_deref().is_some_and(|state| {
            ["unhealthy", "exited", "dead", "failed"]
                .iter()
                .any(|bad| state.to_ascii_lowercase().contains(bad))
        })
    }) {
        "degraded".into()
    } else {
        "running".into()
    };
    let services = resource_rows(&name, resources, manifest);
    let last_operation =
        operation.map(|operation| format!("{} {}", operation.kind, operation.status));
    DeploymentEntry {
        name,
        bundle,
        state,
        services,
        last_operation,
    }
}

fn resource_rows(
    deployment: &str,
    resources: &[OwnedResourceObservation],
    manifest: &[ManifestService],
) -> Vec<ServiceRow> {
    let containers = resources
        .iter()
        .filter(|resource| matches!(resource.kind, switchyard_state::ResourceKind::Container))
        .collect::<Vec<_>>();
    let mut matched = BTreeSet::new();
    let mut rows = manifest
        .iter()
        .map(|service| {
            let resource = containers
                .iter()
                .enumerate()
                .find(|(_, resource)| resource.name.contains(&service.service));
            if let Some((index, _)) = resource {
                matched.insert(index);
            }
            let status = resource
                .and_then(|(_, resource)| resource.state.clone())
                .unwrap_or_else(|| "stopped".into());
            ServiceRow {
                instance: service.instance.clone(),
                service: service.component.clone(),
                health: health_label(&status).into(),
                status,
            }
        })
        .collect::<Vec<_>>();
    rows.extend(
        containers
            .into_iter()
            .enumerate()
            .filter(|(index, _)| !matched.contains(index))
            .map(|(_, resource)| {
                let logical = resource
                    .name
                    .trim_start_matches('/')
                    .strip_prefix(&format!("sy-{deployment}-"))
                    .unwrap_or(&resource.name);
                let (instance, service) = logical.split_once('-').unwrap_or((logical, "container"));
                let status = resource.state.clone().unwrap_or_else(|| "observed".into());
                ServiceRow {
                    instance: instance.into(),
                    service: service.into(),
                    health: health_label(&status).into(),
                    status,
                }
            }),
    );
    rows
}

fn health_label(status: &str) -> &'static str {
    if status.to_ascii_lowercase().contains("unhealthy") {
        "unhealthy"
    } else if status.to_ascii_lowercase().contains("healthy") {
        "healthy"
    } else {
        "-"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn add_form_requires_exactly_one_source_location() {
        let mut form = AddForm {
            name: "demo".into(),
            ..AddForm::default()
        };
        assert!(form.validate().is_err());
        form.local_path = "src".into();
        form.git_url = "https://example.invalid/repo.git".into();
        assert!(form.validate().is_err());
        form.git_url.clear();
        assert_eq!(
            form.validate().unwrap(),
            AddRequest::Local {
                name: "demo".into(),
                path: "src".into()
            }
        );
    }
    #[test]
    fn confirm_no_closes_without_starting_work() {
        let mut app = App::with_sources(PathBuf::from("."), Vec::new());
        app.overlay = Overlay::ConfirmRemove {
            name: "demo".into(),
            error: None,
        };
        app.handle_key(key(KeyCode::Char('n')));
        assert_eq!(app.overlay, Overlay::None);
        assert!(app.busy.is_none());
    }
    #[test]
    fn submitted_invalid_form_keeps_inline_error() {
        let mut app = App::with_sources(PathBuf::from("."), Vec::new());
        app.overlay = Overlay::Add(AddForm::default());
        app.handle_key(key(KeyCode::Enter));
        let Overlay::Add(form) = app.overlay else {
            panic!("add form unexpectedly closed")
        };
        assert!(form.error.unwrap().contains("name"));
    }
    #[test]
    fn new_script_modal_switches_mode_and_validates_unique_names() {
        let mut app = App::with_sources(PathBuf::from("."), Vec::new());
        app.active_view = ActiveView::Instances;
        app.scripts.push(RunScript {
            name: "smoke".into(),
            description: None,
            command: Some(StructuredCommand::Plan),
            overlays: Vec::new(),
            variation: None,
            set: Vec::new(),
            shell: None,
        });
        app.handle_key(key(KeyCode::Char('n')));
        let Overlay::Script(form) = &mut app.overlay else {
            panic!("script modal missing")
        };
        form.name = "smoke".into();
        form.active_field = 2;
        app.handle_key(key(KeyCode::Char(' ')));
        let Overlay::Script(form) = &mut app.overlay else {
            panic!()
        };
        assert_eq!(form.mode, ScriptMode::Shell);
        form.shell = "true".into();
        app.handle_key(key(KeyCode::Enter));
        let Overlay::Script(form) = app.overlay else {
            panic!()
        };
        assert!(form.error.unwrap().contains("already exists"));
    }
    #[test]
    fn down_confirmation_no_does_not_run() {
        let mut app = App::with_sources(PathBuf::from("."), Vec::new());
        app.overlay = Overlay::ConfirmDown {
            deployment: "demo".into(),
        };
        app.handle_key(key(KeyCode::Char('n')));
        assert_eq!(app.overlay, Overlay::None);
        assert!(app.busy.is_none());
    }
    #[test]
    fn delete_confirmation_removes_and_persists() {
        let root =
            std::env::temp_dir().join(format!("switchyard-tui-delete-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let mut app = App::with_sources(root.clone(), Vec::new());
        app.scripts.push(RunScript {
            name: "smoke".into(),
            description: None,
            command: Some(StructuredCommand::Plan),
            overlays: Vec::new(),
            variation: None,
            set: Vec::new(),
            shell: None,
        });
        app.overlay = Overlay::ConfirmDeleteScript {
            index: 0,
            name: "smoke".into(),
            error: None,
        };
        app.handle_key(key(KeyCode::Char('y')));
        assert!(app.scripts.is_empty());
        assert_eq!(run_scripts::load(&root), (Vec::new(), None));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn applied_deployment_without_observations_is_stopped() {
        let stored = StoredDeployment {
            deployment: "demo".into(),
            definition_hash: Some("definition".into()),
            snapshot_json: None,
            applied_at: Some(1),
            last_operation: None,
        };
        let entry = deployment_entry(
            "demo".into(),
            "deployment.yaml".into(),
            Some(&stored),
            &[],
            &[],
        );
        assert_eq!(entry.state, "stopped");
    }
}
