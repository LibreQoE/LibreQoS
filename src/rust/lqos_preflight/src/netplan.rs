//! The netplan-bridge adoption arm (the v2→v3 upgrade card §2a).
//!
//! Most v2 installs define the two dataplane NICs as members of a Linux bridge
//! in `/etc/netplan/*.yaml`. The v3 dataplane wants those NICs for DPDK, so the
//! bridge must be adopted (removed from the netplan the operator flow rewrites)
//! before binding, or the post-upgrade reboot can race networkd for members
//! DPDK owns. This module DETECTS and DISCLOSES only — pure file reads, no
//! writes, no daemon (card §4: no auto-adoption).
//!
//! The dataplane pair names come from the host config (`/etc/lqos.conf`); the
//! caller supplies them so the pure core stays testable.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_yaml::Value;

/// The default netplan directory.
pub const NETPLAN_DIR: &str = "/etc/netplan";

/// One bridge found in a netplan file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetplanBridge {
    /// The bridge name (the `bridges:` key).
    pub name: String,
    /// The file the bridge was defined in.
    pub file: PathBuf,
    /// The member interfaces (`interfaces:`).
    pub members: Vec<String>,
    /// True when the bridge (or a dataplane member) carries L3 config —
    /// management likely rides it and a remote reboot is the danger.
    pub management_at_risk: bool,
}

/// The scan result: every bridge, plus the subset conflicting with the pair.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetplanReport {
    /// The dataplane pair the scan was run against.
    pub pair: Vec<String>,
    /// Every bridge found across the netplan directory.
    pub bridges: Vec<NetplanBridge>,
    /// Bridges whose members intersect the dataplane pair.
    pub conflicting: Vec<NetplanBridge>,
    /// True when a conflicting bridge (or a pair member) carries management.
    pub management_at_risk: bool,
}

impl NetplanReport {
    /// True when at least one bridge enslaves a dataplane interface.
    pub fn has_conflict(&self) -> bool {
        !self.conflicting.is_empty()
    }
}

/// Extract the dataplane pair names from the host config (`bridge` or
/// `single_interface`).
pub fn pair_from_config(config: &lqos_config::Config) -> Vec<String> {
    if let Some(bridge) = &config.bridge {
        let wan = bridge.to_internet.trim();
        let lan = bridge.to_network.trim();
        let mut pair: Vec<String> = Vec::new();
        for name in [wan, lan] {
            if !name.is_empty() && !pair.iter().any(|p| p == name) {
                pair.push(name.to_string());
            }
        }
        return pair;
    }
    if let Some(single) = &config.single_interface {
        let name = single.interface.trim();
        if !name.is_empty() {
            return vec![name.to_string()];
        }
    }
    Vec::new()
}

/// On-a-stick from the typed `bridge.conf` JSON: `interface_topology.mode` is
/// exactly `"stick"`. Unknown/malformed JSON is not a stick.
pub fn on_a_stick_bridge_conf(body: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("interface_topology").cloned())
        .and_then(|topology| topology.get("mode").and_then(|m| m.as_str()).map(str::to_owned))
        .is_some_and(|mode| mode == "stick")
}

/// Scan the default netplan directory for bridges over `pair`.
pub fn check(pair: &[String]) -> NetplanReport {
    check_at(Path::new(NETPLAN_DIR), pair)
}

/// The testable surface: scan `dir` for bridges over `pair`.
pub fn check_at(dir: &Path, pair: &[String]) -> NetplanReport {
    let pair: Vec<String> = pair
        .iter()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect();

    let mut report = NetplanReport {
        pair: pair.clone(),
        ..Default::default()
    };

    let Ok(entries) = std::fs::read_dir(dir) else {
        return report;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_yaml = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml"));
        if !is_yaml {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(document) = serde_yaml::from_str::<Document>(&text) else {
            continue;
        };
        for (name, bridge) in document.network.bridges {
            let mut members = bridge.interfaces.clone();
            let member_mgmt = members.iter().any(|member| {
                pair.contains(member)
                    && document
                        .network
                        .ethernets
                        .get(member)
                        .is_some_and(interface_management)
            });
            let management_at_risk = bridge_management(&bridge) || member_mgmt;
            let intersects = members.iter().any(|member| pair.contains(member));
            members.shrink_to_fit();
            let entry = NetplanBridge {
                name,
                file: path.clone(),
                members,
                management_at_risk,
            };
            if intersects {
                report.management_at_risk |= management_at_risk;
                report.conflicting.push(entry.clone());
            }
            report.bridges.push(entry);
        }
    }
    report
}

#[derive(Debug, Default, Deserialize)]
struct Document {
    #[serde(default)]
    network: Network,
}

#[derive(Debug, Default, Deserialize)]
struct Network {
    #[serde(default)]
    ethernets: BTreeMap<String, Interface>,
    #[serde(default)]
    bridges: BTreeMap<String, Bridge>,
}

#[derive(Debug, Default, Deserialize)]
struct Interface {
    #[serde(default, deserialize_with = "de_opt_bool")]
    dhcp4: Option<bool>,
    #[serde(default, deserialize_with = "de_opt_bool")]
    dhcp6: Option<bool>,
    #[serde(default)]
    addresses: Vec<String>,
    #[serde(default)]
    gateway4: Option<String>,
    #[serde(default)]
    gateway6: Option<String>,
    #[serde(default)]
    routes: Vec<Value>,
}

#[derive(Debug, Default, Deserialize)]
struct Bridge {
    #[serde(default)]
    interfaces: Vec<String>,
    #[serde(default, deserialize_with = "de_opt_bool")]
    dhcp4: Option<bool>,
    #[serde(default, deserialize_with = "de_opt_bool")]
    dhcp6: Option<bool>,
    #[serde(default)]
    addresses: Vec<String>,
    #[serde(default)]
    gateway4: Option<String>,
    #[serde(default)]
    gateway6: Option<String>,
    #[serde(default)]
    routes: Vec<Value>,
}

fn interface_management(iface: &Interface) -> bool {
    l3_management(
        iface.gateway4.is_some(),
        iface.gateway6.is_some(),
        &iface.routes,
        iface.dhcp4 == Some(true) || iface.dhcp6 == Some(true),
        !iface.addresses.is_empty(),
    )
}

fn bridge_management(bridge: &Bridge) -> bool {
    l3_management(
        bridge.gateway4.is_some(),
        bridge.gateway6.is_some(),
        &bridge.routes,
        bridge.dhcp4 == Some(true) || bridge.dhcp6 == Some(true),
        !bridge.addresses.is_empty(),
    )
}

fn l3_management(
    gateway4: bool,
    gateway6: bool,
    routes: &[Value],
    dhcp: bool,
    has_address: bool,
) -> bool {
    gateway4 || gateway6 || has_default_route(routes) || dhcp || has_address
}

fn has_default_route(routes: &[Value]) -> bool {
    routes.iter().any(|route| {
        route
            .as_mapping()
            .and_then(|mapping| mapping.get(Value::String("to".to_string())))
            .and_then(Value::as_str)
            .is_some_and(|to| to == "default" || to == "0.0.0.0/0" || to == "::/0")
    })
}

/// netplan accepts booleans as YAML bools or as `true/false/yes/no/on/off`
/// strings; mirror the helper's lenient parse.
fn de_opt_bool<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    let Some(value) = value else {
        return Ok(None);
    };
    match value {
        Value::Bool(value) => Ok(Some(value)),
        Value::String(raw) => match raw.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" => Ok(Some(true)),
            "false" | "no" | "off" => Ok(Some(false)),
            _ => Err(serde::de::Error::custom(format!(
                "invalid netplan boolean value {raw:?}"
            ))),
        },
        other => Err(serde::de::Error::custom(format!(
            "invalid netplan boolean type: {other:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "lqos_preflight_netplan_{}_{}",
                name,
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn write(&self, file: &str, text: &str) {
            std::fs::write(self.0.join(file), text).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn pair() -> Vec<String> {
        vec!["enp1s0f0".to_string(), "enp1s0f1".to_string()]
    }

    #[test]
    fn a_bridge_over_the_pair_conflicts() {
        let dir = Scratch::new("over_pair");
        dir.write(
            "50-cloud-init.yaml",
            "network:\n  version: 2\n  bridges:\n    br0:\n      interfaces:\n        - enp1s0f0\n        - enp1s0f1\n",
        );
        let report = check_at(&dir.0, &pair());
        assert!(report.has_conflict());
        assert_eq!(report.conflicting[0].name, "br0");
        assert!(!report.management_at_risk);
    }

    #[test]
    fn a_management_bridge_raises_the_louder_class() {
        let dir = Scratch::new("mgmt_bridge");
        dir.write(
            "50-cloud-init.yaml",
            "network:\n  version: 2\n  bridges:\n    br0:\n      interfaces: [enp1s0f0, enp1s0f1]\n      addresses: [10.0.0.2/24]\n      gateway4: 10.0.0.1\n",
        );
        let report = check_at(&dir.0, &pair());
        assert!(report.has_conflict());
        assert!(report.management_at_risk);
    }

    #[test]
    fn management_on_a_member_also_raises_it() {
        let dir = Scratch::new("mgmt_member");
        dir.write(
            "50-cloud-init.yaml",
            "network:\n  version: 2\n  ethernets:\n    enp1s0f0:\n      dhcp4: true\n  bridges:\n    br0:\n      interfaces: [enp1s0f0, enp1s0f1]\n",
        );
        assert!(check_at(&dir.0, &pair()).management_at_risk);
    }

    #[test]
    fn an_unrelated_bridge_is_reported_but_not_conflicting() {
        let dir = Scratch::new("unrelated");
        dir.write(
            "90-other.yaml",
            "network:\n  version: 2\n  bridges:\n    br9:\n      interfaces: [enp9s0, enp9s1]\n",
        );
        let report = check_at(&dir.0, &pair());
        assert!(!report.has_conflict());
        assert_eq!(report.bridges.len(), 1);
    }

    #[test]
    fn interface_topology_mode_stick_is_on_a_stick() {
        assert!(on_a_stick_bridge_conf(r#"{"interface_topology":{"mode":"stick"}}"#));
        assert!(!on_a_stick_bridge_conf(r#"{"interface_topology":{"mode":"inline"}}"#));
        assert!(!on_a_stick_bridge_conf("not json"));
    }

    #[test]
    fn a_single_interface_definition_has_no_bridge_conflict() {
        let dir = Scratch::new("single");
        dir.write(
            "50-cloud-init.yaml",
            "network:\n  version: 2\n  ethernets:\n    enp1s0f0:\n      dhcp4: false\n",
        );
        assert!(!check_at(&dir.0, &pair()).has_conflict());
    }

    #[test]
    fn an_absent_directory_is_no_conflict() {
        let report = check_at(Path::new("/nonexistent-lqos-netplan"), &pair());
        assert!(!report.has_conflict());
    }
}
