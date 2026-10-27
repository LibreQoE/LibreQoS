//! `lqos-74t.11` — the file-backed NO-PATH LATCH that defers the orphans attach.
//!
//! ## Why this exists
//!
//! The UISP graph builder (`full2.rs`) attaches a site with no root path to the
//! generated `orphans` umbrella. A transient UISP input gap flips a site
//! `native-parent <-> orphans`, and the move is an R3 owner-set change
//! (`lqos-74t.11`). The measured capture confirms C2 — a transient device-path
//! drop — so a site is attached to `orphans` only after it has been
//! root-unreachable **strictly longer than [`ORPHAN_LATCH_HOLD_OFF_SECONDS`]**.
//!
//! ## Time-based, not count-based
//!
//! The first revision counted consecutive integration **runs** (`K = 2`). That was
//! refuted LIVE: the runtime runs **1/min exactly**, yet the no-path spanned **≥2
//! runs (~2 min)** (`flash3-11-g2verif`), so the count was under-derived. The guard
//! is now a **wall-clock hold-off** keyed on `first_no_path_unix` — robust to any
//! number of runs per poll.
//!
//! ## Why a FILE (mandatory)
//!
//! The scheduler runs `uisp_integration` as a **fresh subprocess per poll**
//! (`scheduler.py:609`), so in-memory state cannot survive a poll. The latch is
//! therefore a small dedicated file, `<state>/topology/topology_orphan_latch.json`.
//!
//! ## Bounds
//!
//! - **Prune-absent:** entries for sites not present in the current poll are
//!   dropped, so the file is bounded by the present site count.
//! - **Reset on a full reload / import-identity change:** keyed by the ROOT node
//!   id; a changed root clears the latch. (NOT the ~3 s dataplane generation.)
//! - **Retain last parent only if still present:** the caller resolves the
//!   latched parent against the current graph and falls back to `orphans` when it
//!   is absent — a stale latched parent can never produce a dangling parent.
//! - **NTP/clock-jump:** `first_no_path_unix` is wall-clock; a backward jump
//!   clamps `first = now` (a jump can never un-orphan a site).

use lqos_config::Config;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use tracing::warn;

/// The wall-clock hold-off: a site is attached to `orphans` only after it has been
/// root-unreachable for **strictly longer** than this many seconds.
///
/// **SET by the DRIVER to 600 s** (2× the observed ~5-min max transient): the value
/// must strictly exceed the max transient + a margin (`300 s` equals the cited 5-min
/// max and has no margin). Do not change without a fresh census + the DRIVER.
pub const ORPHAN_LATCH_HOLD_OFF_SECONDS: u64 = 600;

/// The dedicated latch file, sibling of `topology_canonical_state.json`.
pub const ORPHAN_LATCH_FILENAME: &str = "topology_orphan_latch.json";

const ORPHAN_LATCH_VERSION: u32 = 2;

/// The per-site decision recorded for the persisted readout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LatchDecision {
    Defer,
    Orphan,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct LatchFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    root: Option<String>,
    #[serde(default)]
    last_run_unix: Option<u64>,
    #[serde(default)]
    sites: BTreeMap<String, LatchEntry>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct LatchEntry {
    /// The wall-clock second the site FIRST became root-unreachable in a poll.
    #[serde(default)]
    first_no_path_unix: Option<u64>,
    #[serde(default)]
    last_parent: Option<String>,
    /// `lqos-74t.63` L1: the FIRST-selected parent among the site's currently
    /// VALID candidate parents, held across polls so a flap between two valid
    /// redundant backhaul links does NOT re-parent. Cleared once the pinned
    /// parent is no longer a valid candidate, or the site orphans.
    #[serde(default)]
    pinned_parent: Option<String>,
    /// The last decision for this site (the persisted readout).
    #[serde(default)]
    last_decision: Option<LatchDecision>,
}

/// The bounded, file-backed per-site no-path latch.
///
/// All methods are keyed by the site's `network_json_id` (the same key the
/// emitted `parents` map uses). One instance lives for exactly one integration
/// poll: `load` at the start, mutate during the build, `prune` + `save` at the
/// end. `Default` (no path) is used by tests — it never touches the filesystem
/// and never defers (an empty latch has no recorded parent).
#[derive(Debug, Default)]
pub struct OrphanLatch {
    /// `None` for the in-memory test latch (no persistence).
    path: Option<PathBuf>,
    root: Option<String>,
    last_run_unix: Option<u64>,
    sites: BTreeMap<String, LatchEntry>,
    /// Sites observed this poll (for prune).
    seen: BTreeSet<String>,
}

impl OrphanLatch {
    /// Load the latch from the topology state directory. Absent/corrupt ⇒ an
    /// empty latch (fail-safe: no deferral until a site has a recorded parent).
    /// A read error is surfaced but never fatal.
    pub fn load(config: &Config) -> Self {
        let path = config.topology_state_read_path(ORPHAN_LATCH_FILENAME);
        let mut latch = Self {
            path: Some(path.clone()),
            ..Default::default()
        };
        match std::fs::read_to_string(&path) {
            Ok(raw) => match serde_json::from_str::<LatchFile>(&raw) {
                Ok(file) => {
                    latch.root = file.root;
                    latch.last_run_unix = file.last_run_unix;
                    latch.sites = file.sites;
                }
                Err(e) => warn!(
                    event = "ORPHAN_LATCH_READ",
                    path = %path.display(),
                    "orphan latch: ignoring an unreadable file ({e}) — starting empty"
                ),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => warn!(
                event = "ORPHAN_LATCH_READ",
                path = %path.display(),
                "orphan latch: cannot read the file ({e}) — starting empty"
            ),
        }
        latch
    }

    /// Record the current ROOT node id and clear the latch when it changes (a
    /// full reload / import-identity change). Called once per poll.
    pub fn set_root(&mut self, root: &str) {
        if self.root.as_deref() != Some(root) {
            if self.root.is_some() {
                warn!(
                    event = "ORPHAN_LATCH_RESET",
                    old_root = ?self.root,
                    new_root = %root,
                    "orphan latch: root/import identity changed — clearing the no-path latch"
                );
                self.sites.clear();
            }
            self.root = Some(root.to_string());
        }
    }

    /// Stamp the poll's wall-clock second (the persisted `last_run_unix`).
    pub fn set_run_unix(&mut self, now_unix: u64) {
        self.last_run_unix = Some(now_unix);
    }

    /// A site WITH a root path: clear its no-path state (and mark it seen).
    pub fn note_path(&mut self, site_id: &str) {
        self.seen.insert(site_id.to_string());
        let entry = self.sites.entry(site_id.to_string()).or_default();
        entry.first_no_path_unix = None;
        entry.last_decision = None;
    }

    /// A site with NO root path: start/advance the hold-off and return the latched
    /// parent to DEFER to while the no-path is **≤** `ORPHAN_LATCH_HOLD_OFF_SECONDS`
    /// old; `None` once it has persisted **strictly longer** (a genuine orphan).
    ///
    /// `now_unix` is injected so the boundary is deterministic in tests.
    pub fn note_no_path(&mut self, site_id: &str, now_unix: u64) -> Option<String> {
        self.seen.insert(site_id.to_string());
        let entry = self.sites.entry(site_id.to_string()).or_default();
        // STICKY ORPHAN: once a site has crossed the hold-off it stays attached to
        // `orphans` until the path returns (`note_path` clears it). This also makes
        // the NTP clamp below incapable of re-deferring an orphaned site.
        if entry.last_decision == Some(LatchDecision::Orphan) {
            return None;
        }
        // NTP/time-jump clamp: a backward jump must not EXTEND the hold-off.
        // Applied only while still DEFERRING (an orphan never reaches here).
        let first = match entry.first_no_path_unix {
            Some(first) if now_unix >= first => first,
            _ => {
                entry.first_no_path_unix = Some(now_unix);
                now_unix
            }
        };
        let age = now_unix.saturating_sub(first);
        if age <= ORPHAN_LATCH_HOLD_OFF_SECONDS {
            entry.last_decision = Some(LatchDecision::Defer);
            entry.last_parent.clone()
        } else {
            entry.last_decision = Some(LatchDecision::Orphan);
            // L1 (`lqos-74t.63`): an orphaned site releases its candidate pin —
            // on recovery it re-pins to whatever parent is then selected.
            entry.pinned_parent = None;
            None
        }
    }

    /// Record a site's CURRENT (non-orphan) parent, for a future deferral.
    pub fn remember_parent(&mut self, site_id: &str, parent_id: &str) {
        self.seen.insert(site_id.to_string());
        let entry = self.sites.entry(site_id.to_string()).or_default();
        entry.last_parent = Some(parent_id.to_string());
    }

    /// L1 (`lqos-74t.63`) — the multi-CANDIDATE parent pin. Hold the FIRST
    /// selected parent among the site's currently-VALID candidate parents
    /// (`valid_logical_parents`) until the held parent is no longer a valid
    /// candidate; return the parent to USE this poll (the held one while it
    /// stays valid, else `derived_parent`, which then becomes the new pin).
    ///
    /// A UISP backhaul-parent oscillation re-selects between two VALID redundant
    /// links (a genuine owner-set change → the bridge's R3 cold rebuild, which
    /// drops residual backlog); pinning suppresses that re-parent while the held
    /// parent stays reachable. A no-op when `derived_parent` is the only valid
    /// candidate (there is no alternative to flap between).
    pub fn pin_parent(
        &mut self,
        site_id: &str,
        derived_parent: &str,
        valid_logical_parents: &BTreeSet<String>,
    ) -> String {
        self.seen.insert(site_id.to_string());
        let entry = self.sites.entry(site_id.to_string()).or_default();
        if let Some(pinned) = entry.pinned_parent.as_deref()
            && valid_logical_parents.contains(pinned)
        {
            return pinned.to_string();
        }
        entry.pinned_parent = Some(derived_parent.to_string());
        derived_parent.to_string()
    }

    /// Drop entries for sites not seen this poll (bounds the file to the present
    /// site count).
    pub fn prune(&mut self) {
        self.sites.retain(|id, _| self.seen.contains(id));
        self.seen.clear();
    }

    /// Persist the latch. Best-effort: a write error is logged, never fatal.
    pub fn save(&self) {
        let Some(path) = self.path.as_ref() else {
            return;
        };
        let file = LatchFile {
            version: ORPHAN_LATCH_VERSION,
            root: self.root.clone(),
            last_run_unix: self.last_run_unix,
            sites: self.sites.clone(),
        };
        let raw = match serde_json::to_string(&file) {
            Ok(raw) => raw,
            Err(e) => {
                warn!(
                    event = "ORPHAN_LATCH_WRITE",
                    "orphan latch: serialize failed ({e}) — latch not persisted"
                );
                return;
            }
        };
        if let Some(parent) = path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            warn!(
                event = "ORPHAN_LATCH_WRITE",
                path = %parent.display(),
                "orphan latch: cannot create the state directory ({e}) — latch not persisted"
            );
            return;
        }
        if let Err(e) = write_atomically(path, &raw) {
            warn!(
                event = "ORPHAN_LATCH_WRITE",
                path = %path.display(),
                "orphan latch: cannot write the file ({e}) — latch not persisted"
            );
        }
    }
}

fn write_atomically(path: &Path, raw: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, raw)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_000_000;

    fn latch_with_parent(site: &str, parent: &str) -> OrphanLatch {
        let mut latch = OrphanLatch::default();
        latch.note_path(site);
        latch.remember_parent(site, parent);
        latch
    }

    /// Prime a no-path episode that started `age_secs` before `NOW`.
    fn latch_with_no_path(site: &str, parent: &str, age_secs: u64) -> OrphanLatch {
        let mut latch = latch_with_parent(site, parent);
        latch.sites.get_mut(site).expect("site").first_no_path_unix =
            Some(NOW - age_secs);
        latch
    }

    #[test]
    fn a_fresh_no_path_defers_to_the_last_parent() {
        let mut latch = latch_with_parent("site-a", "site-parent");
        // First no-path poll: first == now ⇒ age 0 ⇒ defer.
        assert_eq!(
            latch.note_no_path("site-a", NOW).as_deref(),
            Some("site-parent")
        );
        assert_eq!(
            latch.sites["site-a"].last_decision,
            Some(LatchDecision::Defer)
        );
    }

    #[test]
    fn the_boundary_defers_and_one_second_past_orphans() {
        // age == T_hold ⇒ STILL defer (the guard is "strictly longer").
        let mut at = latch_with_no_path("site-a", "site-parent", ORPHAN_LATCH_HOLD_OFF_SECONDS);
        assert_eq!(at.note_no_path("site-a", NOW).as_deref(), Some("site-parent"));
        // age == T_hold + 1 ⇒ orphan.
        let mut past =
            latch_with_no_path("site-a", "site-parent", ORPHAN_LATCH_HOLD_OFF_SECONDS + 1);
        assert_eq!(past.note_no_path("site-a", NOW), None);
        assert_eq!(
            past.sites["site-a"].last_decision,
            Some(LatchDecision::Orphan)
        );
    }

    #[test]
    fn the_hold_off_uses_elapsed_wall_clock_across_polls() {
        // Started at t0; a later poll at t0 + T_hold still defers; +1 orphans.
        let mut latch = latch_with_parent("site-a", "site-parent");
        assert_eq!(latch.note_no_path("site-a", NOW).as_deref(), Some("site-parent"));
        assert_eq!(
            latch
                .note_no_path("site-a", NOW + ORPHAN_LATCH_HOLD_OFF_SECONDS)
                .as_deref(),
            Some("site-parent")
        );
        assert_eq!(
            latch.note_no_path("site-a", NOW + ORPHAN_LATCH_HOLD_OFF_SECONDS + 1),
            None
        );
    }

    #[test]
    fn a_path_poll_clears_the_no_path_state() {
        let mut latch = latch_with_no_path("site-a", "site-parent", 1);
        latch.note_path("site-a"); // regained the path
        assert_eq!(latch.sites["site-a"].first_no_path_unix, None);
        // The next no-path is a fresh transient ⇒ defer.
        assert_eq!(
            latch.note_no_path("site-a", NOW).as_deref(),
            Some("site-parent")
        );
    }

    #[test]
    fn a_backward_clock_jump_cannot_orphan_or_extend() {
        // first was 100s in the future (a backward NTP step): clamp to now.
        let mut latch = latch_with_no_path("site-a", "site-parent", 0);
        latch.sites.get_mut("site-a").expect("site").first_no_path_unix = Some(NOW + 100);
        assert_eq!(latch.note_no_path("site-a", NOW).as_deref(), Some("site-parent"));
        assert_eq!(latch.sites["site-a"].first_no_path_unix, Some(NOW));
    }

    #[test]
    fn a_backward_clock_jump_cannot_re_defer_an_orphaned_site() {
        let mut latch = latch_with_no_path(
            "site-a",
            "site-parent",
            ORPHAN_LATCH_HOLD_OFF_SECONDS + 1,
        );
        // Cross the boundary ⇒ orphan, then sticky.
        assert_eq!(latch.note_no_path("site-a", NOW), None);
        // A backward jump (now < first) must NOT re-defer the orphaned site.
        assert_eq!(latch.note_no_path("site-a", NOW.saturating_sub(1000)), None);
        assert_eq!(
            latch.sites["site-a"].last_decision,
            Some(LatchDecision::Orphan)
        );
    }

    #[test]
    fn a_first_seen_no_path_site_has_no_parent_to_defer_to() {
        let mut latch = OrphanLatch::default();
        assert_eq!(latch.note_no_path("site-new", NOW), None);
    }

    #[test]
    fn prune_drops_sites_not_seen_this_poll() {
        let mut latch = OrphanLatch::default();
        latch.sites.insert(
            "site-old".to_string(),
            LatchEntry {
                first_no_path_unix: Some(NOW),
                last_parent: Some("p".to_string()),
                pinned_parent: None,
                last_decision: None,
            },
        );
        latch.note_path("site-a");
        latch.prune();
        assert!(latch.sites.contains_key("site-a"));
        assert!(!latch.sites.contains_key("site-old"));
        assert!(latch.seen.is_empty());
    }

    #[test]
    fn a_root_change_clears_the_latch() {
        let mut latch = latch_with_parent("site-a", "p");
        latch.set_root("root-1");
        latch.note_path("site-a");
        assert!(latch.sites.contains_key("site-a"));
        latch.set_root("root-2");
        assert!(latch.sites.is_empty(), "a new root clears the latch");
    }

    #[test]
    fn a_round_trip_through_the_file_preserves_the_first_no_path() {
        let dir = std::env::temp_dir().join(format!("orphan-latch-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(ORPHAN_LATCH_FILENAME);
        let mut latch = OrphanLatch {
            path: Some(path.clone()),
            ..Default::default()
        };
        latch.set_root("root-1");
        latch.set_run_unix(NOW);
        latch.note_path("site-a");
        latch.remember_parent("site-a", "site-parent");
        latch.note_no_path("site-a", NOW - 10);
        latch.save();

        let file: LatchFile =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("latch file written"))
                .expect("latch file parses");
        assert_eq!(file.last_run_unix, Some(NOW));
        let mut loaded = OrphanLatch {
            path: Some(path.clone()),
            root: file.root,
            last_run_unix: file.last_run_unix,
            sites: file.sites,
            ..Default::default()
        };
        // first_no_path carried ⇒ age now T_hold + 11 ⇒ orphan.
        assert_eq!(
            loaded.note_no_path("site-a", NOW + ORPHAN_LATCH_HOLD_OFF_SECONDS + 1),
            None
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn candidates(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_first_candidate_is_pinned_and_held_until_it_becomes_invalid() {
        let mut latch = OrphanLatch::default();
        let both = candidates(&["p1", "p2"]);
        // The first selection pins the derived parent.
        assert_eq!(latch.pin_parent("site-a", "p1", &both), "p1");
        // A flap to the OTHER valid candidate is SUPPRESSED (the pin holds).
        assert_eq!(latch.pin_parent("site-a", "p2", &both), "p1");
        // The pinned parent drops out of the candidate set ⇒ re-pin.
        let only_p2 = candidates(&["p2"]);
        assert_eq!(latch.pin_parent("site-a", "p2", &only_p2), "p2");
    }

    #[test]
    fn a_single_candidate_is_a_noop() {
        let mut latch = OrphanLatch::default();
        let one = candidates(&["p"]);
        assert_eq!(latch.pin_parent("site-a", "p", &one), "p");
        assert_eq!(latch.pin_parent("site-a", "p", &one), "p");
    }

    #[test]
    fn orphaning_a_site_releases_the_candidate_pin() {
        let mut latch = OrphanLatch::default();
        let both = candidates(&["p1", "p2"]);
        assert_eq!(latch.pin_parent("site-a", "p1", &both), "p1");
        // Cross the no-path hold-off ⇒ orphan ⇒ the pin is released.
        latch.sites.get_mut("site-a").expect("site").first_no_path_unix =
            Some(NOW - ORPHAN_LATCH_HOLD_OFF_SECONDS - 1);
        assert_eq!(latch.note_no_path("site-a", NOW), None);
        assert!(latch.sites["site-a"].pinned_parent.is_none());
        // On recovery the site re-pins to the then-selected parent.
        assert_eq!(latch.pin_parent("site-a", "p2", &both), "p2");
    }
}
