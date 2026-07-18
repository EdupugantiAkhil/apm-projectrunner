use appcui::prelude::*;

#[ModalWindow(events = ButtonEvents, response = bool)]
struct DestructiveConfirm {
    remove: Handle<Button>,
    cancel: Handle<Button>,
}

impl DestructiveConfirm {
    fn new(title: &str, preview: &str) -> Self {
        let mut dialog = Self {
            base: ModalWindow::new(title, layout!("a:c,w:72,h:13"), window::Flags::None),
            remove: Handle::None,
            cancel: Handle::None,
        };
        dialog.add(Label::new(
            &format!("WARNING — destructive action\n\n{preview}"),
            layout!("l:2,t:1,r:2,h:6"),
        ));
        dialog.remove = dialog.add(Button::new("&Remove", layout!("x:35%,y:100%,p:b,w:16,h:1")));
        dialog.cancel = dialog.add(Button::new("&Cancel", layout!("x:65%,y:100%,p:b,w:16,h:1")));
        let cancel = dialog.cancel;
        dialog.request_focus_for_control(cancel);
        dialog
    }
}

impl ButtonEvents for DestructiveConfirm {
    fn on_pressed(&mut self, handle: Handle<Button>) -> EventProcessStatus {
        if handle == self.remove {
            self.exit_with(true);
        } else if handle == self.cancel {
            self.exit_with(false);
        } else {
            return EventProcessStatus::Ignored;
        }
        EventProcessStatus::Processed
    }
}

pub(crate) fn safe_remove(preview: &str) -> bool {
    DestructiveConfirm::new("Confirm safe removal", preview)
        .show()
        .unwrap_or(false)
}
