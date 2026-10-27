//! The NIC determinism plan (`docs/nic_determinism_card.md` §2a): compute and
//! print the WHOLE bind plan with pure reads, so the operator pre-approves a
//! plan instead of discovering the NIC surface by trial-and-error.
//!
//! The bind-mode decision tree (the card §1, the driver's IOMMU ruling #2):
//! 1. IOMMU present -> `vfio-pci` (the free hardening).
//! 2. IOMMU absent -> `vfio-pci` + `enable_unsafe_noiommu_mode=1` — the
//!    SUPPORTED DEFAULT (no reboot demanded; the DMA tradeoff is disclosed).
//! 3. vfio unavailable -> `uio_pci_generic` (the legacy fallback; disclosed).
//!
//! Anything outside the tested matrix prints UNTESTED honestly.

use std::path::Path;

/// The canonical PMD directory the package ships (the wrapper also exports
/// `RTE_EAL_PMD_PATH` to it).
pub const PMD_DIR: &str = "/usr/libexec/libreqos/pmds";

/// The read-only facts the bind-mode decision needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BindFacts {
    /// `/sys/kernel/iommu_groups` is non-empty.
    pub iommu: bool,
    /// `vfio-pci` is loaded or loadable.
    pub vfio_available: bool,
    /// `uio_pci_generic` is loaded or loadable.
    pub uio_available: bool,
}

/// The selected bind mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BindMode {
    /// IOMMU present; plain vfio-pci.
    Vfio,
    /// IOMMU absent; vfio-pci with unsafe-noiommu (the supported default).
    VfioNoiommu,
    /// vfio unavailable; the legacy uio fallback.
    Uio,
    /// Outside the tested matrix; disclosed honestly.
    Untested(String),
}

impl BindMode {
    /// A one-line operator label.
    pub fn label(&self) -> String {
        match self {
            BindMode::Vfio => "vfio-pci (IOMMU present — the free hardening)".to_string(),
            BindMode::VfioNoiommu => {
                "vfio-pci + enable_unsafe_noiommu_mode=1 (IOMMU absent — the supported \
                 default; the DMA-isolation tradeoff is disclosed in the docs)"
                    .to_string()
            }
            BindMode::Uio => {
                "uio_pci_generic (vfio unavailable — the legacy fallback; disclosed)".to_string()
            }
            BindMode::Untested(why) => format!("UNTESTED ({why})"),
        }
    }

    /// True for the honest-untested class.
    pub fn is_untested(&self) -> bool {
        matches!(self, BindMode::Untested(_))
    }
}

/// The bind-mode decision (the card §1); pure so it is unit-testable.
pub fn bind_mode(facts: BindFacts) -> BindMode {
    if facts.iommu && facts.vfio_available {
        BindMode::Vfio
    } else if facts.vfio_available {
        BindMode::VfioNoiommu
    } else if facts.uio_available {
        BindMode::Uio
    } else {
        BindMode::Untested(
            "neither vfio-pci nor uio_pci_generic is available on this host".to_string(),
        )
    }
}

/// The computed NIC plan (the preflight prints it; the operator pre-approves).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NicPlan {
    /// The configured dataplane pair (`to_internet`, `to_network`).
    pub pair: Vec<String>,
    /// The resolved PCI BDFs (WAN then LAN), when the netdevs are still bound.
    pub bdfs: Vec<String>,
    /// The bind mode.
    pub mode: BindMode,
    /// The PMD search directory.
    pub pmd_dir: String,
    /// Whether the PMD directory exists on this host.
    pub pmd_present: bool,
}

impl NicPlan {
    /// A stable one-line summary of the orientation, for the printout.
    pub fn orientation(&self) -> String {
        if self.bdfs.len() == 2 {
            format!("{} (WAN) + {} (LAN)", self.bdfs[0], self.bdfs[1])
        } else {
            "unresolved (NICs already bound, or not a PCI pair)".to_string()
        }
    }
}

/// Resolve a configured interface to its PCI BDF (`/sys/class/net/<iface>/device`).
/// Valid only BEFORE the bind (the netdev vanishes after it).
pub fn bdf_from_iface(iface: &str) -> Option<String> {
    let iface = iface.trim();
    if iface.is_empty() {
        return None;
    }
    let target = std::fs::read_link(Path::new("/sys/class/net").join(iface).join("device")).ok()?;
    target
        .file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned)
}

/// Gather the bind facts from the running host (read-only).
pub fn probe_bind_facts() -> BindFacts {
    BindFacts {
        iommu: iommu_present(),
        vfio_available: driver_available("vfio-pci"),
        uio_available: driver_available("uio_pci_generic"),
    }
}

/// The plan for a configured pair (pure except the read-only probes).
pub fn check(pair: &[String]) -> NicPlan {
    check_with(pair, probe_bind_facts(), Path::new(PMD_DIR).is_dir())
}

/// The testable surface: plan from explicit facts.
pub fn check_with(pair: &[String], facts: BindFacts, pmd_present: bool) -> NicPlan {
    let bdfs: Vec<String> = pair
        .iter()
        .filter_map(|iface| bdf_from_iface(iface))
        .collect();
    NicPlan {
        pair: pair.to_vec(),
        bdfs,
        mode: bind_mode(facts),
        pmd_dir: PMD_DIR.to_string(),
        pmd_present,
    }
}

fn iommu_present() -> bool {
    std::fs::read_dir("/sys/kernel/iommu_groups")
        .map(|entries| entries.flatten().next().is_some())
        .unwrap_or(false)
}

/// True when a PCI driver is registered now, or its module is loadable.
fn driver_available(driver: &str) -> bool {
    if Path::new(&format!("/sys/bus/pci/drivers/{driver}")).exists() {
        return true;
    }
    let release = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    let release = release.trim();
    if release.is_empty() {
        return false;
    }
    let Ok(dep) = std::fs::read_to_string(format!("/lib/modules/{release}/modules.dep")) else {
        return false;
    };
    let module = driver.replace('-', "_");
    dep.lines()
        .any(|line| line.contains(&format!("{module}.ko")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mode_tree_matches_the_card() {
        assert_eq!(
            bind_mode(BindFacts {
                iommu: true,
                vfio_available: true,
                uio_available: true
            }),
            BindMode::Vfio
        );
        assert_eq!(
            bind_mode(BindFacts {
                iommu: false,
                vfio_available: true,
                uio_available: true
            }),
            BindMode::VfioNoiommu
        );
        assert_eq!(
            bind_mode(BindFacts {
                iommu: false,
                vfio_available: false,
                uio_available: true
            }),
            BindMode::Uio
        );
        assert!(
            bind_mode(BindFacts {
                iommu: false,
                vfio_available: false,
                uio_available: false
            })
            .is_untested()
        );
    }

    #[test]
    fn the_plan_summarizes_orientation_and_pmd() {
        let facts = BindFacts {
            iommu: false,
            vfio_available: true,
            uio_available: true,
        };
        let plan = check_with(&[], facts, true);
        assert!(plan.mode == BindMode::VfioNoiommu);
        assert!(plan.pmd_present);
        assert!(plan.orientation().contains("unresolved"));
    }

    #[test]
    fn the_iommu_absent_default_is_the_noiommu_mode() {
        let plan = check_with(
            &[],
            BindFacts {
                iommu: false,
                vfio_available: true,
                uio_available: true,
            },
            false,
        );
        assert!(plan.mode.label().contains("enable_unsafe_noiommu_mode=1"));
    }
}
