use allocative::Allocative;
use ip_network::{IpNetwork, IpNetworkError};
use ip_network_table::IpNetworkTable;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::net::IpAddr;
use std::sync::{Mutex, OnceLock};
use tracing::warn;

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Allocative)]
pub struct IpRanges {
    pub ignore_subnets: Vec<String>,
    pub allow_subnets: Vec<String>,
    pub unknown_ip_honors_ignore: Option<bool>,
    pub unknown_ip_honors_allow: Option<bool>,
}

impl Default for IpRanges {
    fn default() -> Self {
        Self {
            // v2 parity (`src/src/configMigrator.py:39` defaults
            // `ignoreSubnets = ["192.168.0.0/16"]`). Consumed by the topology
            // choke (`lqos_topology::build_shaping_inputs_with`): ignored device
            // IPs never become `mapping_ops`.
            ignore_subnets: vec!["192.168.0.0/16".to_string()],
            allow_subnets: vec![
                "172.16.0.0/12".to_string(),
                "10.0.0.0/8".to_string(),
                "100.64.0.0/10".to_string(),
                "192.168.0.0/16".to_string(),
            ],
            unknown_ip_honors_ignore: Some(true),
            unknown_ip_honors_allow: Some(true),
        }
    }
}

impl IpRanges {
    /// Maps the ignored IP ranges to an LPM table.
    ///
    /// A malformed entry (`192.168.0.0` with no `/<prefix length>`, an
    /// unparsable address, a prefix length the address family cannot carry) is
    /// SKIPPED, named in one `warn!`, and the remaining entries still load: one
    /// typo must not disable the lines the operator wrote correctly, and it must
    /// never panic a caller (the topology fold rebuilds this table on the publish
    /// thread).
    ///
    /// An entry with host bits set (`192.168.0.5/24`) is not skipped: it means
    /// the network the address sits in, which is what the fold's prefix
    /// arithmetic normalizes to as well. It loads truncated, with its own
    /// `warn!`, so the operator is told what was read instead of being sent
    /// hunting for a prefix length that is already correct.
    ///
    /// Side effects: logs one `warn!` the first time an entry is skipped or
    /// truncated.
    pub fn ignored_network_table(&self) -> Result<IpNetworkTable<bool>, IpRangeError> {
        Ok(network_table("ignore_subnets", &self.ignore_subnets))
    }

    /// Maps the allowed IP ranges to an LPM table.
    ///
    /// Same lenient contract as [`Self::ignored_network_table`]: a malformed
    /// entry is skipped with one `warn!`, never an abort.
    ///
    /// Side effects: logs one `warn!` the first time an entry is skipped.
    pub fn allowed_network_table(&self) -> Result<IpNetworkTable<bool>, IpRangeError> {
        Ok(network_table("allow_subnets", &self.allow_subnets))
    }
}

/// One configured CIDR list entry, as the lenient parser reads it.
///
/// The three outcomes are distinct operator situations, not parse internals:
/// only a truncated entry is used as configured, and the unusable reasons are
/// the text the operator reads, so "fix the prefix length" can never be printed
/// for an entry whose prefix length is already correct.
enum ParseOutcome {
    /// The entry is a network as written.
    Network(IpNetwork),
    /// Host bits are set: the entry means the network the address sits in.
    HostBits(IpNetwork),
    /// Not usable coverage, with the reason to report.
    Unusable(&'static str),
}

/// Builds one LPM table from a configured CIDR list, skipping (and reporting) the
/// entries that do not parse.
fn network_table(list: &str, entries: &[String]) -> IpNetworkTable<bool> {
    let mut table = IpNetworkTable::new();
    for entry in entries {
        match parse_entry(entry) {
            ParseOutcome::Network(network) => {
                table.insert(network, true);
            }
            ParseOutcome::HostBits(network) => {
                warn_invalid_entry(
                    list,
                    entry,
                    &format!("host bits are set; using the enclosing network {network}"),
                );
                table.insert(network, true);
            }
            ParseOutcome::Unusable(reason) => warn_invalid_entry(list, entry, reason),
        }
    }
    table
}

/// Reads one configured entry, tolerating a missing mask, a bad address, a
/// wrong prefix length and host bits set.
fn parse_entry(entry: &str) -> ParseOutcome {
    let Some((address, prefix)) = entry.trim().split_once('/') else {
        return ParseOutcome::Unusable("expected `address/<prefix length>`");
    };
    let Ok(address) = address.trim().parse::<IpAddr>() else {
        return ParseOutcome::Unusable("not a valid IPv4 or IPv6 address");
    };
    let Ok(prefix) = prefix.trim().parse::<u8>() else {
        return ParseOutcome::Unusable("prefix length is not a number");
    };
    match IpNetwork::new(address, prefix) {
        Ok(network) => ParseOutcome::Network(network),
        // u8 REV-C2: `IpNetwork::new` refuses BOTH an impossible prefix length
        // (`10.0.0.0/33`) and host bits set (`192.168.0.5/24`), so a single
        // error arm here used to report the second as the first. The lenient
        // path takes the operator's evident meaning for host bits — the whole
        // subnet — and says so.
        Err(IpNetworkError::HostBitsSet) => match IpNetwork::new_truncate(address, prefix) {
            Ok(network) => ParseOutcome::HostBits(network),
            // `new` already accepted this prefix length, so truncation cannot
            // fail here; an impossible state must not insert partial coverage.
            Err(_) => {
                ParseOutcome::Unusable("host bits are set and the address cannot be normalized")
            }
        },
        Err(IpNetworkError::NetmaskError(_)) => ParseOutcome::Unusable(
            "prefix length is longer than the address family allows (32 for IPv4, 128 for IPv6)",
        ),
    }
}

/// Reports one malformed `ip_ranges` entry ONCE per process.
///
/// The fold rebuilds these tables on every topology publish, so an undeduplicated
/// warning would re-log a standing config typo on every round (a per-publish WARN
/// is a bug). One line per entry is what tells the operator which line to edit.
fn warn_invalid_entry(list: &str, entry: &str, reason: &str) {
    if reported_entries().first_report(list, entry) {
        warn!("ip_ranges.{list}: ignoring {entry:?} ({reason})");
    }
}

/// The once-per-process report gate for malformed `ip_ranges` entries.
#[derive(Default)]
struct ReportedInvalidEntries {
    seen: HashSet<String>,
}

impl ReportedInvalidEntries {
    /// True the FIRST time this `list`/`entry` pair is reported.
    fn first_report(&mut self, list: &str, entry: &str) -> bool {
        self.seen.insert(format!("{list}: {entry}"))
    }
}

/// The process-wide gate behind [`warn_invalid_entry`].
///
/// A poisoned lock RECOVERS instead of silencing: the guarded state is a set of
/// strings that cannot be left half-updated, and degrading to "warn every time"
/// is the bug this gate exists to prevent, not a worse failure than hiding a
/// malformed entry forever.
fn reported_entries() -> std::sync::MutexGuard<'static, ReportedInvalidEntries> {
    static REPORTED: OnceLock<Mutex<ReportedInvalidEntries>> = OnceLock::new();
    REPORTED
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(thiserror::Error, Debug)]
pub enum IpRangeError {
    #[error("Unable to parse IP range: {e:?}")]
    IpParseError { e: Box<dyn std::error::Error> },
    #[error("Invalid network: {e:?}")]
    InvalidNetwork { e: Box<dyn std::error::Error> },
    #[error("Invalid netmask")]
    InvalidNetmask,
}

#[cfg(test)]
mod tests {
    use super::{IpRanges, ParseOutcome};
    use std::net::IpAddr;

    fn ignored_contains(ranges: &IpRanges, address: &str) -> bool {
        let table = ranges
            .ignored_network_table()
            .expect("the lenient parse has no error arm");
        table
            .longest_match(address.parse::<IpAddr>().expect("test address"))
            .is_some()
    }

    /// u8 A1: an entry without `/<prefix length>` used to PANIC every consumer
    /// (`split('/')[1]`). One typo must cost that line only, never the process
    /// and never the entries around it.
    #[test]
    fn a_malformed_ignore_entry_is_skipped_and_the_rest_still_load() {
        let ranges = IpRanges {
            ignore_subnets: vec![
                "192.168.0.0".to_string(),
                "bad".to_string(),
                "10.0.0.0/8".to_string(),
            ],
            ..IpRanges::default()
        };

        assert!(
            ignored_contains(&ranges, "10.1.2.3"),
            "the valid /8 still prunes"
        );
        assert!(
            !ignored_contains(&ranges, "192.168.0.0"),
            "the mask-less NETWORK ADDRESS is not ignored: a skipped entry is not a /32"
        );
        assert!(
            !ignored_contains(&ranges, "192.168.0.1"),
            "not even the address next to it: the entry is skipped, not pinned"
        );
        assert!(
            !ignored_contains(&ranges, "192.168.4.4"),
            "the mask-less entry is skipped, never guessed as a /32 or /0"
        );
        assert!(
            !ignored_contains(&ranges, "203.0.113.9"),
            "an address outside every valid entry is not ignored"
        );
    }

    /// u8 A1 (REV-T6): the negative probes above must not be satisfiable by a
    /// `/32` guess — the skipped entry is `192.168.0.0`, so the address inside a
    /// guessed `/32` is exactly the one a lenient fallback would pin. Asserted
    /// separately so the `192.168.4.4` probe cannot stand in for it.
    #[test]
    fn a_skipped_entry_does_not_become_a_host_route() {
        let ranges = IpRanges {
            ignore_subnets: vec!["192.168.0.0".to_string()],
            ..IpRanges::default()
        };

        assert!(
            !ignored_contains(&ranges, "192.168.0.0"),
            "no /32 guess for the skipped network address"
        );
        assert!(
            !ignored_contains(&ranges, "192.168.0.1"),
            "no /31 or /32 guess around it either"
        );
        assert!(
            !ignored_contains(&ranges, "192.168.1.1"),
            "and no /16 guess (which would swallow the operator's whole block)"
        );
    }

    /// u8 A1 (REV-T8): `allow_subnets` had the same panic and now shares the same
    /// lenient parser, so a malformed allow entry must not deny the valid ones.
    #[test]
    fn a_malformed_allow_entry_is_skipped_and_the_rest_still_load() {
        let ranges = IpRanges {
            allow_subnets: vec!["10.0.0.0".to_string(), "10.1.0.0/16".to_string()],
            ..IpRanges::default()
        };

        let table = ranges
            .allowed_network_table()
            .expect("the lenient parse has no error arm");
        let allowed = |address: &str| {
            table
                .longest_match(address.parse::<IpAddr>().expect("test address"))
                .is_some()
        };
        assert!(allowed("10.1.2.3"), "the valid /16 still allows");
        assert!(!allowed("10.0.0.0"), "the mask-less entry is skipped");
        assert!(!allowed("10.2.0.1"), "and it is not guessed into a /8");
    }

    /// u8 A1 (REV-T9): padded entries are new tolerance — an operator who aligned
    /// a column with spaces meant the same list the unpadded one means.
    #[test]
    fn padded_entries_load_like_their_trimmed_form() {
        let ranges = IpRanges {
            ignore_subnets: vec![" 10.0.0.0 / 8 ".to_string(), "\t192.168.0.0/16\n".to_string()],
            ..IpRanges::default()
        };

        assert!(ignored_contains(&ranges, "10.9.9.9"));
        assert!(ignored_contains(&ranges, "192.168.4.4"));
        assert!(!ignored_contains(&ranges, "172.16.0.1"));
    }

    /// u8 A1 (REV-T4): the once-per-process dedupe is the property the A1 fix
    /// relies on (a standing typo must cost ONE line, not one per publish).
    #[test]
    fn the_warn_gate_reports_each_list_entry_once() {
        let mut gate = super::ReportedInvalidEntries::default();
        assert!(
            gate.first_report("ignore_subnets", "not-a-subnet"),
            "the first sighting is news"
        );
        assert!(
            !gate.first_report("ignore_subnets", "not-a-subnet"),
            "the second call for the same entry is silent"
        );
        assert!(
            gate.first_report("ignore_subnets", "10.0.0.0"),
            "a different entry is its own line"
        );
        assert!(
            gate.first_report("allow_subnets", "not-a-subnet"),
            "the same text in the other list is its own line"
        );
        assert!(
            !gate.first_report("allow_subnets", "not-a-subnet"),
            "and it dedupes on its own list"
        );
    }

    /// u8 A1 (REV-T4/T9): a POISONED gate must recover, not silence. Degrading to
    /// "warn every time" is the bug the gate guards; degrading to silence hides a
    /// malformed entry for the life of the process.
    #[test]
    fn a_poisoned_warn_gate_still_reports() {
        let poisoner = std::thread::spawn(|| {
            let _guard = super::reported_entries();
            panic!("poison the process warn gate on purpose");
        });
        assert!(
            poisoner.join().is_err(),
            "PREMISE: the probe thread must have panicked while holding the gate"
        );

        let mut gate = super::reported_entries();
        assert!(
            gate.first_report("ignore_subnets", "poison-probe"),
            "a poisoned gate reports again instead of going silent"
        );
    }

    /// A bad mask and a bad address are the same class as a missing mask: skip and
    /// keep going. `10.0.0.0/33` and `10.0.0.0/abc` must not abort the table.
    #[test]
    fn out_of_range_and_non_numeric_prefixes_are_skipped() {
        let ranges = IpRanges {
            ignore_subnets: vec![
                "10.0.0.0/33".to_string(),
                "10.0.0.0/abc".to_string(),
                "172.16.0.0/12".to_string(),
            ],
            ..IpRanges::default()
        };

        assert!(ignored_contains(&ranges, "172.20.0.1"));
        assert!(!ignored_contains(&ranges, "10.9.9.9"));
    }

    /// IPv6 keeps working through the same lenient path.
    #[test]
    fn ipv6_entries_load_alongside_ipv4() {
        let ranges = IpRanges {
            ignore_subnets: vec!["2001:db8::/32".to_string(), "10.0.0.0/8".to_string()],
            ..IpRanges::default()
        };

        assert!(ignored_contains(&ranges, "2001:db8::1"));
        assert!(ignored_contains(&ranges, "10.1.1.1"));
    }

    /// u8 REV-C2: host bits set is its OWN operator mistake — the prefix length
    /// in `192.168.0.5/24` is already correct. The entry means the subnet the
    /// address sits in (the same normalization the fold's prefix arithmetic
    /// applies), and it must load rather than silently leave the block ignored.
    #[test]
    fn an_entry_with_host_bits_set_loads_its_enclosing_network() {
        let ranges = IpRanges {
            ignore_subnets: vec!["192.168.0.5/24".to_string(), "2001:db8::7/32".to_string()],
            ..IpRanges::default()
        };

        assert!(
            ignored_contains(&ranges, "192.168.0.5"),
            "the configured address is inside the subnet it named"
        );
        assert!(
            ignored_contains(&ranges, "192.168.0.99"),
            "the whole enclosing /24 is the entry's meaning"
        );
        assert!(
            !ignored_contains(&ranges, "192.168.1.1"),
            "truncation never widens the entry past the prefix the operator wrote"
        );
        assert!(
            ignored_contains(&ranges, "2001:db8::99"),
            "the v6 entry truncates the same way"
        );
    }

    /// The REASONS are the operator's only diagnostic, so they are asserted, not
    /// just the resulting table: a host-bits entry must never be reported as a
    /// prefix-length problem, and an impossible length must name the limit
    /// (u8 REV-C2).
    #[test]
    fn an_unusable_entry_is_reported_for_the_field_that_is_actually_wrong() {
        let ParseOutcome::HostBits(network) = super::parse_entry("192.168.0.5/24") else {
            panic!("host bits set must load TRUNCATED, not be skipped");
        };
        assert_eq!(
            network.network_address(),
            "192.168.0.0".parse::<IpAddr>().expect("test address")
        );
        assert_eq!(network.netmask(), 24, "the operator's prefix is kept");

        let ParseOutcome::Unusable(reason) = super::parse_entry("10.0.0.0/33") else {
            panic!("a /33 is not coverage");
        };
        assert!(
            reason.contains("longer than the address family allows"),
            "an impossible length is a length problem: {reason}"
        );
        assert!(
            reason.contains("32"),
            "and it names the IPv4 limit the operator must respect: {reason}"
        );

        let ParseOutcome::Unusable(reason) = super::parse_entry("192.168.0.0") else {
            panic!("a mask-less entry is not coverage");
        };
        assert!(
            reason.contains("address/<prefix length>"),
            "a missing mask is shown as the shape the operator must write: {reason}"
        );

        let ParseOutcome::Unusable(reason) = super::parse_entry("not-an-address/24") else {
            panic!("a bad address is not coverage");
        };
        assert!(
            reason.contains("not a valid IPv4 or IPv6 address"),
            "a bad address is named as an address problem: {reason}"
        );
    }
}

