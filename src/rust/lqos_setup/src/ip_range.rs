use cursive::{
    view::{Nameable, Resizable},
    views::{Button, Dialog, EditView, LinearLayout, SelectView, TextView},
    Cursive,
};
use ip_network::IpNetwork;
use crate::config_builder::CURRENT_CONFIG;

/// Shows and manages the list of allowed IP ranges.
pub fn ranges(s: &mut Cursive) {
    let initial_ranges = {
        let config = CURRENT_CONFIG.lock().unwrap();
        config.allow_subnets.clone()
    };

    let select_view = SelectView::<String>::new()
        .with_all(initial_ranges.iter().map(|range| (range.clone(), range.clone())))
        .on_submit(|_s, range: &str| {
            let mut config = CURRENT_CONFIG.lock().unwrap();
            config.allow_subnets.push(range.parse().unwrap());
        })
        .with_name("ip_ranges")
        .fixed_width(30);

    let layout = LinearLayout::horizontal()
        .child(LinearLayout::vertical()
            .child(TextView::new("Allowed IP Ranges:"))
            .child(select_view)
            .child(Button::new("Remove Selected", |s| {
                    s.call_on_name("ip_ranges", |view: &mut SelectView<String>| {
                        if let Some(selected) = view.selected_id() {
                            let mut config = CURRENT_CONFIG.lock().unwrap();
                            config.allow_subnets.remove(selected);
                            view.remove_item(selected);
                        }
                    });
                })
            )
        )
        .child(TextView::new(" ")) // 1-character spacer between columns
        .child(LinearLayout::vertical()
            .child(TextView::new("Add New Range:"))
            .child(
                EditView::new()
                    .on_submit(|s, content| {
                        let parsed = content.parse::<IpNetwork>();
                        if parsed.is_ok() {
                            let range = content.to_string();
                            let mut config = CURRENT_CONFIG.lock().unwrap();
                            config.allow_subnets.push(range);
                            s.call_on_name("ip_ranges", |view: &mut SelectView<String>| {
                                view.add_item(content.to_string(), content.to_string());
                            });
                        } else {
                            s.add_layer(Dialog::info("Invalid IP range format. Use CIDR notation, e.g., 192.168.0.0/16"));
                        }
                    })
                    .fixed_width(20)
            )
            .child(TextView::new("Press Enter to add the range"))
        );

        s.add_layer(
            Dialog::around(layout)
                .title("Allowed IP Ranges")
                .button("OK", |s| { s.pop_layer(); })
                .full_screen()
        );
}