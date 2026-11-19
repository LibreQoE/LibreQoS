//! IEEE OUI vendor hints: the registry assignment a MAC falls in, as triage
//! evidence.
//!
//! [`lookup`] answers in one fixed order: the U/L bit first, then the I/G bit,
//! then the longest registry prefix. A locally administered MAC names itself
//! and never reaches the table. A universal group (I/G-set) address names a
//! group, not a device, and never reaches the table either. A universal
//! unicast MAC tries the MA-S (36-bit), MA-M (28-bit), and MA-L (24-bit)
//! assignment lengths in that order, so the longest block wins over the
//! shorter block that contains it. An unassigned universal MAC says so. The
//! table is `data/oui.tsv`, generated from the IEEE registry and
//! compiled in; nothing is fetched at runtime.
//!
//! The hint is triage evidence only. It never identifies a device, never
//! groups two devices, and never selects an adapter document: an OUI block is
//! shared by many products, and a locally administered MAC carries no vendor
//! at all.

use std::collections::HashMap;
use std::fmt;
use std::sync::LazyLock;

use serde::Serialize;

use crate::identity::{Mac, MacClass};

/// The generated prefix table, compiled in.
const TABLE: &str = include_str!("../data/oui.tsv");

/// The label for a locally administered MAC.
const LOCALLY_ADMINISTERED: &str = "locally administered";

/// The label for a universal MAC that no registry assignment covers.
const UNKNOWN_VENDOR: &str = "unknown vendor";

/// The label for a group (I/G-set) address: it names a group, not a device.
const GROUP_ADDRESS: &str = "group address";

/// Which IEEE registry assignment a prefix comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum VendorRegistry {
    /// MA-L: a 24-bit assignment, 6 hex digits.
    #[serde(rename = "ma-l")]
    MaL,
    /// MA-M: a 28-bit assignment, 7 hex digits.
    #[serde(rename = "ma-m")]
    MaM,
    /// MA-S: a 36-bit assignment, 9 hex digits.
    #[serde(rename = "ma-s")]
    MaS,
}

impl VendorRegistry {
    /// The table's registry field spelling.
    fn parse(field: &str) -> Option<Self> {
        match field {
            "MA-L" => Some(Self::MaL),
            "MA-M" => Some(Self::MaM),
            "MA-S" => Some(Self::MaS),
            _ => None,
        }
    }

    /// The assignment's width in hex digits.
    fn digits(self) -> usize {
        match self {
            Self::MaL => 6,
            Self::MaM => 7,
            Self::MaS => 9,
        }
    }
}

impl fmt::Display for VendorRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::MaL => "ma-l",
            Self::MaM => "ma-m",
            Self::MaS => "ma-s",
        })
    }
}

/// A MAC's vendor hint: the label, the registry assignment it came from, and
/// whether the address is locally administered. The structured form lets a
/// page explain the prefix instead of restating a bare name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VendorHint {
    /// The registry organization name, or "locally administered", or
    /// "unknown vendor".
    pub label: String,
    /// The assignment kind, when a table row matched.
    pub registry: Option<VendorRegistry>,
    /// The matched prefix, in the registry's uppercase hex form.
    pub prefix: Option<String>,
    /// Whether the MAC's U/L bit marks it locally administered.
    pub locally_administered: bool,
    /// Whether the MAC's I/G bit marks it as a group address. A group
    /// address never carries a vendor name.
    pub group_addressed: bool,
}

impl VendorHint {
    /// The claim text: the label, plus the registry prefix that produced it
    /// when one matched.
    pub fn claim_value(&self) -> String {
        match (&self.registry, &self.prefix) {
            (Some(registry), Some(prefix)) => format!("{} ({registry} {prefix})", self.label),
            _ => self.label.clone(),
        }
    }
}

/// One registry row.
struct Entry {
    registry: VendorRegistry,
    organization: &'static str,
}

/// The parsed table, built once on first lookup.
struct Registry {
    entries: HashMap<&'static str, Entry>,
}

impl Registry {
    /// Parses a table's text. A malformed row is a build defect in the
    /// generated data, so it fails loudly instead of degrading every lookup
    /// to "unknown vendor". Taking the text as a parameter keeps the
    /// rejection paths testable on inline tables.
    fn parse(table: &'static str) -> Self {
        let mut entries: HashMap<&'static str, Entry> = HashMap::new();
        for line in table.lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut fields = line.split('\t');
            let (Some(registry), Some(prefix), Some(organization), None) =
                (fields.next(), fields.next(), fields.next(), fields.next())
            else {
                panic!("malformed row in data/oui.tsv: {line:?}");
            };
            let Some(registry) = VendorRegistry::parse(registry) else {
                panic!("unknown registry {registry:?} in data/oui.tsv");
            };
            if prefix.len() != registry.digits()
                || !prefix.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                panic!("bad prefix {prefix:?} in data/oui.tsv");
            }
            if entries
                .insert(
                    prefix,
                    Entry {
                        registry,
                        organization,
                    },
                )
                .is_some()
            {
                panic!("duplicate prefix {prefix:?} in data/oui.tsv");
            }
        }
        Self { entries }
    }
}

/// The table, parsed on first use.
static REGISTRY: LazyLock<Registry> = LazyLock::new(|| Registry::parse(TABLE));

/// The vendor hint for `mac`. Locally administered addresses and group
/// (I/G-set) addresses never consult the table; a universal unicast address
/// takes the longest matching registry prefix.
pub fn lookup(mac: &Mac) -> VendorHint {
    if mac.class() == MacClass::Local {
        return VendorHint {
            label: LOCALLY_ADMINISTERED.to_owned(),
            registry: None,
            prefix: None,
            locally_administered: true,
            group_addressed: false,
        };
    }
    if mac.is_group() {
        return VendorHint {
            label: GROUP_ADDRESS.to_owned(),
            registry: None,
            prefix: None,
            locally_administered: false,
            group_addressed: true,
        };
    }
    // The registry spells prefixes as uppercase hex without separators; the
    // MAC's normalized text is lowercase hex pairs.
    let hex: String = mac
        .as_str()
        .bytes()
        .filter(|byte| *byte != b':')
        .map(|byte| char::from(byte.to_ascii_uppercase()))
        .collect();
    // Longest assignment first: an MA-S sub-block outranks the MA-L block
    // that contains it, and an MA-M block outranks the MA-L block.
    for registry in [
        VendorRegistry::MaS,
        VendorRegistry::MaM,
        VendorRegistry::MaL,
    ] {
        let Some(prefix) = hex.get(..registry.digits()) else {
            continue;
        };
        if let Some(entry) = REGISTRY.entries.get(prefix) {
            return VendorHint {
                label: entry.organization.to_owned(),
                registry: Some(entry.registry),
                prefix: Some(prefix.to_owned()),
                locally_administered: false,
                group_addressed: false,
            };
        }
    }
    VendorHint {
        label: UNKNOWN_VENDOR.to_owned(),
        registry: None,
        prefix: None,
        locally_administered: false,
        group_addressed: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_valid_inline_table_parses_and_skips_comments() {
        let registry =
            Registry::parse("# header\n\nMA-L\t001F5E\tAcme\nMA-S\t8C1F64AFA\tSub Vendor\n");
        assert_eq!(registry.entries.len(), 2);
        assert_eq!(registry.entries["001F5E"].organization, "Acme");
        assert_eq!(registry.entries["8C1F64AFA"].registry, VendorRegistry::MaS);
    }

    #[test]
    #[should_panic(expected = "malformed row")]
    fn a_row_without_three_fields_is_refused() {
        let _ = Registry::parse("MA-L\t001F5E\n");
    }

    #[test]
    #[should_panic(expected = "bad prefix")]
    fn a_wrong_width_prefix_is_refused() {
        let _ = Registry::parse("MA-L\t001F5EA\tAcme\n");
    }

    #[test]
    #[should_panic(expected = "unknown registry")]
    fn an_unknown_registry_is_refused() {
        let _ = Registry::parse("MA-X\t001F5E\tAcme\n");
    }

    #[test]
    #[should_panic(expected = "duplicate prefix")]
    fn a_duplicate_prefix_is_refused() {
        let _ = Registry::parse("MA-L\t001F5E\tAcme\nMA-L\t001F5E\tOther\n");
    }
}
