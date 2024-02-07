use crate::{
  heimdall_watch_ip,
  pcap::{PcapFileHeader, PcapPacketHeader},
  perf_interface::{HeimdallEvent, PACKET_OCTET_SIZE},
  set_heimdall_mode, HeimdallMode, SESSION_EXPIRE_SECONDS,
  TIMELINE_EXPIRE_SECS,
};
use dashmap::{DashMap, DashSet};
use lqos_bus::{tos_parser, PacketHeader};
use lqos_utils::{unix_time::time_since_boot, XdpIpAddress};
use once_cell::sync::Lazy;
use std::{
  fs::{remove_file, File},
  io::Write,
  path::Path,
  sync::atomic::{AtomicBool, AtomicUsize},
  time::Duration,
};
use zerocopy::AsBytes;

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
}

struct Timeline {
  data: DashSet<HeimdallEvent>,
}

impl Timeline {
  fn new() -> Self {
    Self { data: DashSet::new() }
  }
}

static TIMELINE: Lazy<Timeline> = Lazy::new(Timeline::new);

pub(crate) fn store_on_timeline(event: HeimdallEvent) {
  TIMELINE.data.insert(event); // We're moving here deliberately
}

pub(crate) fn expire_timeline() {
  if let Ok(now) = time_since_boot() {
    let since_boot = Duration::from(now);
    let expire = (since_boot - Duration::from_secs(TIMELINE_EXPIRE_SECS))
      .as_nanos() as u64;
    TIMELINE.data.retain(|v| v.timestamp > expire);
    FOCUS_SESSIONS.retain(|_, v| v.expire < since_boot.as_nanos() as u64);
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
static FOCUS_SESSIONS: Lazy<DashMap<usize, FocusSession>> =
  Lazy::new(DashMap::new);

/// Tell Heimdall to spend the next 10 seconds obsessing over an IP address,
/// collecting full packet headers. This hurts your CPU, so use it sparingly.
///
/// This spawns a thread that keeps Heimdall in Analysis mode (saving packet
/// data to userspace) for 10 seconds, before reverting to WatchOnly mode.
///
/// You can only do this on one target at a time.
///
/// ## Returns
///
/// * Either `None` or...
/// * The id number of the collection session for analysis.
pub fn hyperfocus_on_target(ip: XdpIpAddress) -> Option<(usize, usize)> {
  if HYPERFOCUSED.compare_exchange(
    false,
    true,
    std::sync::atomic::Ordering::Relaxed,
    std::sync::atomic::Ordering::Relaxed,
  ) == Ok(false)
  {
    // If explicitly set, obtain the capture time. Otherwise, default to
    // a reasonable 10 seconds.
    let capture_time = if let Ok(cfg) = lqos_config::load_config() {
      cfg.packet_capture_time
    } else {
      10
    };
    let new_id =
      FOCUS_SESSION_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::thread::spawn(move || {
      for _ in 0..capture_time {
        let _ = set_heimdall_mode(HeimdallMode::Analysis);
        heimdall_watch_ip(ip);
        std::thread::sleep(Duration::from_secs(1));
      }
      let _ = set_heimdall_mode(HeimdallMode::WatchOnly);

      if let Ok(now) = time_since_boot() {
        let since_boot = Duration::from(now);
        let expire = (since_boot - Duration::from_secs(SESSION_EXPIRE_SECONDS))
          .as_nanos() as u64;
        FOCUS_SESSIONS.insert(
          new_id,
          FocusSession {
            expire,
            data: TIMELINE.data.clone(),
            dump_filename: None,
          },
        );
      }

      HYPERFOCUSED.store(false, std::sync::atomic::Ordering::Relaxed);
    });
    Some((new_id, capture_time))
  } else {
    log::warn!(
      "Heimdall was busy and won't start another collection session."
    );
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
    let mut out = File::create(path).expect("Unable to create {filename}");
    out
      .write_all(PcapFileHeader::new().as_bytes())
      .expect("Unable to write to {filename}");

    session
    .data
    .iter()
    .map(|e| (e.packet_data, e.size, PcapPacketHeader::from_heimdall(&e)))
    .for_each(
      |(data, size, p)| {
        out.write_all(p.as_bytes()).expect("Unable to write to {filename}");
        if size < PACKET_OCTET_SIZE as u32 {
          out.write_all(&data[0 .. size as usize]).unwrap();
        } else {
          out.write_all(&data).unwrap();
        }
      },
    );

    Some(filename)
  } else {
    None
  }
}
