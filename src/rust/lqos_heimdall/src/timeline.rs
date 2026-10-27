use crate::{
    SESSION_EXPIRE_SECONDS, TIMELINE_EXPIRE_SECS,
    events::{HeimdallEvent, PACKET_OCTET_SIZE},
    pcap::{PcapFileHeader, PcapPacketHeader},
};
use dashmap::{DashMap, DashSet};
use lqos_bus::{PacketHeader, tos_parser};
use lqos_utils::{XdpIpAddress, unix_time::time_since_boot};
use once_cell::sync::Lazy;
use std::{
    fs::{File, remove_file},
    io::Write,
    net::IpAddr,
    path::Path,
    sync::atomic::{AtomicBool, AtomicUsize},
    time::Duration,
};
use tracing::{debug, info, warn};
use zerocopy::IntoBytes;

impl HeimdallEvent {
    fn as_header(&self) -> PacketHeader {
        let (dscp, ecn) = tos_parser(self.tos);
        PacketHeader {
            timestamp: self.timestamp,
            src: self.src.as_ip().to_string(),
            dst: self.dst.as_ip().to_string(),
            src_port: self.src_port,
            dst_port: self.dst_port,
            ip_protocol: self.ip_protocol,
            ecn,
            dscp,
            size: self.size,
            tcp_flags: self.tcp_flags,
            tcp_window: self.tcp_window,
            tcp_tsecr: self.tcp_tsecr,
            tcp_tsval: self.tcp_tsval,
        }
    }

    /// §1.4a: converts one bridge capture record into the timeline's
    /// `HeimdallEvent` shape. The prefix is already the inner Ethernet frame;
    /// the bridge's direction tag has no `HeimdallEvent` equivalent (the
    /// bridge sees both directions — the disclosed superset, §1.7).
    fn from_capture(record: analytics_ipc::CaptureRecord) -> Self {
        Self {
            timestamp: record.timestamp_ns,
            src: XdpIpAddress::from(record.src),
            dst: XdpIpAddress::from(record.dst),
            src_port: record.src_port,
            dst_port: record.dst_port,
            ip_protocol: record.ip_protocol,
            tos: record.tos,
            size: record.size,
            tcp_flags: record.tcp_flags,
            tcp_window: record.tcp_window,
            tcp_tsval: record.tcp_tsval,
            tcp_tsecr: record.tcp_tsecr,
            packet_data: record.prefix.0,
        }
    }
}

struct Timeline {
    data: DashSet<HeimdallEvent>,
}

impl Timeline {
    fn new() -> Self {
        Self {
            data: DashSet::new(),
        }
    }
}

static TIMELINE: Lazy<Timeline> = Lazy::new(Timeline::new);

pub(crate) fn store_on_timeline(event: HeimdallEvent) {
    TIMELINE.data.insert(event); // We're moving here deliberately
}

pub(crate) fn expire_timeline() {
    if let Ok(now) = time_since_boot() {
        let since_boot = Duration::from(now);
        let Some(expire) = since_boot.checked_sub(Duration::from_secs(TIMELINE_EXPIRE_SECS)) else {
            return;
        };
        let expire = expire.as_nanos() as u64;
        TIMELINE.data.retain(|v| v.timestamp > expire);
        // Drop sessions that have passed their expiry time
        FOCUS_SESSIONS.retain(|_, v| v.expire > since_boot.as_nanos() as u64);
    }
}

struct FocusSession {
    expire: u64,
    data: DashSet<HeimdallEvent>,
    dump_filename: Option<String>,
}

impl Drop for FocusSession {
    fn drop(&mut self) {
        if let Some(df) = &self.dump_filename {
            let path = Path::new(df);
            if path.exists() {
                let _ = remove_file(path);
            }
        }
    }
}

static HYPERFOCUSED: AtomicBool = AtomicBool::new(false);
static FOCUS_SESSION_ID: AtomicUsize = AtomicUsize::new(0);
static FOCUS_SESSIONS: Lazy<DashMap<usize, FocusSession>> = Lazy::new(DashMap::new);

/// RAII guard for the single-session latch. Resets [`HYPERFOCUSED`] on drop,
/// including during unwind, so a panicking capture-session thread cannot leak
/// the latch and permanently wedge capture until restart.
struct HyperfocusLatch;

impl Drop for HyperfocusLatch {
    fn drop(&mut self) {
        HYPERFOCUSED.store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

/// §1.4a: the bridge watch is coarse (by circuit); the requested device IP is
/// the precise userspace filter. Pure — unit-gated.
fn record_matches(record: &analytics_ipc::CaptureRecord, slot_id: u64, filter_ip: IpAddr) -> bool {
    record.circuit_id == slot_id && (record.src == filter_ip || record.dst == filter_ip)
}

/// §1.4a: a bridge-transport watch session. Watches one bridge leaf slot,
/// drains capture records every second, filters them to `filter_ip`, and
/// stores matching events on the timeline. A transport loss ends the session
/// loudly (no silent cross-over — the two transports keep different timestamp
/// domains); the watch is always cleared at session end.
pub fn hyperfocus_on_target_bridge(
    slot_id: u64,
    filter_ip: IpAddr,
    socket: &Path,
) -> Option<(usize, usize)> {
    let capture_time = if let Ok(cfg) = lqos_config::load_config() {
        cfg.packet_capture_time
    } else {
        10
    };
    hyperfocus_on_target_bridge_with(slot_id, filter_ip, socket, capture_time)
}

/// Testable core of [`hyperfocus_on_target_bridge`]: the capture window is
/// injected so the feed-swap oracle runs in seconds.
fn hyperfocus_on_target_bridge_with(
    slot_id: u64,
    filter_ip: IpAddr,
    socket: &Path,
    capture_time: usize,
) -> Option<(usize, usize)> {
    if HYPERFOCUSED.compare_exchange(
        false,
        true,
        std::sync::atomic::Ordering::Relaxed,
        std::sync::atomic::Ordering::Relaxed,
    ) == Ok(false)
    {
        let new_id = FOCUS_SESSION_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let socket = socket.to_owned();
        let spawned = std::thread::Builder::new()
            .name("HeimdalBridgeSession".to_string())
            .spawn(move || {
                // The guard releases the latch on normal return AND on panic.
                let _latch = HyperfocusLatch;
                let watch_set = match analytics_ipc::connect_watch(&socket, &[slot_id]) {
                    Ok(_) => true,
                    Err(error) => {
                        warn!("Heimdall bridge session could not set the watch: {error:?}");
                        false
                    }
                };
                if watch_set {
                    for _ in 0..capture_time {
                        match analytics_ipc::connect_drain(&socket, analytics_ipc::DEFAULT_MAX_ROWS)
                        {
                            Ok(batch) => {
                                for record in batch.records {
                                    if !record_matches(&record, slot_id, filter_ip) {
                                        continue;
                                    }
                                    store_on_timeline(HeimdallEvent::from_capture(record));
                                }
                                if batch.dropped > 0 {
                                    debug!(
                                        "Heimdall bridge capture dropped {} record(s)",
                                        batch.dropped
                                    );
                                }
                            }
                            Err(error) => {
                                warn!(
                                    "Heimdall bridge session transport lost; ending session: {error:?}"
                                );
                                break;
                            }
                        }
                        std::thread::sleep(Duration::from_secs(1));
                    }
                }
                // The session owns the watch: clear it on the way out.
                let _ = analytics_ipc::connect_watch(&socket, &[]);
                if let Ok(now) = time_since_boot() {
                    let since_boot = Duration::from(now);
                    let expire = since_boot + Duration::from_secs(SESSION_EXPIRE_SECONDS);
                    FOCUS_SESSIONS.insert(
                        new_id,
                        FocusSession {
                            expire: expire.as_nanos() as u64,
                            data: TIMELINE.data.clone(),
                            dump_filename: None,
                        },
                    );
                }
            });
        if let Err(error) = spawned {
            // The closure never ran, so its guard cannot release the latch.
            HYPERFOCUSED.store(false, std::sync::atomic::Ordering::Relaxed);
            warn!("Heimdall could not spawn the bridge capture session: {error:?}");
            return None;
        }
        Some((new_id, capture_time))
    } else {
        warn!("Heimdall was busy and won't start another collection session.");
        None
    }
}

/// Request a dump of the packet headers collected during a hyperfocus session.
/// This will return `None` if the session id is invalid or the session has
/// expired.
/// ## Returns
/// * Either `None` or a vector of packet headers.
/// ## Arguments
/// * `session_id` - The session id of the hyperfocus session.
pub fn n_second_packet_dump(session_id: usize) -> Option<Vec<PacketHeader>> {
    if let Some(session) = FOCUS_SESSIONS.get(&session_id) {
        Some(session.data.iter().map(|e| e.as_header()).collect())
    } else {
        None
    }
}

/// Request a dump of the packet headers collected during a hyperfocus session,
/// in LibPCAP format. This will return `None` if the session id is invalid or
/// the session has expired, or the temporary filename used to store the dump
/// if it is available.
/// ## Returns
/// * Either `None` or the filename of the dump.
/// ## Arguments
/// * `session_id` - The session id of the hyperfocus session.
pub fn n_second_pcap(session_id: usize) -> Option<String> {
    if let Some(mut session) = FOCUS_SESSIONS.get_mut(&session_id) {
        let filename = format!("/tmp/cap_sess_{session_id}");
        session.dump_filename = Some(filename.clone());
        let path = Path::new(&filename);
        let mut out = match File::create(path) {
            Ok(out) => out,
            Err(err) => {
                info!("Unable to create {filename}: {err:?}");
                session.dump_filename = None;
                return None;
            }
        };
        if let Err(err) = out.write_all(PcapFileHeader::new().as_bytes()) {
            info!("Unable to write to {filename}: {err:?}");
            return None;
        }

        for e in session.data.iter() {
            let p = PcapPacketHeader::from_heimdall(&e);
            if let Err(err) = out.write_all(p.as_bytes()) {
                info!("Unable to write to {filename}: {:?}", err);
                return None;
            }
            if e.size < PACKET_OCTET_SIZE as u32 {
                if let Err(err) = out.write_all(&e.packet_data[0..e.size as usize]) {
                    info!("Unable to write to {filename}: {:?}", err);
                    return None;
                }
            } else if let Err(err) = out.write_all(&e.packet_data) {
                info!("Unable to write to {filename}: {:?}", err);
                return None;
            }
        }

        Some(filename)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use analytics_ipc::{CAPTURE_PREFIX_BYTES, CaptureDirection, CapturePrefix, CaptureRecord};

    /// Serializes tests that mutate the process-global `HYPERFOCUSED` latch.
    static LATCH_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn record(slot: u64, src: &str, dst: &str) -> CaptureRecord {
        CaptureRecord {
            timestamp_ns: 42,
            circuit_id: slot,
            src: src.parse().expect("test invariant"),
            dst: dst.parse().expect("test invariant"),
            src_port: 1234,
            dst_port: 80,
            ip_protocol: 6,
            tos: 0x2e,
            size: 1500,
            tcp_flags: 0x10,
            tcp_window: 64240,
            tcp_tsval: 7,
            tcp_tsecr: 9,
            direction: CaptureDirection::Download,
            prefix: CapturePrefix([0xAB; CAPTURE_PREFIX_BYTES]),
        }
    }

    /// §1.4a schema parity: a bridge `CaptureRecord` maps field-for-field onto
    /// the timeline's `HeimdallEvent` (the prefix is already the inner frame).
    #[test]
    fn capture_record_maps_to_the_heimdall_event_shape() {
        let event = HeimdallEvent::from_capture(record(7, "192.0.2.1", "198.51.100.2"));
        assert_eq!(event.timestamp, 42);
        assert_eq!(event.src.as_ip().to_string(), "192.0.2.1");
        assert_eq!(event.dst.as_ip().to_string(), "198.51.100.2");
        assert_eq!(event.src_port, 1234);
        assert_eq!(event.dst_port, 80);
        assert_eq!(event.ip_protocol, 6);
        assert_eq!(event.tos, 0x2e);
        assert_eq!(event.size, 1500);
        assert_eq!(event.tcp_flags, 0x10);
        assert_eq!(event.tcp_window, 64240);
        assert_eq!(event.tcp_tsval, 7);
        assert_eq!(event.tcp_tsecr, 9);
        assert_eq!(event.packet_data, [0xAB; CAPTURE_PREFIX_BYTES]);
    }

    /// §1.4a/§1.5 userspace filter: only records on the watched slot that
    /// carry the requested device IP (either direction) are stored.
    #[test]
    fn bridge_session_filters_by_slot_and_ip() {
        let ip: IpAddr = "192.0.2.1".parse().expect("test invariant");
        assert!(record_matches(&record(7, "192.0.2.1", "203.0.113.9"), 7, ip));
        assert!(record_matches(&record(7, "203.0.113.9", "192.0.2.1"), 7, ip));
        assert!(
            !record_matches(&record(8, "192.0.2.1", "203.0.113.9"), 7, ip),
            "another slot's traffic must not be stored"
        );
        assert!(
            !record_matches(&record(7, "192.0.2.2", "203.0.113.9"), 7, ip),
            "another IP on the watched circuit must not be stored"
        );
    }

    /// Heimdall card §3 AUTOMATED FEED-SWAP ORACLE: the full v3 client arm
    /// through a REAL analytics socket — SetWatchList → drain →
    /// store_on_timeline → n_second_pcap, with the produced file asserted.
    /// A decoy record (another IP and another slot) must be filtered out.
    #[test]
    fn feed_swap_oracle_drains_filters_and_writes_the_pcap() {
        use analytics_ipc::{
            CaptureBatch, SnapshotService, WatchAck, drain_channel, health_channel,
            queue_stats_channel, snapshot_channel, state_channel, watch_channel,
        };
        use std::thread;

        let _guard = LATCH_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let dir = std::env::temp_dir().join(format!(
            "lqos-heimdall-oracle-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("test invariant")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("test invariant");
        let socket = dir.join("analytics.sock");

        let (requester, _rx) = snapshot_channel(Duration::from_secs(2));
        let (health_requester, _hrx) = health_channel(Duration::from_secs(2));
        let (state_requester, _srx) = state_channel(Duration::from_secs(2));
        let (watch_requester, watch_rx) = watch_channel(Duration::from_secs(2));
        let (drain_requester, drain_rx) = drain_channel(Duration::from_secs(2));
        let (queue_stats_requester, _queue_stats_rx) =
            queue_stats_channel(Duration::from_secs(2));
        let service = SnapshotService::bind_with_capture(
            &socket,
            requester,
            health_requester,
            state_requester,
            watch_requester,
            drain_requester,
            queue_stats_requester,
        )
        .expect("bind capture service");
        thread::spawn(move || {
            let _ = service.serve();
        });

        let collector = thread::spawn(move || {
            if let Ok(req) = watch_rx.recv() {
                assert_eq!(req.watch, vec![7], "the session watches the resolved slot");
                let _ = req.reply.try_send(WatchAck {
                    accepted: 1,
                    rejected: 0,
                });
            }
            if let Ok(req) = drain_rx.recv() {
                let _ = req.reply.try_send(CaptureBatch {
                    records: vec![
                        record(7, "192.0.2.1", "203.0.113.9"),
                        record(7, "192.0.2.2", "203.0.113.9"),
                        record(8, "192.0.2.1", "203.0.113.9"),
                    ],
                    dropped: 0,
                });
            }
            if let Ok(req) = watch_rx.recv() {
                assert!(req.watch.is_empty(), "the session clears its watch on exit");
                let _ = req.reply.try_send(WatchAck {
                    accepted: 0,
                    rejected: 0,
                });
            }
        });

        let filter_ip: IpAddr = "192.0.2.1".parse().expect("test invariant");
        let (session_id, _) =
            hyperfocus_on_target_bridge_with(7, filter_ip, &socket, 1).expect("session starts");

        let mut dump = None;
        for _ in 0..50 {
            if let Some(d) = n_second_packet_dump(session_id) {
                dump = Some(d);
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        let dump = dump.expect("the session finishes and is retained");
        assert_eq!(dump.len(), 1, "only the matching slot+IP record is stored");
        assert_eq!(dump[0].src, "192.0.2.1");

        let pcap = n_second_pcap(session_id).expect("the pcap is written");
        let len = std::fs::metadata(&pcap).expect("pcap exists").len();
        assert_eq!(
            len,
            24 + 16 + CAPTURE_PREFIX_BYTES as u64,
            "24-byte file header + 16-byte record header + the 128-byte inner prefix"
        );
        let _ = std::fs::remove_file(&pcap);
        let _ = std::fs::remove_dir_all(&dir);
        collector.join().expect("collector completes");
    }

    /// Drop-guard: a panicking capture-session thread must still release the
 /// single-session latch (the latent leak named). Without the
    /// guard, the latch stays true and every later session is rejected forever.
    #[test]
    fn hyperfocus_latch_guard_releases_on_unwind() {
        let _guard = LATCH_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert!(!HYPERFOCUSED.load(std::sync::atomic::Ordering::Relaxed));
        let result = std::panic::catch_unwind(|| {
            HYPERFOCUSED.store(true, std::sync::atomic::Ordering::Relaxed);
            let _latch = HyperfocusLatch;
            panic!("simulated capture-session panic");
        });
        assert!(result.is_err(), "the panic propagates to the catch");
        assert!(
            !HYPERFOCUSED.load(std::sync::atomic::Ordering::Relaxed),
            "the guard releases the latch during unwind"
        );
    }

    /// Error propagation: a failed dump-file create (a path occupied by a
    /// directory, or a foreign-owned file under `fs.protected_regular`) must
    /// return `None`, never panic and kill the caller's thread.
    #[test]
    fn n_second_pcap_create_failure_returns_none() {
        let id = FOCUS_SESSION_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = format!("/tmp/cap_sess_{id}");
        std::fs::create_dir_all(&path).expect("test invariant");
        FOCUS_SESSIONS.insert(
            id,
            FocusSession {
                expire: u64::MAX,
                data: TIMELINE.data.clone(),
                dump_filename: None,
            },
        );
        let out = n_second_pcap(id);
        assert!(out.is_none(), "a create failure returns None, never panics");
        assert!(
            FOCUS_SESSIONS
                .get(&id)
                .expect("session retained")
                .dump_filename
                .is_none(),
            "a failed create does not claim a dump filename"
        );
        let _ = std::fs::remove_dir_all(&path);
        FOCUS_SESSIONS.remove(&id);
    }
}
