use appcui::prelude::*;

pub(crate) fn add(tab: &mut Tab, index: u32) {
    super::add_placeholder(
        tab,
        index,
        "Operations",
        "Operations holds project run actions, the ordered activity timeline, and streaming logs. These controls arrive in part 6.",
    );
}
