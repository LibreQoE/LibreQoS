//! `GET /local-api/ap-polling` — the read-only status surface for the
//! AP-polling page.
//!
//! The data comes from [`crate::ap_polling_state`], which the poller task
//! updates after each sweep. This module only shapes a snapshot into JSON:
//! no polling, no config writes, and no shaping reads. Empty or unavailable
//! is explicit (`state: "unavailable"`, null aggregates) and never a fake
//! all-zero list — the same convention as the CPU & Workers surface.

use axum::Json;
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::ap_polling_state::{DeviceState, DeviceStatus, ServiceState, Snapshot};

/// Aggregates over the latest sweep. `observing` counts devices that are
/// tracked but have no completed poll yet, so the three states always sum to
/// `devices_total` after a sweep. `stations_total` sums only devices whose
/// latest poll succeeded: a failing device's retained station count is stale
/// and must not look current.
fn aggregate(devices: &[DeviceStatus]) -> Value {
    let mut ok = 0u64;
    let mut failing = 0u64;
    let mut observing = 0u64;
    let mut stations_total = 0u64;
    for device in devices {
        match device.state() {
            DeviceState::Ok => {
                ok += 1;
                if let Some(count) = device.station_count {
                    stations_total = stations_total.saturating_add(count);
                }
            }
            DeviceState::Failing => failing += 1,
            DeviceState::Observing => observing += 1,
        }
    }
    json!({
        "devices_total": devices.len(),
        "ok": ok,
        "failing": failing,
        "observing": observing,
        "stations_total": stations_total,
    })
}

fn device_row(device: &DeviceStatus) -> Value {
    json!({
        "node_id": device.node_id,
        "model": device.model,
        "mgmt_ip": device.mgmt_ip.map(|ip| ip.to_string()),
        "capacity_mbps": device.capacity_mbps,
        "capacity_source": device.capacity_source,
        "station_count": device.station_count,
        "crm_pinned_count": device.crm_pinned_count,
        "drift_delta": device.drift_delta,
        "last_poll": device.last_poll,
        "last_ok": device.last_ok,
        "consecutive_failures": device.consecutive_failures,
        "adapter_id": device.adapter_id,
        "alerting": device.alerted,
        "state": device.state().as_str(),
    })
}

/// Explicit unavailable: no device list and no aggregate, never zeros.
fn unavailable(
    reason: &str,
    service_state: Option<&str>,
    interval_s: Option<u64>,
    failures_before_alert: Option<u32>,
) -> Value {
    json!({
        "state": "unavailable",
        "unavailable_reasons": [reason],
        "service_state": service_state,
        "interval_s": interval_s,
        "failures_before_alert": failures_before_alert,
        "last_sweep_at": Value::Null,
        "devices": Value::Null,
        "aggregate": Value::Null,
    })
}

/// Build the exact endpoint document from a snapshot (pure; unit-testable).
pub(crate) fn assemble_ap_polling(snapshot: Option<Snapshot>) -> Value {
    let Some(snapshot) = snapshot else {
        return unavailable("ap_polling_not_started", None, None, None);
    };
    let service = snapshot.service.as_str();
    let interval = Some(snapshot.interval_s);
    let threshold = Some(snapshot.failures_before_alert);

    match snapshot.service {
        ServiceState::Disabled => {
            return unavailable("ap_polling_disabled", Some(service), interval, threshold);
        }
        ServiceState::Running | ServiceState::Stopped => {}
    }
    let Some(last_sweep_at) = snapshot.last_sweep_at else {
        let reason = if snapshot.service == ServiceState::Running {
            "first_sweep_pending"
        } else {
            "ap_polling_stopped"
        };
        return unavailable(reason, Some(service), interval, threshold);
    };

    let mut reasons: Vec<&str> = Vec::new();
    if snapshot.service == ServiceState::Stopped {
        reasons.push("ap_polling_stopped");
    }
    let devices: Vec<Value> = snapshot.devices.iter().map(device_row).collect();
    let aggregate = aggregate(&snapshot.devices);
    let any_failing = aggregate["failing"].as_u64().unwrap_or(0) > 0;
    if any_failing {
        reasons.push("devices_failing");
    }
    let state = if reasons.is_empty() { "ok" } else { "partial" };

    json!({
        "state": state,
        "unavailable_reasons": reasons,
        "service_state": service,
        "interval_s": interval,
        "failures_before_alert": threshold,
        "last_sweep_at": last_sweep_at,
        "devices": devices,
        "aggregate": aggregate,
    })
}

/// `GET /local-api/ap-polling`: read one snapshot and serialize it. Same-origin
/// only; the node-manager `auth_layer` is the session gate.
pub async fn get_ap_polling() -> Response {
    let body = assemble_ap_polling(crate::ap_polling_state::snapshot());
    let mut response = Json(body).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn device(node_id: &str, state: DeviceState) -> DeviceStatus {
        let (last_poll, last_ok, consecutive_failures, station_count) = match state {
            DeviceState::Ok => (Some(1_000), Some(1_000), 0, Some(3)),
            DeviceState::Failing => (Some(1_000), Some(900), 2, Some(3)),
            DeviceState::Observing => (None, None, 0, None),
        };
        DeviceStatus {
            node_id: node_id.to_string(),
            model: Some("ePMP 4500L".to_string()),
            mgmt_ip: Some("100.124.6.121".parse().expect("test IP")),
            capacity_mbps: Some(192.5),
            capacity_source: Some("per_sm_rate".to_string()),
            station_count,
            crm_pinned_count: station_count.map(|count| count.saturating_sub(1)),
            drift_delta: station_count.map(|_| 1),
            last_poll,
            last_ok,
            consecutive_failures,
            adapter_id: Some("cambium_epmp".to_string()),
            alerted: false,
        }
    }

    fn snapshot(service: ServiceState, devices: Vec<DeviceStatus>) -> Snapshot {
        Snapshot {
            service,
            interval_s: 300,
            failures_before_alert: 3,
            last_sweep_at: Some(1_000),
            devices,
        }
    }

    #[test]
    fn populated_store_exposes_every_required_field() {
        let assembly = assemble_ap_polling(Some(snapshot(
            ServiceState::Running,
            vec![
                device("node-ap-1", DeviceState::Ok),
                device("node-ap-2", DeviceState::Failing),
            ],
        )));

        let keys: BTreeSet<&str> = assembly
            .as_object()
            .expect("object")
            .keys()
            .map(String::as_str)
            .collect();
        let expected: BTreeSet<&str> = [
            "state",
            "unavailable_reasons",
            "service_state",
            "interval_s",
            "failures_before_alert",
            "last_sweep_at",
            "devices",
            "aggregate",
        ]
        .into_iter()
        .collect();
        assert_eq!(keys, expected);
        assert_eq!(assembly["state"].as_str(), Some("partial"));
        assert_eq!(assembly["service_state"].as_str(), Some("running"));
        assert_eq!(assembly["interval_s"].as_u64(), Some(300));
        assert_eq!(assembly["failures_before_alert"].as_u64(), Some(3));

        let row = &assembly["devices"][0];
        let row_keys: BTreeSet<&str> = row
            .as_object()
            .expect("row object")
            .keys()
            .map(String::as_str)
            .collect();
        let expected_row: BTreeSet<&str> = [
            "node_id",
            "model",
            "mgmt_ip",
            "capacity_mbps",
            "capacity_source",
            "station_count",
            "crm_pinned_count",
            "drift_delta",
            "last_poll",
            "last_ok",
            "consecutive_failures",
            "adapter_id",
            "alerting",
            "state",
        ]
        .into_iter()
        .collect();
        assert_eq!(row_keys, expected_row);
        assert_eq!(row["node_id"].as_str(), Some("node-ap-1"));
        assert_eq!(row["model"].as_str(), Some("ePMP 4500L"));
        assert_eq!(row["mgmt_ip"].as_str(), Some("100.124.6.121"));
        assert_eq!(row["capacity_mbps"].as_f64(), Some(192.5));
        assert_eq!(row["capacity_source"].as_str(), Some("per_sm_rate"));
        assert_eq!(row["station_count"].as_u64(), Some(3));
        assert_eq!(row["crm_pinned_count"].as_u64(), Some(2));
        assert_eq!(row["drift_delta"].as_i64(), Some(1));
        assert_eq!(row["last_poll"].as_u64(), Some(1_000));
        assert_eq!(row["last_ok"].as_u64(), Some(1_000));
        assert_eq!(row["adapter_id"].as_str(), Some("cambium_epmp"));
        assert_eq!(row["state"].as_str(), Some("ok"));
        assert_eq!(row["alerting"], false);
    }

    #[test]
    fn aggregate_counts_the_latest_sweep() {
        let mut failing = device("node-bad", DeviceState::Failing);
        failing.consecutive_failures = 3;
        failing.alerted = true;
        let assembly = assemble_ap_polling(Some(snapshot(
            ServiceState::Running,
            vec![
                device("node-ok", DeviceState::Ok),
                failing,
                device("node-new", DeviceState::Observing),
            ],
        )));
        let aggregate = &assembly["aggregate"];
        assert_eq!(aggregate["devices_total"].as_u64(), Some(3));
        assert_eq!(aggregate["ok"].as_u64(), Some(1));
        assert_eq!(aggregate["failing"].as_u64(), Some(1));
        assert_eq!(aggregate["observing"].as_u64(), Some(1));
        assert_eq!(
            aggregate["stations_total"].as_u64(),
            Some(3),
            "only the healthy device's fresh station count is summed"
        );
        assert_eq!(assembly["devices"][1]["state"].as_str(), Some("failing"));
        assert_eq!(assembly["devices"][1]["alerting"], true);
    }

    #[test]
    fn empty_store_is_explicitly_unavailable_never_zeros() {
        let assembly = assemble_ap_polling(None);
        assert_eq!(assembly["state"].as_str(), Some("unavailable"));
        assert_eq!(
            assembly["unavailable_reasons"][0].as_str(),
            Some("ap_polling_not_started")
        );
        assert!(assembly["service_state"].is_null());
        assert!(assembly["interval_s"].is_null());
        assert!(assembly["failures_before_alert"].is_null());
        assert!(assembly["devices"].is_null());
        assert!(assembly["aggregate"].is_null());
        assert!(!assembly.to_string().contains("\"devices_total\":0"));
    }

    #[test]
    fn disabled_poller_is_explicitly_unavailable() {
        let assembly = assemble_ap_polling(Some(Snapshot {
            service: ServiceState::Disabled,
            interval_s: 300,
            failures_before_alert: 3,
            last_sweep_at: None,
            devices: Vec::new(),
        }));
        assert_eq!(assembly["state"].as_str(), Some("unavailable"));
        assert_eq!(
            assembly["unavailable_reasons"][0].as_str(),
            Some("ap_polling_disabled")
        );
        assert_eq!(assembly["service_state"].as_str(), Some("disabled"));
        assert!(assembly["devices"].is_null());
        assert!(assembly["aggregate"].is_null());
        assert_ne!(assembly["aggregate"].as_u64(), Some(0));
    }

    #[test]
    fn running_without_a_sweep_is_pending_not_empty() {
        let assembly = assemble_ap_polling(Some(Snapshot {
            service: ServiceState::Running,
            interval_s: 300,
            failures_before_alert: 3,
            last_sweep_at: None,
            devices: Vec::new(),
        }));
        assert_eq!(assembly["state"].as_str(), Some("unavailable"));
        assert_eq!(
            assembly["unavailable_reasons"][0].as_str(),
            Some("first_sweep_pending")
        );
        assert!(assembly["devices"].is_null());
    }

    #[test]
    fn a_completed_sweep_over_an_empty_topology_reports_real_zeros() {
        let assembly = assemble_ap_polling(Some(snapshot(ServiceState::Running, Vec::new())));
        assert_eq!(assembly["state"].as_str(), Some("ok"));
        assert_eq!(assembly["devices"].as_array().map(Vec::len), Some(0));
        assert_eq!(assembly["aggregate"]["devices_total"].as_u64(), Some(0));
    }

    #[test]
    fn a_stopped_task_keeps_the_last_sweep_but_says_so() {
        let assembly = assemble_ap_polling(Some(snapshot(
            ServiceState::Stopped,
            vec![device("node-ap-1", DeviceState::Ok)],
        )));
        assert_eq!(assembly["state"].as_str(), Some("partial"));
        assert_eq!(assembly["service_state"].as_str(), Some("stopped"));
        assert_eq!(
            assembly["unavailable_reasons"][0].as_str(),
            Some("ap_polling_stopped")
        );
        assert_eq!(assembly["aggregate"]["ok"].as_u64(), Some(1));
    }

    #[test]
    fn unknown_capacity_is_null_not_zero() {
        let mut unknown = device("node-ap-1", DeviceState::Ok);
        unknown.capacity_mbps = None;
        unknown.capacity_source = None;
        unknown.station_count = None;
        unknown.crm_pinned_count = None;
        unknown.drift_delta = None;
        unknown.adapter_id = None;
        let assembly = assemble_ap_polling(Some(snapshot(ServiceState::Running, vec![unknown])));
        let row = &assembly["devices"][0];
        assert!(row["capacity_mbps"].is_null());
        assert!(row["capacity_source"].is_null());
        assert!(row["station_count"].is_null());
        assert!(row["crm_pinned_count"].is_null());
        assert!(row["drift_delta"].is_null());
        assert!(row["adapter_id"].is_null());
        assert!(row["mgmt_ip"].is_string(), "the known address still shows");
    }
}
