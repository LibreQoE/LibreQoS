//! dudarchy poller library (vendored): the vault, credential profiles, scan
//! policy, discovery walk, SNMP and RouterOS transports, the identity/claim
//! model, IEEE OUI vendor hints, the port inventory, and the known-host store.
//!
//! Vendored from LibreQoE/dudarchy at commit `f941fbc`; see `README.md`.
//! The HTTP/auth server stack is intentionally absent.

pub mod adapters;
mod commands;
mod identity;
mod mikrotik;
mod mikrotik_proto;
mod mikrotik_transport;
mod net_policy;
pub mod oui;
pub mod ports;
pub mod prefix;
mod profiles;
mod snmp;
mod snmp_ber;
mod snmp_tables;
mod snmp_transport;
pub mod stored;
mod vault;
mod walk;
mod walk_source;

pub use identity::{
    BridgeHost, BridgePort, BridgeVlan, CapacityDerivation, CapacitySource, Claim, ClaimKind,
    Conflict, Device, DeviceDetails, DhcpLease, IfState, Interface, InterfaceKind, Mac,
    RadioDetails, Source, Sources, WirelessStation, resolve,
};
pub use mikrotik::{
    ApiNeighbour, ApiTarget, MAX_NEIGHBOUR_ROWS, MikrotikError, Password, RouterOsAddress,
    RouterOsInterface, command, resolve_api_target,
};
pub use net_policy::{AddressGate, ScanDecision};
pub use profiles::{Credential, Profile, ProfileError, Protocol, resolve_profiles};
pub use snmp::{
    AirStation, ArpRow, Community, IpAddressRow, LldpNeighbour, ResolvedTarget, SnmpError, SnmpFdb,
    SnmpInterface, SnmpVersion, WalkOutcome, WalkValue, WalkedVarbind, resolve_target,
    walk_subtree,
};
pub use vault::{Vault, VaultError, default_vault_path};
pub use walk::{
    ApiCredentials, CredentialCandidate, CredentialClass, CredentialOrigin, HookFuture,
    NeighbourSource, NetworkSource, NodeCredentials, ObservedLink, Probe, Probed, SnmpAccess,
    WalkBounds, WalkHooks, WalkNode, WalkProgress, WalkReport, walk_observing,
};
