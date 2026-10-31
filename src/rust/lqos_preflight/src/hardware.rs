//! The card's §1.1 hardware verdict: hugepages, the NIC-pair gate, and the
//! kernel/AF_XDP fallback.
//!
//! Ported from `packaging/libexec/libreqos-setup-check`, split into pure
//! parsers (tested) and thin probes that read `/proc` and `/sys`. The verdict
//! is three-way: SUPPORTED / FALLBACK-WITH-CAVEATS / BLOCKED.

/// Hugepage state, from `/proc/meminfo`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HugePages {
    /// `HugePages_Total`.
    pub total: u64,
    /// `HugePages_Free`.
    pub free: u64,
}

/// The three-way hardware class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HardwareClass {
    /// Native DPDK with the required resources.
    Supported,
    /// Usable with caveats (the AF_XDP fallback, 2M-only hugepages).
    Fallback,
    /// The hardware cannot run v3.
    Blocked,
}

/// The default count of 2M hugepages the installer targets (the fleet's
/// `2M x 3072` — the runtime-alloc ladder's first rung, `INSTALL.md`).
pub const HUGEPAGES_2M_TARGET: u64 = 3_072;

/// The 2M page size in KiB.
const HUGEPAGES_2M_KIB: u64 = 2_048;

/// Available memory headroom required to attempt the runtime 2M hugepage
/// allocation (the preflight's precondition check).
pub const HUGEPAGES_2M_REQUIRED_KB: u64 = HUGEPAGES_2M_TARGET * HUGEPAGES_2M_KIB;

/// The composed hardware verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HardwareVerdict {
    /// The overall class (worst of the checks).
    pub class: HardwareClass,
    /// Non-fatal caveats.
    pub warnings: Vec<String>,
    /// Fatal hardware problems.
    pub blocks: Vec<String>,
    /// The observed hugepage state.
    pub hugepages: HugePages,
    /// `MemAvailable` from `/proc/meminfo`, in KiB (the runtime-alloc
    /// headroom the automatic hugepage ladder checks).
    pub mem_available_kb: u64,
    /// The observed dataplane port count.
    pub dataplane_ports: usize,
    /// The parsed kernel `(major, minor)`, when available.
    pub kernel: Option<(u32, u32)>,
    /// AVAIL 0(e) Part B: whether the hugepage reservation is BOOT-PERSISTENT
    /// (declared on the kernel command line / grub / sysctl). `None` = no pages
    /// reserved (the missing-pages arm covers it).
    pub hugepages_persistent: Option<bool>,
}

impl HardwareVerdict {
    /// A clean verdict used by callers that supply their own facts (tests).
    pub fn all_ok() -> Self {
        Self {
            class: HardwareClass::Supported,
            warnings: Vec::new(),
            blocks: Vec::new(),
            hugepages: HugePages {
                total: 4,
                free: 4,
            },
            mem_available_kb: u64::MAX,
            dataplane_ports: 2,
            kernel: Some((6, 8)),
            hugepages_persistent: Some(true),
        }
    }
}

/// True when the hugepage reservation is declared in a BOOT-PERSISTENT
/// location: the kernel command line, the grub default, or sysctl
/// (`vm.nr_hugepages`). A bare RUNTIME reservation is lost on reboot — the
/// class the availability card §4 calls out (the bridge died at EAL with 0
/// hugepages and nothing pre-flighted it).
pub fn hugepages_persistent(kernel_cmdline: &str, grub_cfg: &str, sysctl: &str) -> bool {
    /// Drop inline comments, then a fully-commented line becomes empty and is
 /// SKIPPED — a COMMENTED-OUT declaration is NOT persistent (the implementer #12).
    fn uncommented(line: &str) -> &str {
        line.split('#').next().unwrap_or("").trim()
    }
    fn declares_hugepages(text: &str) -> bool {
        text.lines().map(uncommented).any(|line| {
            // Match within the line (grub wraps it as GRUB_CMDLINE_LINUX="…").
            line.contains("default_hugepagesz") || line.contains("hugepages=")
        })
    }
    fn declares_nr_hugepages(text: &str) -> bool {
        // Whole-token match so a future `vm.nr_hugepages_*` cannot false-match.
        text.lines().map(uncommented).any(|line| {
            line.split(|c: char| c == '=' || c.is_whitespace())
                .any(|token| token == "vm.nr_hugepages")
        })
    }
    declares_hugepages(kernel_cmdline)
        || declares_hugepages(grub_cfg)
        || declares_nr_hugepages(sysctl)
}

/// Concatenate the sysctl hugepage declarations (`/etc/sysctl.conf` +
/// `/etc/sysctl.d/*.conf`).
fn read_sysctl_hugepages() -> String {
    let mut text = std::fs::read_to_string("/etc/sysctl.conf").unwrap_or_default();
    if let Ok(entries) = std::fs::read_dir("/etc/sysctl.d") {
        for entry in entries.flatten() {
            if entry.path().extension().is_some_and(|ext| ext == "conf")
                && let Ok(body) = std::fs::read_to_string(entry.path())
            {
                text.push('\n');
                text.push_str(&body);
            }
        }
    }
    text
}

/// Parse the hugepage fields from `/proc/meminfo`-style text.
pub fn parse_meminfo_hugepages(meminfo: &str) -> HugePages {
    let mut hp = HugePages::default();
    for line in meminfo.lines() {
        if let Some(rest) = line.strip_prefix("HugePages_Total:") {
            hp.total = first_u64(rest);
        } else if let Some(rest) = line.strip_prefix("HugePages_Free:") {
            hp.free = first_u64(rest);
        }
    }
    hp
}

/// Parse `MemAvailable` (KiB) from `/proc/meminfo`-style text.
pub fn parse_meminfo_available_kb(meminfo: &str) -> u64 {
    meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemAvailable:").map(first_u64))
        .unwrap_or(0)
}

fn first_u64(text: &str) -> u64 {
    text.split_whitespace()
        .next()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

/// Parse a kernel release into `(major, minor)`.
pub fn parse_kernel_release(release: &str) -> Option<(u32, u32)> {
    let mut parts = release
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty());
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

/// True when the kernel release is at least `want`.
pub fn kernel_at_least(release: &str, want: (u32, u32)) -> bool {
    parse_kernel_release(release).is_some_and(|have| have >= want)
}

/// True for an Ethernet-controller PCI class id (`0x020000`…`0x0200ff`).
pub fn is_ethernet_class(class: &str) -> bool {
    class.trim().starts_with("0x0200")
}

/// Compose the hardware verdict from already-gathered facts (pure).
pub fn verdict(
    hugepages: HugePages,
    mem_available_kb: u64,
    dataplane_ports: usize,
    kernel: Option<(u32, u32)>,
    af_xdp_fallback: bool,
    hugepages_persistent: Option<bool>,
) -> HardwareVerdict {
    let mut warnings = Vec::new();
    let mut blocks = Vec::new();

    // The automatic hugepages ladder (§2): 2M is the default (the fleet's
    // 2M x 3072); the installer attempts the runtime allocation. Absent pages
    // are a WARN when there is memory headroom to attempt it, a BLOCK only
    // when the headroom is missing.
    if hugepages.total > 0 {
        // AVAIL 0(e) Part B: pages reserved but NOT boot-persistent are lost
        // on reboot — a WARN (report-only; never a refuse, §0/§7).
        if hugepages_persistent == Some(false) {
            warnings.push(
                "hugepages are reserved but NOT boot-persistent (no \
                 default_hugepagesz/hugepages= on the kernel command line and no \
                 vm.nr_hugepages in sysctl) — a reboot may lose them and the \
                 dataplane will fail at EAL; add the reservation to the kernel \
                 command line or sysctl (packaging/INSTALL.md)"
                    .to_string(),
            );
        }
    } else if mem_available_kb >= HUGEPAGES_2M_REQUIRED_KB {
        warnings.push(format!(
            "no hugepages reserved — the installer will attempt automatic 2M \
             allocation ({} pages); a reboot may be required if it fails \
             (packaging/INSTALL.md)",
            HUGEPAGES_2M_TARGET
        ));
    } else {
        blocks.push(format!(
            "no hugepages reserved and only {mem_available_kb} KiB available \
             (need {} KiB for the automatic 2M allocation) — free memory, or \
             add default_hugepagesz=2M hugepages={} (or 1G pages) to the kernel \
             command line (packaging/INSTALL.md)",
            HUGEPAGES_2M_REQUIRED_KB, HUGEPAGES_2M_TARGET
        ));
    }

    if dataplane_ports < 2 {
        blocks.push(format!(
            "inline deployment needs a PAIR of dataplane NICs; found {dataplane_ports}"
        ));
    }

    if af_xdp_fallback {
        warnings.push(
            "AF_XDP fallback: the dataplane ports here are virtually-bound / without a \
             native DPDK driver (the management interfaces), NOT necessarily the box's \
             capability — if a native DPDK pair (ConnectX/i40e/etc.) is present, bind it \
             for the dataplane (LIBREQOS_SETUP_NIC_DRIVER / dpdk-devbind); otherwise \
             expect reduced performance versus native DPDK"
                .to_string(),
        );
    }

    let class = if !blocks.is_empty() {
        HardwareClass::Blocked
    } else if !warnings.is_empty() {
        HardwareClass::Fallback
    } else {
        HardwareClass::Supported
    };

    HardwareVerdict {
        class,
        warnings,
        blocks,
        hugepages,
        mem_available_kb,
        dataplane_ports,
        kernel,
        hugepages_persistent,
    }
}

/// The expected dataplane NIC driver (the prod ConnectX class); a lab host can
/// override it in `/etc/libreqos/bridge.env`.
fn expected_nic_driver() -> String {
    std::fs::read_to_string("/etc/libreqos/bridge.env")
        .ok()
        .and_then(|text| {
            text.lines()
                .rev()
                .find_map(|line| line.strip_prefix("LIBREQOS_SETUP_NIC_DRIVER=").map(str::to_string))
        })
        .unwrap_or_else(|| "mlx5_core".to_string())
}

/// Known native-DPDK NIC drivers (the v3 support matrix: the ConnectX/mlx5
/// class plus the Intel i40e/ice/igb/ixgbe classes), and the uio/vfio binds a
/// DPDK-ready port may already carry.
const NATIVE_DPDK_DRIVERS: &[&str] = &[
    "mlx5_core",
    "i40e",
    "ice",
    "igb",
    "ixgbe",
    "uio_pci_generic",
    "vfio-pci",
];

fn driver_name(pci: &std::path::Path) -> Option<String> {
    let target = std::fs::read_link(pci.join("driver")).ok()?;
    target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
}

/// True for the virtio family, the AF_XDP/management class. At the PCI level
/// the bound driver is `virtio-pci` (the netdev-level driver is `virtio_net`);
/// matching the whole prefix catches either form.
fn is_virtio_driver(name: &str) -> bool {
    name.starts_with("virtio")
}

/// Count the dataplane ports: Ethernet-class PCI functions whose driver is a
/// known native-DPDK driver (or the configured one) OR which are UNBOUND (the
/// pre-install posture — the NIC exists, not yet bound for DPDK). A virtio
/// controller is the AF_XDP/management class and is reported separately, never
/// counted as a dataplane port.
fn count_ports() -> (usize, bool) {
    let configured = expected_nic_driver();
    let mut native_ports = 0usize;
    let mut virtio = false;

    let Ok(entries) = std::fs::read_dir("/sys/bus/pci/devices") else {
        return (0, false);
    };
    for entry in entries.flatten() {
        let class = std::fs::read_to_string(entry.path().join("class")).unwrap_or_default();
        if !is_ethernet_class(&class) {
            continue;
        }
        match driver_name(&entry.path()).as_deref() {
            Some(name) if is_virtio_driver(name) => virtio = true,
            Some(name) if name == configured || NATIVE_DPDK_DRIVERS.contains(&name) => {
                native_ports += 1;
            }
            // Unbound Ethernet controller: a DPDK candidate (pre-install).
            None => native_ports += 1,
            // Another kernel driver (a management NIC) is not a dataplane port.
            Some(_) => {}
        }
    }

    (native_ports, virtio)
}

/// Gather the hardware facts from the running system.
pub fn probe() -> HardwareVerdict {
    let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let hugepages = parse_meminfo_hugepages(&meminfo);
    let mem_available_kb = parse_meminfo_available_kb(&meminfo);
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .ok()
        .and_then(|raw| parse_kernel_release(raw.trim()));
    let (native_ports, virtio) = count_ports();
    let af_xdp_fallback = is_af_xdp_fallback(native_ports, virtio);
    let ports_for_verdict = if af_xdp_fallback { 2 } else { native_ports };
    let hugepages_persistent = if hugepages.total > 0 {
        Some(hugepages_persistent(
            &std::fs::read_to_string("/proc/cmdline").unwrap_or_default(),
            &std::fs::read_to_string("/etc/default/grub").unwrap_or_default(),
            &read_sysctl_hugepages(),
        ))
    } else {
        None
    };
    verdict(
        hugepages,
        mem_available_kb,
        ports_for_verdict,
        kernel,
        af_xdp_fallback,
        hugepages_persistent,
    )
}

/// The AF_XDP posture applies only to a genuinely virtio-only box (no native
/// dataplane ports): its dataplane is AF_XDP, so the pair block does not apply.
/// A mixed box (a native port plus a virtio management NIC) is judged on its
/// native ports — a single native NIC still BLOCKs on the missing pair (the
/// jackrabbit X710 class has a full native pair and passes).
fn is_af_xdp_fallback(native_ports: usize, virtio: bool) -> bool {
    virtio && native_ports == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_meminfo_hugepages() {
        let text = "MemTotal:       16333628 kB\nHugePages_Total:       4\nHugePages_Free:        3\n";
        assert_eq!(parse_meminfo_hugepages(text), HugePages { total: 4, free: 3 });
    }

    #[test]
    fn parses_kernel_releases() {
        assert_eq!(parse_kernel_release("6.8.0-45-generic"), Some((6, 8)));
        assert_eq!(parse_kernel_release("5.15.0"), Some((5, 15)));
        assert_eq!(parse_kernel_release("no-digits"), None);
        assert!(kernel_at_least("6.8.0-45-generic", (6, 0)));
        assert!(!kernel_at_least("5.15.0", (6, 0)));
    }

    #[test]
    fn ethernet_class_detection() {
        assert!(is_ethernet_class("0x020000"));
        assert!(is_ethernet_class(" 0x0200ff "));
        assert!(!is_ethernet_class("0x030000"));
    }

    #[test]
    fn detects_the_virtio_family() {
        // At the PCI level the driver is `virtio-pci`; netdev-level is
        // `virtio_net`. Both are virtio (the AF_XDP/management class).
        assert!(is_virtio_driver("virtio-pci"));
        assert!(is_virtio_driver("virtio_net"));
        assert!(!is_virtio_driver("i40e"));
        assert!(!is_virtio_driver("mlx5_core"));
    }

    #[test]
    fn only_a_virtio_only_box_is_the_af_xdp_fallback() {
        assert!(is_af_xdp_fallback(0, true)); // pure virtio VM
        assert!(!is_af_xdp_fallback(2, true)); // the jackrabbit X710 + virtio mgmt
        assert!(!is_af_xdp_fallback(1, true)); // one native: still needs a pair
        assert!(!is_af_xdp_fallback(0, false)); // no virtio: pair block stands
    }

    #[test]
    fn a_full_box_is_supported() {
        let v = verdict(
            HugePages { total: 4, free: 4 },
            u64::MAX,
            2,
            Some((6, 8)),
            false,
            Some(true),
        );
        assert_eq!(v.class, HardwareClass::Supported);
        assert!(v.warnings.is_empty() && v.blocks.is_empty());
    }

    /// AVAIL 0(e) Part B: reserved-but-not-persistent hugepages are a WARN
    /// (report-only; never a block).
    #[test]
    fn reserved_but_not_persistent_hugepages_warn() {
        let v = verdict(
            HugePages { total: 4, free: 4 },
            u64::MAX,
            2,
            Some((6, 8)),
            false,
            Some(false),
        );
        assert_eq!(v.class, HardwareClass::Fallback);
        assert!(
            v.warnings.iter().any(|w| w.contains("NOT boot-persistent")),
            "{:?}",
            v.warnings
        );
    }

    /// The persistence predicate: a kernel-cmdline / grub / sysctl declaration
    /// is persistent; a bare runtime reservation is not.
    #[test]
    fn hugepages_persistence_predicate() {
        assert!(hugepages_persistent("default_hugepagesz=1G hugepages=4", "", ""));
        assert!(hugepages_persistent(
            "quiet splash",
            "# GRUB_CMDLINE\nGRUB_CMDLINE_LINUX=\"hugepages=2048\"",
            ""
        ));
        assert!(hugepages_persistent("", "", "vm.nr_hugepages = 2048"));
        assert!(!hugepages_persistent("quiet splash", "", ""));
 // A COMMENTED-OUT declaration is NOT persistent (the implementer #12).
        assert!(!hugepages_persistent(
            "",
            "# GRUB_CMDLINE_LINUX=\"hugepages=2048\"",
            ""
        ));
        assert!(!hugepages_persistent("", "", "# vm.nr_hugepages = 2048"));
        // Inline comments are stripped, then the live declaration still counts.
        assert!(hugepages_persistent("hugepages=2048 # reserved", "", ""));
    }

    #[test]
    fn absent_hugepages_with_headroom_is_a_warn_not_a_block() {
        // §2: absent pages + enough memory for the runtime 2M allocation.
        let v = verdict(
            HugePages { total: 0, free: 0 },
            HUGEPAGES_2M_REQUIRED_KB,
            2,
            Some((6, 8)),
            false,
            None,
        );
        assert_eq!(v.class, HardwareClass::Fallback);
        assert!(v.warnings.iter().any(|w| w.contains("automatic 2M")));
        assert!(v.blocks.is_empty());
    }

    #[test]
    fn absent_hugepages_without_headroom_blocks() {
        let v = verdict(HugePages::default(), 0, 2, Some((6, 8)), false, None);
        assert_eq!(v.class, HardwareClass::Blocked);
        assert!(v.blocks.iter().any(|b| b.contains("available")));
    }

    #[test]
    fn no_hugepages_and_a_lone_nic_block() {
        let v = verdict(HugePages::default(), 0, 1, Some((6, 8)), true, None);
        assert_eq!(v.class, HardwareClass::Blocked);
        assert_eq!(v.blocks.len(), 2);
    }

    #[test]
    fn parses_mem_available() {
        let text = "MemTotal:       16333628 kB\nMemAvailable:    8388608 kB\n";
        assert_eq!(parse_meminfo_available_kb(text), 8_388_608);
    }
}
