use cursive::{
    Cursive,
    view::Resizable,
    views::{Dialog, LinearLayout, RadioGroup, TextView},
};

use crate::config_builder::{BridgeMode, CURRENT_CONFIG};

pub fn bridge_mode(s: &mut Cursive) {
    let current_mode = CURRENT_CONFIG.lock().bridge_mode;

    // create the group and buttons
    let mut group = RadioGroup::new().on_change(|_s, mode| {
        // update the current config with the selected mode
        let mut config = CURRENT_CONFIG.lock();
        config.bridge_mode = *mode;
    });
    let mut linux_btn = group.button(
        BridgeMode::Linux,
        "Linux Bridge (2 interfaces) - LibreQoS will inspect and stage the managed Netplan change",
    );
    let mut xdp_btn = group.button(
        BridgeMode::XDP,
        "XDP Bridge (2 interfaces; supported bond masters allowed)",
    );
    let mut single_btn = group.button(BridgeMode::Single, "Single Interface (1 interface)");

    // mark the one we want as selected
    match current_mode {
        BridgeMode::Single => {
            single_btn.select();
        }
        BridgeMode::XDP => {
            xdp_btn.select();
        }
        _ => {
            linux_btn.select();
        }
    }

    // now add them (in any order) to your layout
    let mut layout = LinearLayout::vertical()
        .child(TextView::new("Select the bridge mode you want to use:"))
        .child(linux_btn)
        .child(xdp_btn)
        .child(TextView::new(
            "XDP supports bond masters in native-XDP modes. Configure bonds in Netplan first and select the master, not a member.",
        ));
    layout.add_child(single_btn);

    s.add_layer(
        Dialog::around(layout)
            .title("Select Bridge Mode")
            .button("OK", |s| {
                s.pop_layer();
            })
            .full_screen(),
    );
}
