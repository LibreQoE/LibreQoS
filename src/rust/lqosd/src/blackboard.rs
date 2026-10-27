use crossbeam_channel::Sender;
use lqos_bus::BlackboardSystem;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::OnceLock;
use tracing::{info, warn};

pub static BLACKBOARD_SENDER: OnceLock<Sender<BlackboardCommand>> = OnceLock::new();

pub enum BlackboardCommand {
    FinishSession,
    BlackboardData {
        subsystem: BlackboardSystem,
        key: String,
        value: String,
    },
    BlackboardBlob {
        tag: String,
        part: usize,
        blob: Vec<u8>,
    },
}

#[derive(Serialize)]
struct Blackboard {
    system: HashMap<String, String>,
    sites: HashMap<String, String>,
    circuits: HashMap<String, String>,
    devices: HashMap<String, String>,
    blobs: HashMap<String, Vec<u8>>,
}

impl Blackboard {
    /// Route one `BlackboardData` command into its named map. Pure state, no
    /// I/O — the A2 oracle pins the `BlackboardSystem` → map routing.
    fn insert_data(&mut self, subsystem: BlackboardSystem, key: String, value: String) {
        let target = match subsystem {
            BlackboardSystem::System => &mut self.system,
            BlackboardSystem::Site => &mut self.sites,
            BlackboardSystem::Circuit => &mut self.circuits,
            BlackboardSystem::Device => &mut self.devices,
        };
        target.insert(key, value);
    }

    /// Apply one `BlackboardBlob` part: part 0 starts a tag, later parts append
    /// in order. A part > 0 for an unseen tag is dropped (never a fabricated
    /// partial blob).
    fn append_blob(&mut self, tag: String, part: usize, blob: Vec<u8>) {
        if part == 0 {
            self.blobs.insert(tag, blob);
        } else if let Some(existing) = self.blobs.get_mut(&tag) {
            existing.extend_from_slice(&blob);
        }
    }
}

/// Sum of key + value byte lengths of a `String → String` map. This is an
/// attribution estimate only (map overhead/ordering excluded); it never affects
/// serialization. Removal gate 2026-12-31 (`.2.2` instrument).
fn map_bytes(map: &HashMap<String, String>) -> usize {
    map.iter().map(|(k, v)| k.len() + v.len()).sum()
}

/// Per-blob-tag byte sizes, largest first, so an operator/log reader sees the
/// dominant tag immediately. Removal gate 2026-12-31 (`.2.2` instrument).
fn blob_tag_sizes(blobs: &HashMap<String, Vec<u8>>) -> Vec<(String, usize)> {
    let mut sizes: Vec<(String, usize)> = blobs
        .iter()
        .map(|(tag, blob)| (tag.clone(), blob.len()))
        .collect();
    sizes.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    sizes
}

/// Split the serialized board CBOR into ordered fragments of at most `max`
/// bytes. `.2.2` Shape A: the lts2 ingestor byte-concatenates the fragments
/// within one arena batch and decodes ONE `Blackboard`, so the split must be
/// contiguous and ordered. Removal gate: none — permanent fix.
fn split_board_fragments(bytes: &[u8], max: usize) -> Vec<Vec<u8>> {
    assert!(max > 0, "fragment size must be positive");
    bytes.chunks(max).map(<[u8]>::to_vec).collect()
}

pub fn start_blackboard() {
    let (tx, rx) = crossbeam_channel::bounded(65535);
    std::thread::spawn(move || {
        let mut board = Blackboard {
            system: HashMap::new(),
            sites: HashMap::new(),
            circuits: HashMap::new(),
            devices: HashMap::new(),
            blobs: HashMap::new(),
        };

        loop {
            match rx.recv() {
                Ok(BlackboardCommand::FinishSession) => {
                    // If empty, do nothing
                    if board.circuits.is_empty()
                        && board.sites.is_empty()
                        && board.system.is_empty()
                        && board.blobs.is_empty()
                    {
                        continue;
                    }

                    // `.2.2` per-tag byte attribution (DRIVER-directed; removal
                    // gate 2026-12-31). Behavior-neutral: names the dominant
                    // blackboard field before any bound/decouple is chosen. The
                    // `blobs` map holds the UISP-grown payloads (`uisp_sites` /
                    // `uisp_devices` / `uisp_data_links` / debug `uisp-graph`);
                    // the four string maps are small.
                    let tag_sizes = blob_tag_sizes(&board.blobs);
                    let tag_total: usize = tag_sizes.iter().map(|(_, bytes)| bytes).sum();

                    // Serialize CBOR to a vec of u8
                    info!("Sending blackboard data");
                    let cbor = match serde_cbor::to_vec(&board) {
                        Ok(j) => j,
                        Err(e) => {
                            warn!("Failed to serialize blackboard: {}", e);
                            continue;
                        }
                    };
                    let by_tag = tag_sizes
                        .iter()
                        .map(|(tag, bytes)| format!("{tag}={bytes}"))
                        .collect::<Vec<_>>()
                        .join(" ");
                    info!(
                        "Blackboard session bytes: serialized={} blobs_total={} blobs_tags={} \
                         system={{count={},bytes={}}} sites={{count={},bytes={}}} \
                         circuits={{count={},bytes={}}} devices={{count={},bytes={}}} \
                         by_tag=[{}]",
                        cbor.len(),
                        tag_total,
                        tag_sizes.len(),
                        board.system.len(),
                        map_bytes(&board.system),
                        board.sites.len(),
                        map_bytes(&board.sites),
                        board.circuits.len(),
                        map_bytes(&board.circuits),
                        board.devices.len(),
                        map_bytes(&board.devices),
                        by_tag,
                    );
                    // `.2.2` Shape A: split the serialized board into ordered
                    // fragments so each becomes its own ≤budget session; the
                    // lts2 server byte-concatenates them and decodes one board.
                    let max = crate::lts2_sys::blackboard_fragment_max_bytes().max(1);
                    let fragments = split_board_fragments(&cbor, max);
                    if let Err(e) = crate::lts2_sys::blackboard_fragments(fragments) {
                        warn!("Failed to send blackboard data: {}", e);
                    }
                    board.circuits.clear();
                    board.sites.clear();
                    board.system.clear();
                    board.devices.clear();
                    board.blobs.clear();
                }
                Ok(BlackboardCommand::BlackboardData {
                    subsystem,
                    key,
                    value,
                }) => {
                    info!("Received data: {} = {}", key, value);
                    board.insert_data(subsystem, key, value);
                }
                Ok(BlackboardCommand::BlackboardBlob { tag, part, blob }) => {
                    info!("Received blob: {tag}, part {part}");
                    board.append_blob(tag, part, blob);
                }
                Err(_) => break,
            }
        }
        warn!("Blackboard thread exiting");
    });
    let _ = BLACKBOARD_SENDER.set(tx);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_board() -> Blackboard {
        Blackboard {
            system: HashMap::new(),
            sites: HashMap::new(),
            circuits: HashMap::new(),
            devices: HashMap::new(),
            blobs: HashMap::new(),
        }
    }

    /// A2 oracle: each `BlackboardSystem` names the map it lands in, so a
    /// session emits the expected system/sites/circuits/devices shape.
    #[test]
    fn data_routes_each_subsystem_to_its_named_map() {
        let mut board = empty_board();
        board.insert_data(BlackboardSystem::System, "s".into(), "1".into());
        board.insert_data(BlackboardSystem::Site, "si".into(), "2".into());
        board.insert_data(BlackboardSystem::Circuit, "c".into(), "3".into());
        board.insert_data(BlackboardSystem::Device, "d".into(), "4".into());

        assert_eq!(board.system.get("s").map(String::as_str), Some("1"));
        assert_eq!(board.sites.get("si").map(String::as_str), Some("2"));
        assert_eq!(board.circuits.get("c").map(String::as_str), Some("3"));
        assert_eq!(board.devices.get("d").map(String::as_str), Some("4"));
        assert!(board.blobs.is_empty());
    }

    /// Re-inserting a key replaces the value (last write wins) — the maps are
    /// keyed, not append-only.
    #[test]
    fn data_insert_replaces_a_prior_value() {
        let mut board = empty_board();
        board.insert_data(BlackboardSystem::System, "k".into(), "old".into());
        board.insert_data(BlackboardSystem::System, "k".into(), "new".into());
        assert_eq!(board.system.get("k").map(String::as_str), Some("new"));
        assert_eq!(board.system.len(), 1);
    }

    /// A2 oracle: the CBOR flush carries the four named maps + the blob map, so
    /// the consumer can decode system/sites/circuits/devices.
    #[test]
    fn cbor_serialization_carries_the_named_maps() {
        let mut board = empty_board();
        board.insert_data(BlackboardSystem::System, "version".into(), "3.0.7".into());
        board.insert_data(BlackboardSystem::Circuit, "c1".into(), "up".into());
        board.append_blob("b".into(), 0, vec![1, 2]);
        board.append_blob("b".into(), 1, vec![3]);

        let cbor = serde_cbor::to_vec(&board).expect("blackboard serializes");
        let value: serde_cbor::Value = serde_cbor::from_slice(&cbor).expect("cbor decodes");
        let serde_cbor::Value::Map(fields) = value else {
            panic!("blackboard must serialize as a CBOR map");
        };
        let find = |name: &str| {
            fields
                .iter()
                .find_map(|(k, v)| (k == &serde_cbor::Value::Text(name.to_string())).then_some(v))
        };
        for name in ["system", "sites", "circuits", "devices", "blobs"] {
            assert!(find(name).is_some(), "CBOR carries the `{name}` map");
        }
    }

    /// Blob parts concatenate in order; a later part with no part-0 start is
    /// dropped (never a fabricated partial blob).
    #[test]
    fn blob_parts_append_and_orphan_parts_are_dropped() {
        let mut board = empty_board();
        board.append_blob("tag".into(), 1, vec![9]);
        assert!(board.blobs.is_empty(), "orphan part ignored");
        board.append_blob("tag".into(), 0, vec![1, 2]);
        board.append_blob("tag".into(), 1, vec![3, 4]);
        assert_eq!(board.blobs.get("tag"), Some(&vec![1, 2, 3, 4]));
    }

    /// `.2.2` oracle: `map_bytes` sums key + value lengths (the attribution
    /// estimate), so the logged map bytes are non-vacuous.
    #[test]
    fn map_bytes_sums_key_and_value_lengths() {
        let mut map = HashMap::new();
        assert_eq!(map_bytes(&map), 0, "empty map is zero bytes");
        map.insert("ab".to_string(), "cde".to_string());
        map.insert("f".to_string(), "g".to_string());
        assert_eq!(map_bytes(&map), (2 + 3) + (1 + 1));
    }

    /// `.2.2` oracle: per-tag sizes are reported largest-first, so the viewer
    /// sees the dominant UISP tag without sorting the log by eye.
    #[test]
    fn blob_tag_sizes_are_largest_first() {
        let mut board = empty_board();
        board.append_blob("small".into(), 0, vec![0; 3]);
        board.append_blob("big".into(), 0, vec![0; 30]);
        board.append_blob("mid".into(), 0, vec![0; 9]);
        assert_eq!(
            blob_tag_sizes(&board.blobs),
            vec![
                ("big".to_string(), 30),
                ("mid".to_string(), 9),
                ("small".to_string(), 3)
            ]
        );
    }

    /// `.2.2` Shape A oracle: the ordered fragments reassemble to the original
    /// board bytes (contiguous, in order), each within the requested cap.
    #[test]
    fn split_board_fragments_reassembles_in_order() {
        let board: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        let fragments = split_board_fragments(&board, 256);
        assert_eq!(fragments.len(), 4);
        assert!(fragments.iter().all(|f| f.len() <= 256));
        let rebuilt: Vec<u8> = fragments.into_iter().flatten().collect();
        assert_eq!(rebuilt, board, "fragments must reassemble in order");
    }
}
