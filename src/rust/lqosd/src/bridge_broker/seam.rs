//! Seam client for the LibreQoS v3 bridge control segment.
//!
//! Mirrors the bridge's `/dev/shm/libreqos-v3-ctl` SPSC protocol
//! (`lqos_dataplane_types::wire` for the frame bodies; the segment
//! layout mirrors the bridge's `control_channel.rs` and the reference
//! producer in the bridge repo). Attach is retry-tolerant: the bridge
//! may be absent (boot ordering, crash restart) — `attach()`
//! fails softly and the caller re-tries on its cadence.

use std::ffi::CString;
use std::sync::atomic::{AtomicU64, Ordering};

use lqos_dataplane_types::wire::{FrameBody, MAX_BATCH_FRAMES, PROTOCOL_VERSION};

pub(crate) const SEGMENT_NAME: &str = "libreqos-v3-ctl";
pub(crate) const SEGMENT_DIR: &str = "/dev/shm";
// U1/W1: the segment + ack-frame contract lives in ONE home
// (`lqos_dataplane_types::wire`) — both ends compile THESE, never a hand-copy
// (the previous two hand-copies could drift silently: identical `size_of`
// asserts, different fields ⇒ a silent layout mismatch).
pub(crate) use lqos_dataplane_types::wire::{ACK_SLOTS, ACK_SLOT_BYTES, AckFrame};
use lqos_dataplane_types::wire::{
    CMD_LEN_PREFIX_BYTES, CMD_SLOTS, CMD_SLOT_BYTES, HEADER_BYTES, SEGMENT_MAGIC as MAGIC,
};

/// Mirror of the bridge's fixed segment header (`repr(C)`, same field
/// order and paddings — the cross-process layout contract).
#[repr(C)]
struct SegmentHeader {
    magic: u32,
    version: u32,
    boot_id: [u8; 16],
    cmd_slots: u32,
    cmd_slot_bytes: u32,
    ack_slots: u32,
    ack_slot_bytes: u32,
    ack_dropped: u64,
    _pad0: [u8; 16],
    cmd_producer: AtomicU64,
    _pad1: [u8; 56],
    cmd_consumer: AtomicU64,
    _pad2: [u8; 56],
    ack_producer: AtomicU64,
    _pad3: [u8; 56],
    ack_consumer: AtomicU64,
    _pad4: [u8; 56],
}

const _: () = assert!(std::mem::size_of::<SegmentHeader>() == HEADER_BYTES);

// U1/W1: `AckFrame` is the SHARED definition (`lqos_dataplane_types::wire`) —
// imported above; the shared crate owns the layout + the size assert.

/// Why an attach attempt did not yield a usable segment.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AttachError {
    Absent,
    BadLayout(String),
    /// U1/W5 (the atomic-upgrade guard): the segment EXISTS with the right magic
    /// but a DIFFERENT `PROTOCOL_VERSION` — an OLD or NEW bridge against this
    /// lqosd. The broker REFUSES to attach (never a silent ack-ring stride
    /// mismatch) and surfaces it LOUDLY: "upgrade the bridge and the control
    /// plane together".
    ProtocolMismatch { bridge: u32, expected: u32 },
}

/// One live attach: the mapped segment plus this client's cursors.
pub(crate) struct SeamSegment {
    fd: i32,
    base: *mut u8,
    map_len: usize,
    cmd_produced: u64,
    ack_consumed: u64,
    /// The bridge boot that created this segment (restart detection).
    pub boot_id: [u8; 16],
}

// The mapping is shared with the bridge process; the raw pointer is
// only derefererved through the (single-threaded) broker actor.
unsafe impl Send for SeamSegment {}

#[derive(Debug)]
pub(crate) struct PushError;

impl SeamSegment {
    /// Opens + maps the segment, verifying magic/version/layout and
    /// adopting the published indices. `Err(Absent)` = the bridge is
    /// not up yet (the caller retries).
    pub(crate) fn attach(name: &str) -> Result<Self, AttachError> {
        // F-E1 live-pairing defect (bench, 2026-09-06): shm_open PREPENDS
        // /dev/shm to its name argument — passing the full /dev/shm path
        // opened /dev/shm//dev/shm/... and attach failed as Absent on
        // every poll while the segment sat right there. Pass the bare
        // name (SEGMENT_DIR stays for the plain-open is_replaced path).
        let path = CString::new(name.trim_start_matches('/')).unwrap_or_else(|_| {
            CString::new(SEGMENT_NAME).expect("fallback name")
        });
        let fd = unsafe { libc::shm_open(path.as_ptr(), libc::O_RDWR, 0o0) };
        if fd < 0 {
            return Err(AttachError::Absent);
        }
        let map_len = HEADER_BYTES + CMD_SLOTS * CMD_SLOT_BYTES + ACK_SLOTS * ACK_SLOT_BYTES;
        let base = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                map_len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        if base as isize == -1 {
            unsafe { libc::close(fd) };
            return Err(AttachError::BadLayout("mmap failed".into()));
        }
        let base = base as *mut u8;
        let header = base as *const SegmentHeader;
        let (magic, version, boot_id) =
            unsafe { ((*header).magic, (*header).version, (*header).boot_id) };
        if magic != MAGIC {
            unsafe {
                libc::munmap(base as *mut libc::c_void, map_len);
                libc::close(fd);
            }
            return Err(AttachError::BadLayout(format!(
                "magic mismatch ({magic:#x})"
            )));
        }
        if version != PROTOCOL_VERSION {
            // U1/W5: a REAL bridge whose protocol differs — the fail-closed
            // guard (no silent stride mismatch). Named distinctly so the broker
            // tells the operator to upgrade the PAIR together.
            unsafe {
                libc::munmap(base as *mut libc::c_void, map_len);
                libc::close(fd);
            }
            return Err(AttachError::ProtocolMismatch {
                bridge: version,
                expected: PROTOCOL_VERSION,
            });
        }
        let adopted = unsafe {
            (
                (*header).cmd_producer.load(Ordering::Acquire),
                (*header).ack_producer.load(Ordering::Acquire),
            )
        };
        Ok(Self {
            fd,
            base,
            map_len,
            cmd_produced: adopted.0,
            ack_consumed: adopted.1,
            boot_id,
        })
    }

    /// Liveness (fresh-#12 G4): compares the on-disk identity with the
    /// mapped fd's — a bridge restart replaces the segment (new inode).
    pub(crate) fn is_replaced(&self, name: &str) -> bool {
        let mut fd_stat: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(self.fd, &mut fd_stat) } != 0 {
            return true;
        }
        let path = CString::new(format!("{SEGMENT_DIR}/{name}")).unwrap_or_else(|_| {
            CString::new(format!("{SEGMENT_DIR}/{SEGMENT_NAME}")).expect("fallback path")
        });
        let mut path_stat: libc::stat = unsafe { std::mem::zeroed() };
        // F-E1 sibling: shm_open PREPENDS /dev/shm, so the full SEGMENT_DIR
        // path must go through a plain open() — shm_open(full_path) fails
        // EINVAL and made this predicate always-true (constant re-attach).
        let rc = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY) };
        if rc < 0 {
            return true; // gone entirely
        }
        let ok = unsafe { libc::fstat(rc, &mut path_stat) } == 0;
        unsafe { libc::close(rc) };
        !ok || fd_stat.st_dev != path_stat.st_dev || fd_stat.st_ino != path_stat.st_ino
    }

    fn header(&self) -> &SegmentHeader {
        unsafe { &*(self.base as *const SegmentHeader) }
    }

    fn slot_ptr(&self, slot: u64, within: usize, len: usize) -> *mut u8 {
        unsafe {
            self.base
                .add(within + (slot as usize) * CMD_SLOT_BYTES + len)
        }
    }

    fn ack_ptr(&self, slot: u64) -> *const AckFrame {
        unsafe {
            (self
                .base
                .add(HEADER_BYTES + CMD_SLOTS * CMD_SLOT_BYTES + (slot as usize) * ACK_SLOT_BYTES))
                as *const AckFrame
        }
    }

    /// Test-only: inject a bridge ack into the ack ring. This is the producer
    /// side of the real `resolve_pending` wait path.
    #[cfg(test)]
    pub(crate) fn push_test_ack(&mut self, frame: &AckFrame) {
        let produced = self.header().ack_producer.load(Ordering::Acquire);
        let slot = produced % ACK_SLOTS as u64;
        unsafe {
            std::ptr::write(self.ack_ptr(slot) as *mut AckFrame, *frame);
        }
        self.header()
            .ack_producer
            .store(produced.saturating_add(1), Ordering::Release);
    }

    /// Writes one frame into the next command slot with backpressure
    /// (the bridge frees slots as it consumes them; bounded wait).
    pub(crate) fn push_frame(&mut self, frame: &[u8]) -> Result<(), PushError> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let produced = self.cmd_produced;
            let consumed = self.header().cmd_consumer.load(Ordering::Acquire);
            if produced.saturating_sub(consumed) < CMD_SLOTS as u64 {
                let slot = produced % CMD_SLOTS as u64;
                // Slot contract (bridge control_channel.rs pop_command):
                // [frame_len: u64 LE][frame bytes]. The frame itself
                // carries [PROTOCOL_VERSION: u32][body_len: u32][CBOR]
                // (encode_frame). The original defect wrote
                // [version: u32][frame_len: u32] — the bridge read one
                // nonsense u64 length and typed the frame Corrupt
                // (REJECTED_INVALID_FRAME).
                let prefix = (frame.len() as u64).to_le_bytes();
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        prefix.as_ptr(),
                        self.slot_ptr(slot, HEADER_BYTES, 0),
                        8,
                    );
                    std::ptr::copy_nonoverlapping(
                        frame.as_ptr(),
                        self.slot_ptr(slot, HEADER_BYTES, 8),
                        frame.len(),
                    );
                }
                // G9: publish the index only after the full frame is in place.
                self.header()
                    .cmd_producer
                    .store(produced + 1, Ordering::Release);
                self.cmd_produced = produced + 1;
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err(PushError);
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// Waits for the next ack (single ack per transaction/batch; the
    /// bridge acks once per completed publish). `None` = timeout.
    pub(crate) fn wait_ack(&mut self, timeout: std::time::Duration) -> Option<AckFrame> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let produced = self.header().ack_producer.load(Ordering::Acquire);
            if produced > self.ack_consumed {
                let slot = self.ack_consumed % ACK_SLOTS as u64;
                let frame = unsafe { *self.ack_ptr(slot) };
                self.ack_consumed += 1;
                self.header()
                    .ack_consumer
                    .store(self.ack_consumed, Ordering::Release);
                return Some(frame);
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

impl Drop for SeamSegment {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.base as *mut libc::c_void, self.map_len);
            libc::close(self.fd);
        }
    }
}

/// CBOR-encodes one frame body with the v2 prefix.
pub(crate) fn encode_frame(body: &FrameBody) -> Result<Vec<u8>, PushError> {
    let mut cbor = Vec::new();
    ciborium::into_writer(body, &mut cbor).map_err(|_| PushError)?;
    let mut frame = Vec::with_capacity(8 + cbor.len());
    frame.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    frame.extend_from_slice(&(cbor.len() as u32).to_le_bytes());
    frame.extend_from_slice(&cbor);
    Ok(frame)
}

/// Encodes a transaction as one frame, or a bounded batch when it
/// exceeds a slot's payload budget (idx 0 carries schema_version).
pub(crate) fn encode_transaction(
    tx: &lqos_dataplane_types::DataplaneTransaction,
    desired_world_content_key: Option<&str>,
) -> Result<Vec<Vec<u8>>, PushError> {
    let single = FrameBody::Transaction {
        transaction: tx.clone(),
        desired_world_content_key: desired_world_content_key.map(str::to_string),
    };
    let single = encode_frame(&single)?;
    if single.len() <= CMD_SLOT_BYTES - CMD_LEN_PREFIX_BYTES {
        return Ok(vec![single]);
    }
    let mut cbor = Vec::new();
    ciborium::into_writer(tx, &mut cbor).map_err(|_| PushError)?;
    let chunk_budget = CMD_SLOT_BYTES - CMD_LEN_PREFIX_BYTES - 8 - 1024;
    let n = cbor.len().div_ceil(chunk_budget);
    if n > MAX_BATCH_FRAMES as usize {
        return Err(PushError);
    }
    let n = n as u16;
    let mut frames = Vec::with_capacity(n as usize);
    for idx in 0..n {
        let start = idx as usize * chunk_budget;
        let end = ((idx as usize + 1) * chunk_budget).min(cbor.len());
        let seg = FrameBody::Batch {
            n,
            idx,
            generation: tx.generation.clone(),
            schema_version: if idx == 0 { tx.schema_version } else { None },
            desired_world_content_key: if idx == 0 {
                desired_world_content_key.map(str::to_string)
            } else {
                None
            },
            payload: cbor[start..end].to_vec(),
        };
        frames.push(encode_frame(&seg)?);
    }
    Ok(frames)
}

/// Test-only: creates a minimal valid control segment at `name` (small
/// file, valid header) so `attach()` succeeds. The broker never touches
/// the ring while idle, so a 4 KiB object is sufficient for the
/// ghost-segment liveness units.
#[cfg(test)]
pub(crate) fn create_test_segment(name: &str, boot_id: [u8; 16]) {
    let cname = CString::new(name.trim_start_matches('/')).expect("test segment name");
    unsafe {
        libc::shm_unlink(cname.as_ptr());
        let fd = libc::shm_open(cname.as_ptr(), libc::O_CREAT | libc::O_RDWR, 0o600);
        assert!(fd >= 0, "shm_open failed");
        assert_eq!(libc::ftruncate(fd, 4096), 0, "ftruncate failed");
        let base = libc::mmap(
            std::ptr::null_mut(),
            4096,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            0,
        );
        assert!(base as isize != -1, "mmap failed");
        let header = base as *mut SegmentHeader;
        (*header).magic = MAGIC;
        (*header).version = PROTOCOL_VERSION;
        (*header).boot_id = boot_id;
        (*header).cmd_slots = CMD_SLOTS as u32;
        (*header).cmd_slot_bytes = CMD_SLOT_BYTES as u32;
        (*header).ack_slots = ACK_SLOTS as u32;
        (*header).ack_slot_bytes = ACK_SLOT_BYTES as u32;
        libc::munmap(base, 4096);
        libc::close(fd);
    }
}

/// Test-only: create a FULL-LAYOUT segment (the 4 KiB helper above cannot
/// address the ack ring at `HEADER + CMD_SLOTS*CMD_SLOT_BYTES`). Used by the
/// real ack-path tests; the file is sparse and only the touched pages commit.
#[cfg(test)]
pub(crate) fn create_full_test_segment(name: &str, boot_id: [u8; 16]) {
    let cname = CString::new(name.trim_start_matches('/')).expect("test segment name");
    let map_len = HEADER_BYTES + CMD_SLOTS * CMD_SLOT_BYTES + ACK_SLOTS * ACK_SLOT_BYTES;
    unsafe {
        libc::shm_unlink(cname.as_ptr());
        let fd = libc::shm_open(cname.as_ptr(), libc::O_CREAT | libc::O_RDWR, 0o600);
        assert!(fd >= 0, "shm_open failed");
        assert_eq!(
            libc::ftruncate(fd, map_len as libc::off_t),
            0,
            "ftruncate failed"
        );
        let base = libc::mmap(
            std::ptr::null_mut(),
            map_len,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            0,
        );
        assert!(base as isize != -1, "mmap failed");
        let header = base as *mut SegmentHeader;
        (*header).magic = MAGIC;
        (*header).version = PROTOCOL_VERSION;
        (*header).boot_id = boot_id;
        (*header).cmd_slots = CMD_SLOTS as u32;
        (*header).cmd_slot_bytes = CMD_SLOT_BYTES as u32;
        (*header).ack_slots = ACK_SLOTS as u32;
        (*header).ack_slot_bytes = ACK_SLOT_BYTES as u32;
        libc::munmap(base, map_len);
        libc::close(fd);
    }
}

/// Test-only: unlinks the named control segment (simulates the bridge's
/// unconditional recreate or death).
#[cfg(test)]
pub(crate) fn unlink_test_segment(name: &str) {
    let cname = CString::new(name.trim_start_matches('/')).expect("test segment name");
    unsafe {
        libc::shm_unlink(cname.as_ptr());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F-E1 live-pairing regression: `attach` must treat `name` as an
    /// shm_open NAME (glibc prepends /dev/shm) — the original defect
    /// passed a full path, opening /dev/shm//dev/shm/... and failed as
    /// `Absent` on every poll while the segment existed. A fresh empty
    /// object reaches attach's LAYOUT verification (BadLayout), proving
    /// the open succeeded.
    #[test]
    fn attach_opens_a_bare_named_shm_segment() {
        let name = format!("lqosd-seam-test-{}", std::process::id());
        let cname = CString::new(name.as_str()).expect("test name");
        unsafe {
            let fd = libc::shm_open(cname.as_ptr(), libc::O_CREAT | libc::O_RDWR, 0o600);
            assert!(fd >= 0, "shm create failed");
            assert_eq!(libc::ftruncate(fd, 4096), 0, "ftruncate failed");
            libc::close(fd);
        }
        let result = SeamSegment::attach(&name);
        unsafe {
            libc::shm_unlink(cname.as_ptr());
        }
        assert!(
            matches!(result, Err(AttachError::BadLayout(_))),
            "attach must reach layout verification (open succeeded); got {:?}",
            result.map(|_| ())
        );
    }

    /// U1/W5 (the atomic-upgrade guard): a segment with the RIGHT magic but a
    /// DIFFERENT protocol version is a typed `ProtocolMismatch` — the broker
    /// REFUSES (never a silent ack-ring stride mismatch between an old/new
    /// bridge and this lqosd).
    #[test]
    fn attach_refuses_a_protocol_version_mismatch() {
        let name = format!("lqosd-seam-ver-{}", std::process::id());
        let cname = CString::new(name.as_str()).expect("test name");
        unsafe {
            libc::shm_unlink(cname.as_ptr());
            let fd = libc::shm_open(cname.as_ptr(), libc::O_CREAT | libc::O_RDWR, 0o600);
            assert!(fd >= 0, "shm create failed");
            assert_eq!(libc::ftruncate(fd, 4096), 0, "ftruncate failed");
            let base = libc::mmap(
                std::ptr::null_mut(),
                4096,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            );
            assert!(base as isize != -1, "mmap failed");
            let header = base as *mut SegmentHeader;
            (*header).magic = MAGIC;
            (*header).version = PROTOCOL_VERSION.wrapping_sub(1); // an OLD bridge
            libc::munmap(base, 4096);
            libc::close(fd);
        }
        let result = SeamSegment::attach(&name);
        unsafe {
            libc::shm_unlink(cname.as_ptr());
        }
        match result {
            Err(AttachError::ProtocolMismatch { bridge, expected }) => {
                assert_eq!(expected, PROTOCOL_VERSION, "the expected version is ours");
                assert_ne!(bridge, expected, "the bridge's version differs");
            }
            Err(other) => panic!("expected ProtocolMismatch, got {other:?}"),
            Ok(_) => panic!("expected ProtocolMismatch, got a successful attach"),
        }
    }
}
