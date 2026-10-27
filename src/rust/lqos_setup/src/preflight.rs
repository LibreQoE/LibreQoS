use std::path::Path;

use cursive::views::{Dialog, LinearLayout, TextView};

use crate::interfaces;

/// The dataplane artifact's install root (the distribution split): the bridge
/// binary + its DPDK closure. `src/src/update_bridge.sh` fetches the portal
/// artifact into this path; a control-plane-only install lacks it.
const DATAPLANE_LIBEXEC: &str = "/usr/libexec/libreqos";

/// Whether the separate dataplane artifact has been installed.
fn dataplane_artifact_present() -> bool {
    let root = Path::new(DATAPLANE_LIBEXEC);
    root.join("bridge_rs").is_file() && root.join("lib").is_dir()
}

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

    // The split: the public/control-plane package ships no dataplane. Diagnose
    // its absence here (fail-closed made visible) — setup can configure the
    // control plane, but shaping needs the portal artifact.
    if !dataplane_artifact_present() {
        let mut ui = cursive::default();
        ui.add_layer(
            Dialog::new()
                .title("LibreQoS Dataplane Not Installed")
                .content(LinearLayout::vertical()
                    .child(TextView::new("The DPDK dataplane (the bridge) is not installed."))
                    .child(TextView::new("The control plane can be configured, but traffic shaping requires it."))
                    .child(TextView::new("Fetch the dataplane artifact from the LibreQoS portal"))
                    .child(TextView::new("(the packaged update_bridge.sh), then re-run setup."))
                )
                .button("Continue", |s| s.quit()),
        );
        ui.run();
    }
}

