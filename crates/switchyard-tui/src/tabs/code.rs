use appcui::prelude::*;

pub(crate) fn add(tab: &mut Tab, index: u32) {
    super::add_placeholder(
        tab,
        index,
        "Code",
        "Code is where repositories, checkouts, and worktrees become available to Switchyard. Adding and inspecting code arrives in part 2 of this rewrite.",
    );
}
