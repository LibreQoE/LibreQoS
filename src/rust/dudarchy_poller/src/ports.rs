//! The per-device port inventory: one pure projection joining a device's
//! own interface list with the evidence that names its ports — its ARP
//! bindings, its bridge's learned hosts, its bridge-port memberships and
//! VLAN configuration, and its own neighbour observations.
//!
//! Every inventoried interface appears, including empty, disabled,
//! bridge, VLAN, and tunnel ports. Evidence joins by exact port name
//! inside the reporting device and nothing else: a label that matches no
//! interface stays in the inventory's `unresolved` section with its raw
//! identifier and source — no synthetic port is ever invented, and no
//! attachment ever propagates down a VLAN/bridge hierarchy or becomes an
//! adjacency edge. The parent and bridge annotations are per-port facts
//! read straight off the device's rows and never traversed, so cyclic
//! parent or bridge relations cannot loop. The PVID is carried as
//! configured data and never read as VLAN membership; VLAN membership
//! comes only from bridge-VLAN rows, whose existence is the
//! VLAN-filtering evidence.
//!
//! The model and the join are pure and deterministic: the same inputs
//! always produce the same output in the same order — ports sorted by
//! name then kind, attachments by relation order then MAC/IP, unresolved
//! entries by port label then relation. No function here reads a clock or
//! the network.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::stored::{StoredDetails, StoredLink};

/// What kind of statement a piece of port evidence makes. Declaration
/// order is the attachment sort and display order; `Vlan` and
/// `Membership` occur only in the unresolved section, where they mark
/// bridge-VLAN and bridge-port rows whose port label matched no
/// interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Relation {
    /// A neighbour the device itself saw on the port (its own LLDP/API
    /// feed) — adjacency evidence.
    Neighbor,
    /// A MAC the device's bridge learned on the port — FDB reachability.
    Fdb,
    /// An IP-to-MAC binding the device reported on the port — ARP
    /// mapping.
    Arp,
    /// A wireless association the device reported — an attachment
    /// snapshot, never port-derived.
    Wireless,
    /// A bridge-VLAN row naming the port label (unresolved only).
    Vlan,
    /// A bridge-port row naming the port label (unresolved only).
    Membership,
}

impl fmt::Display for Relation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Neighbor => "neighbour",
            Self::Fdb => "fdb",
            Self::Arp => "arp",
            Self::Wireless => "wireless",
            Self::Vlan => "vlan",
            Self::Membership => "membership",
        })
    }
}

/// One attachment on a port: what the device itself reported about that
/// port. The known-host `host` is filled when the attachment's MAC is a
/// host the walk or store knows by name; it never comes from the
/// reporting device.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Attachment {
    /// The relation the evidence claims.
    pub relation: Relation,
    /// The observed MAC, normalized, when the evidence carried one.
    pub mac: Option<String>,
    /// The observed IP, when the evidence carried one (ARP).
    pub ip: Option<String>,
    /// The known-host name for `mac`, when one exists.
    pub host: Option<String>,
    /// The remote end's own port id, when reported (neighbour rows).
    pub remote_port: Option<String>,
    /// The reported status or flags, as compact display text.
    pub status: Option<String>,
    /// The concrete query that produced the evidence.
    pub source: String,
}

/// The bridge-port membership a device reports for one of its own
/// interfaces. The flags stay separate: `disabled` is configuration,
/// `inactive` is the dataplane's verdict, and `status` is the router's
/// own word. The `pvid` is configured data and is never read as VLAN
/// membership.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BridgeMembership {
    /// The bridge the port belongs to, when the row named it.
    pub bridge: Option<String>,
    /// The port's configured untagged VLAN id, when reported.
    pub pvid: Option<u32>,
    /// The `disabled` flag, when reported.
    pub disabled: Option<bool>,
    /// The `inactive` flag, when reported.
    pub inactive: Option<bool>,
    /// The router's raw port status, when it reports one.
    pub status: Option<String>,
}

impl BridgeMembership {
    /// The membership as compact display text: the bridge, the PVID, and
    /// the three separate state fields.
    fn summary(&self) -> String {
        let flag = |value: Option<bool>| match value {
            Some(true) => "true".to_owned(),
            Some(false) => "false".to_owned(),
            None => "-".to_owned(),
        };
        format!(
            "bridge {} pvid {} status {} inactive {} disabled {}",
            self.bridge.as_deref().unwrap_or("-"),
            self.pvid
                .map(|pvid| pvid.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            self.status.as_deref().unwrap_or("-"),
            flag(self.inactive),
            flag(self.disabled),
        )
    }
}

/// One VLAN a bridge-VLAN row puts the port in, with its role. Rows
/// exist only where VLAN filtering is configured, so this — never the
/// PVID — is VLAN-membership evidence.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct VlanMembership {
    /// The VLAN ids the row assigns to the role, sorted.
    pub vlan_ids: Vec<u32>,
    /// Tagged or untagged carriage.
    pub tagged: bool,
}

impl VlanMembership {
    /// The membership as compact display text.
    fn summary(&self) -> String {
        format!(
            "vlans {} {}",
            self.vlan_ids
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(","),
            if self.tagged { "tagged" } else { "untagged" },
        )
    }
}

/// One inventoried port: the device's own interface row plus everything
/// that names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Port {
    /// The interface name, when the device gave one.
    pub name: Option<String>,
    /// The normalized kind, as display text.
    pub kind: String,
    /// The interface's own MAC, normalized, when reported.
    pub mac: Option<String>,
    /// The addresses bound to the interface, in device order.
    pub addresses: Vec<String>,
    /// The administrative state, when reported.
    pub admin: Option<String>,
    /// The operational state, when reported.
    pub oper: Option<String>,
    /// The VLAN id, when the device carries VLAN detail for the row.
    pub vlan_id: Option<u32>,
    /// The parent interface, for VLAN sub-interfaces.
    pub parent: Option<String>,
    /// The bridge membership, when a bridge-port row names this port.
    pub membership: Option<BridgeMembership>,
    /// The VLAN memberships bridge-VLAN rows give this port, untagged
    /// rows first, ids sorted within each role.
    pub vlans: Vec<VlanMembership>,
    /// The attachments, sorted by relation then MAC/IP.
    pub attachments: Vec<Attachment>,
}

/// Evidence whose port label matched no interface of the reporting
/// device: preserved raw — identifier, relation, source, and a compact
/// summary — never guessed onto a port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unresolved {
    /// The raw port label as the device reported it; `-` when the row
    /// named no label at all.
    pub port: String,
    /// What kind of statement the evidence makes.
    pub relation: Relation,
    /// The concrete query that produced the evidence.
    pub source: String,
    /// The evidence's own fields, as compact display text.
    pub summary: String,
}

/// One device's port inventory: every interface the device reported,
/// each with the evidence that names it, plus the evidence whose labels
/// matched nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortInventory {
    /// The ports, sorted by name then kind.
    pub ports: Vec<Port>,
    /// The unjoinable evidence, sorted by port label then relation.
    pub unresolved: Vec<Unresolved>,
}

/// The shared join: attaches evidence and memberships to the ports whose
/// names equal their labels, routes the rest to unresolved, and puts
/// every list in its deterministic order. `host_of` resolves an
/// attachment MAC (normalized text) to a known-host name when one exists.
fn join(
    mut ports: Vec<Port>,
    memberships: Vec<(Option<String>, BridgeMembership)>,
    vlan_memberships: Vec<(Option<String>, VlanMembership)>,
    attachments: Vec<Evidence>,
    host_of: &dyn Fn(&str) -> Option<String>,
) -> PortInventory {
    let mut unresolved: Vec<Unresolved> = Vec::new();
    // The indices of the ports whose name is one of the label's
    // comma-separated parts, in first-match order, deduplicated.
    // RouterOS reports one port as a comma-separated list when an
    // observation spans several interfaces (`e1-vlan94,br_94`); every
    // part that exactly names an inventoried interface carries the
    // evidence.
    fn matching_ports(ports: &[Port], label: &Option<String>) -> Vec<usize> {
        let Some(label) = label.as_deref() else {
            return Vec::new();
        };
        let mut indices: Vec<usize> = Vec::new();
        for part in label.split(',').filter(|part| !part.is_empty()) {
            if let Some(index) = ports
                .iter()
                .position(|port| port.name.as_deref() == Some(part))
                && !indices.contains(&index)
            {
                indices.push(index);
            }
        }
        indices
    }
    // The unjoinable label's entry: one, carrying the raw label as the
    // device reported it.
    fn unresolved_of(
        label: &Option<String>,
        relation: Relation,
        source: &str,
        summary: String,
    ) -> Unresolved {
        Unresolved {
            port: label.clone().unwrap_or_else(|| "-".to_owned()),
            relation,
            source: source.to_owned(),
            summary,
        }
    }
    for (label, membership) in memberships {
        let indices = matching_ports(&ports, &label);
        if indices.is_empty() {
            unresolved.push(unresolved_of(
                &label,
                Relation::Membership,
                "routeros-bridge-ports",
                membership.summary(),
            ));
            continue;
        }
        for index in indices {
            // One interface, one membership row: the first row in device
            // order wins; a device reporting two rows for one port is
            // anomalous and the raw details keep both.
            let port = &mut ports[index];
            if port.membership.is_none() {
                port.membership = Some(membership.clone());
            }
        }
    }
    for (label, vlan) in vlan_memberships {
        let indices = matching_ports(&ports, &label);
        if indices.is_empty() {
            unresolved.push(unresolved_of(
                &label,
                Relation::Vlan,
                "routeros-bridge-vlans",
                vlan.summary(),
            ));
            continue;
        }
        for index in indices {
            ports[index].vlans.push(vlan.clone());
        }
    }
    for item in attachments {
        let mut attachment = item.attachment;
        attachment.host = attachment.mac.as_deref().and_then(host_of);
        let indices = matching_ports(&ports, &item.port);
        if indices.is_empty() {
            unresolved.push(unresolved_of(
                &item.port,
                attachment.relation,
                &attachment.source,
                unresolved_summary(&attachment),
            ));
            continue;
        }
        for index in indices {
            ports[index].attachments.push(attachment.clone());
        }
    }
    // Deterministic order everywhere: ports by name then kind, VLAN
    // memberships merged per role with sorted ids, attachments by
    // relation then MAC/IP, unresolved by label then relation.
    ports.sort_by(|a, b| {
        a.name
            .clone()
            .unwrap_or_default()
            .cmp(&b.name.clone().unwrap_or_default())
            .then(a.kind.cmp(&b.kind))
    });
    for port in &mut ports {
        port.attachments.sort();
        let mut by_role: BTreeMap<bool, BTreeSet<u32>> = BTreeMap::new();
        for vlan in port.vlans.drain(..) {
            by_role
                .entry(vlan.tagged)
                .or_default()
                .extend(vlan.vlan_ids);
        }
        port.vlans = by_role
            .into_iter()
            .map(|(tagged, ids)| VlanMembership {
                vlan_ids: ids.into_iter().collect(),
                tagged,
            })
            .collect();
    }
    unresolved.sort_by(|a, b| {
        a.port
            .cmp(&b.port)
            .then(a.relation.cmp(&b.relation))
            .then(a.summary.cmp(&b.summary))
    });
    PortInventory { ports, unresolved }
}

/// One piece of port-labelled evidence waiting to be joined.
struct Evidence {
    port: Option<String>,
    attachment: Attachment,
}

/// The unresolved entry's compact evidence text, built from the
/// attachment's own fields.
fn unresolved_summary(attachment: &Attachment) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(mac) = &attachment.mac {
        parts.push(format!("mac {mac}"));
    }
    if let Some(ip) = &attachment.ip {
        parts.push(format!("ip {ip}"));
    }
    if let Some(remote) = &attachment.remote_port {
        parts.push(format!("their-port {remote}"));
    }
    if let Some(status) = &attachment.status {
        parts.push(status.clone());
    }
    if parts.is_empty() {
        parts.push("-".to_owned());
    }
    parts.join(" ")
}

/// The ARP flags as compact status text.
fn arp_status(dynamic: Option<bool>, complete: Option<bool>) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    match dynamic {
        Some(true) => parts.push("dynamic"),
        Some(false) => parts.push("static"),
        None => {}
    }
    match complete {
        Some(true) => parts.push("complete"),
        Some(false) => parts.push("resolving"),
        None => {}
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// The bridge-host flags as compact status text; the age rides along
/// when the router reports one.
fn fdb_status(
    local: Option<bool>,
    external: Option<bool>,
    dynamic: Option<bool>,
    age: Option<&str>,
) -> Option<String> {
    let mut status = match (local, external, dynamic) {
        (Some(true), _, _) => "self".to_owned(),
        (_, Some(true), _) => "external".to_owned(),
        (_, _, Some(true)) => "learned".to_owned(),
        _ => "static".to_owned(),
    };
    if let Some(age) = age {
        status.push_str(&format!(" age {age}"));
    }
    Some(status)
}

/// host's stored details and its link observations in both directions —
/// outgoing links attach on the host's local port, incoming links on the
/// host's own advertised port id (the link's remote port), which skips
/// observations that name none. The field semantics are the live join's,
/// at display fidelity — MACs and provenance arrive as the text U2
/// stored.
pub fn inventory_stored(
    details: &StoredDetails,
    outgoing: &[StoredLink],
    incoming: &[StoredLink],
    host_of: &dyn Fn(&str) -> Option<String>,
) -> PortInventory {
    let ports = spine(
        details
            .interfaces
            .iter()
            .map(|row| (row.name.clone(), row.kind.clone())),
        details.interfaces.iter().map(|row| SpineRow {
            mac: row.mac.clone(),
            addresses: row.addresses.clone(),
            admin: row.admin.clone(),
            oper: row.oper.clone(),
            vlan_id: row.vlan_id,
            parent: row.parent.clone(),
        }),
    );
    let memberships = details
        .bridge_ports
        .as_ref()
        .map(|snapshot| {
            snapshot
                .rows
                .iter()
                .map(|row| {
                    (
                        row.interface.clone(),
                        BridgeMembership {
                            bridge: row.bridge.clone(),
                            pvid: row.pvid,
                            disabled: row.disabled,
                            inactive: row.inactive,
                            status: row.status.clone(),
                        },
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let mut vlan_memberships: Vec<(Option<String>, VlanMembership)> = Vec::new();
    if let Some(vlans) = &details.bridge_vlans {
        for row in &vlans.rows {
            let vlan_ids = row.vlan_ids.clone();
            for (labels, tagged) in [
                (
                    row.untagged
                        .iter()
                        .chain(row.current_untagged.iter())
                        .collect::<Vec<_>>(),
                    false,
                ),
                (
                    row.tagged
                        .iter()
                        .chain(row.current_tagged.iter())
                        .collect::<Vec<_>>(),
                    true,
                ),
            ] {
                for label in labels {
                    vlan_memberships.push((
                        Some(label.clone()),
                        VlanMembership {
                            vlan_ids: vlan_ids.clone(),
                            tagged,
                        },
                    ));
                }
            }
        }
    }
    let mut attachments: Vec<Evidence> = Vec::new();
    if let Some(arp) = &details.arp {
        let source = arp.sources.join(",");
        for row in &arp.rows {
            attachments.push(Evidence {
                port: row.interface.clone(),
                attachment: Attachment {
                    relation: Relation::Arp,
                    mac: row.mac.clone(),
                    ip: row.address.clone(),
                    host: None,
                    remote_port: None,
                    status: arp_status(row.dynamic, row.complete),
                    source: source.clone(),
                },
            });
        }
    }
    if let Some(hosts) = &details.bridge_hosts {
        let source = hosts.sources.join(",");
        for row in &hosts.rows {
            if row.local == Some(true) {
                continue;
            }
            attachments.push(Evidence {
                port: row.on_interface.clone(),
                attachment: Attachment {
                    relation: Relation::Fdb,
                    mac: row.mac.clone(),
                    ip: None,
                    host: None,
                    remote_port: None,
                    status: fdb_status(row.local, row.external, row.dynamic, row.age.as_deref()),
                    source: source.clone(),
                },
            });
        }
    }
    for link in outgoing {
        let Some(label) = &link.local_port else {
            continue;
        };
        attachments.push(Evidence {
            port: Some(label.clone()),
            attachment: Attachment {
                relation: if link.wireless {
                    Relation::Wireless
                } else {
                    Relation::Neighbor
                },
                mac: Some(link.to.clone()),
                ip: None,
                host: None,
                remote_port: link.remote_port.clone(),
                status: None,
                source: link
                    .local_port_source
                    .clone()
                    .unwrap_or_else(|| link.sources.clone()),
            },
        });
    }
    for link in incoming {
        // The observer's report names this host's own port as the link's
        // remote port; without one it attributes nothing of ours.
        let Some(label) = &link.remote_port else {
            continue;
        };
        attachments.push(Evidence {
            port: Some(label.clone()),
            attachment: Attachment {
                relation: if link.wireless {
                    Relation::Wireless
                } else {
                    Relation::Neighbor
                },
                mac: link.from.clone(),
                ip: None,
                host: None,
                remote_port: link.local_port.clone(),
                status: None,
                source: link.sources.clone(),
            },
        });
    }
    join(ports, memberships, vlan_memberships, attachments, host_of)
}

/// One spine row's display fields, the shape both detail projections
/// hand over.
struct SpineRow {
    mac: Option<String>,
    addresses: Vec<String>,
    admin: Option<String>,
    oper: Option<String>,
    vlan_id: Option<u32>,
    parent: Option<String>,
}

/// Builds the port spine from the interface names/kinds and their
/// display fields, zipped in device order.
fn spine(
    names_kinds: impl Iterator<Item = (Option<String>, String)>,
    rows: impl Iterator<Item = SpineRow>,
) -> Vec<Port> {
    names_kinds
        .zip(rows)
        .map(|((name, kind), row)| Port {
            name,
            kind,
            mac: row.mac,
            addresses: row.addresses,
            admin: row.admin,
            oper: row.oper,
            vlan_id: row.vlan_id,
            parent: row.parent,
            membership: None,
            vlans: Vec::new(),
            attachments: Vec::new(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{
        ArpEntry, BridgeHost, BridgePort, BridgeVlan, DeviceDetails, Interface, InterfaceKind, Mac,
        Source, TableSnapshot,
    };
    use crate::walk::WalkLink;
    use std::time::SystemTime;

    /// One interface row with the given name and kind, everything else
    /// absent.
    fn iface(name: &str, kind: InterfaceKind) -> Interface {
        Interface {
            id: name.to_owned(),
            name: Some(name.to_owned()),
            description: None,
            alias: None,
            kind,
            raw_type: None,
            mac: None,
            addresses: Vec::new(),
            admin_state: None,
            oper_state: None,
            vlan_id: None,
            parent_interface: None,
            provenance: Vec::new(),
        }
    }

    /// An ARP evidence row.
    fn arp(interface: Option<&str>, ip: &str, mac: Option<&str>) -> ArpEntry {
        ArpEntry {
            address: Some(ip.to_owned()),
            mac_address: mac.and_then(Mac::parse),
            interface: interface.map(str::to_owned),
            dynamic: Some(true),
            complete: Some(mac.is_some()),
            disabled: None,
        }
    }

    /// A details model carrying the given interfaces and evidence; the
    /// snapshots that are None read as unavailable.
    fn details(
        interfaces: Vec<Interface>,
        arp: Option<Vec<ArpEntry>>,
        bridge_ports: Option<Vec<BridgePort>>,
        bridge_vlans: Option<Vec<BridgeVlan>>,
        bridge_hosts: Option<Vec<BridgeHost>>,
    ) -> DeviceDetails {
        fn snapshot<T>(rows: Vec<T>, source: Source) -> TableSnapshot<T> {
            TableSnapshot {
                rows,
                provenance: vec![source],
                observed_at: SystemTime::UNIX_EPOCH,
            }
        }
        DeviceDetails {
            interfaces,
            system: None,
            temperatures: Vec::new(),
            stations: Vec::new(),
            radio: None,
            arp: arp.map(|rows| snapshot(rows, Source::RouterOsArp)),
            bridge_ports: bridge_ports.map(|rows| snapshot(rows, Source::RouterOsBridgePorts)),
            bridge_vlans: bridge_vlans.map(|rows| snapshot(rows, Source::RouterOsBridgeVlans)),
            bridge_hosts: bridge_hosts.map(|rows| snapshot(rows, Source::RouterOsBridgeHosts)),
            nat: None,
            dhcp_leases: None,
        }
    }

    /// A neighbour link the node observed through `port`.
    fn link(port: &str, to: &str, remote: Option<&str>) -> WalkLink {
        WalkLink {
            from: "self".to_owned(),
            to: to.to_owned(),
            local_port: Some((Source::RouterOsNeighbors, port.to_owned())),
            remote_port: remote.map(str::to_owned),
            sources: crate::identity::Sources::Api,
            wireless: false,
        }
    }

    /// The known-host lookup the tests share: one named host.
    fn host_of(mac: &str) -> Option<String> {
        (mac == "02:00:00:aa:00:01").then(|| "known-host".to_owned())
    }

    /// The server's join, driven from the walk-shaped fixtures above: the
    /// details are projected exactly as the store projects them, and the
    /// node's links are split into the outgoing and incoming directions the
    /// stored join takes.
    fn inventory(
        details: &DeviceDetails,
        links: &[WalkLink],
        self_key: &str,
        host_of: &dyn Fn(&str) -> Option<String>,
    ) -> PortInventory {
        let node = crate::walk::WalkNode {
            mac: Some(self_key.to_owned()),
            address: None,
            name: None,
            depth: 0,
            source: crate::walk::NodeSource::Seed,
            claims: Vec::new(),
            details: Some(details.clone()),
            station: false,
        };
        let stored = crate::stored::StoredHost::from_node(&node, SystemTime::UNIX_EPOCH)
            .expect("a MAC-keyed node projects")
            .details
            .expect("the details project");
        let stored_link = |link: &WalkLink| crate::stored::StoredLink {
            from: Some(link.from.clone()),
            from_is_mac: false,
            to: link.to.clone(),
            local_port: link.local_port.as_ref().map(|(_, port)| port.clone()),
            local_port_source: link
                .local_port
                .as_ref()
                .map(|(source, _)| source.to_string()),
            remote_port: link.remote_port.clone(),
            sources: link.sources.to_string(),
            wireless: link.wireless,
            first_seen: SystemTime::UNIX_EPOCH,
            last_seen: SystemTime::UNIX_EPOCH,
            job: 0,
        };
        let outgoing: Vec<_> = links
            .iter()
            .filter(|link| link.from == self_key)
            .map(stored_link)
            .collect();
        let incoming: Vec<_> = links
            .iter()
            .filter(|link| link.to == self_key)
            .map(stored_link)
            .collect();
        inventory_stored(&stored, &outgoing, &incoming, host_of)
    }

    /// An attachment's compact text: a stable lens on the `Attachment`
    /// fields, with `-` for an absent MAC and `(name)` for a known host.
    fn render_attachment(attachment: &Attachment) -> String {
        let mac = match (&attachment.mac, &attachment.host) {
            (Some(mac), Some(host)) => format!("{mac} ({host})"),
            (Some(mac), None) => mac.clone(),
            (None, _) => "-".to_owned(),
        };
        match attachment.relation {
            Relation::Neighbor => format!(
                "neighbour {mac} their-port {} {}",
                attachment.remote_port.as_deref().unwrap_or("-"),
                attachment.source
            ),
            Relation::Wireless => format!("wireless {mac} {}", attachment.source),
            Relation::Fdb => format!(
                "fdb {mac}{} {}",
                attachment
                    .status
                    .as_deref()
                    .map(|status| format!(" {status}"))
                    .unwrap_or_default(),
                attachment.source
            ),
            Relation::Arp => format!(
                "arp {} mac {mac}{} {}",
                attachment.ip.as_deref().unwrap_or("-"),
                attachment
                    .status
                    .as_deref()
                    .map(|status| format!(" {status}"))
                    .unwrap_or_default(),
                attachment.source
            ),
            Relation::Vlan | Relation::Membership => attachment.source.clone(),
        }
    }

    #[test]
    fn evidence_on_nameless_interfaces_stays_unresolved_in_full() {
        let inventory = inventory(
            &details(
                Vec::new(),
                Some(vec![arp(None, "10.0.0.5", None)]),
                None,
                None,
                None,
            ),
            &[],
            "seed",
            &host_of,
        );
        assert!(inventory.ports.is_empty(), "no interfaces, no ports");
        assert_eq!(inventory.unresolved.len(), 1);
        let item = &inventory.unresolved[0];
        assert_eq!(item.port, "-", "no label at all renders as -");
        assert_eq!(item.relation, Relation::Arp);
        assert_eq!(item.summary, "ip 10.0.0.5 dynamic resolving");
    }

    #[test]
    fn attachments_join_by_exact_name_and_known_hosts_are_named() {
        let details = details(
            vec![
                iface("ether1", InterfaceKind::Ethernet),
                iface("ether2", InterfaceKind::Ethernet),
            ],
            Some(vec![
                arp(Some("ether1"), "10.0.0.5", Some("02:00:00:aa:00:01")),
                arp(Some("ether2"), "10.0.0.6", Some("02:00:00:aa:00:02")),
            ]),
            None,
            None,
            Some(vec![
                BridgeHost {
                    bridge: Some("br1".to_owned()),
                    on_interface: Some("ether2".to_owned()),
                    mac_address: Mac::parse("02:00:00:aa:00:02"),
                    age: Some("3m33s".to_owned()),
                    dynamic: Some(true),
                    local: Some(false),
                    external: None,
                },
                // The bridge's own MAC: context, never an attachment.
                BridgeHost {
                    bridge: Some("br1".to_owned()),
                    on_interface: Some("ether2".to_owned()),
                    mac_address: Mac::parse("d4:ca:6d:54:da:4e"),
                    age: None,
                    dynamic: Some(true),
                    local: Some(true),
                    external: None,
                },
            ]),
        );
        let links = [link("ether1", "02:00:00:aa:00:02", Some("eth9"))];
        let inventory = inventory(&details, &links, "self", &host_of);

        assert_eq!(inventory.ports.len(), 2);
        let ether1 = &inventory.ports[0];
        assert_eq!(ether1.name.as_deref(), Some("ether1"));
        assert_eq!(ether1.attachments.len(), 2, "the ARP row and the link");
        assert_eq!(
            render_attachment(&ether1.attachments[0]),
            "neighbour 02:00:00:aa:00:02 their-port eth9 routeros-neighbors",
            "neighbour sorts before arp; an unknown MAC carries no name"
        );
        assert_eq!(
            render_attachment(&ether1.attachments[1]),
            "arp 10.0.0.5 mac 02:00:00:aa:00:01 (known-host) dynamic complete routeros-arp",
            "a known MAC is named"
        );
        let ether2 = &inventory.ports[1];
        assert_eq!(
            ether2.attachments.len(),
            2,
            "the bridge's own MAC is not an attachment; its ARP row and FDB row are"
        );
        assert_eq!(
            render_attachment(&ether2.attachments[0]),
            "fdb 02:00:00:aa:00:02 learned age 3m33s routeros-bridge-hosts",
            "fdb sorts before arp"
        );
        assert_eq!(
            render_attachment(&ether2.attachments[1]),
            "arp 10.0.0.6 mac 02:00:00:aa:00:02 dynamic complete routeros-arp"
        );
        assert!(inventory.unresolved.is_empty());
    }

    #[test]
    fn membership_joins_vlans_come_only_from_vlan_rows_and_ghosts_stay_unresolved() {
        let details = details(
            vec![
                iface("ether1", InterfaceKind::Ethernet),
                iface("br1", InterfaceKind::Bridge),
            ],
            None,
            Some(vec![BridgePort {
                bridge: Some("br1".to_owned()),
                interface: Some("ether1".to_owned()),
                pvid: Some(92),
                frame_types: None,
                status: Some("in-bridge".to_owned()),
                inactive: Some(false),
                disabled: Some(false),
                dynamic: Some(false),
            }]),
            Some(vec![BridgeVlan {
                bridge: Some("br1".to_owned()),
                vlan_ids: vec![666, 92],
                tagged: vec!["ether1".to_owned(), "ghost".to_owned()],
                untagged: vec![],
                current_tagged: vec![],
                current_untagged: vec![],
            }]),
            None,
        );
        let inventory = inventory(&details, &[], "seed", &host_of);

        let ether1 = inventory
            .ports
            .iter()
            .find(|port| port.name.as_deref() == Some("ether1"))
            .expect("the interface is inventoried");
        let membership = ether1.membership.as_ref().expect("the membership joins");
        assert_eq!(membership.bridge.as_deref(), Some("br1"));
        assert_eq!(membership.pvid, Some(92));
        assert!(
            ether1
                .vlans
                .iter()
                .all(|vlan| !vlan.vlan_ids.contains(&92) || vlan.tagged),
            "the PVID is configured data, never VLAN membership"
        );
        assert_eq!(
            ether1.vlans,
            [VlanMembership {
                vlan_ids: vec![92, 666],
                tagged: true
            }],
            "tagged and current-tagged merge into one sorted entry"
        );
        let unresolved = &inventory.unresolved;
        assert_eq!(unresolved.len(), 1);
        assert_eq!(unresolved[0].port, "ghost");
        assert_eq!(unresolved[0].relation, Relation::Vlan);
        assert_eq!(
            unresolved[0].summary, "vlans 666,92 tagged",
            "unresolved evidence keeps the router's own row order"
        );
    }

    #[test]
    fn an_unresolved_lldp_label_stays_raw_with_its_evidence() {
        let details = details(
            vec![iface("ether1", InterfaceKind::Ethernet)],
            None,
            None,
            None,
            None,
        );
        let links = [link("7", "02:00:00:aa:00:02", Some("eth1"))];
        let inventory = inventory(&details, &links, "self", &host_of);
        assert!(inventory.ports[0].attachments.is_empty());
        assert_eq!(inventory.unresolved.len(), 1);
        let item = &inventory.unresolved[0];
        assert_eq!(item.port, "7");
        assert_eq!(item.relation, Relation::Neighbor);
        assert_eq!(item.summary, "mac 02:00:00:aa:00:02 their-port eth1");
        assert_eq!(item.source, "routeros-neighbors");
    }

    #[test]
    fn incoming_observations_attach_on_the_node_s_own_port_and_skip_portless_ones() {
        let details = details(
            vec![
                iface("ether1", InterfaceKind::Ethernet),
                iface("ether2", InterfaceKind::Ethernet),
            ],
            None,
            None,
            None,
            None,
        );
        // One outgoing observation (the node saw a neighbour through
        // ether1), one incoming (an observer reported the node's own
        // advertised port "ether2"), and one incoming report that names
        // no port of ours.
        let links = [
            link("ether1", "02:00:00:aa:00:02", Some("their-eth9")),
            WalkLink {
                from: "02:00:00:aa:00:03".to_owned(),
                to: "self".to_owned(),
                local_port: Some((Source::SnmpLldp, "swp3".to_owned())),
                remote_port: Some("ether2".to_owned()),
                sources: crate::identity::Sources::Lldp,
                wireless: false,
            },
            WalkLink {
                from: "02:00:00:aa:00:04".to_owned(),
                to: "self".to_owned(),
                local_port: Some((Source::SnmpLldp, "swp4".to_owned())),
                remote_port: None,
                sources: crate::identity::Sources::Lldp,
                wireless: false,
            },
        ];
        let first = inventory(&details, &links, "self", &host_of);
        let again = inventory(&details, &links, "self", &host_of);

        let ether1 = &first.ports[0];
        assert_eq!(ether1.name.as_deref(), Some("ether1"));
        assert_eq!(ether1.attachments.len(), 1);
        assert_eq!(
            render_attachment(&ether1.attachments[0]),
            "neighbour 02:00:00:aa:00:02 their-port their-eth9 routeros-neighbors",
            "the outgoing attachment is unchanged"
        );
        let ether2 = &first.ports[1];
        assert_eq!(ether2.name.as_deref(), Some("ether2"));
        assert_eq!(
            ether2.attachments.len(),
            1,
            "the portless report attaches nothing"
        );
        assert_eq!(
            render_attachment(&ether2.attachments[0]),
            "neighbour 02:00:00:aa:00:03 their-port swp3 lldp",
            "the incoming observation names the observer and its port, sourced by the link's feeds"
        );
        assert!(first.unresolved.is_empty());

        // Deterministic: the same inputs produce the same inventory.
        assert_eq!(first, again);
    }

    #[test]
    fn comma_separated_labels_attach_to_every_matching_part_or_stay_raw() {
        let details = details(
            vec![
                iface("ether1", InterfaceKind::Ethernet),
                iface("br1", InterfaceKind::Bridge),
            ],
            None,
            None,
            None,
            None,
        );
        // The RouterOS `interface` form: one observation spanning several
        // interfaces — both named, one half-named, one naming nothing
        // this device carries, and one naming the same port twice.
        let links = [
            link("ether1,br1", "02:00:00:aa:00:02", Some("swp1")),
            link("ether1,ghost", "02:00:00:aa:00:03", Some("swp2")),
            link("ghost1,ghost2", "02:00:00:aa:00:04", None),
            link("br1,br1", "02:00:00:aa:00:05", Some("swp4")),
        ];
        let inventory = inventory(&details, &links, "self", &host_of);

        // br1 sorts first. The shared label lands once on each of its
        // ports; the doubled part is not duplicated.
        let br1 = &inventory.ports[0];
        assert_eq!(br1.name.as_deref(), Some("br1"));
        assert_eq!(
            br1.attachments.len(),
            2,
            "one per naming link, deduplicated"
        );
        assert_eq!(
            render_attachment(&br1.attachments[0]),
            "neighbour 02:00:00:aa:00:02 their-port swp1 routeros-neighbors"
        );
        assert_eq!(
            render_attachment(&br1.attachments[1]),
            "neighbour 02:00:00:aa:00:05 their-port swp4 routeros-neighbors"
        );
        let ether1 = &inventory.ports[1];
        assert_eq!(ether1.name.as_deref(), Some("ether1"));
        assert_eq!(
            ether1.attachments.len(),
            2,
            "the ghost part adds no fake port"
        );
        assert_eq!(
            render_attachment(&ether1.attachments[1]),
            "neighbour 02:00:00:aa:00:03 their-port swp2 routeros-neighbors"
        );

        // A label whose parts name nothing stays unresolved as the raw
        // full string.
        assert_eq!(inventory.unresolved.len(), 1);
        assert_eq!(inventory.unresolved[0].port, "ghost1,ghost2");
        assert_eq!(inventory.unresolved[0].summary, "mac 02:00:00:aa:00:04");
    }

    #[test]
    fn cyclic_parent_and_self_bridge_relations_join_without_propagation() {
        let mut a = iface("vlan-a", InterfaceKind::Vlan);
        a.vlan_id = Some(10);
        a.parent_interface = Some("vlan-b".to_owned());
        let mut b = iface("vlan-b", InterfaceKind::Vlan);
        b.vlan_id = Some(20);
        b.parent_interface = Some("vlan-a".to_owned());
        // br1 is a member of itself: a cyclic bridge relation.
        let details = details(
            vec![a, b, iface("br1", InterfaceKind::Bridge)],
            None,
            Some(vec![BridgePort {
                bridge: Some("br1".to_owned()),
                interface: Some("br1".to_owned()),
                pvid: Some(1),
                frame_types: None,
                status: None,
                inactive: None,
                disabled: None,
                dynamic: None,
            }]),
            None,
            None,
        );
        let first = inventory(&details, &[], "seed", &host_of);

        // Every port carries its own parent label exactly once and the
        // self-membership lands on its own row: no traversal, no
        // duplication, no propagation.
        assert_eq!(
            first,
            inventory(&details, &[], "seed", &host_of),
            "deterministic"
        );
        assert_eq!(first.ports.len(), 3);
        let port = |name: &str| {
            first
                .ports
                .iter()
                .find(|port| port.name.as_deref() == Some(name))
                .unwrap_or_else(|| panic!("{name} is inventoried"))
        };
        assert_eq!(port("vlan-a").vlan_id, Some(10));
        assert_eq!(port("vlan-a").parent.as_deref(), Some("vlan-b"));
        assert_eq!(port("vlan-b").vlan_id, Some(20));
        assert_eq!(port("vlan-b").parent.as_deref(), Some("vlan-a"));
        let bridge = port("br1").membership.as_ref().expect("self-membership");
        assert_eq!(bridge.bridge.as_deref(), Some("br1"));
        assert_eq!(port("br1").parent, None);
        assert!(port("br1").vlans.is_empty(), "no VLAN rows, no membership");
    }
}
