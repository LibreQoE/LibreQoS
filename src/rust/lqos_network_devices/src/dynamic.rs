//! The dynamic-circuit store's supersede rule, and the published-world coverage
//! authority both it and the promotion gate answer from.
//!
//! One pure predicate lives here: which store rows a provisioned device now
//! supersedes. The eligibility/promotion/expiry decisions live in
//! [`crate::dynamic_engine`].

use ip_network::IpNetwork;
use ip_network_table::IpNetworkTable;
use lqos_utils::{DynamicOriginRows, XdpIpAddress};
use std::net::IpAddr;

/// K1 §1.6: the store file schema lives in `lqos_config` (the shared low-level
/// crate) so `lqos_topology` can fold it without depending on this crate.
/// Re-exported here so existing consumers keep their path.
pub use lqos_config::DynamicCircuit;
use lqos_config::ShapedDevice;

/// The PROVISIONED half of the published shaped world: the only rows the
/// supersede rule may match.
///
/// The fold synthesizes dynamic circuits into the same shaped-device snapshot it
/// publishes, so a published dynamic circuit appears in the catalog as a row
/// that looks exactly like a provisioned one. Matching that row would make every
/// dynamic circuit supersede itself one publish later (the u2 finding). The
/// origin marker the fold writes (`TopologyShapingDeviceInput::dynamic`) names
/// those rows, and this type is that knowledge in the form the rule needs:
/// per-ROW origin, plus the provisioned coverage authority below.
///
/// Its two inputs stay shared: the catalog is `Arc`-backed (its clone is two
/// pointer bumps) and the origin record is the `Arc` the state already
/// publishes, so a prune pass never deep-copies the row key set.
pub(crate) struct ProvisionedShapedRows {
    /// The published catalog the rule looks identities up in.
    catalog: crate::ShapedDevicesCatalog,
    /// The rows the fold synthesized (the `dynamic_circuits.json` overlay rows and
    /// the `[[dynamic_circuits.ranges]]` fallbacks — origin, not lifecycle),
    /// keyed by normalized circuit id plus device id.
    dynamic_origin_rows: std::sync::Arc<DynamicOriginRows>,
    /// The provisioned-prefix coverage operand, shared with the promotion gate.
    coverage: PublishedCoverage,
}

impl ProvisionedShapedRows {
    /// Splits the catalog into its provisioned half.
    ///
    /// Side effects: none. Cost is one pass over the catalog's device rows plus
    /// one table build, paid once per prune pass.
    pub(crate) fn new(
        catalog: &crate::ShapedDevicesCatalog,
        dynamic_origin_rows: &std::sync::Arc<DynamicOriginRows>,
    ) -> Self {
        Self {
            catalog: catalog.clone(),
            dynamic_origin_rows: std::sync::Arc::clone(dynamic_origin_rows),
            coverage: PublishedCoverage::new(catalog, dynamic_origin_rows),
        }
    }

    /// Whether a provisioned device now owns this dynamic circuit's identity or
    /// addresses.
    ///
    /// Two arms, both restricted to provisioned ROWS:
    /// - identity: a row carries the same device/circuit hash (a re-provisioned
    ///   device keeps its identity even when its addresses move);
    /// - coverage: a provisioned prefix contains one of the circuit's addresses,
    ///   including a broader one the circuit's own `/32` shadows by
    ///   longest-prefix match.
    pub(crate) fn is_superseded_by(&self, circuit: &DynamicCircuit) -> bool {
        if let Some(device) = self.catalog.device_by_hashes(
            Some(circuit.shaped.device_hash),
            Some(circuit.shaped.circuit_hash),
        ) && !self.is_dynamic_origin(device)
        {
            return true;
        }

        for (ip, _) in circuit.shaped.ipv4.iter() {
            if self
                .coverage
                .provisioned_covers(&XdpIpAddress::from_ip(IpAddr::V4(*ip)))
            {
                return true;
            }
        }
        for (ip, _) in circuit.shaped.ipv6.iter() {
            if self
                .coverage
                .provisioned_covers(&XdpIpAddress::from_ip(IpAddr::V6(*ip)))
            {
                return true;
            }
        }

        false
    }

    /// Whether this ROW is one the fold synthesized (not a provisioned device).
    fn is_dynamic_origin(&self, device: &ShapedDevice) -> bool {
        self.dynamic_origin_rows
            .contains_row(&device.circuit_id, &device.device_id)
    }
}

/// THE published-world coverage authority (u5 A4, REV-D2/REV-T1).
///
/// Two consumers ask "is this address already owned?", and both must answer from
/// the same provisioned operand:
/// - the supersede rule asks [`Self::provisioned_covers`];
/// - the promotion gate asks [`Self::promotion_covered`], which adds the overlay
///   store's own published rows.
///
/// The overlay operand cannot join the supersede rule's query — a store circuit
/// covers its own address, so it would supersede itself — but it is named exactly
/// once, here, beside the operand both consumers share. Neither consumer composes
/// coverage itself any more.
pub(crate) struct PublishedCoverage {
    /// Prefixes of the rows the fold did NOT synthesize (provisioned devices).
    provisioned: IpNetworkTable<()>,
}

impl PublishedCoverage {
    /// Builds the authority from the published shaped world.
    ///
    /// Side effects: none. Cost is one pass over the catalog's device rows plus
    /// one table build.
    pub(crate) fn new(
        catalog: &crate::ShapedDevicesCatalog,
        dynamic_origin_rows: &DynamicOriginRows,
    ) -> Self {
        Self {
            provisioned: provisioned_prefixes(catalog, dynamic_origin_rows),
        }
    }

    /// Whether a PROVISIONED device owns `ip` — the supersede rule's question.
    pub(crate) fn provisioned_covers(&self, ip: &XdpIpAddress) -> bool {
        prefixes_cover_ip(&self.provisioned, ip)
    }

    /// Whether `ip` is already owned for PROMOTION purposes: a provisioned device
    /// covers it, or the overlay store already publishes a circuit for it.
    ///
    /// The second arm is what keeps a promoted host from consuming another
    /// promotion slot in the window between its store write and the next shaped
    /// publish (and after any TTL prune that has not reached the shaped snapshot).
    /// The fold's synthesized range FALLBACK is in neither arm: it sits in the
    /// shaped snapshot carrying the dynamic-origin marker, and it covers its whole
    /// rule prefix — counting it would stop promotion for every host inside a
    /// catch-all range (the u5 gate).
    pub(crate) fn promotion_covered(
        &self,
        overlay: &crate::NetworkDevicesCatalog,
        ip: &XdpIpAddress,
    ) -> bool {
        self.provisioned_covers(ip) || overlay.dynamic_covers_ip(ip)
    }
}

/// The PROVISIONED prefixes of a shaped-devices catalog: every row the fold did
/// NOT synthesize (the `dynamic_origin_rows` marker names those).
///
/// Built from the device ROWS, not the catalog's trie iteration: the trie holds
/// one entry per PREFIX, and a fold row with an identical prefix takes the slot,
/// which would hide a provisioned device on the same address.
///
/// Side effects: none. Cost is one pass over the rows plus one table build.
fn provisioned_prefixes(
    catalog: &crate::ShapedDevicesCatalog,
    dynamic_origin_rows: &DynamicOriginRows,
) -> IpNetworkTable<()> {
    let mut prefixes = IpNetworkTable::new();
    for device in catalog.iter_devices() {
        if dynamic_origin_rows.contains_row(&device.circuit_id, &device.device_id) {
            continue;
        }
        insert_row_prefixes(&mut prefixes, device);
    }
    prefixes
}

/// Whether any prefix in `prefixes` covers `ip` (v4 mapped into v6 — the space
/// both the catalog trie and these tables store).
fn prefixes_cover_ip(prefixes: &IpNetworkTable<()>, ip: &XdpIpAddress) -> bool {
    let lookup = match ip.as_ip() {
        IpAddr::V4(ip) => ip.to_ipv6_mapped(),
        IpAddr::V6(ip) => ip,
    };
    prefixes.longest_match(lookup).is_some()
}

/// Inserts one device row's IPv4 and IPv6 prefixes, mapping the v4 ones into v6
/// exactly as the shaped-devices trie does.
fn insert_row_prefixes(prefixes: &mut IpNetworkTable<()>, device: &ShapedDevice) {
    for (ip, prefix_len) in &device.ipv4 {
        let Some(prefix_len) = u8::try_from(prefix_len.saturating_add(96)).ok() else {
            continue;
        };
        if let Ok(network) = IpNetwork::new(ip.to_ipv6_mapped(), prefix_len) {
            prefixes.insert(network, ());
        }
    }
    for (ip, prefix_len) in &device.ipv6 {
        let Some(prefix_len) = u8::try_from(*prefix_len).ok() else {
            continue;
        };
        if let Ok(network) = IpNetwork::new(*ip, prefix_len) {
            prefixes.insert(network, ());
        }
    }
}
