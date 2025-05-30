use cursive::{view::Resizable, views::{Dialog, LinearLayout, RadioGroup, TextView}, Cursive};

use crate::config_builder::{BridgeMode, CURRENT_CONFIG};

pub fn bridge_mode(s: &mut Cursive) {
    let mode = CURRENT_CONFIG.lock().unwrap().bridge_mode;

    // create the group and buttons
    let mut group = RadioGroup::new()
    .on_change(|_s, mode| {
        // update the current config with the selected mode
        let mut config = CURRENT_CONFIG.lock().unwrap();
        config.bridge_mode = *mode;
    });
    let mut linux_btn  = group.button(BridgeMode::Linux,  "Linux Bridge (2 interfaces) - Please set up the bridge in your netplan");
    let mut xdp_btn    = group.button(BridgeMode::XDP,    "XDP Bridge   (2 interfaces) - Please DO NOT have a bridge in your netplan");
    let mut single_btn = group.button(BridgeMode::Single, "Single Interface (1 interface)");

    // mark the one we want as selected
    match mode {
        BridgeMode::Linux  => linux_btn.select(),
        BridgeMode::XDP    => xdp_btn.select(),
        BridgeMode::Single => single_btn.select(),
    };

    // now add them (in any order) to your layout
    let layout = LinearLayout::vertical()
        .child(TextView::new("Select the bridge mode you want to use:"))
        .child(linux_btn)
        .child(xdp_btn)
        .child(single_btn);

    s.add_layer(
        Dialog::around(layout)
            .title("Select Bridge Mode")
            .button("OK", |s| { s.pop_layer(); })
            .full_screen(),
    );
}
