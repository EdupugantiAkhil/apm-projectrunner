use appcui::prelude::*;

use crate::state::OperationLog;

#[derive(Clone, Copy)]
pub(crate) struct Handles {
    pub(crate) log: Handle<TextArea>,
}

pub(crate) fn add(tab: &mut Tab, index: u32, log: &OperationLog) -> Handles {
    let mut panel = Panel::new("Operations — timeline and streaming output", layout!("d:f"));
    panel.add(Label::new(
        "Run-action management arrives in part 6. Lifecycle output already streams and remains available here.",
        layout!("l:1,t:1,r:1,h:2"),
    ));
    let text = if log.entries().is_empty() {
        "No operations have run in this session. Use F7–F10 from Instances."
    } else {
        &log.render()
    };
    let log = panel.add(TextArea::new(
        text,
        layout!("l:1,t:4,r:1,b:1"),
        textarea::Flags::ReadOnly | textarea::Flags::ScrollBars,
    ));
    tab.add(index, panel);
    Handles { log }
}
