use appcui::prelude::*;

pub(crate) fn add(tab: &mut Tab, index: u32) {
    super::add_placeholder(
        tab,
        index,
        "Startup profiles",
        "Startup profiles define the reusable services that an instance runs. Profile discovery, validation, import, and editing arrive in part 3.",
    );
}
