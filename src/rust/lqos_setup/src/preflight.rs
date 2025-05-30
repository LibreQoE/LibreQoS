use cursive::views::{Dialog, LinearLayout, TextView};

use crate::interfaces;


pub fn preflight() {
    let interfaces = interfaces::get_interfaces().unwrap();
    if interfaces.is_empty() {
        let mut ui = cursive::default();
        ui.add_layer(
            Dialog::new()
                .title("No Compatible Network Interfaces Found")
                .content(LinearLayout::vertical()
                    .child(TextView::new("LQOS requires at least one compatible network interface."))
                    .child(TextView::new("Please ensure that your system has network interfaces that support XDP and multiple queues."))
                    .child(TextView::new("For more information, please refer to the documentation."))
                    .child(TextView::new("https://libreqos.readthedocs.io/en/latest/docs/v2.0/requirements.html."))
                )
                .button("Quit", |s| s.quit()),
        );
        ui.run();
        std::process::exit(1);
    }
}