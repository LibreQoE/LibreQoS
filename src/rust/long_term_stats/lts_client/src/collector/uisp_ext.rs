use lqos_utils::unix_time::unix_now;
use tokio::sync::mpsc::Sender;
use crate::{submission_queue::{comm_channel::SenderChannelMessage, new_submission}, transport_data::{StatsSubmission, UispExtDevice}, collector::collection_manager::DEVICE_ID_LIST};

pub(crate) async fn gather_uisp_data(comm_tx: Sender<SenderChannelMessage>) {
    log::info!("Gathering UISP Data for Long-Term Stats");
    let timestamp = unix_now().unwrap_or(0);
    if timestamp == 0 {
        return; // We're not ready
    }

    if let Ok(config) = lqos_config::LibreQoSConfig::load() {
        if let Ok(devices) = uisp::load_all_devices_with_interfaces(config).await {
            log::info!("Loaded {} UISP devices", devices.len());

            // Collate the data
            let uisp_devices: Vec<UispExtDevice> = devices
                .into_iter()
                .filter(|d| DEVICE_ID_LIST.contains(&d.identification.id))
                .map(|device| device.into())
                .collect();
            log::info!("Retained {} relevant UISP devices", uisp_devices.len());

            // Build a queue message containing just UISP info
            // Submit it
            let submission = StatsSubmission {
                timestamp,
                totals: None,
                hosts: None,
                tree: None,
                cpu_usage: None,
                ram_percent: None,
                uisp_devices: Some(uisp_devices),
            };
            new_submission(submission, comm_tx).await;
        } else {
            log::warn!("Unable to load UISP devices");
        }
    } else {
        log::warn!("UISP data collection requested, but no LibreQoS configuration found");
    }
}