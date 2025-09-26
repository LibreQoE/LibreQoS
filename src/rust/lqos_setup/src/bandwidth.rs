use cursive::{Cursive, view::Resizable, views::Dialog};

use crate::config_builder::CURRENT_CONFIG;

pub fn bandwidth_view(s: &mut Cursive) {
    let (down_speed, up_speed) = {
        let lock = CURRENT_CONFIG.lock();
        (lock.mbps_to_internet, lock.mbps_to_network)
    };
    let layout = cursive::views::LinearLayout::vertical()
        .child(cursive::views::TextView::new(
            "Set the available bandwidth for each direction:",
        ))
        .child(
            cursive::views::LinearLayout::horizontal()
                .child(cursive::views::TextView::new("To Internet (Mbps):"))
                .child(
                    cursive::views::EditView::new()
                        .content(down_speed.to_string())
                        .on_edit(|s, content, _cursor| {
                            if let Ok(value) = content.parse::<u64>() {
                                let mut config = CURRENT_CONFIG.lock();
                                config.mbps_to_internet = value;
                            } else {
                                s.add_layer(Dialog::info("Invalid bandwidth value"));
                            }
                        })
                        .fixed_width(15),
                ),
        )
        .child(
            cursive::views::LinearLayout::horizontal()
                .child(cursive::views::TextView::new("To Network (Mbps): "))
                .child(
                    cursive::views::EditView::new()
                        .content(up_speed.to_string())
                        .on_edit(|s, content, _cursor| {
                            if let Ok(value) = content.parse::<u64>() {
                                let mut config = CURRENT_CONFIG.lock();
                                config.mbps_to_network = value;
                            } else {
                                s.add_layer(Dialog::info("Invalid bandwidth value"));
                            }
                        })
                        .fixed_width(15),
                ),
        );

    s.add_layer(
        Dialog::around(layout)
            .title("Available Bandwidth")
            .button("OK", |s| {
                s.pop_layer();
            })
            .full_screen(),
    );
}
