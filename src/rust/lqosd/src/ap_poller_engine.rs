//! The vendored dudarchy engine wired into the AP polling seam.
//!
//! The engine owns adapter detection, the read plans, and the capacity
//! derivation (3c). This module only supplies the live SNMP reader, the
//! per-device capacity policy, and the observation JSON the poller writes.
//!
//! Everything here is offline-testable: [`observe_document`] takes any
//! [`Reader`], so the replay transcript exercises the whole decode/map path
//! without a socket.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dudarchy_poller::adapters::engine::{self, CapacityPolicy, Selection};
use dudarchy_poller::adapters::live::SnmpReader;
use dudarchy_poller::adapters::reader::{Outcome, Reader};
use dudarchy_poller::adapters::{self, Document, Transport};
use dudarchy_poller::{Community, SnmpAccess, SnmpVersion, WalkValue, WirelessStation};
use lqos_config::{Config, DeviceRef};
use serde_json::json;
use tracing::{debug, warn};

use crate::ap_polling::{DeviceObservation, PollContext, PollFuture, Poller};

/// The shipped adapter documents, compiled into the daemon. The generic
/// default is included for parse coverage but is never selected here: an
/// unknown family must yield no observation, not a generic base probe.
const BUILTIN_ADAPTERS: &[(&str, &str)] = &[
    (
        "airos.json",
        include_str!("../../dudarchy_poller/adapters/airos.json"),
    ),
    (
        "cambium_epmp.json",
        include_str!("../../dudarchy_poller/adapters/cambium_epmp.json"),
    ),
    (
        "edgeos.json",
        include_str!("../../dudarchy_poller/adapters/edgeos.json"),
    ),
    (
        "generic-base.json",
        include_str!("../../dudarchy_poller/adapters/generic-base.json"),
    ),
    (
        "ltu_rocket.json",
        include_str!("../../dudarchy_poller/adapters/ltu_rocket.json"),
    ),
    (
        "netonix.json",
        include_str!("../../dudarchy_poller/adapters/netonix.json"),
    ),
    (
        "procurve.json",
        include_str!("../../dudarchy_poller/adapters/procurve.json"),
    ),
    (
        "routeros.json",
        include_str!("../../dudarchy_poller/adapters/routeros.json"),
    ),
    (
        "swos.json",
        include_str!("../../dudarchy_poller/adapters/swos.json"),
    ),
    (
        "wave_ap.json",
        include_str!("../../dudarchy_poller/adapters/wave_ap.json"),
    ),
    (
        "wave_ap_legacy.json",
        include_str!("../../dudarchy_poller/adapters/wave_ap_legacy.json"),
    ),
];

/// `sysDescr.0`.
const SYS_DESCR: [u32; 9] = [1, 3, 6, 1, 2, 1, 1, 1, 0];
/// `sysObjectID.0`.
const SYS_OBJECT_ID: [u32; 9] = [1, 3, 6, 1, 2, 1, 1, 2, 0];

/// The engine-backed poller used by [`super::ap_polling`].
pub struct DudarchyPoller {
    documents: Vec<Document>,
}

impl DudarchyPoller {
    /// Parses the shipped adapter documents. A document that fails to parse
    /// is logged and omitted; the remaining families still poll.
    pub fn new() -> Self {
        let documents = BUILTIN_ADAPTERS
            .iter()
            .filter_map(|(name, json)| match adapters::parse(name, json) {
                Ok(document) => Some(document),
                Err(errors) => {
                    warn!(
                        "AP polling adapter `{name}` is invalid: {}",
                        errors
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join("; ")
                    );
                    None
                }
            })
            .collect();
        Self { documents }
    }

    async fn poll_device_inner(
        &self,
        device: &DeviceRef,
        timeout: Duration,
        context: PollContext<'_>,
    ) -> Option<DeviceObservation> {
        let Some(mgmt_ip) = device.mgmt_ip else {
            debug!(node = %device.node_id, "AP poll skipped: no management IP");
            return None;
        };
        let config = context.config;
        let version = config.ap_polling.snmp_version;
        if version == 3 {
            warn!(
                node = %device.node_id,
                "AP poll skipped: SNMPv3 is not supported by the vendored engine"
            );
            return None;
        }
        let community = Community::new(config.ap_polling_community().to_owned());
        let address = mgmt_ip.to_string();
        let observed_at = SystemTime::now();
        let policy = capacity_policy(config, device, context.crm_rate_mbps);
        let cancel = || false;
        let preferred = if version == 1 {
            SnmpVersion::V1
        } else {
            SnmpVersion::V2c
        };

        // A per-device pin is authoritative and skips detection entirely, so
        // a generic sysObjectID still reaches the intended document. An
        // unknown pin yields no observation rather than another family.
        if let Some(pinned) = pinned_document(&self.documents, config, device) {
            let Some(document) = pinned else {
                warn!(
                    node = %device.node_id,
                    adapter = %device_pin(config, device).unwrap_or(""),
                    "AP poll skipped: pinned adapter is not loaded"
                );
                return None;
            };
            let access = SnmpAccess {
                community: community.clone(),
                version: preferred,
            };
            let mut reader = SnmpReader::new(address.clone(), &access, timeout, &cancel);
            if let Some(observation) =
                observe_document(document, &mut reader, observed_at, &policy).await
            {
                return Some(observation);
            }
            if preferred == SnmpVersion::V2c {
                let access = SnmpAccess {
                    community,
                    version: SnmpVersion::V1,
                };
                let mut reader = SnmpReader::new(address, &access, timeout, &cancel);
                return observe_document(document, &mut reader, observed_at, &policy).await;
            }
            return None;
        }

        {
            let access = SnmpAccess {
                community: community.clone(),
                version: preferred,
            };
            let mut reader = SnmpReader::new(address.clone(), &access, timeout, &cancel);
            if let Some(document) = detect_document(&self.documents, &mut reader).await {
                return observe_document(document, &mut reader, observed_at, &policy).await;
            }
        }

        // airOS 8 (and some other legacy agents) answer v1 only. When the
        // configured v2c attempt found no family, retry the same community as
        // v1; the observation then records that it came from the v1 surface.
        if preferred == SnmpVersion::V2c {
            let access = SnmpAccess {
                community,
                version: SnmpVersion::V1,
            };
            let mut reader = SnmpReader::new(address, &access, timeout, &cancel);
            let document = detect_document(&self.documents, &mut reader).await?;
            return observe_document(document, &mut reader, observed_at, &policy).await;
        }

        None
    }
}

impl Poller for DudarchyPoller {
    fn poll_device<'a>(
        &'a self,
        device: &'a DeviceRef,
        timeout: Duration,
        context: PollContext<'a>,
    ) -> PollFuture<'a> {
        Box::pin(async move { self.poll_device_inner(device, timeout, context).await })
    }
}

/// The 3c capacity policy for one device: config factors, the DeviceRef
/// model, and the compiled topology's rate (when one exists).
fn capacity_policy(
    config: &Config,
    device: &DeviceRef,
    crm_rate_mbps: Option<f64>,
) -> CapacityPolicy {
    CapacityPolicy {
        default_factor: config.ap_polling.capacity_factor_default,
        by_model: config.ap_polling.capacity_factor.clone(),
        model: device.model.clone(),
        crm_rate_mbps,
    }
}

/// The adapter document id pinned for this device, when `[ap_polling]`
/// names one. The integration node id wins over the management IP.
fn device_pin<'a>(config: &'a Config, device: &DeviceRef) -> Option<&'a str> {
    let pins = &config.ap_polling.device_adapter;
    if let Some(adapter) = pins.get(&device.node_id) {
        return Some(adapter.as_str());
    }
    let ip = device.mgmt_ip?.to_string();
    pins.get(&ip).map(String::as_str)
}

/// The document a device pin names. `Some(None)` means a pin exists but
/// names an adapter that is not loaded; the caller must not fall back to
/// detection in that case.
fn pinned_document<'a>(
    documents: &'a [Document],
    config: &Config,
    device: &DeviceRef,
) -> Option<Option<&'a Document>> {
    let pin = device_pin(config, device)?;
    Some(documents.iter().find(|document| document.id == pin))
}

/// Reads `sysDescr.0`/`sysObjectID.0` and selects a non-default adapter.
async fn detect_document<'a>(
    documents: &'a [Document],
    reader: &mut (dyn Reader + Send),
) -> Option<&'a Document> {
    let sys_descr = read_scalar(reader, &SYS_DESCR)
        .await
        .and_then(|value| match value {
            WalkValue::Bytes(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
            _ => None,
        });
    let sys_object_id = read_scalar(reader, &SYS_OBJECT_ID)
        .await
        .and_then(|value| match value {
            WalkValue::Oid(text) => Some(text),
            _ => None,
        });
    select_document(documents, sys_object_id.as_deref(), sys_descr.as_deref())
}

/// Selects the family document by fingerprint. The registry's generic
/// default is deliberately not a family: an unknown device yields no
/// observation (the caller counts it), never a generic probe pass.
fn select_document<'a>(
    documents: &'a [Document],
    sys_object_id: Option<&str>,
    sys_descr: Option<&str>,
) -> Option<&'a Document> {
    match engine::select(documents, sys_object_id, sys_descr) {
        Selection::Selected(document) if !document.detect.default => Some(document),
        _ => None,
    }
}

async fn read_scalar(reader: &mut (dyn Reader + Send), oid: &[u32]) -> Option<WalkValue> {
    match reader.get(oid).await {
        Outcome::Present(varbinds) => varbinds.into_iter().next().map(|varbind| varbind.value),
        _ => None,
    }
}

/// Runs a selected document's SNMP surface and maps the engine
/// observation. `None` means the document has no SNMP surface or a required
/// probe did not answer (timeout/error/cancelled): no observation.
pub(crate) async fn observe_document(
    document: &Document,
    reader: &mut (dyn Reader + Send),
    observed_at: SystemTime,
    policy: &CapacityPolicy,
) -> Option<DeviceObservation> {
    let surface = snmp_surface(document)?;
    let cancel = || false;
    let observation = engine::probe_surface_with_capacity(
        document,
        surface,
        reader,
        observed_at,
        &cancel,
        policy,
    )
    .await;
    if !observation.supported() {
        return None;
    }
    Some(build_device_observation(
        document.id.as_str(),
        &observation,
        policy,
        observed_at,
    ))
}

/// The document's SNMP surface id, when it declares one. Command probes are
/// not run here: this poller owns the SNMP transport.
fn snmp_surface(document: &Document) -> Option<&str> {
    document
        .surfaces
        .iter()
        .find(|surface| matches!(surface.transport, Transport::SnmpV1 | Transport::SnmpV2c))
        .map(|surface| surface.id.as_str())
}

/// The observation JSON written per device. Capacity is derived through the
/// 3c path; the station rows are display inventory only. `adapter_id` names
/// the shipped document that answered, so the status surface can say which
/// family last observed the device (it is additive to the schema).
fn build_device_observation(
    adapter_id: &str,
    observation: &engine::Observation,
    policy: &CapacityPolicy,
    observed_at: SystemTime,
) -> DeviceObservation {
    let details = observation.details.as_ref();
    let mut stations: Vec<WirelessStation> = details
        .map(|details| details.stations.clone())
        .unwrap_or_default();
    // The 3c derivation reads `dl_rate_mbps`; adapters that only expose
    // TX/RX bps (airOS) contribute their TX rate in Mbps.
    for station in &mut stations {
        if station.dl_rate_mbps.is_none() {
            station.dl_rate_mbps = station
                .tx_bps
                .or(station.rx_bps)
                .map(|bps| bps / 1_000_000)
                .filter(|rate| *rate > 0);
        }
    }
    let capacity = engine::derive_capacity(
        details.and_then(|details| details.radio.as_ref()),
        &stations,
        policy,
    );
    let rows: Vec<serde_json::Value> = stations
        .iter()
        .map(|station| {
            json!({
                "mac": station.mac.as_str(),
                "ip": station.last_ip.clone(),
                "signal": station.signal_dbm.or(station.dl_rssi).or(station.ul_rssi),
                "rate": station.dl_rate_mbps,
            })
        })
        .collect();
    DeviceObservation {
        observed_at: unix_seconds(observed_at),
        fields: json!({
            "adapter_id": adapter_id,
            "station_count": rows.len(),
            "stations": rows,
            "capacity_mbps": capacity.mbps,
            "capacity_source": capacity.source.to_string(),
        }),
    }
}

fn unix_seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dudarchy_poller::adapters::reader::Varbind;
    use serde_json::Value;
    use std::future::Future;
    use std::pin::Pin;

    const AIROS_TRANSCRIPT: &str =
        include_str!("../../dudarchy_poller/tests/fixtures/adapters/replay/airos/transcript.json");

    fn airos_document() -> Document {
        adapters::parse(
            "airos.json",
            include_str!("../../dudarchy_poller/adapters/airos.json"),
        )
        .expect("the shipped airos document validates")
    }

    /// A scripted reader over the recorded replay transcript.
    struct ReplayReader {
        steps: Vec<(String, Outcome<Vec<Varbind>>)>,
        next: usize,
    }

    impl ReplayReader {
        fn new(raw: &str) -> Self {
            let transcript: Value =
                serde_json::from_str(raw).expect("the replay transcript parses");
            let steps = transcript["steps"]
                .as_array()
                .expect("transcript steps")
                .iter()
                .map(|step| {
                    let op = step["op"].as_str().expect("op");
                    let request = match op {
                        "get" => format!("get {}", step["oid"].as_str().expect("get oid")),
                        "walk" => format!("walk {}", step["root"].as_str().expect("walk root")),
                        "command" => format!(
                            "command {}",
                            step["command"].as_str().expect("command name")
                        ),
                        other => panic!("unknown transcript op `{other}`"),
                    };
                    let outcome = match step["outcome"].as_str().expect("outcome") {
                        "present" => Outcome::Present(
                            step["varbinds"]
                                .as_array()
                                .map(|varbinds| varbinds.iter().map(varbind).collect())
                                .unwrap_or_default(),
                        ),
                        "empty" => Outcome::Empty,
                        "absent" => Outcome::Absent,
                        "timeout" => Outcome::Timeout,
                        "cancelled" => Outcome::Cancelled,
                        "error" => Outcome::Error {
                            reason: step["reason"].as_str().unwrap_or("error").to_owned(),
                        },
                        other => panic!("unknown transcript outcome `{other}`"),
                    };
                    (request, outcome)
                })
                .collect();
            Self { steps, next: 0 }
        }

        fn take(&mut self, request: &str) -> Outcome<Vec<Varbind>> {
            let Some((expected, outcome)) = self.steps.get(self.next) else {
                panic!("replay has no step for `{request}`");
            };
            assert_eq!(expected, request, "replay request order");
            self.next += 1;
            outcome.clone()
        }
    }

    impl Reader for ReplayReader {
        fn get<'a>(
            &'a mut self,
            oid: &'a [u32],
        ) -> Pin<Box<dyn Future<Output = Outcome<Vec<Varbind>>> + Send + 'a>> {
            Box::pin(async move { self.take(&format!("get {}", arcs(oid))) })
        }

        fn walk<'a>(
            &'a mut self,
            root: &'a [u32],
        ) -> Pin<Box<dyn Future<Output = Outcome<Vec<Varbind>>> + Send + 'a>> {
            Box::pin(async move { self.take(&format!("walk {}", arcs(root))) })
        }

        fn command<'a>(
            &'a mut self,
            name: dudarchy_poller::adapters::CommandName,
        ) -> Pin<
            Box<
                dyn Future<Output = Outcome<Vec<dudarchy_poller::adapters::reader::CommandRow>>>
                    + Send
                    + 'a,
            >,
        > {
            Box::pin(async move {
                let _ = name;
                Outcome::Absent
            })
        }
    }

    fn varbind(value: &Value) -> Varbind {
        let oid = value["oid"]
            .as_str()
            .expect("varbind oid")
            .split('.')
            .map(|arc| arc.parse().expect("oid arc"))
            .collect();
        let value = &value["value"];
        let value = match value["type"].as_str().expect("value type") {
            "octet_string" => WalkValue::Bytes(match value["hex"].as_str() {
                Some(hex) => hex
                    .split([':', '-'])
                    .map(|octet| u8::from_str_radix(octet, 16).expect("hex octet"))
                    .collect(),
                None => value["text"].as_str().unwrap_or("").as_bytes().to_vec(),
            }),
            "integer" => WalkValue::Integer(value["value"].as_i64().expect("integer")),
            "counter64" => WalkValue::Counter64(value["value"].as_u64().expect("counter64")),
            "time_ticks" => WalkValue::TimeTicks(value["value"].as_u64().expect("ticks")),
            "ip_address" => WalkValue::IpAddress(value["value"].as_str().expect("ip").to_owned()),
            "oid" => WalkValue::Oid(value["value"].as_str().expect("oid").to_owned()),
            other => panic!("unknown value type `{other}`"),
        };
        Varbind { oid, value }
    }

    fn arcs(oid: &[u32]) -> String {
        oid.iter().map(u32::to_string).collect::<Vec<_>>().join(".")
    }

    struct TimeoutReader;

    impl Reader for TimeoutReader {
        fn get<'a>(
            &'a mut self,
            _oid: &'a [u32],
        ) -> Pin<Box<dyn Future<Output = Outcome<Vec<Varbind>>> + Send + 'a>> {
            Box::pin(async { Outcome::Timeout })
        }

        fn walk<'a>(
            &'a mut self,
            _root: &'a [u32],
        ) -> Pin<Box<dyn Future<Output = Outcome<Vec<Varbind>>> + Send + 'a>> {
            Box::pin(async { Outcome::Timeout })
        }

        fn command<'a>(
            &'a mut self,
            _name: dudarchy_poller::adapters::CommandName,
        ) -> Pin<
            Box<
                dyn Future<Output = Outcome<Vec<dudarchy_poller::adapters::reader::CommandRow>>>
                    + Send
                    + 'a,
            >,
        > {
            Box::pin(async { Outcome::Timeout })
        }
    }

    #[tokio::test]
    async fn airos_replay_yields_per_sm_capacity_and_station_count() {
        let document = airos_document();
        let mut reader = ReplayReader::new(AIROS_TRANSCRIPT);
        let observation = observe_document(
            &document,
            &mut reader,
            SystemTime::UNIX_EPOCH,
            &CapacityPolicy::default(),
        )
        .await
        .expect("the recorded airOS capture yields an observation");

        assert_eq!(observation.fields["station_count"].as_u64(), Some(2));
        assert_eq!(observation.fields["adapter_id"].as_str(), Some("airos"));
        assert_eq!(
            observation.fields["capacity_source"].as_str(),
            Some("per_sm_rate")
        );
        assert_eq!(observation.fields["capacity_mbps"].as_f64(), Some(400.0));
        let stations = observation.fields["stations"]
            .as_array()
            .expect("station rows");
        assert_eq!(stations.len(), 2);
        assert_eq!(stations[0]["mac"].as_str(), Some("00:27:22:1a:f9:4e"));
        assert_eq!(stations[0]["rate"].as_u64(), Some(400));
        assert_eq!(stations[0]["signal"].as_i64(), Some(-47));
    }

    #[test]
    fn unknown_family_is_not_selected_and_never_falls_back_to_generic() {
        let poller = DudarchyPoller::new();
        assert!(
            select_document(
                &poller.documents,
                Some("1.3.6.1.4.1.999999"),
                Some("unknown family")
            )
            .is_none()
        );
    }

    #[test]
    fn an_exact_sys_object_id_beats_a_broader_prefix() {
        let poller = DudarchyPoller::new();
        // Wave reports exactly 41112; airOS and EdgeOS sit under it.
        let selected = select_document(&poller.documents, Some("1.3.6.1.4.1.41112"), None)
            .expect("the exact Wave OID selects");
        assert_eq!(selected.id, "wave_ap");
        let selected = select_document(&poller.documents, Some("1.3.6.1.4.1.41112.1.4"), None)
            .expect("the airOS prefix still selects");
        assert_eq!(selected.id, "airos");
        let selected = select_document(&poller.documents, Some("1.3.6.1.4.1.41112.1.5"), None)
            .expect("the EdgeOS prefix still selects");
        assert_eq!(selected.id, "edgeos");
    }

    #[test]
    fn a_device_pin_overrides_detection_and_an_unknown_pin_does_not_fall_back() {
        fn device(node_id: &str, ip: &str) -> DeviceRef {
            DeviceRef {
                node_id: node_id.to_owned(),
                mgmt_ip: Some(ip.parse().expect("mgmt ip")),
                vendor_hint: None,
                model: None,
                family_hint: None,
            }
        }

        let poller = DudarchyPoller::new();
        let ltu_device = device("node-ltu", "100.124.2.246");
        // The live LTU answers the generic Linux OID: detection alone yields
        // nothing, so only the pin can reach the family document.
        assert!(
            select_document(
                &poller.documents,
                Some("1.3.6.1.4.1.8072.3.2.10"),
                Some("Linux JackRabbit_T 3.14.65 #1 SMP PREEMPT RT")
            )
            .is_none()
        );

        let mut config = Config::default();
        config
            .ap_polling
            .device_adapter
            .insert("node-ltu".to_owned(), "ltu_rocket".to_owned());
        let pinned =
            pinned_document(&poller.documents, &config, &ltu_device).expect("the node id pins");
        assert_eq!(pinned.expect("the pinned document loads").id, "ltu_rocket");

        // The management IP key also pins when the node id does not.
        config.ap_polling.device_adapter.clear();
        config
            .ap_polling
            .device_adapter
            .insert("100.124.2.246".to_owned(), "ltu_rocket".to_owned());
        let pinned = pinned_document(&poller.documents, &config, &ltu_device)
            .expect("the management IP pins");
        assert_eq!(pinned.expect("the pinned document loads").id, "ltu_rocket");

        // A pin-only legacy Wave variant is reachable only by id; the shared
        // exact OID still auto-selects wave_ap.
        let legacy = device("node-wave-legacy", "100.124.9.239");
        config.ap_polling.device_adapter.clear();
        config
            .ap_polling
            .device_adapter
            .insert("node-wave-legacy".to_owned(), "wave_ap_legacy".to_owned());
        let pinned =
            pinned_document(&poller.documents, &config, &legacy).expect("the legacy pin exists");
        assert_eq!(
            pinned.expect("the legacy document loads").id,
            "wave_ap_legacy"
        );
        let selected = select_document(&poller.documents, Some("1.3.6.1.4.1.41112"), None)
            .expect("the exact Wave OID selects");
        assert_eq!(selected.id, "wave_ap");

        // An unknown pin is reported, never replaced by a detected family.
        config.ap_polling.device_adapter.clear();
        config
            .ap_polling
            .device_adapter
            .insert("node-ltu".to_owned(), "no_such_adapter".to_owned());
        assert!(
            pinned_document(&poller.documents, &config, &ltu_device)
                .expect("the pin exists")
                .is_none()
        );
    }

    #[tokio::test]
    async fn timeout_yields_no_observation() {
        let document = airos_document();
        let mut reader = TimeoutReader;
        assert!(
            observe_document(
                &document,
                &mut reader,
                SystemTime::UNIX_EPOCH,
                &CapacityPolicy::default()
            )
            .await
            .is_none()
        );
    }
}
