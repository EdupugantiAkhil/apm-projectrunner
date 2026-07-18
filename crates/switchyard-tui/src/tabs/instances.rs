use appcui::prelude::*;

pub(crate) fn add(tab: &mut Tab, index: u32) {
    super::add_placeholder(
        tab,
        index,
        "Instances",
        "Instances combine one checkout, startup profile, device, name, and parameters. Creation and lifecycle controls arrive in part 4.",
    );
}
