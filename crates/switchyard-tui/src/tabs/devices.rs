use appcui::prelude::*;

pub(crate) fn add(tab: &mut Tab, index: u32) {
    super::add_placeholder(
        tab,
        index,
        "Devices",
        "Devices are the local or SSH hosts where instances may run, with explicit connectivity and eligibility. Device management arrives in part 6.",
    );
}
