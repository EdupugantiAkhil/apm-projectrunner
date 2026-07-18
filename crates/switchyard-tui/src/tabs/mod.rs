pub(crate) mod code;
pub(crate) mod connections;
pub(crate) mod devices;
pub(crate) mod home;
pub(crate) mod instances;
pub(crate) mod operations;
pub(crate) mod profiles;

use appcui::prelude::*;

pub(crate) fn add_placeholder(tab: &mut Tab, index: u32, title: &str, text: &str) {
    let mut panel = Panel::new(title, layout!("d:f"));
    panel.add(Label::new(text, layout!("l:2,t:2,r:2,h:5")));
    tab.add(index, panel);
}
