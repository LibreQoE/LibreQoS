use cursive::{
    Cursive,
    view::Resizable,
    views::{Dialog, LinearLayout, RadioGroup, TextView},
};

use crate::config_builder::{BridgeMode, CURRENT_CONFIG, existing_config_uses_xdp};

pub fn bridge_mode(s: &mut Cursive) {
    let current_mode = CURRENT_CONFIG.lock().bridge_mode;
    let show_legacy_xdp = existing_config_uses_xdp() || current_mode == BridgeMode::XDP;

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
    let mut xdp_btn = show_legacy_xdp.then(|| {
        group.button(
            BridgeMode::XDP,
            "Legacy XDP Bridge (existing installs only - keep only if you already run it)",
        )
    });
    let mut single_btn = group.button(BridgeMode::Single, "Single Interface (1 interface)");

    // mark the one we want as selected
    match current_mode {
        BridgeMode::Single => {
            single_btn.select();
        }
        BridgeMode::XDP if show_legacy_xdp => {
            if let Some(button) = xdp_btn.as_mut() {
                button.select();
            }
        }
        _ => {
            linux_btn.select();
        }
    }

    // now add them (in any order) to your layout
    let mut layout = LinearLayout::vertical()
        .child(TextView::new("Select the bridge mode you want to use:"))
        .child(linux_btn);
    if let Some(button) = xdp_btn {
        layout.add_child(TextView::new(
            "Legacy XDP mode was detected on this install. New installs should use Linux Bridge; leave XDP selected only if you intend to keep the existing XDP deployment.",
        ));
        layout.add_child(button);
    }
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
