//! The setup-owned DPDK NIC bind (`docs/nic_determinism_card.md` §2b):
//! idempotent, fail-loud with NAMED errors, and reversible (unbind-on-rollback).
//!
//! Pure core over an injectable sysfs root so the sequence is unit-testable;
//! the real path uses `/sys` + `modprobe`. The bind-mode tree is the card §1
//! (IOMMU present -> vfio-pci; absent -> vfio-pci + unsafe-noiommu; no-vfio ->
//! uio_pci_generic).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The PCI sysfs root a bind operates on (`/sys` in production).
#[derive(Clone, Debug)]
pub struct PciSysfs {
    /// The sysfs mount root.
    pub root: PathBuf,
}

impl PciSysfs {
    /// The system sysfs root.
    pub fn system() -> Self {
        Self {
            root: PathBuf::from("/sys"),
        }
    }

    /// A specific root (tests use a fake tree).
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The driver directory (`.../bus/pci/drivers/<driver>`).
    pub fn driver_dir(&self, driver: &str) -> PathBuf {
        self.root.join("bus/pci/drivers").join(driver)
    }

    /// The device directory (`.../bus/pci/devices/<bdf>`).
    pub fn device_dir(&self, bdf: &str) -> PathBuf {
        self.root.join("bus/pci/devices").join(bdf)
    }
}

/// The bind target derived from the card §1 decision tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindMode {
    /// IOMMU present; plain vfio-pci.
    Vfio,
    /// IOMMU absent; vfio-pci with unsafe-noiommu (the supported default).
    VfioNoiommu,
    /// vfio unavailable; the legacy uio fallback.
    Uio,
}

impl BindMode {
    /// The kernel driver this mode binds to.
    pub fn driver(self) -> &'static str {
        match self {
            Self::Vfio | Self::VfioNoiommu => "vfio-pci",
            Self::Uio => "uio_pci_generic",
        }
    }
}

/// The kernel driver a device is currently bound to, if any.
pub fn current_driver(sysfs: &PciSysfs, bdf: &str) -> Option<String> {
    let link = fs::read_link(sysfs.device_dir(bdf).join("driver")).ok()?;
    link.file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned)
}

fn device_id(sysfs: &PciSysfs, bdf: &str) -> Option<String> {
    let read = |name: &str| {
        fs::read_to_string(sysfs.device_dir(bdf).join(name))
            .ok()
            .map(|value| value.trim().trim_start_matches("0x").to_string())
    };
    let vendor = read("vendor")?;
    let device = read("device")?;
    Some(format!("{vendor} {device}"))
}

/// Load the kernel module backing `driver` (via `modprobe`) when it is not
/// registered yet. Named error when the module cannot be loaded.
pub fn ensure_driver_loaded(sysfs: &PciSysfs, driver: &str) -> Result<(), String> {
    if sysfs.driver_dir(driver).exists() {
        return Ok(());
    }
    let status = Command::new("modprobe")
        .arg(driver)
        .status()
        .map_err(|e| format!("unable to run modprobe {driver}: {e}"))?;
    if !status.success() {
        return Err(format!(
            "unable to load the {driver} kernel module (modprobe exit {status})"
        ));
    }
    Ok(())
}

/// Enable vfio's unsafe-noiommu mode (the supported default when IOMMU is
/// absent; ruling #2). Idempotent.
pub fn enable_noiommu(sysfs: &PciSysfs) -> Result<(), String> {
    let path = sysfs
        .root
        .join("module/vfio/parameters/enable_unsafe_noiommu_mode");
    fs::write(&path, "Y").map_err(|e| {
        format!(
            "unable to enable vfio unsafe-noiommu mode ({}): {e}",
            path.display()
        )
    })
}

/// Bind `bdf` to the mode's driver. Idempotent (a device already bound to the
/// target is a no-op); every failure is a NAMED error — never a hang.
pub fn bind(sysfs: &PciSysfs, bdf: &str, mode: BindMode) -> Result<(), String> {
    let driver = mode.driver();
    if !sysfs.device_dir(bdf).exists() {
        return Err(format!(
            "PCI device {bdf} not found under {}",
            sysfs.device_dir(bdf).display()
        ));
    }
    if current_driver(sysfs, bdf).as_deref() == Some(driver) {
        return Ok(());
    }
    ensure_driver_loaded(sysfs, driver)?;
    if mode == BindMode::VfioNoiommu {
        enable_noiommu(sysfs)?;
    }
    // Register the device id with the target driver (tolerate an existing id).
    if let Some(id) = device_id(sysfs, bdf) {
        let _ = fs::write(sysfs.driver_dir(driver).join("new_id"), format!("{id}\n"));
    }
    if let Some(current) = current_driver(sysfs, bdf) {
        let _ = fs::write(
            sysfs.driver_dir(&current).join("unbind"),
            format!("{bdf}\n"),
        );
    }
    fs::write(sysfs.driver_dir(driver).join("bind"), format!("{bdf}\n"))
        .map_err(|e| format!("unable to bind {bdf} to {driver}: {e}"))?;
    Ok(())
}

/// Rebind `bdf` to `original_driver` (the rollback leg). Idempotent; NAMED.
pub fn unbind_to(sysfs: &PciSysfs, bdf: &str, original_driver: &str) -> Result<(), String> {
    ensure_driver_loaded(sysfs, original_driver)?;
    if current_driver(sysfs, bdf).as_deref() == Some(original_driver) {
        return Ok(());
    }
    if let Some(current) = current_driver(sysfs, bdf) {
        let _ = fs::write(
            sysfs.driver_dir(&current).join("unbind"),
            format!("{bdf}\n"),
        );
    }
    fs::write(
        sysfs.driver_dir(original_driver).join("bind"),
        format!("{bdf}\n"),
    )
    .map_err(|e| format!("unable to rebind {bdf} to {original_driver}: {e}"))
}

/// The drivers a port must NEVER record as its "original": binding one back on
/// removal would leave the host with no kernel netdev (networkless).
pub fn is_dpdk_driver(driver: &str) -> bool {
    // igb_uio included for parity with the packaged unit's bash case (a legacy
    // DPDK-bound host must not record it as the "original").
    matches!(driver, "uio_pci_generic" | "vfio-pci" | "igb_uio")
}

/// The TRUE pre-bind kernel driver of a port, or `None` when the port is not on
/// a kernel driver (absent) OR is ALREADY on a DPDK driver.
///
/// This is the ONLY correct capture point: it must run BEFORE the first bind
/// (the setup's Rust `run_bind_nics`). By the time the boot-time bash bind runs
/// the NIC is already on uio/vfio, so this correctly yields `None` — the
/// negative case — rather than recording a DPDK driver as the original.
pub fn capture_original(sysfs: &PciSysfs, bdf: &str) -> Option<String> {
    let driver = current_driver(sysfs, bdf)?;
    if is_dpdk_driver(&driver) {
        None
    } else {
        Some(driver)
    }
}

/// The ONE originals precedence. The explicit `arg` is FIRST (an operator
/// override must beat a persisted value); the packaged `--restore` uses the
/// same rule minus the arg tier.
pub fn resolve_original(
    arg: Option<&str>,
    env_per_port: Option<&str>,
    state: Option<&str>,
    singular: Option<&str>,
) -> Option<String> {
    // An EMPTY or whitespace value is absent, not a driver: `--original-driver
    // ""` must not yield an empty path like /sys/bus/pci/drivers//bind.
    [arg, env_per_port, state, singular]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|v| !v.is_empty())
        .map(ToOwned::to_owned)
}

/// True when a sysfs path exists (helper for callers planning).
pub fn driver_registered(sysfs: &PciSysfs, driver: &str) -> bool {
    Path::new(&sysfs.driver_dir(driver)).exists()
}

/// Decide the bind mode from the host facts (card §1): IOMMU present ->
/// vfio-pci; absent -> vfio-pci + unsafe-noiommu (the supported default);
/// no vfio -> uio_pci_generic; neither available -> `None` (UNTESTED).
pub fn decide_mode(sysfs: &PciSysfs) -> Option<BindMode> {
    let iommu = fs::read_dir(sysfs.root.join("kernel/iommu_groups"))
        .map(|entries| entries.flatten().next().is_some())
        .unwrap_or(false);
    let vfio = sysfs.driver_dir("vfio-pci").exists();
    let uio = sysfs.driver_dir("uio_pci_generic").exists();
    if vfio {
        Some(if iommu {
            BindMode::Vfio
        } else {
            BindMode::VfioNoiommu
        })
    } else if uio {
        Some(BindMode::Uio)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_sysfs(name: &str) -> (PathBuf, PciSysfs) {
        let root = std::env::temp_dir().join(format!("lqos-nicbind-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for driver in ["vfio-pci", "uio_pci_generic", "i40e"] {
            fs::create_dir_all(root.join("bus/pci/drivers").join(driver)).unwrap();
        }
        let dev = root.join("bus/pci/devices/0000:01:00.0");
        fs::create_dir_all(&dev).unwrap();
        fs::write(dev.join("vendor"), "0x8086\n").unwrap();
        fs::write(dev.join("device"), "0x1572\n").unwrap();
        std::os::unix::fs::symlink(root.join("bus/pci/drivers/i40e"), dev.join("driver")).unwrap();
        fs::create_dir_all(root.join("module/vfio/parameters")).unwrap();
        fs::write(
            root.join("module/vfio/parameters/enable_unsafe_noiommu_mode"),
            "N\n",
        )
        .unwrap();
        let sysfs = PciSysfs::at(&root);
        (root, sysfs)
    }

    fn rebind_link(sysfs: &PciSysfs, bdf: &str, driver: &str) {
        let link = sysfs.device_dir(bdf).join("driver");
        let _ = fs::remove_file(&link);
        std::os::unix::fs::symlink(sysfs.driver_dir(driver), link).unwrap();
    }

    #[test]
    fn resolve_original_puts_the_explicit_arg_first() {
        // the arg beats a persisted per-port env value AND the state
        assert_eq!(
            resolve_original(Some("i40e"), Some("ixgbe"), Some("igb"), Some("ice")).as_deref(),
            Some("i40e")
        );
        // env per-port beats state beats singular
        assert_eq!(
            resolve_original(None, Some("ixgbe"), Some("igb"), Some("ice")).as_deref(),
            Some("ixgbe")
        );
        assert_eq!(
            resolve_original(None, None, Some("igb"), Some("ice")).as_deref(),
            Some("igb")
        );
        assert_eq!(
            resolve_original(None, None, None, Some("ice")).as_deref(),
            Some("ice")
        );
        assert_eq!(resolve_original(None, None, None, None), None);
        // an EMPTY/whitespace arg is absent, not an empty driver
        assert_eq!(
            resolve_original(Some(""), Some("ixgbe"), None, None).as_deref(),
            Some("ixgbe")
        );
        assert_eq!(
            resolve_original(Some("   "), None, None, None),
            None
        );
    }

    #[test]
    fn capture_refuses_a_dpdk_driver_as_the_original() {
        let (root, sysfs) = fake_sysfs("capture-guard");
        // a real kernel driver captures fine
        assert_eq!(
            capture_original(&sysfs, "0000:01:00.0").as_deref(),
            Some("i40e")
        );
        // once the port is on a DPDK driver, the capture yields NOTHING (the
        // negative case) — never records uio/vfio as the "original".
        rebind_link(&sysfs, "0000:01:00.0", "uio_pci_generic");
        assert_eq!(capture_original(&sysfs, "0000:01:00.0"), None);
        rebind_link(&sysfs, "0000:01:00.0", "vfio-pci");
        assert_eq!(capture_original(&sysfs, "0000:01:00.0"), None);
        assert!(is_dpdk_driver("uio_pci_generic"));
        assert!(is_dpdk_driver("vfio-pci"));
        assert!(!is_dpdk_driver("i40e"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn bind_moves_the_device_and_enables_noiommu() {
        let (root, sysfs) = fake_sysfs("bind");
        bind(&sysfs, "0000:01:00.0", BindMode::VfioNoiommu).unwrap();
        assert_eq!(
            fs::read_to_string(sysfs.driver_dir("vfio-pci").join("bind")).unwrap(),
            "0000:01:00.0\n"
        );
        assert_eq!(
            fs::read_to_string(sysfs.driver_dir("i40e").join("unbind")).unwrap(),
            "0000:01:00.0\n"
        );
        assert_eq!(
            fs::read_to_string(sysfs.driver_dir("vfio-pci").join("new_id")).unwrap(),
            "8086 1572\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("module/vfio/parameters/enable_unsafe_noiommu_mode"))
                .unwrap(),
            "Y"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn bind_is_idempotent_when_already_on_the_target() {
        let (root, sysfs) = fake_sysfs("idempotent");
        rebind_link(&sysfs, "0000:01:00.0", "vfio-pci");
        bind(&sysfs, "0000:01:00.0", BindMode::Vfio).unwrap();
        assert!(!sysfs.driver_dir("vfio-pci").join("bind").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn bind_names_a_missing_device() {
        let (root, sysfs) = fake_sysfs("missing");
        let err = bind(&sysfs, "0000:99:00.0", BindMode::Vfio).unwrap_err();
        assert!(err.contains("not found"), "{err}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rollback_rebinds_to_the_original_driver() {
        let (root, sysfs) = fake_sysfs("rollback");
        bind(&sysfs, "0000:01:00.0", BindMode::Vfio).unwrap();
        rebind_link(&sysfs, "0000:01:00.0", "vfio-pci");
        unbind_to(&sysfs, "0000:01:00.0", "i40e").unwrap();
        assert_eq!(
            fs::read_to_string(sysfs.driver_dir("i40e").join("bind")).unwrap(),
            "0000:01:00.0\n"
        );
        let _ = fs::remove_dir_all(&root);
    }
}
