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
    /// The observed SUPPORTED dataplane port count (native + qualified virtio).
    pub dataplane_ports: usize,
    /// Observed but UNSUPPORTED dataplane candidates (e.g. an igb/unknown
    /// Ethernet controller): named, never counted.
    pub unsupported_ports: Vec<String>,
    /// `--vdev=net_af_xdp` was requested in the EAL args (never a port).
    pub af_xdp_requested: bool,
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
            unsupported_ports: Vec::new(),
            af_xdp_requested: false,
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
///
/// `supported_ports` counts native + QUALIFIED virtio ports only; a requested
/// AF_XDP vdev is never a port. `on_a_stick` relaxes the pair gate to a WARN
/// (one shared WAN/LAN interface); `fresh` relaxes an absent pair to a WARN.
#[allow(clippy::too_many_arguments)]
pub fn verdict_with_policy(
    hugepages: HugePages,
    mem_available_kb: u64,
    supported_ports: usize,
    unsupported_ports: Vec<String>,
    fresh: bool,
    on_a_stick: bool,
    af_xdp_requested: bool,
    af_xdp_allowed: bool,
    kernel: Option<(u32, u32)>,
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

    if !unsupported_ports.is_empty() {
        warnings.push(format!(
            "unsupported dataplane NIC(s) ignored: {} — only net_mlx5/net_ixgbe/\
             net_ice/net_iavf/net_i40e and qualified virtio are supported; an \
             unsupported/unknown PMD is never counted (bind a supported NIC or use \
             the supported NIC matrix)",
            unsupported_ports.join(", ")
        ));
    }

    if af_xdp_requested {
        if af_xdp_allowed {
            warnings.push(
                "AF_XDP is UNSUPPORTED for this release but is explicitly allowed by \
                 LQOS_PREFLIGHT_ALLOW_UNSUPPORTED_PMD=1 (lab override) — expect reduced \
                 performance and no support claim"
                    .to_string(),
            );
        } else {
            blocks.push(
                "AF_XDP is UNSUPPORTED for this release: LIBREQOS_DPDK_EAL_ARGS requests \
                 --vdev=net_af_xdp. Remove it and use a supported native/qualified-virtio \
                 port; the lab may set LQOS_PREFLIGHT_ALLOW_UNSUPPORTED_PMD=1 to downgrade \
                 this to a WARN"
                    .to_string(),
            );
        }
    }

    if supported_ports < 2 {
        if on_a_stick && supported_ports >= 1 {
            warnings.push(format!(
                "on-a-stick deployment: one supported dataplane port ({supported_ports}) \
                 carries both WAN and LAN — shared throughput, no port redundancy; verify \
                 the VLAN IDs and the peer trunk before proceeding"
            ));
        } else if fresh {
            warnings.push(format!(
                "inline deployment needs a PAIR of dataplane NICs; found {supported_ports} \
                 (fresh setup will configure the ports)"
            ));
        } else {
            blocks.push(format!(
                "inline deployment needs a PAIR of supported dataplane NICs; found \
                 {supported_ports} — a configured pair with an unsupported/unknown PMD is a \
                 BLOCK (override only with LQOS_PREFLIGHT_ALLOW_UNSUPPORTED_PMD=1)"
            ));
        }
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
        dataplane_ports: supported_ports,
        unsupported_ports,
        af_xdp_requested,
        kernel,
        hugepages_persistent,
    }
}

/// Backwards-compatible pure wrapper (tests / callers without the policy).
pub fn verdict(
    hugepages: HugePages,
    mem_available_kb: u64,
    dataplane_ports: usize,
    kernel: Option<(u32, u32)>,
    af_xdp_fallback: bool,
    hugepages_persistent: Option<bool>,
) -> HardwareVerdict {
    verdict_with_policy(
        hugepages,
        mem_available_kb,
        dataplane_ports,
        Vec::new(),
        false,
        false,
        af_xdp_fallback,
        false,
        kernel,
        hugepages_persistent,
    )
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

/// Kernel drivers whose NIC is backed by a SUPPORTED shipped PMD.
///
/// `igb` maps to `librte_net_e1000` (the DPDK 25.11 e1000 library carries
/// net_e1000_igb; `lqos-74t.146` packages it). `vfio-pci`/`uio_pci_generic`
/// are BIND drivers, not NIC drivers — a port already bound for DPDK is judged
/// by its BDF/config, never by the bind driver name.
const SUPPORTED_NATIVE_DRIVERS: &[&str] = &[
    "mlx5_core", "i40e", "ice", "iavf", "ixgbe", "igb",
];
const BIND_DRIVERS: &[&str] = &["vfio-pci", "uio_pci_generic"];

/// Qualified virtio: modern virtio 1.0 (PCI `0x1af4:0x1041`), at least 2 RX
/// and 2 TX queues, and the mergeable-RX-buffer + guest-checksum feature bits.
/// Legacy (`0x1000`) or insufficient evidence is NOT qualified.
pub const VIRTIO_VENDOR: u32 = 0x1af4;
pub const VIRTIO_MODERN_DEVICE: u32 = 0x1041;
const VIRTIO_NET_F_MRG_RXBUF: u64 = 1 << 15;
const VIRTIO_NET_F_GUEST_CSUM: u64 = 1 << 16;

/// Pure qualification rule for a virtio NIC (the pinned matrix).
pub fn virtio_qualified(
    vendor: Option<u32>,
    device: Option<u32>,
    rx_queues: usize,
    tx_queues: usize,
    feature_bits: Option<u64>,
) -> bool {
    vendor == Some(VIRTIO_VENDOR)
        && device == Some(VIRTIO_MODERN_DEVICE)
        && rx_queues >= 2
        && tx_queues >= 2
        && feature_bits.is_some_and(|bits| {
            bits & VIRTIO_NET_F_MRG_RXBUF != 0 && bits & VIRTIO_NET_F_GUEST_CSUM != 0
        })
}

/// AF_XDP is requested when the EAL args carry `--vdev=net_af_xdp` (the vdev
/// form; `net_af_xdp0` also counts). The lab may override the BLOCK.
pub fn af_xdp_requested_in(eal_args: &str) -> bool {
    eal_args.split_whitespace().any(|arg| {
        arg == "--vdev=net_af_xdp"
            || arg
                .strip_prefix("--vdev=")
                .is_some_and(|vdev| vdev.split(',').next() == Some("net_af_xdp"))
    })
}

fn read_bridge_env_args() -> String {
    std::fs::read_to_string("/etc/libreqos/bridge.env")
        .ok()
        .and_then(|text| {
            text.lines()
                .rev()
                .find_map(|line| line.strip_prefix("LIBREQOS_DPDK_EAL_ARGS=").map(str::to_string))
        })
        .unwrap_or_default()
}

fn af_xdp_requested() -> bool {
    let env = std::env::var("LIBREQOS_DPDK_EAL_ARGS").unwrap_or_default();
    af_xdp_requested_in(&env) || af_xdp_requested_in(&read_bridge_env_args())
}

/// The lab override for an unsupported PMD (AF_XDP): downgrade BLOCK -> WARN.
pub fn unsupported_pmd_allowed() -> bool {
    std::env::var("LQOS_PREFLIGHT_ALLOW_UNSUPPORTED_PMD")
        .ok()
        .is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
}

fn driver_name(pci: &std::path::Path) -> Option<String> {
    let target = std::fs::read_link(pci.join("driver")).ok()?;
    target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
}

fn is_virtio_driver(name: &str) -> bool {
    name.starts_with("virtio")
}

fn pci_attr(path: &std::path::Path, name: &str) -> Option<String> {
    std::fs::read_to_string(path.join(name))
        .ok()
        .map(|v| v.trim().to_owned())
}

fn parse_hex_u32(value: &str) -> Option<u32> {
    let v = value.trim().trim_start_matches("0x");
    u32::from_str_radix(v, 16).ok()
}

fn parse_hex_u64(value: &str) -> Option<u64> {
    let v = value.trim().trim_start_matches("0x");
    u64::from_str_radix(v, 16).ok()
}

fn netdev_of(path: &std::path::Path) -> Option<std::path::PathBuf> {
    let entries = std::fs::read_dir(path.join("net")).ok()?;
    entries
        .flatten()
        .next()
        .map(|entry| entry.path())
}

fn queue_count(netdev: &std::path::Path, prefix: &str) -> usize {
    let Ok(entries) = std::fs::read_dir(netdev.join("queues")) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(prefix))
        .count()
}

/// Virtio feature bits: `/sys/bus/pci/devices/<bdf>/virtio*/features` (or the
/// PCI device `features` attr). Absent ⇒ unqualified (conservative).
fn virtio_features(path: &std::path::Path) -> Option<u64> {
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("virtio") {
                if let Ok(value) = std::fs::read_to_string(entry.path().join("features")) {
                    return parse_hex_u64(&value);
                }
            }
        }
    }
    pci_attr(path, "features").and_then(|v| parse_hex_u64(&v))
}

/// Classify the host's Ethernet PCI functions into SUPPORTED dataplane ports,
/// unsupported candidates, and the AF_XDP request.
fn probe_ports() -> (usize, Vec<String>, bool) {
    let configured = expected_nic_driver();
    let mut supported = 0usize;
    let mut unsupported = Vec::new();

    let Ok(entries) = std::fs::read_dir("/sys/bus/pci/devices") else {
        return (0, unsupported, af_xdp_requested());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let class = std::fs::read_to_string(path.join("class")).unwrap_or_default();
        if !is_ethernet_class(&class) {
            continue;
        }
        let bdf = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match driver_name(&path).as_deref() {
            Some(name) if is_virtio_driver(name) => {
                let vendor = pci_attr(&path, "vendor").and_then(|v| parse_hex_u32(&v));
                let device = pci_attr(&path, "device").and_then(|v| parse_hex_u32(&v));
                let (rx, tx) = netdev_of(&path)
                    .map(|nd| (queue_count(&nd, "rx-"), queue_count(&nd, "tx-")))
                    .unwrap_or((0, 0));
                let features = virtio_features(&path);
                if virtio_qualified(vendor, device, rx, tx, features) {
                    supported += 1;
                } else {
                    unsupported.push(format!(
                        "{bdf} virtio (vendor {vendor:?} device {device:?}, rx={rx} tx={tx})"
                    ));
                }
            }
            Some(name) if name == configured || SUPPORTED_NATIVE_DRIVERS.contains(&name) => {
                supported += 1;
            }
            // Already bound for DPDK: judged by the BDF/config, not the bind name.
            Some(name) if BIND_DRIVERS.contains(&name) => supported += 1,
            // Unbound Ethernet controller: a DPDK candidate (pre-install).
            None => supported += 1,
            // Another kernel driver: a management NIC, not a dataplane port.
            Some(_) => {}
        }
    }

    (supported, unsupported, af_xdp_requested())
}

/// Gather the hardware facts from the running system (default policy:
/// not-fresh, not on-a-stick).
pub fn probe() -> HardwareVerdict {
    probe_with_policy(false, false)
}

/// Gather the hardware facts with the fresh/on-a-stick pair policy.
pub fn probe_with_policy(fresh: bool, on_a_stick: bool) -> HardwareVerdict {
    let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let hugepages = parse_meminfo_hugepages(&meminfo);
    let mem_available_kb = parse_meminfo_available_kb(&meminfo);
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .ok()
        .and_then(|raw| parse_kernel_release(raw.trim()));
    let (supported, unsupported, af_xdp) = probe_ports();
    let hugepages_persistent = if hugepages.total > 0 {
        Some(hugepages_persistent(
            &std::fs::read_to_string("/proc/cmdline").unwrap_or_default(),
            &std::fs::read_to_string("/etc/default/grub").unwrap_or_default(),
            &read_sysctl_hugepages(),
        ))
    } else {
        None
    };
    verdict_with_policy(
        hugepages,
        mem_available_kb,
        supported,
        unsupported,
        fresh,
        on_a_stick,
        af_xdp,
        unsupported_pmd_allowed(),
        kernel,
        hugepages_persistent,
    )
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

    /// The pinned virtio matrix and the AF_XDP detection are PURE.
    #[test]
    fn virtio_matrix_and_af_xdp_detection() {
        let bits = (1u64 << 15) | (1u64 << 16);
        assert!(virtio_qualified(Some(0x1af4), Some(0x1041), 2, 2, Some(bits)));
        assert!(!virtio_qualified(Some(0x1af4), Some(0x1000), 2, 2, Some(bits)), "legacy");
        assert!(!virtio_qualified(Some(0x1af4), Some(0x1041), 1, 2, Some(bits)), "queues");
        assert!(!virtio_qualified(Some(0x1af4), Some(0x1041), 2, 2, Some(bits & !(1 << 15))), "mrg");
        assert!(!virtio_qualified(Some(0x1af4), Some(0x1041), 2, 2, None), "no evidence");
        assert!(!virtio_qualified(Some(0x8086), Some(0x1041), 2, 2, Some(bits)), "not virtio");
        assert!(af_xdp_requested_in("-a 0000:01:00.0 --vdev=net_af_xdp"));
        assert!(!af_xdp_requested_in("--vdev=net_af_xdp0"), "narrow vdev name");
        assert!(af_xdp_requested_in("--vdev=net_af_xdp,iface=eth0"));
        assert!(!af_xdp_requested_in("-a 0000:01:00.0"));
    }

    #[test]
    fn qualified_virtio_pair_passes() {
        let bits = (1u64 << 15) | (1u64 << 16);
        assert!(virtio_qualified(Some(0x1af4), Some(0x1041), 2, 2, Some(bits)));
        let v = verdict_with_policy(
            HugePages { total: 4, free: 4 },
            u64::MAX,
            2,
            Vec::new(),
            false,
            false,
            false,
            false,
            Some((6, 8)),
            Some(true),
        );
        assert_eq!(v.class, HardwareClass::Supported);
    }

    #[test]
    fn legacy_or_insufficient_virtio_fresh_warns_configured_blocks() {
        let fresh = verdict_with_policy(
            HugePages { total: 4, free: 4 },
            u64::MAX,
            0,
            vec!["0000:00:03.0 virtio (legacy)".to_string()],
            true,
            false,
            false,
            false,
            Some((6, 8)),
            Some(true),
        );
        assert_eq!(fresh.class, HardwareClass::Fallback);
        assert!(fresh.warnings.iter().any(|w| w.contains("unsupported dataplane NIC")));
        let configured = verdict_with_policy(
            HugePages { total: 4, free: 4 },
            u64::MAX,
            0,
            vec!["0000:00:03.0 virtio (legacy)".to_string()],
            false,
            false,
            false,
            false,
            Some((6, 8)),
            Some(true),
        );
        assert_eq!(configured.class, HardwareClass::Blocked);
    }

    #[test]
    fn af_xdp_blocks_and_the_lab_override_warns() {
        let blocked = verdict_with_policy(
            HugePages { total: 4, free: 4 },
            u64::MAX,
            2,
            Vec::new(),
            false,
            false,
            true,
            false,
            Some((6, 8)),
            Some(true),
        );
        assert_eq!(blocked.class, HardwareClass::Blocked);
        assert!(blocked.blocks.iter().any(|b| b.contains("AF_XDP")));
        let allowed = verdict_with_policy(
            HugePages { total: 4, free: 4 },
            u64::MAX,
            2,
            Vec::new(),
            false,
            false,
            true,
            true,
            Some((6, 8)),
            Some(true),
        );
        assert_eq!(allowed.class, HardwareClass::Fallback);
        assert!(allowed.warnings.iter().any(|w| w.contains("lab override")));
    }

    #[test]
    fn on_a_stick_zero_supported_configured_blocks_fresh_warns() {
        let configured = verdict_with_policy(
            HugePages { total: 4, free: 4 },
            u64::MAX,
            0,
            vec!["0000:00:03.0 virtio (legacy)".to_string()],
            false,
            true,
            false,
            false,
            Some((6, 8)),
            Some(true),
        );
        assert_eq!(configured.class, HardwareClass::Blocked);
        let fresh = verdict_with_policy(
            HugePages { total: 4, free: 4 },
            u64::MAX,
            0,
            vec!["0000:00:03.0 virtio (legacy)".to_string()],
            true,
            true,
            false,
            false,
            Some((6, 8)),
            Some(true),
        );
        assert_eq!(fresh.class, HardwareClass::Fallback);
    }

    #[test]
    fn on_a_stick_one_supported_port_is_a_warn_not_a_block() {
        let v = verdict_with_policy(
            HugePages { total: 4, free: 4 },
            u64::MAX,
            1,
            Vec::new(),
            false,
            true,
            false,
            false,
            Some((6, 8)),
            Some(true),
        );
        assert_eq!(v.class, HardwareClass::Fallback);
        assert!(v.warnings.iter().any(|w| w.contains("on-a-stick")));
        let configured_pair = verdict_with_policy(
            HugePages { total: 4, free: 4 },
            u64::MAX,
            1,
            Vec::new(),
            false,
            false,
            false,
            false,
            Some((6, 8)),
            Some(true),
        );
        assert_eq!(configured_pair.class, HardwareClass::Blocked);
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
        let v = verdict_with_policy(
            HugePages::default(),
            0,
            1,
            Vec::new(),
            false,
            false,
            false,
            false,
            Some((6, 8)),
            None,
        );
        assert_eq!(v.class, HardwareClass::Blocked);
        assert!(v.blocks.iter().any(|b| b.contains("available")));
        assert!(v.blocks.iter().any(|b| b.contains("PAIR")));
    }

    #[test]
    fn parses_mem_available() {
        let text = "MemTotal:       16333628 kB\nMemAvailable:    8388608 kB\n";
        assert_eq!(parse_meminfo_available_kb(text), 8_388_608);
    }
}
