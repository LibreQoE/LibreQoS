//! Canonical prefix-membership evidence over the addresses the store already
//! observed.
//!
//! Interface rows and NAT rule fields can carry a prefix; ARP bindings and
//! wireless station addresses are bare. This module canonicalizes prefixed
//! text into an [`IpNet`] (host bits cleared), keeps every parseable observed
//! address as a [`PrefixObservation`], and answers the containment questions
//! the topology projection asks: which statements cover an observation, which
//! observations one statement's observer reported inside it, which statements
//! share or overlap a prefix, and which prefix the same observer's longest
//! statement assigns to an observed address.
//!
//! Canonical prefix lengths are resolved for every observation at
//! construction, so membership queries reuse one answer per observer and
//! address.
//!
//! The table is a sorted vector, not a radix trie. A projection is rebuilt per
//! request from a bounded store dump, and statement and observation counts are
//! bounded by discovery rows; `ipnet` already canonicalizes and orders
//! networks, so a trie crate would add a dependency without changing an
//! answer. Sorted output is part of the contract: a projection must never
//! depend on the order rows arrived in.
//!
//! Membership is inferred reachability, never a cable. Identical private
//! prefixes can be separate customer ranges, and a virtual data plane (EoIP,
//! VXLAN, VPLS) can carry one prefix across sites; only the observer that
//! stated a prefix is joined to the addresses it reported inside that prefix.
//!
//! No statement is inferred from an identity claim: a stored claim drops the
//! observer that reported it, so a neighbour-reported management address
//! cannot be scoped to one observer's prefix. DHCP leases and connected
//! routes, and IPv6 NAT, are future sources.

use std::fmt;
use std::net::IpAddr;

use ipnet::IpNet;
use serde::{Serialize, Serializer};

/// Where a prefix statement came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrefixBasis {
    /// An interface address row that carried a prefix.
    InterfaceAddress,
    /// An address field of an enabled `srcnat`/`dstnat` rule.
    NatRule,
}

/// One observer's statement about an IPv4 or IPv6 prefix.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PrefixStatement {
    /// The canonical network: host bits are already cleared.
    pub network: IpNet,
    /// The reporting observer, normalized MAC text.
    pub observer: String,
    /// The evidence class that stated the prefix.
    pub basis: PrefixBasis,
    /// The queries that produced the statement, sorted and deduplicated.
    pub sources: Vec<String>,
    /// The interface that carried an address statement; absent for NAT.
    pub locator: Option<String>,
}

/// One address a host reported, scoped to the observer that reported it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PrefixObservation {
    /// The reporting observer, normalized MAC text.
    pub observer: String,
    /// The device the address belongs to, normalized MAC text.
    pub node: String,
    /// The observed host address, prefix stripped.
    pub address: IpAddr,
}

/// An observed host address with the canonical prefix length its observer's
/// longest matching statement assigns, or the host route when no statement
/// matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AddressWithPrefix {
    /// The observed host address.
    pub address: IpAddr,
    /// The canonical prefix length: the longest matching statement's, else
    /// `32` for IPv4 or `128` for IPv6.
    pub prefix_len: u8,
}

impl fmt::Display for AddressWithPrefix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.address, self.prefix_len)
    }
}

impl Serialize for AddressWithPrefix {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

/// Canonical prefix statements and observed addresses, each scoped to the
/// observer that reported it.
///
/// Both lists are sorted and deduplicated at construction, so every query
/// returns evidence in the same order for the same input content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrefixTable {
    statements: Vec<PrefixStatement>,
    /// Each observation paired with its [`AddressWithPrefix`], resolved at
    /// construction.
    observations: Vec<(PrefixObservation, AddressWithPrefix)>,
}

impl PrefixTable {
    /// Builds the table from the statements and observations the store
    /// stated, in any order.
    ///
    /// Callers own evidence policy: observers arrive as normalized MAC text,
    /// statement networks are already filtered to reachability scopes, and
    /// `sources` are already sorted and deduplicated. The table truncates
    /// networks to their canonical prefixes, sorts and deduplicates both
    /// lists, and resolves each observation's canonical prefix.
    pub fn new(
        mut statements: Vec<PrefixStatement>,
        mut observations: Vec<PrefixObservation>,
    ) -> Self {
        for statement in &mut statements {
            statement.network = statement.network.trunc();
        }
        statements.sort();
        statements.dedup();
        observations.sort();
        observations.dedup();
        let observations = observations
            .into_iter()
            .map(|observation| {
                let canonical = AddressWithPrefix {
                    address: observation.address,
                    prefix_len: longest_prefix(
                        &statements,
                        &observation.observer,
                        observation.address,
                    ),
                };
                (observation, canonical)
            })
            .collect();
        Self {
            statements,
            observations,
        }
    }

    /// Every statement, sorted by network, observer, basis, sources, and
    /// locator.
    pub fn statements(&self) -> &[PrefixStatement] {
        &self.statements
    }

    /// Every observation, sorted by observer, node, and address.
    pub fn observations(&self) -> impl Iterator<Item = &PrefixObservation> {
        self.observations.iter().map(|(observation, _)| observation)
    }

    /// Every statement whose network contains `address`, in table order.
    pub fn containing(&self, address: IpAddr) -> impl Iterator<Item = &PrefixStatement> {
        self.statements
            .iter()
            .filter(move |statement| statement.network.contains(&address))
    }

    /// Every statement with exactly this canonical network, in table order.
    /// The same prefix seen from two observers stays two statements.
    pub fn same_prefix(&self, network: IpNet) -> impl Iterator<Item = &PrefixStatement> {
        self.statements
            .iter()
            .filter(move |statement| statement.network == network)
    }

    /// Every statement whose network contains or is contained by `network`,
    /// in table order. IPv4 and IPv6 never overlap.
    pub fn overlapping(&self, network: IpNet) -> impl Iterator<Item = &PrefixStatement> {
        self.statements
            .iter()
            .filter(move |statement| overlaps(statement.network, network))
    }

    /// The statements from `observer` that contain `address`, keeping only
    /// the longest prefix length. Ties (an interface and a NAT statement
    /// naming one network) stay separate statements.
    pub fn longest<'a>(
        &'a self,
        observer: &'a str,
        address: IpAddr,
    ) -> impl Iterator<Item = &'a PrefixStatement> + 'a {
        let longest = longest_prefix(&self.statements, observer, address);
        self.statements.iter().filter(move |statement| {
            statement.observer == observer
                && statement.network.prefix_len() == longest
                && statement.network.contains(&address)
        })
    }

    /// Every observation from the statement's observer that the statement
    /// network contains, in table order, each paired with its
    /// [`AddressWithPrefix`]. This is the observer-scoped membership rule in
    /// one place: a statement joins only the addresses the same observer
    /// reported inside it.
    pub fn members<'a>(
        &'a self,
        statement: &PrefixStatement,
    ) -> impl Iterator<Item = (&'a PrefixObservation, AddressWithPrefix)> + 'a {
        let observer = statement.observer.clone();
        let network = statement.network;
        self.observations
            .iter()
            .filter(move |(observation, _)| {
                observation.observer == observer && network.contains(&observation.address)
            })
            .map(|(observation, canonical)| (observation, *canonical))
    }

    /// The address as the observer's longest matching statement displays it,
    /// or as a host route when no statement from that observer contains it.
    pub fn canonical(&self, observer: &str, address: IpAddr) -> AddressWithPrefix {
        AddressWithPrefix {
            address,
            prefix_len: longest_prefix(&self.statements, observer, address),
        }
    }
}

/// The longest prefix length among the statements from `observer` that
/// contain `address`, else the host route for the address family. One pass
/// over the statements answers both construction and the public query.
fn longest_prefix(statements: &[PrefixStatement], observer: &str, address: IpAddr) -> u8 {
    statements
        .iter()
        .filter(|statement| statement.observer == observer && statement.network.contains(&address))
        .map(|statement| statement.network.prefix_len())
        .max()
        .unwrap_or(match address {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        })
}

/// Parses an observed address with an optional prefix.
///
/// Bare text yields `Some((address, None))`. Prefixed text yields the address
/// and its canonical network (host bits cleared), validated with
/// [`IpNet::new`], so an IPv4 prefix over 32 or an IPv6 prefix over 128, an
/// unparseable address, an empty string, or a trailing slash all yield
/// `None`.
pub fn parse_observed(text: &str) -> Option<(IpAddr, Option<IpNet>)> {
    let text = text.trim();
    let Some((address, prefix)) = text.split_once('/') else {
        return text.parse().ok().map(|address| (address, None));
    };
    let address: IpAddr = address.parse().ok()?;
    let prefix: u8 = prefix.parse().ok()?;
    let network = IpNet::new(address, prefix).ok()?.trunc();
    Some((address, Some(network)))
}

/// Whether a network can evidence reachability: its network address is not
/// unspecified, loopback, link-local, multicast, or broadcast.
///
/// Those scopes are host-local or group addresses, not a segment an observer
/// can place another device on.
pub fn is_reachability_prefix(network: IpNet) -> bool {
    match network {
        IpNet::V4(network) => {
            let address = network.network();
            !(address.is_unspecified()
                || address.is_loopback()
                || address.is_link_local()
                || address.is_multicast()
                || address.is_broadcast())
        }
        IpNet::V6(network) => {
            let address = network.network();
            !(address.is_unspecified()
                || address.is_loopback()
                || address.is_multicast()
                || address.is_unicast_link_local())
        }
    }
}

/// Whether one network contains the other; different families never overlap.
fn overlaps(left: IpNet, right: IpNet) -> bool {
    match (left, right) {
        (IpNet::V4(left), IpNet::V4(right)) => {
            left.contains(&right.network()) || right.contains(&left.network())
        }
        (IpNet::V6(left), IpNet::V6(right)) => {
            left.contains(&right.network()) || right.contains(&left.network())
        }
        (IpNet::V4(_), IpNet::V6(_)) | (IpNet::V6(_), IpNet::V4(_)) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(text: &str) -> IpAddr {
        text.parse().expect("address")
    }

    fn network(text: &str) -> IpNet {
        text.parse().expect("network")
    }

    fn statement(network: &str, observer: &str) -> PrefixStatement {
        PrefixStatement {
            network: network.parse().expect("network"),
            observer: observer.to_owned(),
            basis: PrefixBasis::InterfaceAddress,
            sources: vec!["test".to_owned()],
            locator: Some("ether1".to_owned()),
        }
    }

    fn observation(observer: &str, node: &str, address: &str) -> PrefixObservation {
        PrefixObservation {
            observer: observer.to_owned(),
            node: node.to_owned(),
            address: address.parse().expect("address"),
        }
    }

    #[test]
    fn parses_prefixed_and_bare_addresses_and_rejects_malformed_text() {
        assert_eq!(
            parse_observed("192.168.1.77/24"),
            Some((address("192.168.1.77"), Some(network("192.168.1.0/24"))))
        );
        assert_eq!(
            parse_observed("192.168.1.77"),
            Some((address("192.168.1.77"), None))
        );
        assert_eq!(
            parse_observed("2001:db8::1/64"),
            Some((address("2001:db8::1"), Some(network("2001:db8::/64"))))
        );
        assert_eq!(
            parse_observed("2001:db8::1"),
            Some((address("2001:db8::1"), None))
        );
        for malformed in [
            "192.168.1.77/33",
            "2001:db8::1/129",
            "garbage",
            "",
            "192.168.1.77/",
            "192.168.1.77/24/8",
        ] {
            assert_eq!(parse_observed(malformed), None, "{malformed:?}");
        }
    }

    #[test]
    fn statements_sort_and_exact_duplicates_collapse() {
        let table = PrefixTable::new(
            vec![
                statement("10.2.0.0/16", "aa:bb:cc:dd:ee:02"),
                statement("10.1.0.0/16", "aa:bb:cc:dd:ee:01"),
                statement("10.1.0.0/16", "aa:bb:cc:dd:ee:01"),
            ],
            Vec::new(),
        );

        assert_eq!(table.statements().len(), 2);
        assert_eq!(table.statements()[0].observer, "aa:bb:cc:dd:ee:01");
        assert_eq!(table.statements()[1].observer, "aa:bb:cc:dd:ee:02");
    }

    #[test]
    fn statement_networks_are_truncated_to_their_canonical_prefix() {
        let table = PrefixTable::new(
            vec![PrefixStatement {
                network: IpNet::new(address("10.1.2.3"), 16).expect("network"),
                observer: "aa:bb:cc:dd:ee:01".to_owned(),
                basis: PrefixBasis::InterfaceAddress,
                sources: vec!["test".to_owned()],
                locator: None,
            }],
            Vec::new(),
        );

        assert_eq!(table.statements()[0].network.to_string(), "10.1.0.0/16");
    }

    #[test]
    fn same_prefix_keeps_one_statement_per_observer() {
        let table = PrefixTable::new(
            vec![
                statement("192.168.100.0/24", "aa:bb:cc:dd:ee:01"),
                statement("192.168.100.0/24", "aa:bb:cc:dd:ee:02"),
            ],
            Vec::new(),
        );

        let same: Vec<&PrefixStatement> = table.same_prefix(network("192.168.100.0/24")).collect();
        assert_eq!(same.len(), 2);
        assert_eq!(same[0].observer, "aa:bb:cc:dd:ee:01");
        assert_eq!(same[1].observer, "aa:bb:cc:dd:ee:02");
    }

    #[test]
    fn containing_finds_nested_networks() {
        let nested = PrefixTable::new(
            vec![
                statement("10.0.0.0/8", "aa:bb:cc:dd:ee:01"),
                statement("10.1.0.0/16", "aa:bb:cc:dd:ee:01"),
                statement("10.2.0.0/16", "aa:bb:cc:dd:ee:01"),
            ],
            Vec::new(),
        );

        let containing: Vec<&PrefixStatement> = nested.containing(address("10.1.2.3")).collect();
        assert_eq!(containing.len(), 2);
        assert_eq!(containing[0].network, network("10.0.0.0/8"));
        assert_eq!(containing[1].network, network("10.1.0.0/16"));
    }

    #[test]
    fn overlapping_sees_nesting_but_not_siblings_or_other_families() {
        let table = PrefixTable::new(
            vec![
                statement("10.0.0.0/8", "aa:bb:cc:dd:ee:01"),
                statement("10.1.0.0/24", "aa:bb:cc:dd:ee:01"),
                statement("10.1.1.0/24", "aa:bb:cc:dd:ee:01"),
                statement("fd00::/8", "aa:bb:cc:dd:ee:02"),
            ],
            Vec::new(),
        );

        let contained: Vec<&PrefixStatement> = table.overlapping(network("10.1.0.0/16")).collect();
        assert_eq!(
            contained
                .iter()
                .map(|statement| statement.network)
                .collect::<Vec<_>>(),
            vec![
                network("10.0.0.0/8"),
                network("10.1.0.0/24"),
                network("10.1.1.0/24")
            ]
        );

        let sibling: Vec<&PrefixStatement> = table.overlapping(network("10.1.0.0/24")).collect();
        assert_eq!(
            sibling
                .iter()
                .map(|statement| statement.network)
                .collect::<Vec<_>>(),
            vec![network("10.0.0.0/8"), network("10.1.0.0/24")],
            "a sibling /24 is not inside this one"
        );
        assert!(
            table
                .overlapping(network("192.168.0.0/24"))
                .next()
                .is_none()
        );
        assert!(table.overlapping(network("2001:db8::/32")).next().is_none());
    }

    #[test]
    fn longest_picks_the_most_specific_statement_and_keeps_ties() {
        let mut tie = statement("10.1.0.0/16", "aa:bb:cc:dd:ee:01");
        tie.basis = PrefixBasis::NatRule;
        tie.locator = None;
        let table = PrefixTable::new(
            vec![
                statement("10.0.0.0/8", "aa:bb:cc:dd:ee:01"),
                statement("10.1.0.0/16", "aa:bb:cc:dd:ee:01"),
                tie,
            ],
            Vec::new(),
        );

        let longest: Vec<&PrefixStatement> = table
            .longest("aa:bb:cc:dd:ee:01", address("10.1.2.3"))
            .collect();
        assert_eq!(longest.len(), 2, "{longest:?}");
        assert!(
            longest
                .iter()
                .all(|statement| statement.network == network("10.1.0.0/16"))
        );
    }

    #[test]
    fn members_scope_to_the_stating_observer_and_pair_canonical_addresses() {
        let table = PrefixTable::new(
            vec![
                statement("10.0.0.0/8", "aa:bb:cc:dd:ee:01"),
                statement("10.1.0.0/16", "aa:bb:cc:dd:ee:01"),
                statement("10.0.0.0/8", "aa:bb:cc:dd:ee:02"),
            ],
            vec![
                observation("aa:bb:cc:dd:ee:01", "02:1f:5e:00:00:01", "10.1.2.3"),
                observation("aa:bb:cc:dd:ee:02", "02:1f:5e:00:00:02", "10.1.2.4"),
                observation("aa:bb:cc:dd:ee:01", "02:1f:5e:00:00:03", "192.168.1.1"),
            ],
        );
        let broad = table
            .statements()
            .iter()
            .find(|statement| {
                statement.observer == "aa:bb:cc:dd:ee:01"
                    && statement.network == network("10.0.0.0/8")
            })
            .expect("broad statement");

        let members: Vec<(&PrefixObservation, AddressWithPrefix)> = table.members(broad).collect();

        assert_eq!(members.len(), 1, "{members:?}");
        assert_eq!(
            members[0].0.node, "02:1f:5e:00:00:01",
            "another observer's address inside the same network is not a member"
        );
        assert_eq!(
            members[0].1.to_string(),
            "10.1.2.3/16",
            "the canonical address uses the observer's narrowest statement"
        );
    }

    #[test]
    fn canonical_uses_the_longest_match_else_the_host_route() {
        let table = PrefixTable::new(
            vec![
                statement("10.0.0.0/8", "aa:bb:cc:dd:ee:01"),
                statement("10.1.0.0/16", "aa:bb:cc:dd:ee:01"),
                statement("fd00::/8", "aa:bb:cc:dd:ee:01"),
            ],
            Vec::new(),
        );

        assert_eq!(
            table
                .canonical("aa:bb:cc:dd:ee:01", address("10.1.2.3"))
                .to_string(),
            "10.1.2.3/16"
        );
        assert_eq!(
            table
                .canonical("aa:bb:cc:dd:ee:01", address("192.0.2.10"))
                .to_string(),
            "192.0.2.10/32"
        );
        assert_eq!(
            table
                .canonical("aa:bb:cc:dd:ee:01", address("2001:db8::1"))
                .to_string(),
            "2001:db8::1/128"
        );
        assert_eq!(
            table
                .canonical("aa:bb:cc:dd:ee:02", address("fd00::1"))
                .to_string(),
            "fd00::1/128",
            "another observer's statements never explain this observer"
        );
    }

    #[test]
    fn reachability_filters_reject_host_local_and_group_scopes() {
        for text in [
            "0.0.0.0/0",
            "127.0.0.0/8",
            "169.254.0.0/16",
            "224.0.0.0/4",
            "255.255.255.255/32",
            "::/0",
            "::1/128",
            "fe80::/64",
            "ff02::/8",
        ] {
            assert!(!is_reachability_prefix(network(text)), "{text}");
        }
        for text in ["192.0.2.0/24", "10.0.0.0/8", "2001:db8::/32", "fd00::/8"] {
            assert!(is_reachability_prefix(network(text)), "{text}");
        }
    }

    #[test]
    fn observations_sort_and_deduplicate() {
        let table = PrefixTable::new(
            Vec::new(),
            vec![
                observation("aa:bb:cc:dd:ee:02", "02:1f:5e:00:00:02", "10.0.0.2"),
                observation("aa:bb:cc:dd:ee:01", "02:1f:5e:00:00:01", "10.0.0.1"),
                observation("aa:bb:cc:dd:ee:01", "02:1f:5e:00:00:01", "10.0.0.1"),
            ],
        );

        assert_eq!(table.observations().count(), 2);
        assert_eq!(
            table
                .observations()
                .map(|observation| observation.observer.as_str())
                .collect::<Vec<_>>(),
            vec!["aa:bb:cc:dd:ee:01", "aa:bb:cc:dd:ee:02"]
        );
    }
}
