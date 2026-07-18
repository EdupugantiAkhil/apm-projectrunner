use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use appcui::prelude::*;
use switchyard_state::RegisteredSourceKind;

use crate::{
    handoff::CloneHandoff,
    state::{ProjectState, SourceProjection},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TreeRow {
    pub(crate) source_index: usize,
    pub(crate) parent: Option<usize>,
    pub(crate) name: String,
    pub(crate) reference: String,
    pub(crate) commit: String,
    pub(crate) dirty: String,
    pub(crate) availability: String,
}

impl ListItem for TreeRow {
    fn columns_count() -> u16 {
        5
    }

    fn column(index: u16) -> Column {
        match index {
            0 => Column::new("Name", 24, TextAlignment::Left),
            1 => Column::new("Branch / ref", 20, TextAlignment::Left),
            2 => Column::new("Commit", 12, TextAlignment::Left),
            3 => Column::new("Dirty state", 18, TextAlignment::Left),
            _ => Column::new("Availability", 14, TextAlignment::Left),
        }
    }

    fn render_method(&self, column_index: u16) -> Option<listview::RenderMethod<'_>> {
        let text = match column_index {
            0 => &self.name,
            1 => &self.reference,
            2 => &self.commit,
            3 => &self.dirty,
            4 => &self.availability,
            _ => return None,
        };
        Some(listview::RenderMethod::Text(text))
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Handles {
    pub(crate) tree: Handle<TreeView<TreeRow>>,
    pub(crate) detail: Handle<Label>,
    pub(crate) empty: Handle<Label>,
    pub(crate) notice: Handle<Label>,
}

pub(crate) fn add(
    tab: &mut Tab,
    index: u32,
    state: &ProjectState,
    notice: Option<&str>,
) -> Handles {
    let mut splitter = VSplitter::new(
        0.62,
        layout!("l:0,t:0,r:0,b:3"),
        vsplitter::ResizeBehavior::PreserveAspectRatio,
    );
    splitter.set_min_width(vsplitter::Panel::Left, 44);
    splitter.set_min_width(vsplitter::Panel::Right, 28);

    let mut left = Panel::new("Repositories and checkouts", layout!("d:f"));
    let mut tree = TreeView::<TreeRow>::new(
        layout!("l:1,t:1,r:1,b:1"),
        // No SearchBar: it consumes every printable key (including the
        // character half of Ctrl+Q), breaking global bindings while focused.
        treeview::Flags::ScrollBars,
    );
    fill_tree(&mut tree, &state.sources, None);
    let tree_handle = left.add(tree);
    let mut empty = Label::new(
        "Code is a repository, checkout, or worktree that Switchyard can use for an instance.\n\nPress F2 to register an existing directory or clone a repository.",
        layout!("l:3,t:3,r:3,h:6"),
    );
    empty.set_visible(state.sources.is_empty());
    let empty_handle = left.add(empty);
    splitter.add(vsplitter::Panel::Left, left);

    let mut right = Panel::new("Selection details", layout!("d:f"));
    let detail_text = state.sources.first().map_or_else(
        || "Select a source to inspect its identity and ownership.".into(),
        |source| detail_text(state, source),
    );
    let detail_handle = right.add(Label::new(&detail_text, layout!("l:2,t:2,r:2,b:2")));
    splitter.add(vsplitter::Panel::Right, right);
    tab.add(index, splitter);

    let notice_handle = tab.add(
        index,
        Label::new(notice.unwrap_or(""), layout!("l:1,b:0,r:1,h:2")),
    );
    Handles {
        tree: tree_handle,
        detail: detail_handle,
        empty: empty_handle,
        notice: notice_handle,
    }
}

pub(crate) fn fill_tree(
    tree: &mut TreeView<TreeRow>,
    sources: &[SourceProjection],
    preferred_source: Option<usize>,
) {
    tree.clear();
    let rows = project_tree_rows(sources);
    let mut handles: Vec<Handle<treeview::Item<TreeRow>>> = Vec::with_capacity(rows.len());
    let mut preferred_handle = None;
    tree.add_batch(|tree| {
        for row in rows {
            let preferred = preferred_source == Some(row.source_index);
            let parent = row
                .parent
                .and_then(|index| handles.as_slice().get(index).copied());
            let handle = if let Some(parent) = parent {
                tree.add_to_parent(row, parent)
            } else {
                tree.add_item(treeview::Item::expandable(row, false))
            };
            if preferred {
                preferred_handle = Some(handle);
            }
            handles.push(handle);
        }
    });
    if let Some(handle) = preferred_handle {
        tree.move_cursor_to(handle);
    }
}

pub(crate) fn project_tree_rows(sources: &[SourceProjection]) -> Vec<TreeRow> {
    let mut result = Vec::with_capacity(sources.len());
    let mut parents = BTreeMap::<PathBuf, usize>::new();
    for (source_index, source) in sources
        .iter()
        .enumerate()
        .filter(|(_, source)| source.inspection.linked_worktree != Some(true))
    {
        let row_index = result.len();
        parents.insert(source.path.clone(), row_index);
        result.push(row(source_index, None, source));
    }
    for (source_index, source) in sources
        .iter()
        .enumerate()
        .filter(|(_, source)| source.inspection.linked_worktree == Some(true))
    {
        let parent = source
            .repository_path
            .as_ref()
            .and_then(|path| parents.get(path))
            .copied();
        result.push(row(source_index, parent, source));
    }
    result
}

fn row(source_index: usize, parent: Option<usize>, source: &SourceProjection) -> TreeRow {
    let reference = source
        .inspection
        .branch
        .clone()
        .or_else(|| source.inspection.identity.r#ref.clone())
        .or_else(|| source.requested_ref.clone())
        .unwrap_or_else(|| "unknown".into());
    let commit = source
        .inspection
        .identity
        .commit
        .as_deref()
        .map(|commit| commit.chars().take(10).collect())
        .unwrap_or_else(|| "unknown".into());
    let dirty = match &source.inspection.changes {
        Some(changes) if changes.is_dirty() => format!(
            "✗ dirty {}/{}/{}",
            changes.staged, changes.unstaged, changes.untracked
        ),
        Some(_) => "clean".into(),
        None => format!(
            "unknown: {}",
            source
                .inspection
                .unknown_code
                .as_deref()
                .unwrap_or("not inspected")
        ),
    };
    TreeRow {
        source_index,
        parent,
        name: source.name.clone(),
        reference,
        commit,
        dirty,
        availability: if source.available {
            "available".into()
        } else {
            "missing".into()
        },
    }
}

pub(crate) fn detail_text(state: &ProjectState, source: &SourceProjection) -> String {
    let remote = source.remote.as_deref().unwrap_or("not configured");
    let ownership = match source.ownership {
        RegisteredSourceKind::Managed => "managed by Switchyard",
        RegisteredSourceKind::Unmanaged => "unmanaged (registration only)",
    };
    let linked = state
        .deployments
        .iter()
        .flat_map(|deployment| {
            deployment.instances.iter().filter_map(move |instance| {
                (instance.source == source.name)
                    .then(|| format!("{}/{}", deployment.name, instance.name))
            })
        })
        .collect::<Vec<_>>();
    format!(
        "Name: {}\nPath: {}\nRemote: {}\nOwnership: {}\nLast inspection: {}\nAvailability: {}\nLinked instances: {}",
        source.name,
        source.path.display(),
        remote,
        ownership,
        inspection_age(source.inspected_at),
        if source.available {
            "available"
        } else {
            "missing"
        },
        if linked.is_empty() {
            "none".into()
        } else {
            linked.join(", ")
        },
    )
}

fn inspection_age(inspected_at_millis: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let seconds = (now - inspected_at_millis).max(0) / 1000;
    match seconds {
        0..=59 => "moments ago (this refresh)".into(),
        60..=3599 => format!("{} minutes ago", seconds / 60),
        3600..=86_399 => format!("{} hours ago", seconds / 3600),
        _ => format!("{} days ago", seconds / 86_400),
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum AddSourceMode {
    #[default]
    Local,
    Git,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct AddForm {
    pub(crate) mode: AddSourceMode,
    pub(crate) location: String,
    pub(crate) destination: String,
    pub(crate) git_ref: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AddRequest {
    Local { name: String, path: PathBuf },
    Clone(CloneHandoff),
}

impl AddForm {
    pub(crate) fn validate(&self, project_dir: &Path) -> Result<AddRequest, String> {
        let location = self.location.trim();
        if location.is_empty() {
            return Err(match self.mode {
                AddSourceMode::Local => "enter an existing local directory",
                AddSourceMode::Git => "enter a Git HTTPS, SSH, or local repository URL",
            }
            .into());
        }
        match self.mode {
            AddSourceMode::Local => {
                let path = PathBuf::from(location);
                let checked = if path.is_absolute() {
                    path.clone()
                } else {
                    project_dir.join(&path)
                };
                if !checked.is_dir() {
                    return Err(format!("directory `{}` does not exist", checked.display()));
                }
                Ok(AddRequest::Local {
                    name: infer_source_name(location),
                    path,
                })
            }
            AddSourceMode::Git => {
                if location.starts_with('-') {
                    return Err("Git clone address may not start with '-'".into());
                }
                if has_embedded_http_credentials(location) {
                    return Err("do not embed HTTPS credentials in the clone address; use your Git credential helper".into());
                }
                let destination = self.destination.trim();
                let name = if destination.is_empty() {
                    infer_source_name(location)
                } else {
                    destination.to_owned()
                };
                validate_name(&name)?;
                Ok(AddRequest::Clone(CloneHandoff {
                    name,
                    url: location.into(),
                    git_ref: nonempty(self.git_ref.trim()),
                }))
            }
        }
    }
}

#[ModalWindow(events = ButtonEvents + ComboBoxEvents + WindowEvents, response = AddRequest)]
pub(crate) struct AddDialog {
    mode: Handle<ComboBox>,
    location: Handle<TextField>,
    location_label: Handle<Label>,
    destination: Handle<TextField>,
    git_ref: Handle<TextField>,
    destination_label: Handle<Label>,
    ref_label: Handle<Label>,
    error: Handle<Label>,
    submit: Handle<Button>,
    cancel: Handle<Button>,
    project_dir: PathBuf,
}

impl AddDialog {
    pub(crate) fn new(project_dir: &Path) -> Self {
        let mut dialog = Self {
            base: ModalWindow::new("Add code", layout!("a:c,w:76,h:20"), window::Flags::None),
            mode: Handle::None,
            location: Handle::None,
            location_label: Handle::None,
            destination: Handle::None,
            git_ref: Handle::None,
            destination_label: Handle::None,
            ref_label: Handle::None,
            error: Handle::None,
            submit: Handle::None,
            cancel: Handle::None,
            project_dir: project_dir.to_path_buf(),
        };
        dialog.add(Label::new("Mode", layout!("l:2,t:1,w:16,h:1")));
        let mut mode = ComboBox::new(layout!("l:20,t:1,r:2,h:1"), combobox::Flags::None);
        mode.add("Register existing local directory");
        mode.add("Clone repository");
        dialog.mode = dialog.add(mode);
        dialog.location_label = dialog.add(Label::new("Directory", layout!("l:2,t:4,w:16,h:1")));
        dialog.location = dialog.add(TextField::new(
            "",
            layout!("l:20,t:4,r:2,h:1"),
            textfield::Flags::None,
        ));
        dialog.destination_label =
            dialog.add(Label::new("Destination name", layout!("l:2,t:7,w:16,h:1")));
        dialog.destination = dialog.add(TextField::new(
            "",
            layout!("l:20,t:7,r:2,h:1"),
            textfield::Flags::None,
        ));
        dialog.ref_label = dialog.add(Label::new(
            "Git ref (optional)",
            layout!("l:2,t:10,w:17,h:1"),
        ));
        dialog.git_ref = dialog.add(TextField::new(
            "",
            layout!("l:20,t:10,r:2,h:1"),
            textfield::Flags::None,
        ));
        dialog.error = dialog.add(Label::new("", layout!("l:20,t:12,r:2,h:2")));
        dialog.submit = dialog.add(Button::new("&Add", layout!("x:40%,y:100%,p:b,w:14,h:1")));
        dialog.cancel = dialog.add(Button::new("&Cancel", layout!("x:62%,y:100%,p:b,w:14,h:1")));
        dialog.update_mode();
        dialog
    }

    fn update_mode(&mut self) {
        let git = self.control(self.mode).and_then(ComboBox::index) == Some(1);
        let destination = self.destination;
        let git_ref = self.git_ref;
        let destination_label = self.destination_label;
        let ref_label = self.ref_label;
        let location_label = self.location_label;
        if let Some(label) = self.control_mut(location_label) {
            label.set_caption(if git { "Clone address" } else { "Directory" });
        }
        if let Some(control) = self.control_mut(destination) {
            control.set_visible(git);
        }
        if let Some(control) = self.control_mut(git_ref) {
            control.set_visible(git);
        }
        if let Some(control) = self.control_mut(destination_label) {
            control.set_visible(git);
        }
        if let Some(control) = self.control_mut(ref_label) {
            control.set_visible(git);
        }
    }

    fn form(&self) -> AddForm {
        AddForm {
            mode: if self.control(self.mode).and_then(ComboBox::index) == Some(1) {
                AddSourceMode::Git
            } else {
                AddSourceMode::Local
            },
            location: self
                .control(self.location)
                .map_or("", TextField::text)
                .into(),
            destination: self
                .control(self.destination)
                .map_or("", TextField::text)
                .into(),
            git_ref: self
                .control(self.git_ref)
                .map_or("", TextField::text)
                .into(),
        }
    }

    fn submit(&mut self) {
        match self.form().validate(&self.project_dir) {
            Ok(request) => self.exit_with(request),
            Err(error) => {
                let error_handle = self.error;
                if let Some(label) = self.control_mut(error_handle) {
                    label.set_caption(&format!("Validation: {error}"));
                }
            }
        }
    }
}

impl ComboBoxEvents for AddDialog {
    fn on_selection_changed(&mut self, handle: Handle<ComboBox>) -> EventProcessStatus {
        if handle == self.mode {
            self.update_mode();
            EventProcessStatus::Processed
        } else {
            EventProcessStatus::Ignored
        }
    }
}

impl ButtonEvents for AddDialog {
    fn on_pressed(&mut self, handle: Handle<Button>) -> EventProcessStatus {
        if handle == self.submit {
            self.submit();
        } else if handle == self.cancel {
            self.exit();
        } else {
            return EventProcessStatus::Ignored;
        }
        EventProcessStatus::Processed
    }
}

impl WindowEvents for AddDialog {
    fn on_accept(&mut self) {
        self.submit();
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorktreeRequest {
    pub(crate) source: String,
    pub(crate) base_ref: String,
    pub(crate) branch: String,
    pub(crate) name: String,
    pub(crate) path: Option<PathBuf>,
}

#[ModalWindow(events = ButtonEvents + ComboBoxEvents + WindowEvents, response = WorktreeRequest)]
pub(crate) struct WorktreeDialog {
    repo: Handle<ComboBox>,
    base_ref: Handle<TextField>,
    branch: Handle<TextField>,
    path: Handle<TextField>,
    error: Handle<Label>,
    create: Handle<Button>,
    cancel: Handle<Button>,
    repositories: Vec<(String, String)>,
}

impl WorktreeDialog {
    pub(crate) fn new(
        sources: &[SourceProjection],
        selected: Option<&SourceProjection>,
    ) -> Option<Self> {
        let repositories = sources
            .iter()
            .filter_map(|source| {
                source
                    .inspection
                    .identity
                    .commit
                    .as_ref()
                    .map(|commit| (source.name.clone(), commit.clone()))
            })
            .collect::<Vec<_>>();
        if repositories.is_empty() {
            return None;
        }
        let selected_name = selected.map(|source| source.name.as_str());
        let selected_index = repositories
            .iter()
            .position(|(name, _)| Some(name.as_str()) == selected_name)
            .unwrap_or(0);
        let mut dialog = Self {
            base: ModalWindow::new(
                "Create managed worktree",
                layout!("a:c,w:78,h:22"),
                window::Flags::None,
            ),
            repo: Handle::None,
            base_ref: Handle::None,
            branch: Handle::None,
            path: Handle::None,
            error: Handle::None,
            create: Handle::None,
            cancel: Handle::None,
            repositories,
        };
        dialog.add(Label::new("Repository", layout!("l:2,t:1,w:16,h:1")));
        let mut repo = ComboBox::new(layout!("l:20,t:1,r:2,h:1"), combobox::Flags::None);
        for (name, _) in &dialog.repositories {
            repo.add(name);
        }
        repo.set_index(selected_index as u32);
        dialog.repo = dialog.add(repo);
        dialog.add(Label::new("Base ref / commit", layout!("l:2,t:4,w:17,h:1")));
        let base = dialog.repositories[selected_index].1.clone();
        dialog.base_ref = dialog.add(TextField::new(
            &base,
            layout!("l:20,t:4,r:2,h:1"),
            textfield::Flags::None,
        ));
        dialog.add(Label::new("New branch", layout!("l:2,t:7,w:16,h:1")));
        dialog.branch = dialog.add(TextField::new(
            "",
            layout!("l:20,t:7,r:2,h:1"),
            textfield::Flags::None,
        ));
        dialog.add(Label::new("Managed path", layout!("l:2,t:10,w:16,h:1")));
        dialog.path = dialog.add(TextField::new(
            "",
            layout!("l:20,t:10,r:2,h:1"),
            textfield::Flags::None,
        ));
        dialog.add(Label::new(
            "Leave path empty for .switchyard/worktrees/<branch>.",
            layout!("l:20,t:12,r:2,h:1"),
        ));
        dialog.error = dialog.add(Label::new("", layout!("l:20,t:14,r:2,h:2")));
        dialog.create = dialog.add(Button::new("&Create", layout!("x:40%,y:100%,p:b,w:14,h:1")));
        dialog.cancel = dialog.add(Button::new("&Cancel", layout!("x:62%,y:100%,p:b,w:14,h:1")));
        Some(dialog)
    }

    fn submit(&mut self) {
        let source = self
            .control(self.repo)
            .and_then(ComboBox::index)
            .and_then(|index| self.repositories.get(index as usize))
            .map(|(name, _)| name.clone());
        let base_ref = self
            .control(self.base_ref)
            .map_or("", TextField::text)
            .trim()
            .to_owned();
        let branch = self
            .control(self.branch)
            .map_or("", TextField::text)
            .trim()
            .to_owned();
        let path_text = self
            .control(self.path)
            .map_or("", TextField::text)
            .trim()
            .to_owned();
        let result = source
            .ok_or_else(|| "select a repository".to_owned())
            .and_then(|source| {
                if base_ref.is_empty() {
                    return Err("enter a base ref or commit".into());
                }
                validate_name(&branch)?;
                let name = if path_text.is_empty() {
                    branch.clone()
                } else {
                    infer_source_name(&path_text)
                };
                Ok(WorktreeRequest {
                    source,
                    base_ref,
                    branch,
                    name,
                    path: nonempty(&path_text).map(PathBuf::from),
                })
            });
        match result {
            Ok(request) => self.exit_with(request),
            Err(error) => {
                let error_handle = self.error;
                if let Some(label) = self.control_mut(error_handle) {
                    label.set_caption(&format!("Validation: {error}"));
                }
            }
        }
    }
}

impl ComboBoxEvents for WorktreeDialog {
    fn on_selection_changed(&mut self, handle: Handle<ComboBox>) -> EventProcessStatus {
        if handle != self.repo {
            return EventProcessStatus::Ignored;
        }
        let commit = self
            .control(self.repo)
            .and_then(ComboBox::index)
            .and_then(|index| self.repositories.get(index as usize))
            .map(|(_, commit)| commit.clone());
        let base_ref = self.base_ref;
        if let (Some(commit), Some(field)) = (commit, self.control_mut(base_ref)) {
            field.set_text(&commit);
        }
        EventProcessStatus::Processed
    }
}

impl ButtonEvents for WorktreeDialog {
    fn on_pressed(&mut self, handle: Handle<Button>) -> EventProcessStatus {
        if handle == self.create {
            self.submit();
        } else if handle == self.cancel {
            self.exit();
        } else {
            return EventProcessStatus::Ignored;
        }
        EventProcessStatus::Processed
    }
}

impl WindowEvents for WorktreeDialog {
    fn on_accept(&mut self) {
        self.submit();
    }
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || matches!(name, "." | "..")
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        Err("name may contain only ASCII letters, digits, '.', '-', and '_'".into())
    } else {
        Ok(())
    }
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
    let mut separator = false;
    for character in candidate.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
            result.push(character);
            separator = false;
        } else if !result.is_empty() && !separator {
            result.push('-');
            separator = true;
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

fn has_embedded_http_credentials(location: &str) -> bool {
    let lowercase = location.to_ascii_lowercase();
    ["https://", "http://"].iter().any(|scheme| {
        lowercase
            .strip_prefix(scheme)
            .and_then(|rest| rest.split('/').next())
            .is_some_and(|authority| authority.contains('@'))
    })
}

fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use switchyard_sources::DirtyState;

    fn source(
        name: &str,
        path: &str,
        repository: Option<&str>,
        worktree: bool,
        dirty: bool,
    ) -> SourceProjection {
        let mut inspection = SourceProjection::default().inspection;
        inspection.identity.path = path.into();
        inspection.identity.repository = repository.map(str::to_owned);
        inspection.identity.r#ref = Some("refs/heads/main".into());
        inspection.identity.commit = Some("0123456789abcdef".into());
        inspection.identity.dirty = Some(dirty);
        inspection.linked_worktree = Some(worktree);
        inspection.branch = Some(if worktree { "feature" } else { "main" }.into());
        inspection.detached = Some(false);
        inspection.changes = Some(if dirty {
            DirtyState {
                staged: 1,
                unstaged: 2,
                untracked: 3,
            }
        } else {
            DirtyState::default()
        });
        inspection.ahead = None;
        inspection.behind = None;
        inspection.unknown_code = None;
        SourceProjection {
            name: name.into(),
            path: path.into(),
            repository_path: repository.map(Into::into),
            requested_ref: None,
            remote: Some("git@example.test:team/repo.git".into()),
            ownership: RegisteredSourceKind::Managed,
            inspection,
            available: true,
            inspected_at: 42,
        }
    }

    #[test]
    fn tree_projection_nests_worktrees_and_keeps_textual_state() {
        let sources = vec![
            source("repo", "/repo", Some("/repo"), false, false),
            source("feature", "/worktrees/feature", Some("/repo"), true, true),
        ];
        let rows = project_tree_rows(&sources);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].parent, None);
        assert_eq!(rows[1].parent, Some(0));
        assert_eq!(rows[0].commit, "0123456789");
        assert_eq!(rows[0].dirty, "clean");
        assert_eq!(rows[1].dirty, "✗ dirty 1/2/3");
        assert_eq!(rows[1].availability, "available");
    }

    #[test]
    fn local_add_requires_an_existing_directory() {
        let form = AddForm {
            location: "missing".into(),
            ..AddForm::default()
        };
        assert!(
            form.validate(Path::new("/definitely/not/here"))
                .unwrap_err()
                .contains("does not exist")
        );
    }

    #[test]
    fn clone_validation_rejects_options_and_embedded_credentials() {
        let mut form = AddForm {
            mode: AddSourceMode::Git,
            location: "-upload-pack=oops".into(),
            ..AddForm::default()
        };
        assert!(
            form.validate(Path::new("/project"))
                .unwrap_err()
                .contains("may not start")
        );
        form.location = "https://user:secret@example.test/repo.git".into();
        assert!(
            form.validate(Path::new("/project"))
                .unwrap_err()
                .contains("credential helper")
        );
    }

    #[test]
    fn clone_validation_derives_or_accepts_a_safe_destination_name() {
        let form = AddForm {
            mode: AddSourceMode::Git,
            location: "git@example.test:team/api.git".into(),
            git_ref: "feature/demo".into(),
            ..AddForm::default()
        };
        assert!(
            matches!(form.validate(Path::new("/project")), Ok(AddRequest::Clone(CloneHandoff { name, git_ref: Some(reference), .. })) if name == "api" && reference == "feature/demo")
        );
    }
}
