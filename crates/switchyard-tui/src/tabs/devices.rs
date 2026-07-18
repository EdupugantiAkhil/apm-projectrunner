use std::path::PathBuf;

use appcui::prelude::*;
use switchyard_state::{DeviceCheckStatus, RegisteredDevice};

use crate::state::ProjectState;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DeviceRowView {
    pub(crate) device_index: Option<usize>,
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) address: String,
    pub(crate) connectivity: String,
    pub(crate) eligibility: String,
    pub(crate) scope: String,
}

impl ListItem for DeviceRowView {
    fn columns_count() -> u16 {
        6
    }

    fn column(index: u16) -> Column {
        match index {
            0 => Column::new("Name", 18, TextAlignment::Left),
            1 => Column::new("Kind", 8, TextAlignment::Left),
            2 => Column::new("Address", 27, TextAlignment::Left),
            3 => Column::new("Connectivity", 15, TextAlignment::Left),
            4 => Column::new("Eligibility", 48, TextAlignment::Left),
            _ => Column::new("Origin / scope", 18, TextAlignment::Left),
        }
    }

    fn render_method(&self, column_index: u16) -> Option<listview::RenderMethod<'_>> {
        let text = match column_index {
            0 => &self.name,
            1 => &self.kind,
            2 => &self.address,
            3 => &self.connectivity,
            4 => &self.eligibility,
            5 => &self.scope,
            _ => return None,
        };
        Some(listview::RenderMethod::Text(text))
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Handles {
    pub(crate) list: Handle<ListView<DeviceRowView>>,
    pub(crate) detail: Handle<TextArea>,
    pub(crate) notice: Handle<Label>,
}

pub(crate) fn add(tab: &mut Tab, index: u32, state: &ProjectState) -> Handles {
    let mut splitter = VSplitter::new(
        0.64,
        layout!("l:0,t:0,r:0,b:3"),
        vsplitter::ResizeBehavior::PreserveAspectRatio,
    );
    splitter.set_min_width(vsplitter::Panel::Left, 62);
    splitter.set_min_width(vsplitter::Panel::Right, 30);
    let mut left = Panel::new("Execution devices", layout!("d:f"));
    let mut list = ListView::new(
        layout!("l:1,t:1,r:1,b:1"),
        // No SearchBar: printable keys must remain available to global bindings.
        listview::Flags::ScrollBars,
    );
    fill_list(&mut list, state, None);
    let list = left.add(list);
    splitter.add(vsplitter::Panel::Left, left);

    let mut right = Panel::new("Device details / last check", layout!("d:f"));
    let detail = right.add(TextArea::new(
        &detail_text(state, &project_rows(state)[0]),
        layout!("l:1,t:1,r:1,b:1"),
        textarea::Flags::ReadOnly | textarea::Flags::ScrollBars,
    ));
    splitter.add(vsplitter::Panel::Right, right);
    tab.add(index, splitter);
    let notice = tab.add(index, Label::new("", layout!("l:1,b:0,r:1,h:2")));
    Handles {
        list,
        detail,
        notice,
    }
}

pub(crate) fn project_rows(state: &ProjectState) -> Vec<DeviceRowView> {
    std::iter::once(DeviceRowView {
        device_index: None,
        name: "local".into(),
        kind: "local".into(),
        address: "this device".into(),
        connectivity: "available".into(),
        eligibility: "eligible".into(),
        scope: "implicit / project".into(),
    })
    .chain(state.devices.iter().enumerate().map(|(index, projection)| {
        let device = &projection.device;
        DeviceRowView {
            device_index: Some(index),
            name: projection.name.clone(),
            kind: "ssh".into(),
            address: format!("{}@{}:{}", device.user, device.host, device.port),
            connectivity: connectivity_text(device.last_check_status).into(),
            eligibility: eligibility_text(device),
            scope: "registered / project".into(),
        }
    }))
    .collect()
}

fn connectivity_text(status: DeviceCheckStatus) -> &'static str {
    match status {
        DeviceCheckStatus::Eligible | DeviceCheckStatus::Ineligible | DeviceCheckStatus::Ok => {
            "reachable"
        }
        DeviceCheckStatus::Unreachable => "unreachable",
        DeviceCheckStatus::AuthFailed => "authentication failed",
        DeviceCheckStatus::Never => "unchecked",
    }
}

pub(crate) fn eligibility_text(device: &RegisteredDevice) -> String {
    if device.last_check_status == DeviceCheckStatus::Eligible {
        "eligible".into()
    } else {
        let reason = switchyard_ops::devices::eligibility_label(device);
        format!("ineligible: {reason}")
    }
}

pub(crate) fn fill_list(
    list: &mut ListView<DeviceRowView>,
    state: &ProjectState,
    _preferred: Option<Option<usize>>,
) {
    let rows = project_rows(state);
    list.clear();
    for row in rows {
        list.add(row);
    }
}

pub(crate) fn detail_text(state: &ProjectState, row: &DeviceRowView) -> String {
    let Some(index) = row.device_index else {
        return "Name: local\nKind: local\nConnectivity: available\nEligibility: eligible\nScope: implicit project device\n\nThe local device is always listed first and cannot be removed. No SSH check is required.".into();
    };
    let Some(device) = state.devices.get(index).map(|item| &item.device) else {
        return "This device is no longer present. Press F5 to refresh.".into();
    };
    format!(
        "Name: {}\nKind: ssh\nAddress: {}@{}:{}\nIdentity: {}\nConnectivity: {}\nEligibility: {}\nScope: registered in this project\nLast checked: {}\n\nLast check output:\n{}",
        device.name,
        device.user,
        device.host,
        device.port,
        device
            .identity_file
            .as_ref()
            .map_or_else(|| "SSH agent/config".into(), |p| p.display().to_string()),
        connectivity_text(device.last_check_status),
        eligibility_text(device),
        device
            .last_checked_at
            .map_or_else(|| "never".into(), |value| value.to_string()),
        device.last_check_detail.as_deref().unwrap_or(
            "No check has been run. Press F6 to check SSH connectivity and Docker eligibility."
        ),
    )
}

#[ModalWindow(events = ButtonEvents + WindowEvents, response = RegisteredDevice)]
pub(crate) struct DeviceDialog {
    name: Handle<TextField>,
    user: Handle<TextField>,
    host: Handle<TextField>,
    port: Handle<TextField>,
    identity: Handle<TextField>,
    error: Handle<Label>,
    check: Handle<Button>,
    cancel: Handle<Button>,
}

impl DeviceDialog {
    pub(crate) fn new() -> Self {
        let mut dialog = Self {
            base: ModalWindow::new(
                "Add SSH device",
                layout!("a:c,w:78,h:23"),
                window::Flags::None,
            ),
            name: Handle::None,
            user: Handle::None,
            host: Handle::None,
            port: Handle::None,
            identity: Handle::None,
            error: Handle::None,
            check: Handle::None,
            cancel: Handle::None,
        };
        dialog.add(Label::new(
            "Uses existing SSH keys or agent; passwords and key material are never stored.",
            layout!("l:2,t:1,r:2,h:2"),
        ));
        // Explicit layouts keep validation text near the form and avoid a dynamic form engine.
        dialog.name = dialog.add(TextField::new(
            "",
            layout!("l:27,t:4,r:2,h:1"),
            textfield::Flags::None,
        ));
        dialog.user = dialog.add(TextField::new(
            "",
            layout!("l:27,t:7,r:2,h:1"),
            textfield::Flags::None,
        ));
        dialog.host = dialog.add(TextField::new(
            "",
            layout!("l:27,t:10,r:2,h:1"),
            textfield::Flags::None,
        ));
        dialog.port = dialog.add(TextField::new(
            "22",
            layout!("l:27,t:13,r:2,h:1"),
            textfield::Flags::None,
        ));
        dialog.identity = dialog.add(TextField::new(
            "",
            layout!("l:27,t:16,r:2,h:1"),
            textfield::Flags::None,
        ));
        dialog.add(Label::new("Name", layout!("l:2,t:4,w:23,h:1")));
        dialog.add(Label::new("SSH user", layout!("l:2,t:7,w:23,h:1")));
        dialog.add(Label::new("Host", layout!("l:2,t:10,w:23,h:1")));
        dialog.add(Label::new("Port", layout!("l:2,t:13,w:23,h:1")));
        dialog.add(Label::new(
            "Identity file (optional)",
            layout!("l:2,t:16,w:23,h:1"),
        ));
        dialog.error = dialog.add(Label::new("", layout!("l:27,t:18,r:2,h:2")));
        dialog.check = dialog.add(Button::new("&Check", layout!("x:40%,y:100%,p:b,w:14,h:1")));
        dialog.cancel = dialog.add(Button::new("&Cancel", layout!("x:62%,y:100%,p:b,w:14,h:1")));
        dialog
    }

    fn submit(&mut self) {
        let text = |dialog: &Self, handle| {
            dialog
                .control(handle)
                .map_or("", TextField::text)
                .trim()
                .to_owned()
        };
        let name = text(self, self.name);
        let user = text(self, self.user);
        let host = text(self, self.host);
        let port = text(self, self.port)
            .parse::<u16>()
            .map_err(|_| "port must be a number from 1 to 65535".to_owned());
        let identity = text(self, self.identity);
        let result = port.and_then(|port| {
            validate_device_fields(&name, &user, &host)?;
            Ok(RegisteredDevice {
                name,
                user,
                host,
                port,
                identity_file: (!identity.is_empty()).then(|| PathBuf::from(identity)),
                created_at: 0,
                last_checked_at: None,
                last_check_status: DeviceCheckStatus::Never,
                last_check_detail: None,
            })
        });
        match result {
            Ok(device) => self.exit_with(device),
            Err(message) => {
                let error = self.error;
                if let Some(label) = self.control_mut(error) {
                    label.set_caption(&format!("Validation: {message}"));
                }
            }
        }
    }
}

impl ButtonEvents for DeviceDialog {
    fn on_pressed(&mut self, handle: Handle<Button>) -> EventProcessStatus {
        if handle == self.check {
            self.submit();
        } else if handle == self.cancel {
            self.exit();
        } else {
            return EventProcessStatus::Ignored;
        }
        EventProcessStatus::Processed
    }
}
impl WindowEvents for DeviceDialog {
    fn on_accept(&mut self) {
        self.submit();
    }
}

fn validate_device_fields(name: &str, user: &str, host: &str) -> Result<(), String> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err("name may contain only ASCII letters, digits, '.', '-', and '_'".into());
    }
    if user.is_empty()
        || user.chars().any(char::is_whitespace)
        || user.starts_with('-')
        || user.contains('@')
    {
        return Err("user cannot be empty, contain whitespace or '@', or start with '-'".into());
    }
    if host.is_empty() || host.chars().any(char::is_whitespace) || host.starts_with('-') {
        return Err("host cannot be empty, contain whitespace, or start with '-'".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::DeviceProjection;
    use switchyard_state::DeviceCheckStatus;

    #[test]
    fn rows_keep_local_first_and_include_concrete_eligibility() {
        let device = RegisteredDevice {
            name: "builder".into(),
            host: "192.0.2.10".into(),
            port: 2222,
            user: "dev".into(),
            identity_file: None,
            created_at: 1,
            last_checked_at: Some(2),
            last_check_status: DeviceCheckStatus::Ineligible,
            last_check_detail: Some("no docker over SSH: daemon unavailable".into()),
        };
        let state = ProjectState {
            devices: vec![DeviceProjection {
                name: device.name.clone(),
                device,
            }],
            ..Default::default()
        };
        let rows = project_rows(&state);
        assert_eq!(rows[0].name, "local");
        assert_eq!(rows[0].kind, "local");
        assert_eq!(rows[1].connectivity, "reachable");
        assert_eq!(
            rows[1].eligibility,
            "ineligible: no docker over SSH: daemon unavailable"
        );
    }
}
