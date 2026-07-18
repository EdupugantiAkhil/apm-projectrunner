use appcui::prelude::*;

pub(crate) fn add(tab: &mut Tab, index: u32) {
    super::add_placeholder(
        tab,
        index,
        "Connections",
        "Connections choose which complete provider group serves each consumer while applications keep fixed addresses. The route matrix arrives in part 5.",
    );
}
