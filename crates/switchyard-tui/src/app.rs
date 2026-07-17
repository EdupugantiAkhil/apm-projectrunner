use std::{
    error::Error,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(test)]
use std::fs;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
pub(crate) use switchyard_ops::projections::{BindingRow, DeploymentEntry, SourceChoice};
use switchyard_ops::{
    execution::{self, OperationEvent, OperationSpec},
    instances::{CreateInstanceRequest, InstancePreview, create_instance, preview_instance},
    profiles::{
        ProfileListing, ProfileOrigin, ProfileRow, ProfileTrust, SourceManifestError,
        import_source_profile, list_profiles, load_profile_block, load_source_profile_block,
        remove_imported_profile,
    },
    projections::{list_deployments, list_devices, list_sources},
    run_scripts::{self, RunScript, StructuredCommand},
};
use switchyard_planner::{Block, Execution, Parameter, Probe};
use switchyard_sources::RegisteredSourceInspection;
use switchyard_state::{DeviceCheckStatus, RegisteredDevice, RegisteredSourceKind, StateStore};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ActiveView {
    Home,
    Sources,
    Profiles,
    Devices,
    Instances,
}

impl ActiveView {
    const fn next(self) -> Self {
        match self {
            Self::Home => Self::Sources,
            Self::Sources => Self::Profiles,
            Self::Profiles => Self::Devices,
            Self::Devices => Self::Instances,
            Self::Instances => Self::Home,
        }
    }

    const fn previous(self) -> Self {
        match self {
            Self::Home => Self::Instances,
            Self::Sources => Self::Home,
            Self::Profiles => Self::Sources,
            Self::Devices => Self::Profiles,
            Self::Instances => Self::Devices,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DeviceForm {
    pub(crate) name: String,
    pub(crate) user: String,
    pub(crate) host: String,
    pub(crate) port: String,
    pub(crate) identity_file: String,
    pub(crate) active_field: usize,
    pub(crate) error: Option<String>,
}

impl Default for DeviceForm {
    fn default() -> Self {
        Self {
            name: String::new(),
            user: String::new(),
            host: String::new(),
            port: "22".into(),
            identity_file: String::new(),
            active_field: 0,
            error: None,
        }
    }
}

impl DeviceForm {
    fn active_value_mut(&mut self) -> &mut String {
        match self.active_field {
            0 => &mut self.name,
            1 => &mut self.user,
            2 => &mut self.host,
            3 => &mut self.port,
            _ => &mut self.identity_file,
        }
    }

    pub(crate) fn device(&self) -> Result<RegisteredDevice, String> {
        let port = self
            .port
            .trim()
            .parse::<u16>()
            .map_err(|_| "port must be between 1 and 65535".to_owned())?;
        let device = RegisteredDevice {
            name: self.name.trim().into(),
            host: self.host.trim().into(),
            port,
            user: self.user.trim().into(),
            identity_file: nonempty(self.identity_file.trim()).map(PathBuf::from),
            created_at: unix_millis(),
            last_checked_at: None,
            last_check_status: DeviceCheckStatus::Never,
            last_check_detail: None,
        };
        validate_device_form(&device)?;
        Ok(device)
    }
}

fn validate_device_form(device: &RegisteredDevice) -> Result<(), String> {
    let valid_name = !device.name.is_empty()
        && device
            .name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    if !valid_name {
        return Err("name may contain only ASCII letters, digits, '.', '-', and '_'".into());
    }
    if device.user.is_empty()
        || device.user.chars().any(char::is_whitespace)
        || device.user.starts_with('-')
        || device.user.contains('@')
    {
        return Err("user cannot be empty, contain whitespace or '@', or start with '-'".into());
    }
    if device.host.is_empty()
        || device.host.chars().any(char::is_whitespace)
        || device.host.starts_with('-')
    {
        return Err("host cannot be empty, contain whitespace, or start with '-'".into());
    }
    if device
        .identity_file
        .as_ref()
        .is_some_and(|path| path.as_os_str().is_empty())
    {
        return Err("identity file path cannot be empty".into());
    }
    Ok(())
}

fn unix_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum AddSourceMode {
    #[default]
    Local,
    Git,
}

impl AddSourceMode {
    const fn toggled(self) -> Self {
        match self {
            Self::Local => Self::Git,
            Self::Git => Self::Local,
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Local => "Local path",
            Self::Git => "Git clone",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum AddSourcePanel {
    #[default]
    Location,
    GitOptions,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AddForm {
    pub(crate) mode: AddSourceMode,
    pub(crate) location: String,
    pub(crate) git_ref: String,
    pub(crate) panel: AddSourcePanel,
    pub(crate) active_field: usize,
    pub(crate) error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorktreeForm {
    pub(crate) source: String,
    pub(crate) base_ref: String,
    pub(crate) name: String,
    pub(crate) error: Option<String>,
}

impl WorktreeForm {
    fn active_value_mut(&mut self) -> &mut String {
        &mut self.name
    }

    fn validate(&self) -> Result<(), String> {
        let name = self.name.trim();
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err("name may contain only ASCII letters, digits, '.', '-', and '_'".into());
        }
        Ok(())
    }
}

impl Default for AddForm {
    fn default() -> Self {
        Self {
            mode: AddSourceMode::Local,
            location: String::new(),
            git_ref: String::new(),
            panel: AddSourcePanel::Location,
            active_field: 1,
            error: None,
        }
    }
}

impl AddForm {
    fn advanced_field_count(&self) -> usize {
        1
    }

    fn append_active(&mut self, value: &str) -> bool {
        match (self.panel, self.active_field) {
            (AddSourcePanel::Location, 1) => self.location.push_str(value),
            (AddSourcePanel::GitOptions, 0) => self.git_ref.push_str(value),
            _ => return false,
        }
        true
    }

    fn pop_active(&mut self) -> bool {
        match (self.panel, self.active_field) {
            (AddSourcePanel::Location, 1) => {
                self.location.pop();
            }
            (AddSourcePanel::GitOptions, 0) => {
                self.git_ref.pop();
            }
            _ => return false,
        }
        true
    }

    pub(crate) fn inferred_name(&self) -> String {
        infer_source_name(&self.location)
    }

    fn validate_location(&self) -> Result<(), String> {
        let location = self.location.trim();
        if location.is_empty() {
            return Err(match self.mode {
                AddSourceMode::Local => "enter an existing local directory",
                AddSourceMode::Git => "enter a Git HTTPS, SSH, or local repository URL",
            }
            .into());
        }
        match self.mode {
            AddSourceMode::Local => Ok(()),
            AddSourceMode::Git if location.starts_with('-') => {
                Err("Git clone address may not start with '-'".into())
            }
            AddSourceMode::Git if has_embedded_http_credentials(location) => Err(
                "do not embed HTTPS credentials in the clone address; use your Git credential helper"
                    .into(),
            ),
            AddSourceMode::Git => Ok(()),
        }
    }

    pub(crate) fn validate(&self) -> Result<AddRequest, String> {
        self.validate_location()?;
        let location = self.location.trim();
        let name = self.inferred_name();
        match self.mode {
            AddSourceMode::Local => Ok(AddRequest::Local {
                name,
                path: PathBuf::from(location),
            }),
            AddSourceMode::Git => Ok(AddRequest::Clone {
                name,
                url: location.into(),
                git_ref: nonempty(self.git_ref.trim()),
            }),
        }
    }
}

fn has_embedded_http_credentials(location: &str) -> bool {
    let lowercase = location.to_ascii_lowercase();
    ["https://", "http://"].iter().any(|scheme| {
        lowercase
            .strip_prefix(scheme)
            .and_then(|remainder| remainder.split('/').next())
            .is_some_and(|authority| authority.contains('@'))
    })
}

fn infer_source_name(location: &str) -> String {
    let trimmed = location.trim().trim_end_matches(['/', '\\']);
    let candidate = trimmed
        .rsplit(['/', '\\', ':'])
        .next()
        .unwrap_or(trimmed)
        .strip_suffix(".git")
        .unwrap_or_else(|| trimmed.rsplit(['/', '\\', ':']).next().unwrap_or(trimmed));
    let mut result = String::new();
    let mut previous_separator = false;
    for character in candidate.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
            result.push(character);
            previous_separator = false;
        } else if !result.is_empty() && !previous_separator {
            result.push('-');
            previous_separator = true;
        }
    }
    while result.ends_with('-') {
        result.pop();
    }
    if result.is_empty() || matches!(result.as_str(), "." | "..") {
        "source".into()
    } else {
        result
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
pub(crate) struct InstanceParameterField {
    pub(crate) name: String,
    pub(crate) value: String,
    pub(crate) required: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InstanceForm {
    pub(crate) name: String,
    pub(crate) profile_index: usize,
    pub(crate) source_index: usize,
    pub(crate) device_index: usize,
    pub(crate) parameters: Vec<InstanceParameterField>,
    pub(crate) active_field: usize,
    pub(crate) field_errors: std::collections::BTreeMap<String, String>,
    pub(crate) preview: Option<InstancePreview>,
    pub(crate) error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PairForm {
    pub(crate) consumer_index: usize,
    pub(crate) group_index: usize,
    pub(crate) error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Overlay {
    None,
    Add(AddForm),
    Worktree(WorktreeForm),
    Device(DeviceForm),
    ConfirmRemoveDevice {
        name: String,
        error: Option<String>,
    },
    ConfirmRemove {
        name: String,
        error: Option<String>,
    },
    Script(ScriptForm),
    Instance(InstanceForm),
    Pair(PairForm),
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
    ProfileInspector {
        name: String,
        lines: Vec<String>,
        scroll: usize,
    },
    ProfileReview {
        name: String,
        source: String,
        yaml: String,
        scroll: usize,
        error: Option<String>,
    },
    ConfirmRemoveProfile {
        name: String,
        error: Option<String>,
    },
    Help,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BusyKind {
    Add,
    WorktreeAdd,
    Remove,
    Refresh,
    DeviceAdd,
    DeviceRemove,
    DeviceCheck,
    ProfileRefresh,
    ProfileInspect,
    ProfileReview,
    ProfileImport,
    ProfileRemove,
    Operation,
}

enum TaskResult {
    Sources(Result<Vec<RegisteredSourceInspection>, String>, BusyKind),
    Devices(Result<Vec<RegisteredDevice>, String>, BusyKind),
    Profiles(Result<ProfileListing, String>, BusyKind),
    ProfileDialog(Result<ProfileDialog, String>, BusyKind),
    Operation(OperationEvent),
}

struct ProfileDialog {
    name: String,
    source: Option<String>,
    lines: Vec<String>,
    yaml: String,
}

pub(crate) struct App {
    pub(crate) project_dir: PathBuf,
    pub(crate) active_view: ActiveView,
    pub(crate) home_selected: usize,
    pub(crate) sources: Vec<RegisteredSourceInspection>,
    pub(crate) selected: usize,
    pub(crate) devices: Vec<RegisteredDevice>,
    pub(crate) device_selected: usize,
    pub(crate) profiles: Vec<ProfileRow>,
    pub(crate) profile_selected: usize,
    pub(crate) profile_source_errors: Vec<SourceManifestError>,
    pub(crate) deployments: Vec<DeploymentEntry>,
    pub(crate) deployment_selected: usize,
    pub(crate) scripts: Vec<RunScript>,
    pub(crate) script_selected: usize,
    pub(crate) binding_selected: usize,
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
    pending_clone: Option<AddRequest>,
}

impl App {
    pub(crate) fn load(project_dir: PathBuf) -> Result<Self, Box<dyn Error>> {
        let sources = list_sources(&project_dir)?;
        let devices = list_devices(&project_dir).map_err(io_error)?;
        let deployments = list_deployments(&project_dir, &sources).map_err(io_error)?;
        let (scripts, scripts_error) = run_scripts::load(&project_dir);
        let mut app = Self::with_data(
            project_dir,
            sources,
            devices,
            deployments,
            scripts,
            scripts_error,
        );
        app.start_profile_refresh();
        Ok(app)
    }

    #[cfg(test)]
    pub(crate) fn with_sources(
        project_dir: PathBuf,
        sources: Vec<RegisteredSourceInspection>,
    ) -> Self {
        Self::with_data(
            project_dir,
            sources,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
        )
    }

    fn with_data(
        project_dir: PathBuf,
        sources: Vec<RegisteredSourceInspection>,
        devices: Vec<RegisteredDevice>,
        deployments: Vec<DeploymentEntry>,
        scripts: Vec<RunScript>,
        scripts_error: Option<String>,
    ) -> Self {
        let shell_notice_shown = run_scripts::shell_notice_acknowledged(&project_dir);
        Self {
            project_dir,
            active_view: ActiveView::Home,
            home_selected: 0,
            sources,
            selected: 0,
            devices,
            device_selected: 0,
            profiles: Vec::new(),
            profile_selected: 0,
            profile_source_errors: Vec::new(),
            deployments,
            deployment_selected: 0,
            scripts,
            script_selected: 0,
            binding_selected: 0,
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
            pending_clone: None,
        }
    }

    pub(crate) const fn should_quit(&self) -> bool {
        self.quit
    }

    pub(crate) fn take_pending_clone(&mut self) -> Option<AddRequest> {
        self.pending_clone.take()
    }

    pub(crate) fn finish_interactive_clone(
        &mut self,
        result: Result<Vec<RegisteredSourceInspection>, String>,
    ) {
        match result {
            Ok(sources) => {
                self.sources = sources;
                self.selected = self.selected.min(self.sources.len().saturating_sub(1));
                self.overlay = Overlay::None;
                self.status = Some("source added".into());
                self.refresh_deployments();
            }
            Err(error) => {
                if let Overlay::Add(form) = &mut self.overlay {
                    form.error = Some(error);
                } else {
                    self.status = Some(format!("error: {error}"));
                }
            }
        }
    }
    pub(crate) fn current_deployment(&self) -> Option<&DeploymentEntry> {
        self.deployments.get(self.deployment_selected)
    }

    pub(crate) fn handle_event(&mut self, event: Event) {
        match event {
            Event::Key(key) if key.kind == crossterm::event::KeyEventKind::Press => {
                self.handle_key(key)
            }
            Event::Paste(value) => {
                let value = value.trim_end_matches(['\r', '\n']);
                match &mut self.overlay {
                    Overlay::Add(form) => {
                        if form.append_active(value) {
                            form.error = None;
                        }
                    }
                    Overlay::Worktree(form) => {
                        form.active_value_mut().push_str(value);
                        form.error = None;
                    }
                    _ => {}
                }
            }
            _ => {}
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
                    Ok(request @ AddRequest::Clone { .. }) => {
                        self.pending_clone = Some(request);
                    }
                    Ok(request) => self.start_add(request),
                    Err(error) => form.error = Some(error),
                },
                _ => {}
            },
            Overlay::Worktree(form) => match Self::handle_worktree_key(form, key) {
                Some(FormAction::Close) => self.overlay = Overlay::None,
                Some(FormAction::Submit) if self.busy.is_none() => {
                    if let Err(error) = form.validate() {
                        form.error = Some(error);
                    } else {
                        let request = (
                            form.source.clone(),
                            form.base_ref.clone(),
                            form.name.trim().to_owned(),
                        );
                        self.start_worktree_add(request.0, request.1, request.2);
                    }
                }
                _ => {}
            },
            Overlay::Device(form) => match Self::handle_device_key(form, key) {
                Some(FormAction::Close) => self.overlay = Overlay::None,
                Some(FormAction::Submit) if self.busy.is_none() => match form.device() {
                    Ok(device) => self.start_device_add(device),
                    Err(error) => form.error = Some(error),
                },
                _ => {}
            },
            Overlay::ConfirmRemoveDevice { .. } => self.handle_device_remove_confirm(key),
            Overlay::ConfirmRemove { .. } => self.handle_remove_confirm(key),
            Overlay::Script(form) => match Self::handle_script_key(form, key) {
                Some(FormAction::Close) => self.overlay = Overlay::None,
                Some(FormAction::Submit) => self.submit_script(),
                _ => {}
            },
            Overlay::Instance(_) => self.handle_instance_key(key),
            Overlay::Pair(_) => self.handle_pair_key(key),
            Overlay::ConfirmDown { .. } => self.handle_down_confirm(key),
            Overlay::ConfirmDeleteScript { .. } => self.handle_delete_confirm(key),
            Overlay::ShellNotice { .. } => self.handle_shell_notice(key),
            Overlay::ProfileInspector { scroll, lines, .. } => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.overlay = Overlay::None,
                KeyCode::Down | KeyCode::Char('j') | KeyCode::PageDown => {
                    *scroll = (*scroll + 1).min(lines.len().saturating_sub(1));
                }
                KeyCode::Up | KeyCode::Char('k') | KeyCode::PageUp => {
                    *scroll = scroll.saturating_sub(1);
                }
                _ => {}
            },
            Overlay::ProfileReview { .. } => self.handle_profile_review(key),
            Overlay::ConfirmRemoveProfile { .. } => self.handle_profile_remove_confirm(key),
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
            KeyCode::Tab | KeyCode::Right => self.active_view = self.active_view.next(),
            KeyCode::BackTab | KeyCode::Left => self.active_view = self.active_view.previous(),
            KeyCode::Down | KeyCode::Char('j') if self.active_view == ActiveView::Home => {
                self.home_selected = (self.home_selected + 1).min(4);
            }
            KeyCode::Up | KeyCode::Char('k') if self.active_view == ActiveView::Home => {
                self.home_selected = self.home_selected.saturating_sub(1);
            }
            KeyCode::Enter if self.active_view == ActiveView::Home => {
                self.active_view = home_destination(self.home_selected);
            }
            KeyCode::Char(number @ '1'..='5') if self.active_view == ActiveView::Home => {
                self.home_selected = (number as usize) - ('1' as usize);
                self.active_view = home_destination(self.home_selected);
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
            KeyCode::Char('w')
                if self.active_view == ActiveView::Sources && self.busy.is_none() =>
            {
                if let Some(source) = self.sources.get(self.selected) {
                    if let Some(base_ref) = source.inspection.identity.commit.clone() {
                        self.overlay = Overlay::Worktree(WorktreeForm {
                            source: source.source.name.clone(),
                            base_ref,
                            name: String::new(),
                            error: None,
                        });
                    } else {
                        self.status = Some(
                            "select a Git repository or linked worktree with a known HEAD before pressing w"
                                .into(),
                        );
                    }
                }
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
            KeyCode::Down | KeyCode::Char('j') if self.active_view == ActiveView::Profiles => {
                if !self.profiles.is_empty() {
                    self.profile_selected =
                        (self.profile_selected + 1).min(self.profiles.len() - 1);
                }
            }
            KeyCode::Up | KeyCode::Char('k') if self.active_view == ActiveView::Profiles => {
                self.profile_selected = self.profile_selected.saturating_sub(1)
            }
            KeyCode::Enter if self.active_view == ActiveView::Profiles && self.busy.is_none() => {
                self.start_profile_inspect()
            }
            KeyCode::Char('i')
                if self.active_view == ActiveView::Profiles && self.busy.is_none() =>
            {
                self.start_profile_review()
            }
            KeyCode::Char('d')
                if self.active_view == ActiveView::Profiles && self.busy.is_none() =>
            {
                self.open_profile_remove()
            }
            KeyCode::Char('r')
                if self.active_view == ActiveView::Profiles && self.busy.is_none() =>
            {
                self.start_profile_refresh()
            }
            KeyCode::Down | KeyCode::Char('j') if self.active_view == ActiveView::Devices => {
                if !self.devices.is_empty() {
                    self.device_selected = (self.device_selected + 1).min(self.devices.len() - 1);
                }
            }
            KeyCode::Up | KeyCode::Char('k') if self.active_view == ActiveView::Devices => {
                self.device_selected = self.device_selected.saturating_sub(1)
            }
            KeyCode::Char('a')
                if self.active_view == ActiveView::Devices && self.busy.is_none() =>
            {
                self.overlay = Overlay::Device(DeviceForm::default())
            }
            KeyCode::Char('c')
                if self.active_view == ActiveView::Devices && self.busy.is_none() =>
            {
                if let Some(device) = self.devices.get(self.device_selected) {
                    self.start_device_check(device.name.clone());
                }
            }
            KeyCode::Char('d')
                if self.active_view == ActiveView::Devices && self.busy.is_none() =>
            {
                if let Some(device) = self.devices.get(self.device_selected) {
                    self.overlay = Overlay::ConfirmRemoveDevice {
                        name: device.name.clone(),
                        error: None,
                    };
                }
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
            KeyCode::Char('i')
                if self.active_view == ActiveView::Instances && self.busy.is_none() =>
            {
                self.open_instance_form();
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
            KeyCode::Char('b')
                if self.active_view == ActiveView::Instances && self.busy.is_none() =>
            {
                self.open_pair_form();
            }
            _ => {}
        }
    }

    fn open_pair_form(&mut self) {
        let Some(deployment) = self.current_deployment() else {
            self.status = Some("error: no deployment definition found".into());
            return;
        };
        if deployment.bindings.is_empty() {
            self.status = Some(
                "no group bindings are defined; add consumer bindings to deployment.yaml first"
                    .into(),
            );
            return;
        }
        let consumer_index = self
            .binding_selected
            .min(deployment.bindings.len().saturating_sub(1));
        let binding = &deployment.bindings[consumer_index];
        let group_index = binding
            .compatible_groups
            .iter()
            .position(|group| group == &binding.group)
            .unwrap_or_default();
        self.overlay = Overlay::Pair(PairForm {
            consumer_index,
            group_index,
            error: None,
        });
    }

    fn open_instance_form(&mut self) {
        let Some(deployment) = self.current_deployment() else {
            self.status = Some("error: no deployment definition found".into());
            return;
        };
        if self.profiles.is_empty() {
            self.status =
                Some("error: no startup profiles are available; refresh Profiles first".into());
            return;
        }
        if deployment.source_choices.is_empty() {
            self.status = Some("error: add or declare a source before adding an instance".into());
            return;
        }
        let profile_index = self
            .profiles
            .iter()
            .position(profile_is_runnable)
            .unwrap_or_default();
        self.overlay = Overlay::Instance(InstanceForm {
            name: String::new(),
            profile_index,
            source_index: 0,
            device_index: 0,
            parameters: Vec::new(),
            active_field: 0,
            field_errors: Default::default(),
            preview: None,
            error: None,
        });
        self.reload_instance_parameters();
    }

    fn handle_instance_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Esc {
            self.overlay = Overlay::None;
            return;
        }
        let Some(deployment) = self.current_deployment() else {
            self.overlay = Overlay::None;
            return;
        };
        let profile_count = self.profiles.len();
        let source_count = deployment.source_choices.len();
        let device_count = self.devices.len() + 1;
        let Overlay::Instance(form) = &mut self.overlay else {
            return;
        };
        let field_count = 4 + form.parameters.len();
        if form.preview.is_some() && key.code == KeyCode::Enter {
            self.submit_instance();
            return;
        }
        let mut profile_changed = false;
        match key.code {
            KeyCode::Tab | KeyCode::Down => {
                form.active_field = (form.active_field + 1) % field_count
            }
            KeyCode::BackTab | KeyCode::Up => {
                form.active_field = (form.active_field + field_count - 1) % field_count
            }
            KeyCode::Left | KeyCode::Char('h') if form.active_field == 0 && profile_count > 0 => {
                form.profile_index = (form.profile_index + profile_count - 1) % profile_count;
                profile_changed = true;
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ')
                if form.active_field == 0 && profile_count > 0 =>
            {
                form.profile_index = (form.profile_index + 1) % profile_count;
                profile_changed = true;
            }
            KeyCode::Left | KeyCode::Char('h') if form.active_field == 1 && source_count > 0 => {
                form.source_index = (form.source_index + source_count - 1) % source_count;
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ')
                if form.active_field == 1 && source_count > 0 =>
            {
                form.source_index = (form.source_index + 1) % source_count;
            }
            KeyCode::Backspace if form.active_field == 2 => {
                form.name.pop();
            }
            KeyCode::Left | KeyCode::Char('h') if form.active_field == 3 => {
                form.device_index = (form.device_index + device_count - 1) % device_count;
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') if form.active_field == 3 => {
                form.device_index = (form.device_index + 1) % device_count;
            }
            KeyCode::Backspace if form.active_field >= 4 => {
                form.parameters[form.active_field - 4].value.pop();
            }
            KeyCode::Char(character)
                if form.active_field == 2
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                form.name.push(character);
            }
            KeyCode::Char(character)
                if form.active_field >= 4
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                form.parameters[form.active_field - 4].value.push(character);
            }
            KeyCode::Enter if form.active_field + 1 < field_count => form.active_field += 1,
            KeyCode::Enter => self.preview_instance(),
            _ => {}
        }
        if profile_changed {
            self.reload_instance_parameters();
        } else if !matches!(
            key.code,
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down | KeyCode::Enter
        ) {
            if let Overlay::Instance(form) = &mut self.overlay {
                form.preview = None;
                form.field_errors.clear();
                form.error = None;
            }
        }
    }

    pub(crate) fn instance_selection(&self) -> Option<(&ProfileRow, &SourceChoice, &str)> {
        let Overlay::Instance(form) = &self.overlay else {
            return None;
        };
        let deployment = self.current_deployment()?;
        let device = if form.device_index == 0 {
            "local"
        } else {
            self.devices.get(form.device_index - 1)?.name.as_str()
        };
        Some((
            self.profiles.get(form.profile_index)?,
            deployment.source_choices.get(form.source_index)?,
            device,
        ))
    }

    fn reload_instance_parameters(&mut self) {
        let Some((profile, definition)) = self.instance_selection().and_then(|(profile, _, _)| {
            self.current_deployment()
                .map(|deployment| (profile.clone(), deployment.bundle.clone()))
        }) else {
            return;
        };
        let loaded = load_profile_block(
            &self.project_dir,
            &definition,
            &profile.name,
            &profile.origin,
        );
        if let Overlay::Instance(form) = &mut self.overlay {
            match loaded {
                Ok(block) => {
                    form.parameters = block
                        .parameters
                        .into_iter()
                        .map(
                            |(name, Parameter { required, default })| InstanceParameterField {
                                name,
                                value: default.unwrap_or_default(),
                                required,
                            },
                        )
                        .collect();
                    form.active_field = form.active_field.min(3 + form.parameters.len());
                    form.error = None;
                }
                Err(error) => {
                    form.parameters.clear();
                    form.error = Some(error.to_string());
                }
            }
            form.preview = None;
            form.field_errors.clear();
        }
    }

    fn instance_request(&self) -> Option<(PathBuf, CreateInstanceRequest)> {
        let (profile, source, device) = self.instance_selection()?;
        let deployment = self.current_deployment()?;
        let Overlay::Instance(form) = &self.overlay else {
            return None;
        };
        Some((
            deployment.bundle.clone(),
            CreateInstanceRequest {
                name: form.name.trim().into(),
                profile: profile.name.clone(),
                profile_origin: profile.origin.clone(),
                source: source.name.clone(),
                device: device.into(),
                parameters: form
                    .parameters
                    .iter()
                    .filter(|parameter| !parameter.value.is_empty())
                    .map(|parameter| (parameter.name.clone(), parameter.value.clone()))
                    .collect(),
            },
        ))
    }

    fn preview_instance(&mut self) {
        let Some((definition, request)) = self.instance_request() else {
            return;
        };
        match preview_instance(&self.project_dir, &definition, &request) {
            Ok(preview) => {
                if let Overlay::Instance(form) = &mut self.overlay {
                    form.field_errors = instance_diagnostic_fields(&preview.diagnostics);
                    form.preview = Some(preview);
                    form.error = None;
                }
            }
            Err(error) => {
                if let Overlay::Instance(form) = &mut self.overlay {
                    form.error = Some(error.to_string());
                }
            }
        }
    }

    fn submit_instance(&mut self) {
        let Some((definition, request)) = self.instance_request() else {
            return;
        };
        let preview_valid = matches!(&self.overlay, Overlay::Instance(form) if form.preview.as_ref().is_some_and(|preview| preview.diagnostics.is_empty()));
        if !preview_valid {
            return;
        }
        match create_instance(&self.project_dir, &definition, &request) {
            Ok(_) => {
                self.overlay = Overlay::None;
                self.status = Some(format!(
                    "instance `{}` added; press u to plan and start the updated deployment",
                    request.name
                ));
                self.refresh_deployments();
            }
            Err(error) => {
                if let Overlay::Instance(form) = &mut self.overlay {
                    form.error = Some(error.to_string());
                }
            }
        }
    }

    fn handle_pair_key(&mut self, key: KeyEvent) {
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('n')) {
            self.overlay = Overlay::None;
            return;
        }
        let binding_count = self
            .current_deployment()
            .map_or(0, |deployment| deployment.bindings.len());
        if binding_count == 0 {
            self.overlay = Overlay::None;
            return;
        }
        let Overlay::Pair(form) = &mut self.overlay else {
            return;
        };
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                form.consumer_index = form.consumer_index.saturating_sub(1);
                self.reset_pair_group();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                form.consumer_index = (form.consumer_index + 1).min(binding_count - 1);
                self.reset_pair_group();
            }
            KeyCode::Left | KeyCode::Char('h') => self.move_pair_group(false),
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') => self.move_pair_group(true),
            KeyCode::Enter | KeyCode::Char('y') if self.busy.is_none() => self.submit_pair(),
            _ => {}
        }
    }

    fn reset_pair_group(&mut self) {
        let Some((current_group, groups)) = self
            .pair_binding()
            .map(|binding| (binding.group.clone(), binding.compatible_groups.clone()))
        else {
            return;
        };
        if let Overlay::Pair(form) = &mut self.overlay {
            form.group_index = groups
                .iter()
                .position(|group| group == &current_group)
                .unwrap_or_default();
            form.error = None;
        }
    }

    fn move_pair_group(&mut self, forward: bool) {
        let count = self
            .pair_binding()
            .map_or(0, |binding| binding.compatible_groups.len());
        if count == 0 {
            return;
        }
        if let Overlay::Pair(form) = &mut self.overlay {
            form.group_index = if forward {
                (form.group_index + 1) % count
            } else {
                (form.group_index + count - 1) % count
            };
            form.error = None;
        }
    }

    fn pair_binding(&self) -> Option<&BindingRow> {
        let Overlay::Pair(form) = &self.overlay else {
            return None;
        };
        self.current_deployment()?.bindings.get(form.consumer_index)
    }

    pub(crate) fn pair_selection(&self) -> Option<(&BindingRow, &str)> {
        let Overlay::Pair(form) = &self.overlay else {
            return None;
        };
        let binding = self
            .current_deployment()?
            .bindings
            .get(form.consumer_index)?;
        let group = binding.compatible_groups.get(form.group_index)?;
        Some((binding, group))
    }

    fn submit_pair(&mut self) {
        let Some((binding, group)) = self
            .pair_selection()
            .map(|(binding, group)| (binding.clone(), group.to_owned()))
        else {
            if let Overlay::Pair(form) = &mut self.overlay {
                form.error = Some("no compatible provider group is available".into());
            }
            return;
        };
        let Some(bundle) = self
            .current_deployment()
            .map(|deployment| deployment.bundle.clone())
        else {
            return;
        };
        self.binding_selected = match &self.overlay {
            Overlay::Pair(form) => form.consumer_index,
            _ => 0,
        };
        self.overlay = Overlay::None;
        self.start_operation(
            format!("pair {} → {group}", binding.consumer),
            OperationSpec::bind(bundle, binding.consumer, group),
        );
    }

    fn handle_add_key(form: &mut AddForm, key: KeyEvent) -> Option<FormAction> {
        match form.panel {
            AddSourcePanel::Location => match key.code {
                KeyCode::Esc => return Some(FormAction::Close),
                KeyCode::F(2) if form.mode == AddSourceMode::Git => {
                    form.panel = AddSourcePanel::GitOptions;
                    form.active_field = 0;
                    form.error = None;
                }
                KeyCode::Tab | KeyCode::Down | KeyCode::BackTab | KeyCode::Up => {
                    form.active_field = usize::from(form.active_field == 0)
                }
                KeyCode::Left | KeyCode::Right => {
                    form.mode = form.mode.toggled();
                    form.error = None;
                }
                KeyCode::Char(' ') if form.active_field == 0 => {
                    form.mode = form.mode.toggled();
                    form.error = None;
                }
                KeyCode::Backspace if form.active_field == 1 => {
                    form.location.pop();
                    form.error = None;
                }
                KeyCode::Char(character)
                    if form.active_field == 1
                        && !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    form.location.push(character);
                    form.error = None;
                }
                KeyCode::Enter if form.mode == AddSourceMode::Git => {
                    if let Err(error) = form.validate_location() {
                        form.error = Some(error);
                    } else {
                        form.panel = AddSourcePanel::GitOptions;
                        form.active_field = 0;
                        form.error = None;
                    }
                }
                KeyCode::Enter => return Some(FormAction::Submit),
                _ => {}
            },
            AddSourcePanel::GitOptions => match key.code {
                KeyCode::Esc | KeyCode::F(2) => {
                    form.panel = AddSourcePanel::Location;
                    form.active_field = 1;
                    form.error = None;
                }
                KeyCode::Enter => return Some(FormAction::Submit),
                KeyCode::Tab | KeyCode::Down => {
                    form.active_field = (form.active_field + 1) % form.advanced_field_count()
                }
                KeyCode::BackTab | KeyCode::Up => {
                    form.active_field = (form.active_field + form.advanced_field_count() - 1)
                        % form.advanced_field_count()
                }
                KeyCode::Backspace => {
                    if form.pop_active() {
                        form.error = None;
                    }
                }
                KeyCode::Char(character)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    if form.append_active(&character.to_string()) {
                        form.error = None;
                    }
                }
                _ => {}
            },
        }
        None
    }

    fn handle_device_key(form: &mut DeviceForm, key: KeyEvent) -> Option<FormAction> {
        match key.code {
            KeyCode::Esc => return Some(FormAction::Close),
            KeyCode::Tab | KeyCode::Down => form.active_field = (form.active_field + 1) % 5,
            KeyCode::BackTab | KeyCode::Up => form.active_field = (form.active_field + 4) % 5,
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

    fn handle_worktree_key(form: &mut WorktreeForm, key: KeyEvent) -> Option<FormAction> {
        match key.code {
            KeyCode::Esc => return Some(FormAction::Close),
            KeyCode::Tab | KeyCode::Down | KeyCode::BackTab | KeyCode::Up => {}
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
    fn handle_device_remove_confirm(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('n') | KeyCode::Esc => self.overlay = Overlay::None,
            KeyCode::Char('y') if self.busy.is_none() => {
                let name = match &self.overlay {
                    Overlay::ConfirmRemoveDevice { name, .. } => name.clone(),
                    _ => return,
                };
                self.start_device_remove(name);
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
                                    BusyKind::WorktreeAdd => "worktree created",
                                    BusyKind::Remove => "source removed",
                                    _ => "sources refreshed",
                                }
                                .into(),
                            );
                            self.refresh_deployments();
                        }
                        Err(error) => match (&mut self.overlay, kind) {
                            (Overlay::Add(form), BusyKind::Add) => form.error = Some(error),
                            (Overlay::Worktree(form), BusyKind::WorktreeAdd) => {
                                form.error = Some(error)
                            }
                            (Overlay::ConfirmRemove { error: slot, .. }, BusyKind::Remove) => {
                                *slot = Some(error)
                            }
                            _ => self.status = Some(format!("error: {error}")),
                        },
                    }
                    break;
                }
                TaskResult::Devices(result, kind) => {
                    self.task = None;
                    self.busy = None;
                    match result {
                        Ok(devices) => {
                            self.devices = devices;
                            self.device_selected = self
                                .device_selected
                                .min(self.devices.len().saturating_sub(1));
                            self.overlay = Overlay::None;
                            self.status = Some(
                                match kind {
                                    BusyKind::DeviceAdd => "device added",
                                    BusyKind::DeviceRemove => "device removed",
                                    BusyKind::DeviceCheck => "device check finished",
                                    _ => "devices refreshed",
                                }
                                .into(),
                            );
                        }
                        Err(error) => match (&mut self.overlay, kind) {
                            (Overlay::Device(form), BusyKind::DeviceAdd) => {
                                form.error = Some(error)
                            }
                            (
                                Overlay::ConfirmRemoveDevice { error: slot, .. },
                                BusyKind::DeviceRemove,
                            ) => *slot = Some(error),
                            _ => self.status = Some(format!("error: {error}")),
                        },
                    }
                    break;
                }
                TaskResult::Profiles(result, kind) => {
                    self.task = None;
                    self.busy = None;
                    match result {
                        Ok(listing) => {
                            self.profiles = listing.rows;
                            self.profile_source_errors = listing.source_errors;
                            self.profile_selected = self
                                .profile_selected
                                .min(self.profiles.len().saturating_sub(1));
                            self.overlay = Overlay::None;
                            self.status = Some(
                                match kind {
                                    BusyKind::ProfileImport => "startup profile imported",
                                    BusyKind::ProfileRemove => "startup profile import removed",
                                    _ => "startup profiles refreshed",
                                }
                                .into(),
                            );
                        }
                        Err(error) => {
                            let status = match kind {
                                BusyKind::ProfileImport | BusyKind::ProfileRemove => error.clone(),
                                _ => format!("refresh startup profiles failed: {error}"),
                            };
                            self.status = Some(status);
                            match (&mut self.overlay, kind) {
                                (
                                    Overlay::ProfileReview { error: slot, .. },
                                    BusyKind::ProfileImport,
                                )
                                | (
                                    Overlay::ConfirmRemoveProfile { error: slot, .. },
                                    BusyKind::ProfileRemove,
                                ) => *slot = Some(error),
                                _ => {}
                            }
                        }
                    }
                    break;
                }
                TaskResult::ProfileDialog(result, kind) => {
                    self.task = None;
                    self.busy = None;
                    match result {
                        Ok(dialog) if kind == BusyKind::ProfileInspect => {
                            self.overlay = Overlay::ProfileInspector {
                                name: dialog.name,
                                lines: dialog.lines,
                                scroll: 0,
                            };
                        }
                        Ok(dialog) if kind == BusyKind::ProfileReview => {
                            self.overlay = Overlay::ProfileReview {
                                name: dialog.name,
                                source: dialog.source.unwrap_or_default(),
                                yaml: dialog.yaml,
                                scroll: 0,
                                error: None,
                            };
                        }
                        Ok(_) => {}
                        Err(error) => {
                            let action = if kind == BusyKind::ProfileInspect {
                                "inspect profile"
                            } else {
                                "review profile import"
                            };
                            self.status = Some(format!("{action} failed: {error}"));
                        }
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
        match list_deployments(&self.project_dir, &self.sources) {
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
            let AddRequest::Local { name, path } = request else {
                return Err("Git clones require the native terminal handoff".into());
            };
            let name = unique_source_name(&store, &name)?;
            let path = if path.is_absolute() {
                path
            } else {
                root.join(path)
            };
            manager
                .register_unmanaged(&store, &name, &path)
                .map_err(|error| error.to_string())?;
            manager.list(&store).map_err(|error| error.to_string())
        });
    }

    fn start_worktree_add(&mut self, source: String, base_ref: String, name: String) {
        self.busy = Some(BusyKind::WorktreeAdd);
        self.spawn_sources(BusyKind::WorktreeAdd, move |root| {
            let store = open_store(&root)?;
            let manager = switchyard_sources::SourceManager::new(&root);
            let name = unique_source_name(&store, &name)?;
            manager
                .create_worktree_branch(&store, &source, &base_ref, &name, &name, None)
                .map_err(|error| error.to_string())?;
            manager.list(&store).map_err(|error| error.to_string())
        });
    }
    fn start_device_add(&mut self, device: RegisteredDevice) {
        self.busy = Some(BusyKind::DeviceAdd);
        self.spawn_devices(BusyKind::DeviceAdd, move |root| {
            let store = open_store(&root)?;
            store
                .register_device(&device)
                .map_err(|error| error.to_string())?;
            store.devices().map_err(|error| error.to_string())
        });
    }
    fn start_device_remove(&mut self, name: String) {
        self.busy = Some(BusyKind::DeviceRemove);
        self.spawn_devices(BusyKind::DeviceRemove, move |root| {
            let store = open_store(&root)?;
            store
                .deregister_device(&name)
                .map_err(|error| error.to_string())?;
            store.devices().map_err(|error| error.to_string())
        });
    }
    fn start_device_check(&mut self, name: String) {
        self.busy = Some(BusyKind::DeviceCheck);
        self.spawn_devices(BusyKind::DeviceCheck, move |root| {
            let store = open_store(&root)?;
            let device = store
                .device(&name)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("device `{name}` is not registered"))?;
            let (status, detail) =
                switchyard_daemon::device::check(&device).map_err(|error| error.to_string())?;
            store
                .record_device_check(&name, unix_millis(), status, Some(&detail))
                .map_err(|error| error.to_string())?;
            store.devices().map_err(|error| error.to_string())
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
    fn profile_definition_path(&self) -> PathBuf {
        self.current_deployment()
            .map(|entry| entry.bundle.clone())
            .unwrap_or_else(|| self.project_dir.join("deployment.yaml"))
    }
    fn start_profile_refresh(&mut self) {
        self.busy = Some(BusyKind::ProfileRefresh);
        let definition = self.profile_definition_path();
        self.spawn_profiles(BusyKind::ProfileRefresh, move |root| {
            list_profiles(&root, &definition).map_err(|error| error.to_string())
        });
    }
    fn start_profile_inspect(&mut self) {
        let Some(row) = self.profiles.get(self.profile_selected).cloned() else {
            self.status = Some("inspect profile failed: no startup profile is selected".into());
            return;
        };
        self.busy = Some(BusyKind::ProfileInspect);
        let definition = self.profile_definition_path();
        let root = self.project_dir.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = load_profile_block(&root, &definition, &row.name, &row.origin)
                .map(|block| ProfileDialog {
                    name: row.name.clone(),
                    source: None,
                    lines: profile_inspector_lines(&row, &block),
                    yaml: String::new(),
                })
                .map_err(|error| error.to_string());
            let _ = sender.send(TaskResult::ProfileDialog(result, BusyKind::ProfileInspect));
        });
        self.task = Some(receiver);
    }
    fn start_profile_review(&mut self) {
        let Some(row) = self.profiles.get(self.profile_selected).cloned() else {
            self.status = Some("import profile failed: no startup profile is selected".into());
            return;
        };
        let source = match (&row.origin, row.trust) {
            (ProfileOrigin::Project, _) => {
                self.status = Some("import profile: project profiles are already trusted".into());
                return;
            }
            (ProfileOrigin::ImportedFromSource { .. }, ProfileTrust::Imported) => {
                self.status =
                    Some("import profile: this profile is already imported and unchanged".into());
                return;
            }
            (ProfileOrigin::ImportedFromSource { source, .. }, ProfileTrust::Changed)
            | (ProfileOrigin::DiscoveredInSource { source, .. }, ProfileTrust::NotImported) => {
                source.clone()
            }
            _ => {
                self.status = Some("import profile: selected profile cannot be imported".into());
                return;
            }
        };
        self.busy = Some(BusyKind::ProfileReview);
        let root = self.project_dir.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = load_source_profile_block(&root, &source, &row.name)
                .and_then(|block| {
                    serde_yaml::to_string(&block).map_err(|error| {
                        switchyard_ops::profiles::ProfileError::Definition(format!(
                            "could not format profile review: {error}"
                        ))
                    })
                })
                .map(|yaml| ProfileDialog {
                    name: row.name,
                    source: Some(source),
                    lines: Vec::new(),
                    yaml,
                })
                .map_err(|error| error.to_string());
            let _ = sender.send(TaskResult::ProfileDialog(result, BusyKind::ProfileReview));
        });
        self.task = Some(receiver);
    }
    fn handle_profile_review(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.overlay = Overlay::None,
            KeyCode::Down | KeyCode::Char('j') | KeyCode::PageDown => {
                if let Overlay::ProfileReview { scroll, yaml, .. } = &mut self.overlay {
                    *scroll = (*scroll + 1).min(yaml.lines().count().saturating_sub(1));
                }
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::PageUp => {
                if let Overlay::ProfileReview { scroll, .. } = &mut self.overlay {
                    *scroll = scroll.saturating_sub(1);
                }
            }
            KeyCode::Enter if self.busy.is_none() => {
                let (name, source) = match &self.overlay {
                    Overlay::ProfileReview { name, source, .. } => (name.clone(), source.clone()),
                    _ => return,
                };
                self.busy = Some(BusyKind::ProfileImport);
                let definition = self.profile_definition_path();
                self.spawn_profiles(BusyKind::ProfileImport, move |root| {
                    import_source_profile(&root, &source, &name)
                        .map_err(|error| format!("import profile `{name}` failed: {error}"))?;
                    list_profiles(&root, &definition).map_err(|error| {
                        format!("refresh after importing profile `{name}` failed: {error}")
                    })
                });
            }
            _ => {}
        }
    }
    fn open_profile_remove(&mut self) {
        let Some(row) = self.profiles.get(self.profile_selected) else {
            self.status = Some("remove import failed: no startup profile is selected".into());
            return;
        };
        match row.origin {
            ProfileOrigin::Project => {
                self.status = Some("remove import: project profiles cannot be removed here".into())
            }
            ProfileOrigin::ImportedFromSource { .. } => {
                self.overlay = Overlay::ConfirmRemoveProfile {
                    name: row.name.clone(),
                    error: None,
                }
            }
            ProfileOrigin::DiscoveredInSource { .. } => {
                self.status = Some("remove import: selected profile has not been imported".into())
            }
        }
    }
    fn handle_profile_remove_confirm(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('n') => self.overlay = Overlay::None,
            KeyCode::Enter if self.busy.is_none() => {
                let name = match &self.overlay {
                    Overlay::ConfirmRemoveProfile { name, .. } => name.clone(),
                    _ => return,
                };
                self.busy = Some(BusyKind::ProfileRemove);
                let definition = self.profile_definition_path();
                self.spawn_profiles(BusyKind::ProfileRemove, move |root| {
                    remove_imported_profile(&root, &name)
                        .map_err(|error| format!("remove import `{name}` failed: {error}"))?;
                    list_profiles(&root, &definition).map_err(|error| {
                        format!("refresh after removing import `{name}` failed: {error}")
                    })
                });
            }
            _ => {}
        }
    }
    fn spawn_profiles(
        &mut self,
        kind: BusyKind,
        operation: impl FnOnce(PathBuf) -> Result<ProfileListing, String> + Send + 'static,
    ) {
        let root = self.project_dir.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(TaskResult::Profiles(operation(root), kind));
        });
        self.task = Some(receiver);
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
    fn spawn_devices(
        &mut self,
        kind: BusyKind,
        operation: impl FnOnce(PathBuf) -> Result<Vec<RegisteredDevice>, String> + Send + 'static,
    ) {
        let root = self.project_dir.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(TaskResult::Devices(operation(root), kind));
        });
        self.task = Some(receiver);
    }
}

const fn home_destination(item: usize) -> ActiveView {
    match item {
        0 => ActiveView::Sources,
        1 => ActiveView::Profiles,
        _ => ActiveView::Instances,
    }
}

fn profile_inspector_lines(row: &ProfileRow, block: &Block) -> Vec<String> {
    let (origin, commit) = match &row.origin {
        ProfileOrigin::Project => ("project".into(), "not applicable".into()),
        ProfileOrigin::ImportedFromSource { source, commit } => (
            format!("imported from {source}"),
            commit.clone().unwrap_or_else(|| "unknown".into()),
        ),
        ProfileOrigin::DiscoveredInSource { source, commit } => (
            format!("found in {source}"),
            commit.clone().unwrap_or_else(|| "unknown".into()),
        ),
    };
    let trust = match row.trust {
        ProfileTrust::Trusted => "trusted",
        ProfileTrust::Imported => "imported",
        ProfileTrust::Changed => "changed — review",
        ProfileTrust::NotImported => "not imported",
    };
    let hash = match row.trust {
        ProfileTrust::Trusted => "project definition is trusted",
        ProfileTrust::Imported => "current content matches the imported hash",
        ProfileTrust::Changed => "current content differs from the imported hash",
        ProfileTrust::NotImported => "no imported content hash",
    };
    let mut lines = vec![
        format!("Origin: {origin}"),
        format!("Trust: {trust}"),
        format!("Commit: {commit}"),
        format!("Content hash: {hash}"),
        format!(
            "Parameters: {}",
            if block.parameters.is_empty() {
                "none".into()
            } else {
                block
                    .parameters
                    .iter()
                    .map(|(name, parameter)| {
                        format!(
                            "{name} ({}{})",
                            if parameter.required {
                                "required"
                            } else {
                                "optional"
                            },
                            parameter
                                .default
                                .as_deref()
                                .map_or_else(String::new, |value| format!(", default {value}"))
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        ),
        String::new(),
    ];
    for (name, service) in &block.services {
        lines.push(format!("Service: {name}"));
        match &service.execution {
            Execution::Container {
                image,
                build,
                command,
                working_directory,
                ..
            } => {
                lines.push("  Adapter: container".into());
                lines.push(format!("  Image: {}", image.as_deref().unwrap_or("-")));
                if let Some(build) = build {
                    lines.push(format!("  Build context: {}", build.context.display()));
                }
                lines.push(format!("  Command: {}", command.join(" ")));
                lines.push(format!(
                    "  Workdir: {}",
                    working_directory
                        .as_deref()
                        .map_or_else(|| "-".into(), |path| path.display().to_string())
                ));
            }
            Execution::Script {
                image,
                command,
                working_directory,
                lifecycle,
                ..
            } => {
                lines.push("  Adapter: script".into());
                lines.push(format!("  Image: {image}"));
                lines.push(format!("  Command: {}", command.join(" ")));
                lines.push(format!(
                    "  Workdir: {}",
                    working_directory
                        .as_deref()
                        .map_or_else(|| "-".into(), |path| path.display().to_string())
                ));
                lines.push(format!("  Lifecycle: {lifecycle:?}"));
            }
            Execution::ProcessCompose {
                image,
                file,
                working_directory,
                ..
            } => {
                lines.push("  Adapter: process-compose".into());
                lines.push(format!("  Image: {image}"));
                lines.push(format!("  Command/file: {}", file.display()));
                lines.push(format!(
                    "  Workdir: {}",
                    working_directory
                        .as_deref()
                        .map_or_else(|| "-".into(), |path| path.display().to_string())
                ));
            }
        }
        lines.push(format!("  Provides: {}", map_names(&service.provides)));
        lines.push(format!("  Consumes: {}", map_names(&service.consumes)));
        lines.push(format!(
            "  Probe: {}",
            service.probe.as_ref().map_or_else(
                || "none".into(),
                |probe| match probe {
                    Probe::Http { path, port, https } =>
                        format!("http{} {path} on {port}", if *https { "s" } else { "" }),
                    Probe::Tcp { port } => format!("tcp {port}"),
                    Probe::Command { command } => format!("command {}", command.join(" ")),
                }
            )
        ));
        lines.push(String::new());
    }
    lines
}

fn map_names<T>(values: &std::collections::BTreeMap<String, T>) -> String {
    if values.is_empty() {
        "none".into()
    } else {
        values.keys().cloned().collect::<Vec<_>>().join(", ")
    }
}

fn profile_is_runnable(row: &ProfileRow) -> bool {
    !row.shadowed && matches!(row.trust, ProfileTrust::Trusted | ProfileTrust::Imported)
}

fn instance_diagnostic_fields(
    diagnostics: &[switchyard_planner::Diagnostic],
) -> std::collections::BTreeMap<String, String> {
    let mut fields = std::collections::BTreeMap::new();
    for diagnostic in diagnostics {
        let field = if diagnostic.path.ends_with(".name") {
            Some("name".to_owned())
        } else if diagnostic.path.ends_with(".block") {
            Some("profile".to_owned())
        } else if diagnostic.path.ends_with(".source") {
            Some("source".to_owned())
        } else if diagnostic.path.ends_with(".device") {
            Some("device".to_owned())
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
pub(crate) fn execute_interactive_clone(
    root: &Path,
    request: AddRequest,
) -> Result<Vec<RegisteredSourceInspection>, String> {
    let AddRequest::Clone { name, url, git_ref } = request else {
        return Err("interactive source request was not a Git clone".into());
    };
    let store = open_store(root)?;
    let manager = switchyard_sources::SourceManager::new(root);
    let name = unique_source_name(&store, &name)?;
    manager
        .create_clone_from_url_interactive(&store, &url, &name, git_ref.as_deref())
        .map_err(|error| error.to_string())?;
    manager.list(&store).map_err(|error| error.to_string())
}

fn unique_source_name(store: &StateStore, base: &str) -> Result<String, String> {
    if store
        .source(base)
        .map_err(|error| error.to_string())?
        .is_none()
    {
        return Ok(base.to_owned());
    }
    for suffix in 2..=10_000 {
        let candidate = format!("{base}-{suffix}");
        if store
            .source(&candidate)
            .map_err(|error| error.to_string())?
            .is_none()
        {
            return Ok(candidate);
        }
    }
    Err(format!(
        "could not find an available source name based on `{base}`"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use switchyard_adapter_sdk::SourceIdentity;
    use switchyard_sources::SourceInspection;
    use switchyard_state::{RegisteredSource, RegisteredSourceKind};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn repository_source(name: &str) -> RegisteredSourceInspection {
        let path = PathBuf::from(format!("/repositories/{name}"));
        RegisteredSourceInspection {
            source: RegisteredSource {
                name: name.into(),
                kind: RegisteredSourceKind::Unmanaged,
                path: path.clone(),
                repository_path: Some(path.clone()),
                requested_ref: None,
                created_at: 1,
                managed_relative_path: None,
            },
            inspection: SourceInspection {
                identity: SourceIdentity {
                    path: path.display().to_string(),
                    repository: Some(path.display().to_string()),
                    r#ref: Some("main".into()),
                    commit: Some("0123456789".into()),
                    dirty: Some(false),
                },
                linked_worktree: Some(false),
                branch: Some("main".into()),
                detached: Some(false),
                changes: None,
                ahead: Some(0),
                behind: Some(0),
                unknown_code: None,
            },
        }
    }

    #[test]
    fn add_form_accepts_one_location_and_derives_the_name() {
        let mut form = AddForm::default();
        assert!(form.validate().is_err());
        form.location = "/work/demo".into();
        assert_eq!(
            form.validate().unwrap(),
            AddRequest::Local {
                name: "demo".into(),
                path: "/work/demo".into()
            }
        );
        form.mode = AddSourceMode::Git;
        form.location = "git@github.com:team/ai-chatbot.git".into();
        form.git_ref = "feature".into();
        assert_eq!(
            form.validate().unwrap(),
            AddRequest::Clone {
                name: "ai-chatbot".into(),
                url: "git@github.com:team/ai-chatbot.git".into(),
                git_ref: Some("feature".into()),
            }
        );
        form.location = "https://user:token@example.invalid/team/repo.git".into();
        assert!(form.validate().unwrap_err().contains("credential helper"));
    }

    #[test]
    fn bracketed_paste_changes_only_the_active_source_field() {
        let mut app = App::with_sources(PathBuf::from("."), Vec::new());
        app.overlay = Overlay::Add(AddForm {
            mode: AddSourceMode::Git,
            ..AddForm::default()
        });
        app.handle_event(Event::Paste(
            "git@github.com:team/ai-chatbot.git\r\n".into(),
        ));
        let Overlay::Add(form) = &mut app.overlay else {
            panic!("add source form closed")
        };
        assert_eq!(form.location, "git@github.com:team/ai-chatbot.git");
        assert!(form.git_ref.is_empty());
    }

    #[test]
    fn selected_checkout_opens_a_minimal_worktree_form() {
        let mut app = App::with_sources(PathBuf::from("."), vec![repository_source("product")]);
        app.active_view = ActiveView::Sources;
        app.handle_key(key(KeyCode::Char('w')));
        let Overlay::Worktree(form) = &mut app.overlay else {
            panic!("worktree form did not open")
        };
        assert_eq!(form.source, "product");
        assert_eq!(form.base_ref, "0123456789");
        assert!(form.validate().is_err());
        form.name = "feature-a".into();
        assert_eq!(form.validate(), Ok(()));
    }

    #[test]
    fn git_options_are_a_separate_keyboard_panel() {
        let mut app = App::with_sources(PathBuf::from("."), Vec::new());
        app.overlay = Overlay::Add(AddForm {
            mode: AddSourceMode::Git,
            location: "git@github.com:team/repo.git".into(),
            ..AddForm::default()
        });
        app.handle_key(key(KeyCode::Enter));
        let Overlay::Add(form) = &mut app.overlay else {
            panic!("add source form closed")
        };
        assert_eq!(form.panel, AddSourcePanel::GitOptions);
        app.handle_event(Event::Paste("feature/native-auth".into()));
        app.handle_key(key(KeyCode::F(2)));
        let Overlay::Add(form) = &mut app.overlay else {
            panic!("add source form closed")
        };
        assert_eq!(form.panel, AddSourcePanel::Location);
        assert_eq!(form.git_ref, "feature/native-auth");
    }

    #[test]
    fn git_clone_requires_authentication_review_before_submit() {
        let mut form = AddForm {
            mode: AddSourceMode::Git,
            location: "git@github.com:team/repo.git".into(),
            ..AddForm::default()
        };
        assert!(App::handle_add_key(&mut form, key(KeyCode::Enter)).is_none());
        assert_eq!(form.panel, AddSourcePanel::GitOptions);
        assert!(matches!(
            App::handle_add_key(&mut form, key(KeyCode::Enter)),
            Some(FormAction::Submit)
        ));
    }

    #[test]
    fn submitted_git_clone_is_queued_for_terminal_handoff() {
        let mut app = App::with_sources(PathBuf::from("."), Vec::new());
        app.overlay = Overlay::Add(AddForm {
            mode: AddSourceMode::Git,
            location: "git@example.test:team/repo.git".into(),
            panel: AddSourcePanel::GitOptions,
            ..AddForm::default()
        });
        app.handle_key(key(KeyCode::Enter));
        let request = app.take_pending_clone().expect("clone was not queued");
        assert!(matches!(
            request,
            AddRequest::Clone { url, .. } if url == "git@example.test:team/repo.git"
        ));
    }
    #[test]
    fn device_form_validates_ssh_fields() {
        let valid = DeviceForm {
            name: "builder".into(),
            user: "dev".into(),
            host: "host.test".into(),
            port: "2222".into(),
            identity_file: "keys/id_ed25519".into(),
            active_field: 0,
            error: None,
        };
        let device = valid.device().unwrap();
        assert_eq!(device.port, 2222);
        assert_eq!(device.identity_file, Some("keys/id_ed25519".into()));
        for (user, host) in [("-oProxyCommand=bad", "host.test"), ("dev", "-x")] {
            let mut invalid = valid.clone();
            invalid.user = user.into();
            invalid.host = host.into();
            assert!(invalid.device().is_err());
        }
    }
    #[test]
    fn tabs_cycle_through_all_control_plane_views() {
        let mut app = App::with_sources(PathBuf::from("."), Vec::new());
        assert_eq!(app.active_view, ActiveView::Home);
        app.handle_key(key(KeyCode::Tab));
        assert_eq!(app.active_view, ActiveView::Sources);
        app.handle_key(key(KeyCode::Tab));
        assert_eq!(app.active_view, ActiveView::Profiles);
        app.handle_key(key(KeyCode::Tab));
        assert_eq!(app.active_view, ActiveView::Devices);
        app.handle_key(key(KeyCode::Tab));
        assert_eq!(app.active_view, ActiveView::Instances);
        app.handle_key(key(KeyCode::Tab));
        assert_eq!(app.active_view, ActiveView::Home);
        app.handle_key(key(KeyCode::BackTab));
        assert_eq!(app.active_view, ActiveView::Instances);
    }

    #[test]
    fn home_jump_keys_route_to_checklist_tabs() {
        let mut app = App::with_sources(PathBuf::from("."), Vec::new());
        app.handle_key(key(KeyCode::Char('2')));
        assert_eq!(app.active_view, ActiveView::Profiles);

        app.active_view = ActiveView::Home;
        app.home_selected = 0;
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.active_view, ActiveView::Instances);

        for number in ['3', '4', '5'] {
            app.active_view = ActiveView::Home;
            app.handle_key(key(KeyCode::Char(number)));
            assert_eq!(app.active_view, ActiveView::Instances);
        }
    }
    #[test]
    fn profile_import_review_and_remove_wait_for_explicit_enter() {
        let mut app = App::with_sources(PathBuf::from("."), Vec::new());
        app.active_view = ActiveView::Profiles;
        app.overlay = Overlay::ProfileReview {
            name: "api".into(),
            source: "checkout".into(),
            yaml: "services: {}".into(),
            scroll: 0,
            error: None,
        };
        app.handle_key(key(KeyCode::Char('y')));
        assert!(matches!(app.overlay, Overlay::ProfileReview { .. }));
        assert!(app.busy.is_none());
        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.overlay, Overlay::None);

        app.overlay = Overlay::ConfirmRemoveProfile {
            name: "api".into(),
            error: None,
        };
        app.handle_key(key(KeyCode::Char('y')));
        assert!(matches!(app.overlay, Overlay::ConfirmRemoveProfile { .. }));
        assert!(app.busy.is_none());
        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.overlay, Overlay::None);
    }
    #[test]
    fn profile_delete_key_opens_confirmation_for_imported_row() {
        let mut app = App::with_sources(PathBuf::from("."), Vec::new());
        app.active_view = ActiveView::Profiles;
        app.profiles.push(ProfileRow {
            name: "api".into(),
            origin: ProfileOrigin::ImportedFromSource {
                source: "checkout".into(),
                commit: Some("abc".into()),
            },
            trust: ProfileTrust::Imported,
            shadowed: false,
            services: Vec::new(),
        });
        app.handle_key(key(KeyCode::Char('d')));
        assert!(matches!(
            app.overlay,
            Overlay::ConfirmRemoveProfile { ref name, .. } if name == "api"
        ));
        assert!(app.busy.is_none());
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
        assert!(form.error.unwrap().contains("local directory"));
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
    fn guided_instance_preview_attaches_errors_and_second_enter_writes() {
        let root = std::env::temp_dir().join(format!(
            "switchyard-tui-guided-instance-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let definition = root.join("deployment.yaml");
        fs::write(
            &definition,
            r#"apiVersion: switchyard.dev/v1alpha1
kind: Deployment
metadata: { name: demo }
spec:
  sources:
    checkout: { path: . }
  blocks:
    api:
      parameters:
        TOKEN: { required: true }
        PORT: { default: "8080" }
      services:
        web:
          execution: { type: container, image: api:1 }
  instances:
  groups:
  bindings:
  routes:
"#,
        )
        .unwrap();
        StateStore::open(root.join(".switchyard/state.sqlite3")).unwrap();
        let mut app = App::with_sources(root.clone(), Vec::new());
        app.active_view = ActiveView::Instances;
        app.deployments.push(DeploymentEntry {
            name: "demo".into(),
            bundle: definition.clone(),
            state: "not applied".into(),
            services: Vec::new(),
            instances: Vec::new(),
            blocks: vec!["api".into()],
            source_choices: vec![SourceChoice {
                name: "checkout".into(),
                path: ".".into(),
                declared: true,
                worktree: false,
                repository: None,
                requested_ref: None,
            }],
            bindings: Vec::new(),
            last_operation: None,
            applied: false,
            consumer_slot_count: 0,
            validation_problems: Vec::new(),
        });
        app.profiles.push(ProfileRow {
            name: "api".into(),
            origin: ProfileOrigin::Project,
            trust: ProfileTrust::Trusted,
            shadowed: false,
            services: Vec::new(),
        });
        app.devices.push(RegisteredDevice {
            name: "builder".into(),
            host: "builder.test".into(),
            port: 22,
            user: "dev".into(),
            identity_file: None,
            created_at: 1,
            last_checked_at: None,
            last_check_status: DeviceCheckStatus::Never,
            last_check_detail: None,
        });
        app.open_instance_form();
        let before = fs::read_to_string(&definition).unwrap();
        let Overlay::Instance(form) = &mut app.overlay else {
            panic!("guided form missing")
        };
        assert_eq!(form.parameters[0].value, "8080");
        form.name = "api-main".into();
        form.device_index = 1;
        form.active_field = 5;
        app.handle_key(key(KeyCode::Enter));
        let Overlay::Instance(form) = &app.overlay else {
            panic!("preview missing")
        };
        assert!(form.preview.is_some());
        assert!(form.field_errors.contains_key("device"));
        assert!(form.field_errors.contains_key("parameter:TOKEN"));
        assert_eq!(fs::read_to_string(&definition).unwrap(), before);

        let Overlay::Instance(form) = &mut app.overlay else {
            unreachable!()
        };
        form.device_index = 0;
        form.parameters[1].value = "secret-reference".into();
        form.preview = None;
        app.preview_instance();
        assert!(
            matches!(&app.overlay, Overlay::Instance(form) if form.preview.as_ref().is_some_and(|preview| preview.diagnostics.is_empty()))
        );
        app.handle_key(key(KeyCode::Enter));
        assert!(matches!(app.overlay, Overlay::None));
        let updated = fs::read_to_string(&definition).unwrap();
        assert!(updated.contains("name: api-main"));
        assert!(updated.contains("device: local"));
        assert!(updated.contains("TOKEN: secret-reference"));
        fs::remove_dir_all(root).unwrap();
    }
}
