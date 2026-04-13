use default_net::{get_default_interface, get_interfaces};
use lqos_config::Config;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const MANAGED_NETPLAN_PATH: &str = "/etc/netplan/libreqos.yaml";
const NETPLAN_DIR: &str = "/etc/netplan";
const PENDING_DIR: &str = "/var/lib/libreqos/netplan-pending";

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DetectedNetplanFile {
    pub path: String,
    #[serde(default)]
    pub relevant_interfaces: Vec<String>,
    pub classification: String,
    #[serde(default)]
    pub details: Vec<String>,
    pub compatible: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkModeInspection {
    pub mode_label: String,
    #[serde(default)]
    pub selected_interfaces: Vec<String>,
    pub inspector_state: String,
    pub summary: String,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub dangerous_changes: Vec<String>,
    #[serde(default)]
    pub conflicts: Vec<String>,
    pub editing_locked: bool,
    pub managed_file_path: String,
    pub managed_preview_yaml: Option<String>,
    pub preview_note: Option<String>,
    pub diff_preview: Option<String>,
    pub diff_preview_label: Option<String>,
    pub can_apply: bool,
    pub can_adopt: bool,
    pub can_take_over: bool,
    pub action_required: Option<String>,
    pub adopt_source_path: Option<String>,
    pub strong_confirmation_text: Option<String>,
    pub has_pending_try: bool,
    #[serde(default)]
    pub detected_files: Vec<DetectedNetplanFile>,
    #[serde(default)]
    pub interface_candidates: Vec<InterfaceCandidate>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct InterfaceCandidate {
    pub name: String,
    #[serde(default)]
    pub details: Vec<String>,
    pub bridge_eligible: bool,
    pub single_interface_eligible: bool,
    #[serde(default)]
    pub current_selection: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RequestedMode {
    LinuxBridge {
        to_internet: String,
        to_network: String,
    },
    XdpBridge {
        to_internet: String,
        to_network: String,
    },
    SingleInterface {
        interface: String,
    },
    Unknown,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct NetplanDocument {
    #[serde(default, skip_serializing_if = "NetplanNetwork::is_empty")]
    network: NetplanNetwork,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct NetplanNetwork {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    renderer: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    ethernets: BTreeMap<String, NetplanInterface>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    bridges: BTreeMap<String, NetplanBridge>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    bonds: BTreeMap<String, NetplanRelationship>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    vlans: BTreeMap<String, NetplanVlan>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct NetplanInterface {
    #[serde(
        default,
        deserialize_with = "deserialize_optional_netplan_bool",
        skip_serializing_if = "Option::is_none"
    )]
    dhcp4: Option<bool>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_netplan_bool",
        skip_serializing_if = "Option::is_none"
    )]
    dhcp6: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    addresses: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    gateway4: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    gateway6: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    routes: Vec<serde_yaml::Value>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct NetplanBridge {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    interfaces: Vec<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_netplan_bool",
        skip_serializing_if = "Option::is_none"
    )]
    dhcp4: Option<bool>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_netplan_bool",
        skip_serializing_if = "Option::is_none"
    )]
    dhcp6: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    addresses: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    gateway4: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    gateway6: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    routes: Vec<serde_yaml::Value>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct NetplanRelationship {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    interfaces: Vec<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct NetplanVlan {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    link: Option<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Clone, Debug, Default)]
struct FileAssessment {
    detected: DetectedNetplanFile,
    has_conflict: bool,
    is_complex: bool,
    is_managed_candidate: bool,
    is_external_candidate: bool,
    dangerous_changes: Vec<String>,
}

fn deserialize_optional_netplan_bool<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_yaml::Value>::deserialize(deserializer)?;
    let Some(value) = value else {
        return Ok(None);
    };

    match value {
        serde_yaml::Value::Bool(value) => Ok(Some(value)),
        serde_yaml::Value::String(raw) => {
            let normalized = raw.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "true" | "yes" | "on" => Ok(Some(true)),
                "false" | "no" | "off" => Ok(Some(false)),
                _ => Err(serde::de::Error::custom(format!(
                    "invalid netplan boolean value {raw:?}"
                ))),
            }
        }
        other => Err(serde::de::Error::custom(format!(
            "invalid netplan boolean type: {other:?}"
        ))),
    }
}

impl NetplanInterface {
    fn dhcp_disabled(&self) -> bool {
        self.dhcp4 == Some(false) && self.dhcp6 == Some(false)
    }

    fn has_l3_config(&self) -> bool {
        !self.addresses.is_empty()
            || self.gateway4.is_some()
            || self.gateway6.is_some()
            || !self.routes.is_empty()
            || self.dhcp4 == Some(true)
            || self.dhcp6 == Some(true)
    }
}

impl NetplanBridge {
    fn has_l3_config(&self) -> bool {
        !self.addresses.is_empty()
            || self.gateway4.is_some()
            || self.gateway6.is_some()
            || !self.routes.is_empty()
            || self.dhcp4 == Some(true)
            || self.dhcp6 == Some(true)
    }
}

impl NetplanNetwork {
    fn is_empty(&self) -> bool {
        self.version.is_none()
            && self.renderer.is_none()
            && self.ethernets.is_empty()
            && self.bridges.is_empty()
            && self.bonds.is_empty()
            && self.vlans.is_empty()
            && self.extra.is_empty()
    }
}

fn has_default_route(routes: &[serde_yaml::Value]) -> bool {
    routes.iter().any(|route| {
        route
            .as_mapping()
            .and_then(|mapping| mapping.get(serde_yaml::Value::String("to".to_string())))
            .and_then(serde_yaml::Value::as_str)
            .is_some_and(|to| to == "default" || to == "0.0.0.0/0" || to == "::/0")
    })
}

fn interface_management_risk(iface: &str, cfg: &NetplanInterface) -> Option<String> {
    if cfg.gateway4.is_some()
        || cfg.gateway6.is_some()
        || has_default_route(&cfg.routes)
        || cfg.dhcp4 == Some(true)
        || cfg.dhcp6 == Some(true)
    {
        Some(format!(
            "Selected interface {iface} appears to be on the current management/default-route path."
        ))
    } else if !cfg.addresses.is_empty() {
        Some(format!(
            "Selected interface {iface} already has static IP addressing in netplan."
        ))
    } else {
        None
    }
}

fn bridge_management_risk(bridge_name: &str, bridge: &NetplanBridge) -> Option<String> {
    if bridge.gateway4.is_some()
        || bridge.gateway6.is_some()
        || has_default_route(&bridge.routes)
        || bridge.dhcp4 == Some(true)
        || bridge.dhcp6 == Some(true)
    {
        Some(format!(
            "Bridge {bridge_name} appears to be on the current management/default-route path."
        ))
    } else if !bridge.addresses.is_empty() {
        Some(format!(
            "Bridge {bridge_name} already has static IP addressing in netplan."
        ))
    } else {
        None
    }
}

fn requested_mode(config: &Config) -> RequestedMode {
    if let Some(bridge) = &config.bridge {
        if bridge.use_xdp_bridge {
            RequestedMode::XdpBridge {
                to_internet: bridge.to_internet.trim().to_string(),
                to_network: bridge.to_network.trim().to_string(),
            }
        } else {
            RequestedMode::LinuxBridge {
                to_internet: bridge.to_internet.trim().to_string(),
                to_network: bridge.to_network.trim().to_string(),
            }
        }
    } else if let Some(single) = &config.single_interface {
        RequestedMode::SingleInterface {
            interface: single.interface.trim().to_string(),
        }
    } else {
        RequestedMode::Unknown
    }
}

fn mode_label(mode: &RequestedMode) -> String {
    match mode {
        RequestedMode::LinuxBridge { .. } => "Linux Bridge".to_string(),
        RequestedMode::XdpBridge { .. } => "XDP Bridge".to_string(),
        RequestedMode::SingleInterface { .. } => "Single Interface".to_string(),
        RequestedMode::Unknown => "Unconfigured".to_string(),
    }
}

fn selected_interfaces(mode: &RequestedMode) -> Vec<String> {
    match mode {
        RequestedMode::LinuxBridge {
            to_internet,
            to_network,
        }
        | RequestedMode::XdpBridge {
            to_internet,
            to_network,
        } => [to_internet.clone(), to_network.clone()]
            .into_iter()
            .filter(|iface| !iface.is_empty())
            .collect(),
        RequestedMode::SingleInterface { interface } => {
            if interface.is_empty() {
                Vec::new()
            } else {
                vec![interface.clone()]
            }
        }
        RequestedMode::Unknown => Vec::new(),
    }
}

fn managed_preview_yaml(mode: &RequestedMode) -> (Option<String>, Option<String>) {
    match mode {
        RequestedMode::LinuxBridge {
            to_internet,
            to_network,
        } if !to_internet.is_empty() && !to_network.is_empty() => (
            Some(format!(
                "network:\n  version: 2\n  ethernets:\n    {to_internet}:\n      dhcp4: false\n      dhcp6: false\n    {to_network}:\n      dhcp4: false\n      dhcp6: false\n  bridges:\n    br0:\n      interfaces:\n        - {to_internet}\n        - {to_network}\n"
            )),
            None,
        ),
        RequestedMode::SingleInterface { interface } if !interface.is_empty() => (
            Some(format!(
                "network:\n  version: 2\n  ethernets:\n    {interface}:\n      dhcp4: false\n      dhcp6: false\n"
            )),
            None,
        ),
        RequestedMode::XdpBridge { .. } => (
            None,
            Some(
                "XDP bridge mode remains a manual workflow. LibreQoS does not generate netplan for this mode."
                    .to_string(),
            ),
        ),
        RequestedMode::Unknown => (
            None,
            Some("LibreQoS does not have a complete network-mode configuration yet.".to_string()),
        ),
        _ => (
            None,
            Some("Select the required interfaces to generate a managed netplan preview.".to_string()),
        ),
    }
}

fn system_interfaces() -> BTreeSet<String> {
    get_interfaces()
        .into_iter()
        .map(|iface| iface.name)
        .collect()
}

fn supports_multi_queue(interface: &str) -> bool {
    let path = Path::new("/sys/class/net").join(interface).join("queues");
    let Ok(entries) = fs::read_dir(path) else {
        return false;
    };

    let mut rx = 0usize;
    let mut tx = 0usize;
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
            continue;
        };
        if name.starts_with("rx-") {
            rx += 1;
        } else if name.starts_with("tx-") {
            tx += 1;
        }
    }

    rx > 1 && tx > 1
}

fn parse_netplan_file(path: &Path) -> Result<NetplanDocument, String> {
    let raw = fs::read_to_string(path)
        .map_err(|err| format!("Unable to read {}: {err}", path.display()))?;
    serde_yaml::from_str::<NetplanDocument>(&raw)
        .map_err(|err| format!("Unable to parse {}: {err}", path.display()))
}

fn pending_try_exists(path: &Path) -> bool {
    let Ok(entries) = fs::read_dir(path) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry
            .file_name()
            .to_str()
            .is_some_and(|name| !name.starts_with('.'))
    })
}

fn add_interface_reason(
    restrictions: &mut BTreeMap<String, BTreeSet<String>>,
    iface: &str,
    reason: impl Into<String>,
) {
    restrictions
        .entry(iface.to_string())
        .or_default()
        .insert(reason.into());
}

fn collect_interface_restrictions(
    doc: &NetplanDocument,
    restrictions: &mut BTreeMap<String, BTreeSet<String>>,
) {
    for (iface, cfg) in &doc.network.ethernets {
        if cfg.has_l3_config() {
            add_interface_reason(
                restrictions,
                iface,
                "Carries DHCP, static addressing, or routes in current netplan.",
            );
        }
    }

    for (bridge_name, bridge) in &doc.network.bridges {
        if bridge.has_l3_config() {
            for member in &bridge.interfaces {
                add_interface_reason(
                    restrictions,
                    member,
                    format!(
                        "Member of bridge {bridge_name}, which carries DHCP, static addressing, or routes."
                    ),
                );
            }
        }
    }
}

fn interface_name_role_reason(iface: &str) -> Option<&'static str> {
    if iface == "lo" {
        Some("Loopback interface.")
    } else if iface.contains('.') || iface.contains(':') {
        Some("VLAN or alias interface.")
    } else if [
        "br",
        "bond",
        "docker",
        "veth",
        "virbr",
        "ifb",
        "wg",
        "tun",
        "tap",
        "tailscale",
        "zt",
        "vmnet",
    ]
    .iter()
    .any(|prefix| iface.starts_with(prefix))
    {
        Some("Virtual, bridge, or tunnel interface.")
    } else {
        None
    }
}

fn build_interface_candidates(
    system_ifaces: &BTreeSet<String>,
    queue_caps: &BTreeMap<String, bool>,
    selected: &[String],
    default_interface_name: Option<&str>,
    interface_restrictions: &BTreeMap<String, BTreeSet<String>>,
) -> Vec<InterfaceCandidate> {
    let selected_set = selected.iter().cloned().collect::<BTreeSet<_>>();
    let mut candidates = system_ifaces
        .iter()
        .map(|iface| {
            let mut details = Vec::new();

            if let Some(reason) = interface_name_role_reason(iface) {
                details.push(reason.to_string());
            }
            if default_interface_name.is_some_and(|default_iface| default_iface == iface) {
                details.push("Current management/default-route interface.".to_string());
            }
            if !queue_caps.get(iface).copied().unwrap_or(false) {
                details.push("Does not appear to have multi-queue RX/TX support.".to_string());
            }
            if let Some(extra) = interface_restrictions.get(iface) {
                details.extend(extra.iter().cloned());
            }
            details.sort();
            details.dedup();

            let eligible = details.is_empty();
            InterfaceCandidate {
                name: iface.clone(),
                details,
                bridge_eligible: eligible,
                single_interface_eligible: eligible,
                current_selection: selected_set.contains(iface),
            }
        })
        .collect::<Vec<_>>();

    candidates.sort_by(|left, right| {
        right
            .bridge_eligible
            .cmp(&left.bridge_eligible)
            .then_with(|| left.name.cmp(&right.name))
    });
    candidates
}

fn read_text_if_exists(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

fn simple_diff(old_text: &str, new_text: &str) -> String {
    let old_lines: Vec<&str> = old_text.lines().collect();
    let new_lines: Vec<&str> = new_text.lines().collect();
    let max_len = old_lines.len().max(new_lines.len());
    let mut output = String::new();
    for idx in 0..max_len {
        match (old_lines.get(idx), new_lines.get(idx)) {
            (Some(old_line), Some(new_line)) if old_line == new_line => {
                output.push(' ');
                output.push_str(old_line);
                output.push('\n');
            }
            (Some(old_line), Some(new_line)) => {
                output.push('-');
                output.push_str(old_line);
                output.push('\n');
                output.push('+');
                output.push_str(new_line);
                output.push('\n');
            }
            (Some(old_line), None) => {
                output.push('-');
                output.push_str(old_line);
                output.push('\n');
            }
            (None, Some(new_line)) => {
                output.push('+');
                output.push_str(new_line);
                output.push('\n');
            }
            (None, None) => {}
        }
    }
    output
}

pub(crate) fn adoption_rewrite_for_path(path: &Path, config: &Config) -> Result<String, String> {
    let mut doc = parse_netplan_file(path)?;
    let mode = requested_mode(config);

    match mode {
        RequestedMode::LinuxBridge {
            to_internet,
            to_network,
        } => {
            let mut removed_bridge = false;
            doc.network.ethernets.remove(&to_internet);
            doc.network.ethernets.remove(&to_network);
            let selected = BTreeSet::from([to_internet.clone(), to_network.clone()]);
            doc.network.bridges.retain(|_, bridge| {
                let members = bridge.interfaces.iter().cloned().collect::<BTreeSet<_>>();
                let keep = members != selected;
                if !keep {
                    removed_bridge = true;
                }
                keep
            });
            if !removed_bridge {
                return Err(format!(
                    "Unable to find the matching bridge in {} for adoption.",
                    path.display()
                ));
            }
        }
        RequestedMode::SingleInterface { interface } => {
            if doc.network.ethernets.remove(&interface).is_none() {
                return Err(format!(
                    "Unable to find interface {interface} in {} for adoption.",
                    path.display()
                ));
            }
        }
        RequestedMode::XdpBridge { .. } | RequestedMode::Unknown => {
            return Err(
                "Adoption is only supported for managed Linux bridge and single-interface modes."
                    .to_string(),
            );
        }
    }

    if doc.network.version.is_none() {
        doc.network.version = Some(2);
    }

    serde_yaml::to_string(&doc).map_err(|err| {
        format!(
            "Unable to serialize rewritten netplan {}: {err}",
            path.display()
        )
    })
}

fn assess_file(path: &Path, doc: &NetplanDocument, mode: &RequestedMode) -> FileAssessment {
    let mut details = Vec::new();
    let mut relevant = BTreeSet::new();
    let mut has_conflict = false;
    let mut is_complex = false;
    let mut is_external_candidate = false;
    let mut dangerous_changes = Vec::new();

    match mode {
        RequestedMode::LinuxBridge {
            to_internet,
            to_network,
        } => {
            for iface in [to_internet, to_network] {
                if let Some(cfg) = doc.network.ethernets.get(iface) {
                    relevant.insert(iface.clone());
                    if cfg.dhcp_disabled() {
                        details.push(format!("{iface} has dhcp4/dhcp6 disabled."));
                    }
                    if cfg.has_l3_config() {
                        details.push(format!(
                            "{iface} carries DHCP, static addressing, or routes in this file."
                        ));
                        if let Some(risk) = interface_management_risk(iface, cfg) {
                            dangerous_changes.push(risk);
                        }
                    }
                }
            }

            for (bridge_name, bridge) in &doc.network.bridges {
                let members: BTreeSet<_> = bridge.interfaces.iter().cloned().collect();
                let touches_selected =
                    members.contains(to_internet.as_str()) || members.contains(to_network.as_str());
                if !touches_selected {
                    continue;
                }

                relevant.insert(to_internet.clone());
                relevant.insert(to_network.clone());
                details.push(format!(
                    "Bridge {bridge_name} contains interfaces: {}.",
                    bridge.interfaces.join(", ")
                ));

                if members.contains(to_internet.as_str())
                    && members.contains(to_network.as_str())
                    && bridge.interfaces.len() == 2
                    && let (Some(internet_cfg), Some(network_cfg)) = (
                        doc.network.ethernets.get(to_internet),
                        doc.network.ethernets.get(to_network),
                    )
                    && internet_cfg.dhcp_disabled()
                    && network_cfg.dhcp_disabled()
                    && !internet_cfg.has_l3_config()
                    && !network_cfg.has_l3_config()
                {
                    details.push(format!(
                        "Bridge {bridge_name} matches the selected Linux bridge interfaces."
                    ));
                    is_external_candidate = true;
                } else {
                    details.push(format!(
                        "Selected shaping interfaces are already part of bridge {bridge_name}, but not in the expected two-interface layout."
                    ));
                    has_conflict = true;
                }

                if bridge.has_l3_config() {
                    details.push(format!(
                        "Bridge {bridge_name} also carries DHCP, addresses, or routes."
                    ));
                    if let Some(risk) = bridge_management_risk(bridge_name, bridge) {
                        dangerous_changes.push(risk);
                    }
                }
            }
        }
        RequestedMode::SingleInterface { interface } => {
            if let Some(cfg) = doc.network.ethernets.get(interface) {
                relevant.insert(interface.clone());
                if cfg.dhcp_disabled() && !cfg.has_l3_config() {
                    details.push(format!(
                        "{interface} already has dhcp4/dhcp6 disabled with no extra addressing."
                    ));
                    is_external_candidate = true;
                }
                if cfg.has_l3_config() {
                    details.push(format!(
                        "{interface} carries DHCP, static addressing, or routes in this file."
                    ));
                    if let Some(risk) = interface_management_risk(interface, cfg) {
                        dangerous_changes.push(risk);
                    }
                }
            }

            for (bridge_name, bridge) in &doc.network.bridges {
                if bridge.interfaces.iter().any(|member| member == interface) {
                    relevant.insert(interface.clone());
                    details.push(format!(
                        "{interface} is already a member of bridge {bridge_name}."
                    ));
                    has_conflict = true;
                }
            }
        }
        RequestedMode::XdpBridge {
            to_internet,
            to_network,
        } => {
            for iface in [to_internet, to_network] {
                if doc.network.ethernets.contains_key(iface) {
                    relevant.insert(iface.clone());
                }
            }
            for (bridge_name, bridge) in &doc.network.bridges {
                let members: BTreeSet<_> = bridge.interfaces.iter().cloned().collect();
                if members.contains(to_internet.as_str()) || members.contains(to_network.as_str()) {
                    relevant.insert(to_internet.clone());
                    relevant.insert(to_network.clone());
                    details.push(format!(
                        "Bridge {bridge_name} already contains one or more selected interfaces."
                    ));
                }
            }
        }
        RequestedMode::Unknown => {}
    }

    for (bond_name, bond) in &doc.network.bonds {
        for iface in selected_interfaces(mode) {
            if bond.interfaces.iter().any(|member| member == &iface) {
                relevant.insert(iface.clone());
                details.push(format!("{iface} is already part of bond {bond_name}."));
                has_conflict = true;
                is_complex = true;
            }
        }
    }

    for (vlan_name, vlan) in &doc.network.vlans {
        for iface in selected_interfaces(mode) {
            if vlan.link.as_deref() == Some(iface.as_str()) {
                relevant.insert(iface.clone());
                details.push(format!(
                    "{iface} is already the lower interface for VLAN {vlan_name}."
                ));
                has_conflict = true;
                is_complex = true;
            }
        }
    }

    let classification = if is_complex {
        "ComplexUnsupported"
    } else if path.ends_with("libreqos.yaml") && is_external_candidate {
        "ManagedByLibreQoS"
    } else if is_external_candidate {
        "ExternalCompatible"
    } else if has_conflict {
        "Conflict"
    } else if relevant.is_empty() {
        "Unrelated"
    } else {
        "Relevant"
    };

    FileAssessment {
        detected: DetectedNetplanFile {
            path: path.display().to_string(),
            relevant_interfaces: relevant.into_iter().collect(),
            classification: classification.to_string(),
            details,
            compatible: is_external_candidate,
        },
        has_conflict,
        is_complex,
        is_managed_candidate: path.ends_with("libreqos.yaml") && is_external_candidate,
        is_external_candidate: !path.ends_with("libreqos.yaml") && is_external_candidate,
        dangerous_changes,
    }
}

pub fn inspect_network_mode_with_paths(
    config: &Config,
    netplan_dir: &Path,
    pending_dir: &Path,
    system_ifaces: &BTreeSet<String>,
    queue_caps: &BTreeMap<String, bool>,
) -> NetworkModeInspection {
    let mode = requested_mode(config);
    let selected = selected_interfaces(&mode);
    let (preview, preview_note) = managed_preview_yaml(&mode);
    let mut warnings = Vec::new();
    let mut dangerous_changes = Vec::new();
    let mut conflicts = Vec::new();
    let mut detected_files = Vec::new();
    let mut has_complex = false;
    let mut has_managed = false;
    let mut has_external = false;
    let mut external_sources = Vec::new();
    let mut takeover_candidate = false;
    let has_pending_try = pending_try_exists(pending_dir);
    let default_interface_name = get_default_interface().ok().map(|iface| iface.name);
    let mut interface_restrictions = BTreeMap::<String, BTreeSet<String>>::new();

    if selected.is_empty() {
        warnings.push(
            "Select the required interfaces before applying managed bridge checks.".to_string(),
        );
    }

    let mut missing = Vec::new();
    for iface in &selected {
        if !system_ifaces.contains(iface) {
            missing.push(iface.clone());
        } else if !queue_caps.get(iface).copied().unwrap_or(false) {
            warnings.push(format!(
                "Interface {iface} does not appear to have multi-queue RX/TX support."
            ));
        }
    }

    let mut entries: Vec<PathBuf> = match fs::read_dir(netplan_dir) {
        Ok(read_dir) => read_dir.flatten().map(|entry| entry.path()).collect(),
        Err(err) => {
            warnings.push(format!("Unable to read {}: {err}", netplan_dir.display()));
            Vec::new()
        }
    };
    entries.sort();

    for path in entries {
        if path.extension().and_then(|ext| ext.to_str()) != Some("yaml") {
            continue;
        }

        match parse_netplan_file(&path) {
            Ok(doc) => {
                collect_interface_restrictions(&doc, &mut interface_restrictions);
                let assessment = assess_file(&path, &doc, &mode);
                if assessment.has_conflict {
                    conflicts.extend(assessment.detected.details.clone());
                }
                dangerous_changes.extend(assessment.dangerous_changes.clone());
                if assessment.is_complex {
                    has_complex = true;
                }
                if assessment.is_managed_candidate {
                    has_managed = true;
                }
                if assessment.is_external_candidate {
                    has_external = true;
                    external_sources.push(path.display().to_string());
                }
                if path.ends_with("libreqos.yaml")
                    && assessment.detected.classification != "ManagedByLibreQoS"
                    && assessment.detected.classification != "Unrelated"
                {
                    takeover_candidate = true;
                }
                if assessment.detected.classification != "Unrelated" {
                    detected_files.push(assessment.detected);
                }
            }
            Err(err) => {
                has_complex = true;
                detected_files.push(DetectedNetplanFile {
                    path: path.display().to_string(),
                    relevant_interfaces: Vec::new(),
                    classification: "ComplexUnsupported".to_string(),
                    details: vec![err],
                    compatible: false,
                });
            }
        }
    }

    dangerous_changes.sort();
    dangerous_changes.dedup();

    let can_take_over = takeover_candidate && !has_pending_try;
    let can_adopt = external_sources.len() == 1 && !has_complex && !has_pending_try;
    if external_sources.len() > 1 {
        warnings.push(
            "Multiple external netplan files match the selected interfaces. Adoption stays locked until the config is simplified."
                .to_string(),
        );
    }

    let action_required = if can_take_over {
        Some("TakeOver".to_string())
    } else if has_external {
        Some("Adopt".to_string())
    } else {
        None
    };

    let diff_preview_label;
    let diff_preview = if let Some(preview_yaml) = preview.as_ref() {
        if let Some(source_path) = external_sources.first() {
            let path = PathBuf::from(source_path);
            diff_preview_label = Some(format!("{source_path} -> {MANAGED_NETPLAN_PATH}"));
            Some(simple_diff(
                &read_text_if_exists(&path).unwrap_or_default(),
                preview_yaml,
            ))
        } else {
            let path = PathBuf::from(MANAGED_NETPLAN_PATH);
            diff_preview_label = Some(format!("{MANAGED_NETPLAN_PATH} -> managed preview"));
            Some(simple_diff(
                &read_text_if_exists(&path).unwrap_or_default(),
                preview_yaml,
            ))
        }
    } else {
        diff_preview_label = None;
        None
    };

    let (inspector_state, summary, editing_locked) = if has_pending_try {
        (
            "PendingTry",
            "A pending network change was detected. Confirm or revert that change before editing network mode.".to_string(),
            true,
        )
    } else if !missing.is_empty() {
        (
            "Missing",
            format!(
                "Selected interface{} {} {} not present on this system.",
                if missing.len() == 1 { "" } else { "s" },
                missing.join(", "),
                if missing.len() == 1 { "is" } else { "are" }
            ),
            false,
        )
    } else if has_managed {
        (
            "ManagedByLibreQoS",
            format!(
                "{} is already defined in {}.",
                mode_label(&mode),
                MANAGED_NETPLAN_PATH
            ),
            false,
        )
    } else if can_take_over {
        (
            "Conflict",
            format!(
                "{} already exists but does not look LibreQoS-managed for the selected interfaces. Review the diff and use Take Over to proceed.",
                MANAGED_NETPLAN_PATH
            ),
            true,
        )
    } else if has_external {
        (
            "ExternalCompatible",
            "A compatible external netplan configuration already matches the selected interfaces. Review the diff and use Adopt into libreqos.yaml if you want LibreQoS to manage it.".to_string(),
            true,
        )
    } else if has_complex {
        (
            "ComplexUnsupported",
            "Detected netplan relationships are too complex for safe automatic management."
                .to_string(),
            true,
        )
    } else if !conflicts.is_empty() {
        (
            "Conflict",
            "Selected interfaces already have conflicting DHCP, static addressing, or bridge membership in netplan.".to_string(),
            false,
        )
    } else {
        (
            "Ready",
            "No blocking netplan conflicts were detected for the selected mode. Review the managed preview before applying changes.".to_string(),
            false,
        )
    };

    if matches!(mode, RequestedMode::XdpBridge { .. }) {
        warnings.push(
            "XDP bridge mode remains manual. LibreQoS only provides inspection for this mode in the current helper slice."
                .to_string(),
        );
    }

    let can_apply = !editing_locked
        && !matches!(
            mode,
            RequestedMode::XdpBridge { .. } | RequestedMode::Unknown
        )
        && !has_pending_try
        && preview.is_some();
    let strong_confirmation_text = if dangerous_changes.is_empty() {
        None
    } else {
        Some(
            "This change may interrupt access to this system. LibreQoS will automatically roll back if you do not confirm within 30 seconds."
                .to_string(),
        )
    };
    let interface_candidates = build_interface_candidates(
        system_ifaces,
        queue_caps,
        &selected,
        default_interface_name.as_deref(),
        &interface_restrictions,
    );

    NetworkModeInspection {
        mode_label: mode_label(&mode),
        selected_interfaces: selected,
        inspector_state: inspector_state.to_string(),
        summary,
        warnings,
        dangerous_changes,
        conflicts,
        editing_locked,
        managed_file_path: MANAGED_NETPLAN_PATH.to_string(),
        managed_preview_yaml: preview,
        preview_note,
        diff_preview,
        diff_preview_label,
        can_apply,
        can_adopt,
        can_take_over,
        action_required,
        adopt_source_path: external_sources.first().cloned(),
        strong_confirmation_text,
        has_pending_try,
        detected_files,
        interface_candidates,
    }
}

pub fn inspect_network_mode(config: &Config) -> NetworkModeInspection {
    let system_ifaces = system_interfaces();
    let queue_caps = system_ifaces
        .iter()
        .map(|iface| (iface.clone(), supports_multi_queue(iface)))
        .collect::<BTreeMap<_, _>>();

    inspect_network_mode_with_paths(
        config,
        Path::new(NETPLAN_DIR),
        Path::new(PENDING_DIR),
        &system_ifaces,
        &queue_caps,
    )
}

#[cfg(test)]
mod tests {
    use super::{adoption_rewrite_for_path, inspect_network_mode_with_paths};
    use std::collections::{BTreeMap, BTreeSet};

    fn linux_bridge_config() -> lqos_config::Config {
        lqos_config::Config {
            bridge: Some(lqos_config::BridgeConfig {
                use_xdp_bridge: false,
                to_internet: "ens19".to_string(),
                to_network: "ens20".to_string(),
            }),
            single_interface: None,
            ..lqos_config::Config::default()
        }
    }

    fn single_interface_config() -> lqos_config::Config {
        lqos_config::Config {
            bridge: None,
            single_interface: Some(lqos_config::SingleInterfaceConfig {
                interface: "ens19".to_string(),
                internet_vlan: 2,
                network_vlan: 3,
            }),
            ..lqos_config::Config::default()
        }
    }

    fn test_env(name: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("lqos-netplan-helper-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("netplan")).expect("create test netplan dir");
        std::fs::create_dir_all(root.join("pending")).expect("create test pending dir");
        root
    }

    fn queue_caps() -> BTreeMap<String, bool> {
        BTreeMap::from([("ens19".to_string(), true), ("ens20".to_string(), true)])
    }

    #[test]
    fn detects_external_compatible_linux_bridge() {
        let root = test_env("external-compatible");
        std::fs::write(
            root.join("netplan").join("50-cloud-init.yaml"),
            r#"
network:
  version: 2
  ethernets:
    ens19:
      dhcp4: false
      dhcp6: false
    ens20:
      dhcp4: false
      dhcp6: false
  bridges:
    br-test:
      interfaces:
        - ens19
        - ens20
"#,
        )
        .expect("write external compatible netplan fixture");

        let inspection = inspect_network_mode_with_paths(
            &linux_bridge_config(),
            &root.join("netplan"),
            &root.join("pending"),
            &BTreeSet::from(["ens19".to_string(), "ens20".to_string()]),
            &queue_caps(),
        );

        assert_eq!(inspection.inspector_state, "ExternalCompatible");
        assert!(inspection.editing_locked);
        assert_eq!(inspection.detected_files.len(), 1);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn detects_documented_libreqos_bridge_with_no_booleans() {
        let root = test_env("managed-no-booleans");
        std::fs::write(
            root.join("netplan").join("libreqos.yaml"),
            r#"
network:
  version: 2
  ethernets:
    ens19:
      dhcp4: no
      dhcp6: no
    ens20:
      dhcp4: no
      dhcp6: no
  bridges:
    br0:
      interfaces:
        - ens19
        - ens20
"#,
        )
        .expect("write managed netplan fixture");

        let inspection = inspect_network_mode_with_paths(
            &linux_bridge_config(),
            &root.join("netplan"),
            &root.join("pending"),
            &BTreeSet::from(["ens19".to_string(), "ens20".to_string()]),
            &queue_caps(),
        );

        assert_eq!(inspection.inspector_state, "ManagedByLibreQoS");
        assert!(!inspection.editing_locked);
        assert_eq!(inspection.detected_files.len(), 1);
        assert_eq!(
            inspection.detected_files[0].classification,
            "ManagedByLibreQoS"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn detects_dangerous_interface_addressing() {
        let root = test_env("conflict");
        std::fs::write(
            root.join("netplan").join("50-cloud-init.yaml"),
            r#"
network:
  version: 2
  ethernets:
    ens19:
      dhcp4: true
"#,
        )
        .expect("write conflict netplan fixture");

        let inspection = inspect_network_mode_with_paths(
            &single_interface_config(),
            &root.join("netplan"),
            &root.join("pending"),
            &BTreeSet::from(["ens19".to_string()]),
            &BTreeMap::from([("ens19".to_string(), true)]),
        );

        assert_eq!(inspection.inspector_state, "Ready");
        assert!(inspection.conflicts.is_empty());
        assert!(!inspection.dangerous_changes.is_empty());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn reports_management_risk_as_dangerous_not_conflict() {
        let root = test_env("management-risk");
        std::fs::write(
            root.join("netplan").join("50-cloud-init.yaml"),
            r#"
network:
  version: 2
  ethernets:
    ens19:
      dhcp4: true
      routes:
        - to: default
          via: 192.0.2.1
"#,
        )
        .expect("write management-risk fixture");

        let inspection = inspect_network_mode_with_paths(
            &single_interface_config(),
            &root.join("netplan"),
            &root.join("pending"),
            &BTreeSet::from(["ens19".to_string()]),
            &BTreeMap::from([("ens19".to_string(), true)]),
        );

        assert_eq!(inspection.inspector_state, "Ready");
        assert!(inspection.conflicts.is_empty());
        assert!(!inspection.dangerous_changes.is_empty());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn adoption_rewrite_removes_selected_bridge_members() {
        let root = test_env("adoption-rewrite");
        let path = root.join("netplan").join("50-cloud-init.yaml");
        std::fs::write(
            &path,
            r#"
network:
  version: 2
  ethernets:
    ens19:
      dhcp4: false
      dhcp6: false
    ens20:
      dhcp4: false
      dhcp6: false
    ens30:
      dhcp4: true
  bridges:
    br-test:
      interfaces:
        - ens19
        - ens20
"#,
        )
        .expect("write adoption fixture");

        let rewritten = adoption_rewrite_for_path(&path, &linux_bridge_config())
            .expect("rewrite should succeed");
        assert!(!rewritten.contains("ens19"));
        assert!(!rewritten.contains("ens20"));
        assert!(rewritten.contains("ens30"));
        assert!(!rewritten.contains(": null"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn adoption_rewrite_omits_null_optional_fields() {
        let root = test_env("adoption-no-nulls");
        let path = root.join("netplan").join("50-cloud-init.yaml");
        std::fs::write(
            &path,
            r#"
network:
  version: 2
  ethernets:
    ens19:
      dhcp4: no
      dhcp6: no
    ens20:
      dhcp4: no
      dhcp6: no
    ens30:
      addresses:
        - 192.0.2.10/24
"#,
        )
        .expect("write adoption null fixture");

        let rewritten = adoption_rewrite_for_path(&path, &single_interface_config())
            .expect("rewrite should succeed");
        assert!(!rewritten.contains(": null"));
        assert!(!rewritten.contains("renderer: null"));
        assert!(!rewritten.contains("gateway4: null"));
        assert!(rewritten.contains("ens30"));

        let _ = std::fs::remove_dir_all(root);
    }
}
