use std::{
    error::Error,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use switchyard_sources::RegisteredSourceInspection;
use switchyard_state::{RegisteredSourceKind, StateStore};

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
enum FormAction {
    Close,
    Submit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Overlay {
    None,
    Add(AddForm),
    ConfirmRemove { name: String, error: Option<String> },
    Help,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BusyKind {
    Add,
    Remove,
    Refresh,
}

enum TaskResult {
    Sources(Result<Vec<RegisteredSourceInspection>, String>, BusyKind),
}

pub(crate) struct App {
    pub(crate) project_dir: PathBuf,
    pub(crate) active_view: ActiveView,
    pub(crate) sources: Vec<RegisteredSourceInspection>,
    pub(crate) selected: usize,
    pub(crate) overlay: Overlay,
    pub(crate) busy: Option<BusyKind>,
    pub(crate) spinner_tick: usize,
    pub(crate) status: Option<String>,
    quit: bool,
    task: Option<Receiver<TaskResult>>,
}

impl App {
    pub(crate) fn load(project_dir: PathBuf) -> Result<Self, Box<dyn Error>> {
        let sources = list_sources(&project_dir)?;
        Ok(Self::with_sources(project_dir, sources))
    }

    pub(crate) fn with_sources(
        project_dir: PathBuf,
        sources: Vec<RegisteredSourceInspection>,
    ) -> Self {
        Self {
            project_dir,
            active_view: ActiveView::Sources,
            sources,
            selected: 0,
            overlay: Overlay::None,
            busy: None,
            spinner_tick: 0,
            status: None,
            quit: false,
            task: None,
        }
    }

    pub(crate) const fn should_quit(&self) -> bool {
        self.quit
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
                Some(FormAction::Submit) if self.busy.is_none() => {
                    let validated = match &self.overlay {
                        Overlay::Add(form) => form.validate(),
                        _ => return,
                    };
                    match validated {
                        Ok(request) => self.start_add(request),
                        Err(error) => {
                            if let Overlay::Add(form) = &mut self.overlay {
                                form.error = Some(error);
                            }
                        }
                    }
                }
                _ => {}
            },
            Overlay::ConfirmRemove { .. } => self.handle_confirm_key(key),
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
                self.active_view = match self.active_view {
                    ActiveView::Sources => ActiveView::Instances,
                    ActiveView::Instances => ActiveView::Sources,
                };
            }
            KeyCode::Down | KeyCode::Char('j') if self.active_view == ActiveView::Sources => {
                if !self.sources.is_empty() {
                    self.selected = (self.selected + 1).min(self.sources.len() - 1);
                }
            }
            KeyCode::Up | KeyCode::Char('k') if self.active_view == ActiveView::Sources => {
                self.selected = self.selected.saturating_sub(1);
            }
            KeyCode::Char('a')
                if self.active_view == ActiveView::Sources && self.busy.is_none() =>
            {
                self.overlay = Overlay::Add(AddForm::default());
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
                self.start_refresh(BusyKind::Refresh);
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

    fn handle_confirm_key(&mut self, key: KeyEvent) {
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

    pub(crate) fn tick(&mut self) {
        self.spinner_tick = self.spinner_tick.wrapping_add(1);
        let result = self
            .task
            .as_ref()
            .and_then(|receiver| match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(TaskResult::Sources(
                    Err("background operation stopped unexpectedly".into()),
                    self.busy.unwrap_or(BusyKind::Refresh),
                )),
            });
        if let Some(TaskResult::Sources(result, kind)) = result {
            self.task = None;
            self.busy = None;
            match result {
                Ok(sources) => {
                    self.sources = sources;
                    self.selected = self.selected.min(self.sources.len().saturating_sub(1));
                    self.overlay = Overlay::None;
                    self.status = Some(match kind {
                        BusyKind::Add => "source added".into(),
                        BusyKind::Remove => "source removed".into(),
                        BusyKind::Refresh => "sources refreshed".into(),
                    });
                }
                Err(error) => match (&mut self.overlay, kind) {
                    (Overlay::Add(form), BusyKind::Add) => form.error = Some(error),
                    (Overlay::ConfirmRemove { error: slot, .. }, BusyKind::Remove) => {
                        *slot = Some(error);
                    }
                    _ => self.status = Some(format!("error: {error}")),
                },
            }
        }
    }

    fn start_add(&mut self, request: AddRequest) {
        self.busy = Some(BusyKind::Add);
        self.spawn(BusyKind::Add, move |root| {
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
        self.spawn(BusyKind::Remove, move |root| {
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
        self.spawn(kind, |root| {
            list_sources(&root).map_err(|error| error.to_string())
        });
    }

    fn spawn(
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

fn open_store(root: &Path) -> Result<StateStore, String> {
    StateStore::open(root.join(".switchyard/state.sqlite3"))
        .map(|value| value.0)
        .map_err(|error| error.to_string())
}

fn list_sources(root: &Path) -> Result<Vec<RegisteredSourceInspection>, Box<dyn Error>> {
    let store = StateStore::open(root.join(".switchyard/state.sqlite3"))?.0;
    Ok(switchyard_sources::SourceManager::new(root).list(&store)?)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        assert_eq!(app.overlay, Overlay::None);
        assert!(app.busy.is_none());
    }

    #[test]
    fn submitted_invalid_form_keeps_inline_error() {
        let mut app = App::with_sources(PathBuf::from("."), Vec::new());
        app.overlay = Overlay::Add(AddForm::default());
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.tick();
        let Overlay::Add(form) = app.overlay else {
            panic!("add form unexpectedly closed");
        };
        assert!(form.error.unwrap().contains("name"));
    }
}
