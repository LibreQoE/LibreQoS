//! The dynamic-circuit origin record: which device ROWS the topology fold
//! SYNTHESIZED — the overlay rows of `dynamic_circuits.json` and the
//! `[[dynamic_circuits.ranges]]` fallback rows (u5 A1).
//!
//! The marker answers ORIGIN, never LIFECYCLE. A store row is transient (TTL,
//! supersede, promotion) and a range fallback lives exactly as long as its rule is
//! configured, but both are rows no operator provisioned, and that is the only
//! question this record answers. A consumer that needs the lifecycle asks the
//! store or the config, not this marker.
//!
//! The origin marker on a shaping device is per row, but a circuit id can hold
//! several rows: a persistent operator override REPLACES a fold row only when
//! the `device_id` matches, and otherwise APPENDS a second row under the same
//! `circuit_id`. Keying the origin by circuit id alone therefore misclassifies
//! whichever row the operator did not replace (and, before this type existed,
//! both rows). Both the fold (which writes the marker) and the supersede rule
//! (which reads it) use this one record, so the two halves cannot drift.

use crate::normalize_circuit_id_key;
use std::collections::{HashMap, HashSet};

/// The device rows the topology fold synthesized, keyed
/// `(normalized circuit id, device id)`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DynamicOriginRows {
    rows_by_circuit: HashMap<String, HashSet<String>>,
}

impl DynamicOriginRows {
    /// An empty record: no row was synthesized by the fold.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one row the fold synthesized. Returns whether it was new.
    ///
    /// Device ids are stored verbatim because the override fold matches them
    /// verbatim; the circuit id is normalized because every other lookup of a
    /// circuit id is.
    pub fn insert_row(&mut self, circuit_id: &str, device_id: &str) -> bool {
        let circuit_key = normalize_circuit_id_key(circuit_id);
        if circuit_key.is_empty() {
            return false;
        }
        self.rows_by_circuit
            .entry(circuit_key)
            .or_default()
            .insert(device_id.to_string())
    }

    /// Drops every origin row a persistent override replaced, given the
    /// `(circuit_id, device_id)` of each row the override fold actually replaced.
    ///
    /// The override fold replaces ONE row per persistent device, but a
    /// `device_id` alone does not identify that row: two dynamic circuits can
    /// carry the same device id (the store dedupes by circuit id only), so a
    /// device-id-keyed subtraction would drop the origin of the circuit the
    /// override did not replace and let it supersede itself one reload later.
    /// The identity is therefore the full row key, exactly like
    /// [`Self::contains_row`].
    pub fn remove_replaced_rows<'a>(
        &mut self,
        replaced_rows: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) {
        for (circuit_id, device_id) in replaced_rows {
            let circuit_key = normalize_circuit_id_key(circuit_id);
            let emptied = match self.rows_by_circuit.get_mut(&circuit_key) {
                Some(rows) => {
                    rows.remove(device_id);
                    rows.is_empty()
                }
                None => false,
            };
            if emptied {
                self.rows_by_circuit.remove(&circuit_key);
            }
        }
    }

    /// Whether the fold synthesized this row.
    ///
    /// The empty check is the static-world fast path: a world with no folded rows
    /// pays no id normalization per catalog row.
    pub fn contains_row(&self, circuit_id: &str, device_id: &str) -> bool {
        if self.rows_by_circuit.is_empty() {
            return false;
        }
        let circuit_key = normalize_circuit_id_key(circuit_id);
        self.rows_by_circuit
            .get(&circuit_key)
            .is_some_and(|rows| rows.contains(device_id))
    }

    /// Whether the fold synthesized ANY row under this circuit id.
    ///
    /// The circuit-level question the operator surfaces ask ("is this row the
    /// dynamic actor's?"); `contains_row` answers the row-level one the supersede
    /// rule needs.
    pub fn contains_circuit(&self, circuit_id: &str) -> bool {
        if self.rows_by_circuit.is_empty() {
            return false;
        }
        self.rows_by_circuit
            .contains_key(&normalize_circuit_id_key(circuit_id))
    }

    /// Whether no row is marked dynamic-origin.
    pub fn is_empty(&self) -> bool {
        self.rows_by_circuit.is_empty()
    }

    /// The number of origin rows.
    pub fn len(&self) -> usize {
        self.rows_by_circuit.values().map(HashSet::len).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::DynamicOriginRows;

    #[test]
    fn a_row_is_matched_by_its_own_circuit_and_device_ids() {
        let mut origins = DynamicOriginRows::new();
        assert!(origins.is_empty());
        assert!(
            !origins.contains_row("dyn-1", "dyn-1-dev"),
            "an empty record matches nothing"
        );

        assert!(origins.insert_row(" DYN-1 ", "dyn-1-dev"));
        assert!(
            origins.contains_row("dyn-1", "dyn-1-dev"),
            "the circuit key is normalized on both sides"
        );
        assert!(
            !origins.contains_row("dyn-1", "dyn-1-extra"),
            "another row under the same circuit is not the synthesized row"
        );
        assert_eq!(origins.len(), 1);
    }

    #[test]
    fn only_rows_a_persistent_override_replaced_leave_the_record() {
        let mut origins = DynamicOriginRows::new();
        origins.insert_row("dyn-1", "dyn-1-dev");
        origins.insert_row("dyn-2", "dyn-2-dev");

        // The override replaced dyn-2's row (same device id) and appended a
        // second row under dyn-1 (different device id).
        origins.remove_replaced_rows([("dyn-2", "dyn-2-dev"), ("dyn-1", "dyn-1-extra")]);

        assert!(
            !origins.contains_row("dyn-2", "dyn-2-dev"),
            "a replaced fold row is provisioned now"
        );
        assert!(
            origins.contains_row("dyn-1", "dyn-1-dev"),
            "an appended override row leaves the fold row synthesized"
        );
        assert_eq!(origins.len(), 1);

        origins.remove_replaced_rows([("dyn-1", "dyn-1-dev"), ("dyn-1", "")]);
        assert!(origins.is_empty(), "the last replaced row empties the record");
    }

    /// u6 A5 (REV-C2): the replacement identity is the ROW, not the device id.
    /// Two dynamic circuits may share a device id; an override replaces exactly
    /// one of them, so the other circuit must keep its origin row.
    #[test]
    fn a_shared_device_id_outside_the_replaced_row_keeps_its_origin() {
        let mut origins = DynamicOriginRows::new();
        origins.insert_row("dyn-stale", "shared-dev");
        origins.insert_row("dyn-live", "shared-dev");

        origins.remove_replaced_rows([("dyn-stale", "shared-dev")]);

        assert!(
            !origins.contains_row("dyn-stale", "shared-dev"),
            "the replaced row is provisioned"
        );
        assert!(
            origins.contains_row("dyn-live", "shared-dev"),
            "the untouched circuit's identical device id stays dynamic-origin"
        );
        assert_eq!(origins.len(), 1);
    }

    /// u6 A5 (REV-T4): the circuit id is normalized on the REMOVAL side too, and
    /// a circuit the record never held is a no-op.
    ///
    /// The fold inserts the circuit id and the override fold reports the id it
    /// replaced; if the two spellings disagree and only the insert normalizes,
    /// the origin row survives its own removal and the circuit supersedes itself
    /// one reload later — the exact leak this type exists to prevent.
    #[test]
    fn a_row_removed_under_another_spelling_still_leaves_the_record() {
        let mut origins = DynamicOriginRows::new();
        assert!(origins.insert_row("dyn-1", "dyn-1-dev"));
        origins.remove_replaced_rows([(" DYN-1 ", "dyn-1-dev")]);
        assert!(
            !origins.contains_row("dyn-1", "dyn-1-dev"),
            "a removal spelling the insert normalized away still finds the row"
        );
        assert!(origins.is_empty(), "the last replaced row empties the record");

        // The mirrored case from the finding: inserted with the spaces, removed bare.
        assert!(origins.insert_row(" dyn-2 ", "dyn-2-dev"));
        origins.remove_replaced_rows([("dyn-2", "dyn-2-dev")]);
        assert!(!origins.contains_row("dyn-2", "dyn-2-dev"));
        assert!(origins.is_empty());

        // Absent-entry arm: a circuit the record never held changes nothing.
        origins.remove_replaced_rows([("dyn-absent", "dyn-absent-dev")]);
        assert!(origins.is_empty(), "an absent circuit is a no-op");
    }
}
