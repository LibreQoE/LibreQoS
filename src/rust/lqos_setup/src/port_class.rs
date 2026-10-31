//! The zero-manual-ports port-class detector + decision table
//! (`docs/zero_manual_ports_installer_card.md` §A).
//!
//! PURE over the facts the caller supplies (the kernel driver, the IOMMU
//! presence, the lqos.conf netdevs) so the whole decision surface is
//! unit-testable and the installer derives — never asks the operator for —
//! the engine path, the kernel-driver binding, the EAL args, the SWAP rule
//! and the hugepage sizing.
//!
//! The site inputs are ONLY `lqos.conf`'s `to_internet` / `to_network`
//! netdevs and the world; everything here is DERIVED.

use crate::nic_bind::BindMode;
use std::path::{Path, PathBuf};

/// The detected dataplane port class (§A decision table).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortClass {
    /// Mellanox ConnectX (`mlx5_core`): the NICs STAY in place (verbs/PCI).
    Mlx5,
    /// Intel `i40e`/`ice`/`ixgbe`/`igb`: the PCI class (`vfio-pci`).
    Intel,
    /// `virtio_net` (a Proxmox VM): `uio_pci_generic` + the net_virtio PMD.
    Virtio,
}

impl PortClass {
    /// The class for a **Tier-1 listed** kernel driver name, or `None` = not on
    /// the official list (the caller then tries Tier 2). The listed set is
    /// exactly A0's table: ConnectX (mlx5), X710/XL710/XXV710 (i40e), X520
    /// (ixgbe), the virtio-net pair. `ice`/`igb`/`e1000e` are the community
    /// class (Tier 2), not Tier 1.
    pub fn detect(driver: &str) -> Option<Self> {
        match driver {
            "mlx5_core" => Some(Self::Mlx5),
            "i40e" | "ixgbe" => Some(Self::Intel),
            "virtio_net" | "virtio-pci" => Some(Self::Virtio),
            _ => None,
        }
    }
}

/// A0 (the tiering ruling, docs/zero_manual_ports_installer_card.md): the
/// three-tier NIC policy. REFUSE only when
/// there is truly no dataplane path — a bond, or no PMD AND no kernel netdev.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NicTier {
    /// **Tier 1** — the v2.0-listed controllers: fully supported, no caveats.
    Tier1(PortClass),
    /// **Tier 2** — a known DPDK-PMD PCI NIC off the official list: PROCEED via
    /// the same PCI/uio derivation, with a one-line "community class" notice.
    Tier2 {
        /// The PCI class the derivation treats it as.
        class: PortClass,
        /// The DPDK PMD it uses.
        pmd: &'static str,
    },
    /// **Tier 3** — a kernel netdev with NO DPDK PMD: the compatibility engine
    /// (AF_XDP/af_packet) with a PROMINENT warning. Never chosen for a Tier-1/2
 /// NIC (the shared host AF_XDP lesson).
    Tier3,
    /// **REFUSE** — no dataplane path exists.
    Refuse(RefuseReason),
}

/// Why the installer refuses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefuseReason {
    /// A bonded interface: a topology refusal, not a driver gap.
    Bonded,
    /// No DPDK PMD AND no kernel netdev.
    NoDataplanePath,
}

/// A0: the one-line "community class" notice text (Tier 2).
pub const TIER2_NOTICE: &str =
    "community class: not on the official support list — DPDK {pmd} detected";

/// A0: the Tier-3 prominent-warning text.
pub const TIER3_WARNING: &str = "WARNING: no DPDK PMD for this NIC — falling \
back to the compatibility engine (AF_XDP/af_packet). Performance and feature \
caveats apply (the AF_XDP adapter is unfinished on virtio).";

/// The known DPDK PCI PMD driver classes (Tier 2 when off the official list).
/// The official Tier-1 drivers are matched first, so this list only ever sees
/// the community class.
fn tier2_pmd(driver: &str) -> Option<&'static str> {
    Some(match driver {
        "ice" => "ice",
        "igb" => "igb",
        "e1000e" => "e1000",
        "vmxnet3" => "vmxnet3",
        "mlx4_core" => "mlx4",
        "ena" => "ena",
        "bnxt_en" => "bnxt",
        "qede" => "qede",
        "sfc" => "sfc",
        _ => return None,
    })
}

impl NicTier {
    /// Classify a kernel driver name (and whether a kernel netdev is present on
    /// this host) against the A0 three-tier policy.
    pub fn classify(driver: &str, has_netdev: bool) -> Self {
        if driver == "bonding" || driver.starts_with("bond") {
            return Self::Refuse(RefuseReason::Bonded);
        }
        if let Some(class) = PortClass::detect(driver) {
            return Self::Tier1(class);
        }
        if let Some(pmd) = tier2_pmd(driver) {
            // Reuse the PCI derivation; the class follows the driver family.
            let class = match pmd {
                "vmxnet3" => PortClass::Intel,
                _ => PortClass::Intel,
            };
            return Self::Tier2 { class, pmd };
        }
        if has_netdev {
            return Self::Tier3;
        }
        Self::Refuse(RefuseReason::NoDataplanePath)
    }

    /// The class for a Tier-1/2 NIC (the caller's happy path).
    pub fn class(self) -> Option<PortClass> {
        match self {
            Self::Tier1(class) => Some(class),
            Self::Tier2 { class, .. } => Some(class),
            _ => None,
        }
    }
}

/// A0: a VM's virtio multiqueue count must equal the vCPU count (the doc's
/// note). A mismatch is a WARN (the shared host box was 8/8).
pub fn multiqueue_matches(rx_queues: usize, vcpus: usize) -> bool {
    rx_queues == vcpus
}

/// The DEFAULT Tier-2 binding (the installer card's E2 condition 2). The DPDK
/// `vfio-pci` is driver-agnostic, so the community class uses the same
/// IOMMU-aware rule as Intel — with per-family overrides: the mlx5 family runs
/// IN PLACE (verbs; never bound) and the virtio family uses `uio_pci_generic`.
pub fn tier2_bind(pmd: &str, iommu_present: bool) -> Option<BindMode> {
    match pmd {
        "mlx5" | "mlx4" => None,
        "net_virtio" | "virtio" => Some(BindMode::Uio),
        _ => Some(if iommu_present {
            BindMode::Vfio
        } else {
            BindMode::VfioNoiommu
        }),
    }
}

/// The Tier-3 compatibility engine's capability (E2 condition 3). A Tier-3
/// install may FORWARD WITHOUT SHAPING — the warning must say which.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompatEngine {
    /// DPDK + AF_XDP: CAN shape (the pin-first fix applies); unfinished on
    /// virtio.
    AfXdpShaping,
    /// The Rust `af_packet` path: forwards only, NO shaping.
    AfPacketPassthrough,
}

impl CompatEngine {
    /// `true` when the compatibility engine actually shapes.
    pub fn provides_shaping(self) -> bool {
        matches!(self, Self::AfXdpShaping)
    }
}

/// The boot assert (E2 condition 4b): a Tier-1/2 NIC must NEVER be on the
/// compatibility engine. Errors with the fix — the shared host bug cannot recur.
pub fn assert_tier12_not_compat(tier: NicTier, on_compat: bool) -> Result<(), String> {
    if on_compat && tier.class().is_some() {
        return Err(
            "a Tier-1/2 NIC is running on the compatibility engine — a stale \
             AF_XDP/vdev posture from a previous install; rerun libreqos-setup"
                .into(),
        );
    }
    Ok(())
}

/// The upgrade cleanup (E2 condition 4a): when a Tier-1/2 NIC is detected,
/// purge any stale AF_XDP/vdev posture (the retro's stale-drop-in class).
pub fn purge_stale_compat_for(tier: NicTier) -> bool {
    tier.class().is_some()
}

/// WAS — worker auto-sizing (docs/worker_autosizing_card.md, the
/// link-speed band. This is the sizing AUTHORITY (in shape); the env writer
/// calls it so the operator never hand-writes `--workers`.
pub fn rate_workers(link_mbps: u64) -> u32 {
    match link_mbps {
        0..=1_000 => 1,
        1_001..=5_000 => 2,
        5_001..=20_000 => 4,
        20_001..=100_000 => 8,
        _ => 16, // beyond the qualified envelope; the caller emits the WARN
    }
}

/// The per-W SHAPED delivered ceiling (docs/campaign_100g_cell2_findings.md,
/// 2800-active, the 7950X): cell 2 measured `W{1,4,8,16}` = `{1.07, 6.09,
/// 21.18, 42.65} Gbps`. This IS the small-job rule: a 1G link maps to W=1 with
/// no special case. **W=2/3 are EXTRAPOLATED** (not measured under shaping).
pub const CELL2_SHAPED_DELIVERED_MBPS: &[(u32, u64)] =
    &[(1, 1_070), (4, 6_090), (8, 21_180), (16, 42_650)];

/// Beyond the measured table the count saturates at the top row (16).
pub const W_MAX_PLACEHOLDER: u32 = 16;

/// The smallest MEASURED/extrapolated W whose cell-2 delivered ceiling meets
/// the link's demand. The demand rule — never a min/max of unrelated bands.
pub fn shaped_ceiling(link_mbps: u64) -> u32 {
    for (w, delivered) in CELL2_SHAPED_DELIVERED_MBPS {
        if *delivered >= link_mbps {
            return *w;
        }
    }
    W_MAX_PLACEHOLDER
}

/// `true` when the link demand lies strictly BETWEEN two adjacent measured
/// ceilings: the chosen W is the next MEASURED point and a LOWER unmeasured W
/// (2/3/5/6/7/9..15) may be the true optimum — extrapolated, never computed
/// (the implementer pin 1). An exact measured demand is NOT a gap.
pub fn demand_in_measured_gap(link_mbps: u64) -> bool {
    let mut prev = 0u64;
    for (_, g) in CELL2_SHAPED_DELIVERED_MBPS {
        if link_mbps <= *g {
            return prev > 0 && link_mbps > prev && link_mbps != *g;
        }
        prev = *g;
    }
    false // beyond the table: its own loud WARN
}

/// `true` when the link demand EXCEEDS the measured shaped envelope (the
/// table's top, W=16 = 42.65G): no measured W satisfies it, so the count caps
/// at W_MAX_PLACEHOLDER and MUST warn (never silently under-provision — the
/// WAS undersized-box path).
pub fn beyond_measured_envelope(link_mbps: u64) -> bool {
    link_mbps
        > CELL2_SHAPED_DELIVERED_MBPS
            .last()
            .map(|(_, g)| *g)
            .unwrap_or(0)
}

/// The reserved cores (`physical_cores - 2`): OS/mgmt-IRQs + bridge-control/
/// lqosd. The physical/HT-collapsed count is the caller's input.
pub const CORE_RESERVE: u32 = 2;

/// Which quantity bound the derived count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CountBound {
    /// The measured ceiling met the demand (no cap below it).
    MeasuredCeiling,
    /// The physical cores minus the reserve capped below the ceiling.
    Cores,
    /// The NIC/VM queue count capped below the ceiling.
    Queues,
}

/// The B target before the hard caps: the cell-2 table, raised for links above
/// the W1 ceiling to at least `ceil(link/250)`. `<=1070` is unchanged.
pub fn worker_target(link_mbps: u64) -> u32 {
    let table = shaped_ceiling(link_mbps) as u64;
    if link_mbps > CELL2_SHAPED_DELIVERED_MBPS[0].1 {
        let anchor = link_mbps.div_ceil(250);
        table.max(anchor).min(u32::MAX as u64) as u32
    } else {
        table.min(u32::MAX as u64) as u32
    }
}

/// Classify which quantity bound the derived count (pre-existing table rule).
pub fn count_bound(link_mbps: u64, physical_cores: u32, nic_queue_count: u32) -> CountBound {
    let ceiling = shaped_ceiling(link_mbps);
    let cores = physical_cores.saturating_sub(CORE_RESERVE).max(1);
    let queues = nic_queue_count.max(1);
    if cores.min(queues) >= ceiling {
        CountBound::MeasuredCeiling
    } else if cores <= queues {
        CountBound::Cores
    } else {
        CountBound::Queues
    }
}

/// The derived worker count (B rule):
/// `min(cores-2, queues, max(table, ceil(link/250)))` for link > 1070;
/// `min(table, cores-2, queues)` below it. The small-job rule is INSIDE the
/// table (1G ⇒ W=1).
pub fn derived_workers(link_mbps: u64, physical_cores: u32, nic_queue_count: u32) -> u32 {
    let cores = physical_cores.saturating_sub(CORE_RESERVE).max(1);
    let queues = nic_queue_count.max(1);
    worker_target(link_mbps).min(cores).min(queues).max(1)
}

/// The legacy WAS-only count (`max(1, min(rate_workers(link), cores-2))`),
/// retained for comparison/tests; [`derived_workers`] is what the writer uses.
pub fn was_workers(link_mbps: u64, physical_cores: u32) -> u32 {
    let reserve = physical_cores.saturating_sub(CORE_RESERVE).max(1);
    rate_workers(link_mbps).min(reserve).max(1)
}

/// The loud LOW-BAND warning (docs/worker_autosizing_card.md + the cell-2
/// evidence): under SHAPING the low band is a MEASURED COLLAPSE — W=1 ~1.0-1.1
/// Gbps (the shard-tin single-worker ingest ceiling) and W=2 is UNMEASURED under
/// shaping (cell 1's empty-world W=2 pathology). Never treat W=1 as conservative.
pub fn low_worker_warning(link_mbps: u64, workers: u32) -> Option<&'static str> {
    // Condition on the LINK, not the cores: W=1 delivers ~1.0-1.1G, exactly a
    // 1G link's capacity, so a 1G box SHOULD stay at W=1 (no warning). Only a
    // demand ABOVE the W=1 shaped ceiling makes the low band pathological.
    let w1_ceiling_mbps = CELL2_SHAPED_DELIVERED_MBPS[0].1;
    (workers <= 2 && link_mbps > w1_ceiling_mbps).then_some(
        "WARNING: derived worker count is 1-2; under SHAPING this low band is a measured \
         collapse (W=1 ~1.0G; W=2 unmeasured under shaping). If this host has spare PHYSICAL \
         cores, set LIBREQOS_DPDK_WORKERS explicitly.",
    )
}

/// `lqos-74t.42`: the VM/virtio install-time warning.
///
/// A hypervisor/virtio NIC that derives **more than 2** workers pays the `.41`
/// per-worker handoff-ring cost (`workers*(workers-1)*2`, monotone) on a small
/// VM. The `> 2` cutoff makes this the EXACT complement of
/// [`low_worker_warning`]'s `W <= 2` domain, so the two never double-warn:
/// at/below 2 the throughput (low-band) risk dominates; above it the VM memory
/// cost does. For a `link > 1070` VM the two bracket a genuine
/// memory-vs-throughput TRADEOFF — so the text names it and NEVER recommends
/// `workers = 1` unconditionally (that cap would trip the low-band warning).
pub fn virtio_worker_warning(class: PortClass, workers: u32) -> Option<&'static str> {
    (class == PortClass::Virtio && workers > 2).then_some(
        "WARNING: this is a virtio/hypervisor NIC and the derived worker count exceeds 2. \
         On a small VM each worker adds handoff rings (workers*(workers-1)*2), so a high \
         count costs memory. If the VM is memory-bound, cap it with the lqos.conf \
         [queues] workers key (or LIBREQOS_DPDK_WORKERS) -- but do NOT go below the count \
         the low-band warning names for a link demand above ~1 Gbps, or shaping throughput \
         collapses.",
    )
}

/// `lqos-74t.160`: where the written worker count came from.
///
/// The managed `50-ports.conf` is never an ONGOING source: it is a one-time
/// migration input only (see [`resolve_worker_source`]). A derived count must
/// not freeze as an override just because an earlier render wrote it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkersSource {
    /// Table/cores/queues; no operator key.
    Derived,
    /// `[queues] workers` in lqos.conf (the only ongoing operator key).
    Override,
    /// A legacy `LIBREQOS_DPDK_WORKERS` from the packaged `bridge.env`, or a
    /// one-time adoption from the managed `50-ports.conf`.
    LegacyEnv,
}

/// The one-time adoption outcome for this render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerAdoption {
    /// No adoption attempted / not applicable.
    None,
    /// `[queues] workers=N` was written (or is being written) from a legacy
    /// source; this render emits the adoption note.
    Adopted,
    /// The legacy W equalled the derived W and was ignored; the check stamp is
    /// written so a later rate/core change cannot adopt it.
    IgnoredEqual,
    /// The legacy W was below the derived W and was ignored (B wins); the
    /// check stamp is written.
    IgnoredLower,
}

/// The pure provenance decision (unit-tested without touching `/etc`).
///
/// Inputs are the config key, the **main** `bridge.env` key, the managed
/// `50-ports.conf` key, whether the one-time check already ran, and the W that
/// would be derived now.
pub fn resolve_worker_source(
    config_workers: Option<u32>,
    main_env_workers: Option<u32>,
    managed_workers: Option<u32>,
    adoption_checked: bool,
    derived: u32,
) -> (u32, WorkersSource, WorkerAdoption) {
    fn positive(n: u32) -> Option<u32> {
        (n > 0).then_some(n)
    }
    if let Some(w) = config_workers.and_then(positive) {
        return (w, WorkersSource::Override, WorkerAdoption::None);
    }
    // Legacy sources are read ONLY before the one-time stamp exists. After it,
    // a stale main/managed W can never lower (or raise) B.
    if !adoption_checked {
        let decide = |w: u32| {
            if w > derived {
                (w, WorkersSource::LegacyEnv, WorkerAdoption::Adopted)
            } else if w == derived {
                (derived, WorkersSource::Derived, WorkerAdoption::IgnoredEqual)
            } else {
                (derived, WorkersSource::Derived, WorkerAdoption::IgnoredLower)
            }
        };
        if let Some(w) = main_env_workers.and_then(positive) {
            return decide(w);
        }
        if let Some(w) = managed_workers.and_then(positive) {
            return decide(w);
        }
    }
    (derived, WorkersSource::Derived, WorkerAdoption::None)
}

/// The facts the wiring slice gathers (B.4.1): the link demand from lqos.conf's
/// `max(downlink, uplink)` bandwidth keys, the HT-COLLAPSED physical core count,
/// the A0 NIC/VM queue fact, and the resolved worker provenance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortsPlanFacts {
    /// The worker-sizing link: **`max(down, up)`**, not the sum. The 250-anchor
    /// is a per-direction rule; a 2.5G/2.5G box sizes at 2.5G. The value is also
    /// used for the warnings/log only.
    pub link_mbps: u64,
    /// HT-collapsed PHYSICAL cores.
    pub physical_cores: u32,
    /// The NIC/VM queue count (A0 multiqueue fact).
    pub nic_queue_count: u32,
    /// The worker count to write.
    pub workers: u32,
    /// Where that count came from.
    pub workers_source: WorkersSource,
    /// The one-time adoption outcome for this render.
    pub worker_adoption: WorkerAdoption,
    /// `false` when the cpu topology was unreadable (the caller WARNs).
    pub topology_ok: bool,
}

/// The outcome of a plan write (what the operator should be told).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ManagedPlanOutcome {
    /// The worker count written.
    pub workers: u32,
    /// `true` when the operator override won.
    pub overridden: bool,
    /// The provenance of the written count.
    pub source: WorkersSource,
}

/// The HT-COLLAPSED physical core count from sysfs (one per `topology/core_id`
/// per socket). The rule must never see a LOGICAL count.
pub fn physical_core_count_at(cpu_root: &Path) -> (u32, bool) {
    use std::collections::BTreeSet;
    // Key by (physical_package_id, core_id): core_id alone is PER-PACKAGE, so a
    // 2x8-core box would collapse to 8. Returns (count, topology_ok): a MISSING
    // topology dir must WARN, never silently under-derive to 1.
    let mut cores: BTreeSet<(String, String)> = BTreeSet::new();
    let mut topology_ok = false;
    let mut any_cpu = false;
    if let Ok(entries) = std::fs::read_dir(cpu_root) {
        for e in entries.flatten() {
            let name = e.file_name();
            let n = name.to_string_lossy();
            let tail = n.strip_prefix("cpu").unwrap_or("");
            if tail.is_empty() || !tail.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            any_cpu = true;
            let topo = e.path().join("topology");
            if !topo.exists() {
                continue;
            }
            topology_ok = true;
            let read = |f: &str| {
                std::fs::read_to_string(topo.join(f))
                    .ok()
                    .map(|v| v.trim().to_string())
                    .unwrap_or_default()
            };
            cores.insert((read("physical_package_id"), read("core_id")));
        }
    }
    let count = if topology_ok {
        (cores.len() as u32).max(1)
    } else {
        // No topology: the logical CPU count is the honest fallback, and the
        // caller WARNS (the flag is false).
        let logical = if any_cpu {
            std::fs::read_dir(cpu_root)
                .map(|d| {
                    d.flatten()
                        .filter(|e| {
                            let n = e.file_name();
                            let n = n.to_string_lossy();
                            n.strip_prefix("cpu")
                                .map(|t| !t.is_empty() && t.chars().all(|c| c.is_ascii_digit()))
                                .unwrap_or(false)
                        })
                        .count() as u32
                })
                .unwrap_or(1)
        } else {
            1
        };
        logical.max(1)
    };
    (count, topology_ok)
}

/// The live HT-collapsed physical core count (`/sys/devices/system/cpu`).
pub fn physical_core_count() -> u32 {
    physical_core_count_at(Path::new("/sys/devices/system/cpu")).0
}

/// The A0 NIC queue FACT: the RX queue count of the netdev owning `bdf` (so
/// `--queues` is not a manual act). None when the netdev is absent (bound).
pub fn nic_rx_queue_count(bdf: &str) -> Option<u32> {
    let net_dir = format!("/sys/bus/pci/devices/{bdf}/net");
    let dev = std::fs::read_dir(&net_dir)
        .ok()?
        .flatten()
        .next()?
        .file_name();
    let queues = format!("/sys/class/net/{}/queues", dev.to_string_lossy());
    let n = std::fs::read_dir(queues)
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("rx-"))
        .count();
    (n > 0).then_some(n as u32)
}

/// IOMMU presence (the same probe `nic_bind::decide_mode` uses). The installer
/// must never HARDCODE this: an Intel host with no IOMMU needs VfioNoiommu, and
/// a hardcoded `true` selects plain vfio, whose bind then fails.
pub fn iommu_present() -> bool {
    std::fs::read_dir("/sys/kernel/iommu_groups")
        .map(|mut e| e.next().is_some())
        .unwrap_or(false)
}

/// Parse a positive worker count from an env-style value.
fn parse_positive_u32(value: Option<String>) -> Option<u32> {
    value
        .and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|n| *n > 0)
}

/// Read one key from the MANAGED `50-ports.conf` only. This is the one-time
/// migration input for worker provenance; it must never become a runtime
/// source (the unit's drop-in glob is for the bridge, not for `lqos_setup`).
fn managed_dropin_value_at(path: &Path, key: &str) -> Option<String> {
    crate::bootstrap::env_file_value(path, key)
}

/// The file surfaces `gather_facts` reads. Production uses [`Default`]; unit
/// tests inject temp paths (the live-W8 and dry-run no-write fixtures).
#[derive(Clone, Debug)]
pub struct GatherEnv {
    /// Packaged `/etc/libreqos/bridge.env` (runtime LegacyEnv source).
    pub main_env_path: PathBuf,
    /// Managed `/etc/libreqos/bridge.env.d/50-ports.conf` (one-time input).
    pub managed_path: PathBuf,
    /// `dpdk-bind-originals.env` (persisted BDF/queue facts).
    pub state_path: PathBuf,
    /// The resolved `lqos.conf` path (`lqos_config::config_path()`).
    pub conf_path: PathBuf,
    /// The one-time worker-adoption stamp.
    pub stamp_path: PathBuf,
    /// Test-only physical-core override; production reads sysfs.
    pub physical_cores_override: Option<u32>,
}

impl Default for GatherEnv {
    fn default() -> Self {
        Self {
            main_env_path: PathBuf::from(crate::bootstrap::BRIDGE_ENV_PATH),
            managed_path: PathBuf::from(MANAGED_DROPIN),
            state_path: PathBuf::from(crate::bootstrap::BIND_ORIGINALS_STATE),
            conf_path: PathBuf::from(lqos_config::config_path()),
            stamp_path: PathBuf::from(crate::config_migrate::WORKER_ADOPTION_STAMP),
            physical_cores_override: None,
        }
    }
}

/// Resolve the NIC/VM queue fact without requiring a post-bind netdev.
///
/// Precedence: explicit `--queues N` → persisted
/// `LQOS_DPDK_NIC_QUEUES` (89's bind-time STATE) → the managed
/// `LIBREQOS_DPDK_RX_QUEUES` once → live `nic_rx_queue_count`. The managed
/// fallback is what lets the one-time W comparison run on an already-bound
/// box, where `/sys/.../net` is gone.
fn pick_queue_count(
    queue_override: Option<u32>,
    persisted: Option<u32>,
    managed: Option<u32>,
    live: Option<u32>,
) -> Result<u32, String> {
    queue_override
        .filter(|q| *q > 0)
        .or(persisted.filter(|q| *q > 0))
        .or(managed.filter(|q| *q > 0))
        .or(live.filter(|q| *q > 0))
        .ok_or_else(|| {
            "NIC queue count undetectable (the netdev is already bound?); pass --queues N"
                .to_string()
        })
}

fn resolve_queue_count(
    wan: &str,
    queue_override: Option<u32>,
    env: &GatherEnv,
) -> Result<u32, String> {
    pick_queue_count(
        queue_override,
        parse_positive_u32(crate::bootstrap::state_value_at(
            &env.state_path,
            "LQOS_DPDK_NIC_QUEUES",
        )),
        parse_positive_u32(managed_dropin_value_at(
            &env.managed_path,
            "LIBREQOS_DPDK_RX_QUEUES",
        )),
        nic_rx_queue_count(wan),
    )
}

/// THE ONE FACT GATHER (the implementer (1c)) shared by the CLI and the setup flow so
/// they cannot drift. Worker provenance is explicit (`WorkersSource`): the
/// config's `[queues] workers` is the only ongoing operator key; a main
/// `bridge.env` W is `LegacyEnv`; the managed `50-ports.conf` W is a ONE-TIME
/// migration input (adopted only if it differs from the W derived now).
///
/// A bound netdev is a NAMED error for the queue fact (the implementer (1b)) — NEVER a
/// silent 1 (which would cap the workers at the W=1 collapse) — unless the
/// explicit/persisted/managed fallback above supplies it.
pub fn gather_facts(
    cfg: &lqos_config::Config,
    wan_bdf: &str,
    queue_override: Option<u32>,
    apply_adoption: bool,
) -> Result<PortsPlanFacts, String> {
    gather_facts_with(
        cfg,
        wan_bdf,
        queue_override,
        apply_adoption,
        &GatherEnv::default(),
    )
}

fn gather_facts_with(
    cfg: &lqos_config::Config,
    wan_bdf: &str,
    queue_override: Option<u32>,
    apply_adoption: bool,
    env: &GatherEnv,
) -> Result<PortsPlanFacts, String> {
 // The WAN BDF is a PARAM (the implementer final #12): the CLI passes --wan and the
    // setup flow passes bridge.env's value, so the queue/driver facts are always
    // read from the SAME NIC the block is written for -- never a silent mismatch.
    let wan = wan_bdf.trim().to_string();
    if wan.is_empty() {
        return Err("no WAN BDF (bridge.env or --wan); run the bind step first".into());
    }
    let down = cfg.queues.downlink_bandwidth_mbps;
    let up = cfg.queues.uplink_bandwidth_mbps;
    // Worker sizing uses PER-DIRECTION max(down, up), not the sum. A decimal
    // config value rounds HERE only (the exact rate stays in lqos.conf).
    let link_mbps = if down <= 0.0 && up <= 0.0 {
        return Err(
            "no bandwidth keys in lqos.conf; set downlink_bandwidth_mbps/uplink_bandwidth_mbps"
                .into(),
        );
    } else {
        down.max(up).round() as u64
    };
    let (physical_cores, topology_ok) = match env.physical_cores_override {
        Some(cores) => (cores, true),
        None => physical_core_count_at(Path::new("/sys/devices/system/cpu")),
    };
    let nic_queue_count = resolve_queue_count(&wan, queue_override, env)?;
    let derived = derived_workers(link_mbps, physical_cores, nic_queue_count);

    let checked = crate::config_migrate::worker_adoption_checked_at(&env.stamp_path);
    let main_env_workers = parse_positive_u32(crate::bootstrap::bridge_env_value_main_at(
        &env.main_env_path,
        crate::config_migrate::ENV_WORKERS,
    ));
    let managed_workers = parse_positive_u32(managed_dropin_value_at(
        &env.managed_path,
        crate::config_migrate::ENV_WORKERS,
    ));
    let (workers, workers_source, mut worker_adoption) = resolve_worker_source(
        cfg.queues.workers,
        main_env_workers,
        managed_workers,
        checked,
        derived,
    );

    // A dry-run previews the adoption but must not claim it wrote lqos.conf;
    // emit the legacy note instead so the operator still sees the action.
    if !apply_adoption && worker_adoption == WorkerAdoption::Adopted {
        worker_adoption = WorkerAdoption::None;
    }

    let legacy_origin = if main_env_workers.is_some() {
        "main bridge.env"
    } else {
        "managed 50-ports"
    };

    if apply_adoption {
        match worker_adoption {
            WorkerAdoption::Adopted => {
                let origin = legacy_origin;
                let conf_path = &env.conf_path;
                let stamp_path = &env.stamp_path;
                if let Err(err) =
                    crate::config_migrate::adopt_workers_at(conf_path, stamp_path, workers, origin)
                {
                    eprintln!(
                        "ports-plan: WARNING could not adopt legacy worker count {workers} from \
                         {origin} into [queues] workers at {}: {err}; the legacy value is used \
                         for this render and the adoption will be retried",
                        conf_path.display()
                    );
                    worker_adoption = WorkerAdoption::None;
                }
            }
            WorkerAdoption::IgnoredEqual | WorkerAdoption::IgnoredLower => {
                let status = if worker_adoption == WorkerAdoption::IgnoredEqual {
                    "ignored-equal"
                } else {
                    "ignored-lower"
                };
                if let Err(err) = crate::config_migrate::stamp_worker_adoption_at(
                    &env.stamp_path,
                    status,
                    Some(workers),
                    Some(derived),
                    Some(legacy_origin),
                ) {
                    eprintln!(
                        "ports-plan: WARNING could not write the worker-adoption stamp {}: {err}",
                        env.stamp_path.display()
                    );
                }
            }
            WorkerAdoption::None => {
                if !checked {
                    let status = if workers_source == WorkersSource::Override {
                        "override-present"
                    } else {
                        "absent"
                    };
                    if let Err(err) = crate::config_migrate::stamp_worker_adoption_at(
                        &env.stamp_path,
                        status,
                        Some(workers),
                        Some(derived),
                        None,
                    ) {
                        eprintln!(
                            "ports-plan: WARNING could not write the worker-adoption stamp {}: {err}",
                            env.stamp_path.display()
                        );
                    }
                }
            }
        }
    }

    Ok(PortsPlanFacts {
        link_mbps,
        physical_cores,
        nic_queue_count,
        workers,
        workers_source,
        worker_adoption,
        topology_ok,
    })
}

/// The kernel driver currently bound to `bdf` (pre-bind), if any.
pub fn driver_of_bdf(bdf: &str) -> Option<String> {
    std::fs::read_link(format!("/sys/bus/pci/devices/{bdf}/driver"))
        .ok()?
        .file_name()
        .and_then(|n| n.to_str())
        .map(ToOwned::to_owned)
}

/// The SETUP-FLOW plan write (`docs/zero_manual_ports_installer_card.md` B.4):
/// called after the BDF + bandwidth keys exist. Reads the BDFs from bridge.env,
/// the link demand from the passed config's bandwidth keys, the driver + A0
/// queue fact from the host, the HT-collapsed physical cores from sysfs, and the
/// operator's override from the same config. Writes MANAGED_DROPIN atomically.
/// Returns a human message (the caller logs it); never panics.
/// The persisted device pair for the managed plan. An on-a-stick trunk has ONE
/// device (the same BDF in both keys); a missing LAN is that same device, never
/// a reason to skip the plan. A two-port box must still carry both.
fn plan_bdf_pair(
    wan: Option<String>,
    lan: Option<String>,
) -> Result<(String, String), String> {
    let Some(wan) = wan else {
        return Err("no WAN BDF in bridge.env yet; run the bind step first".into());
    };
    let lan = lan.unwrap_or_else(|| wan.clone());
    Ok((wan, lan))
}

pub fn write_plan_from_setup(
    cfg: &lqos_config::Config,
) -> Result<(ManagedPlanOutcome, String), String> {
    use crate::bootstrap::bridge_env_value;
    let wan = bridge_env_value("LQOS_DPDK_WAN_BDF").filter(|v| !v.trim().is_empty());
    let lan = bridge_env_value("LQOS_DPDK_LAN_BDF").filter(|v| !v.trim().is_empty());
    let (wan, lan) = plan_bdf_pair(wan, lan)?;
    let wan_bdf = wan.clone();
    let facts = gather_facts(cfg, &wan_bdf, None, true)?;
    // Post-bind the netdev is gone and `driver_of_bdf` reports uio/vfio. The
    // class comes from the pre-bind driver persisted beside the pair (the bind
    // script/Rust both record it); never silently default a bound virtio to
    // mlx5.
    let driver = driver_of_bdf(&wan)
        .filter(|driver| !crate::nic_bind::is_dpdk_driver(driver))
        .or_else(|| bridge_env_value("LQOS_DPDK_ORIG_WAN_DRIVER"))
        .or_else(|| crate::bootstrap::state_original_driver("WAN"))
        .unwrap_or_else(|| "mlx5_core".into());
    let Some(plan) = plan(&driver, iommu_present()) else {
        return Err(format!("unsupported NIC driver '{driver}' (Tier 3)"));
    };
    let out = write_managed_ports_plan_at(Path::new(MANAGED_DROPIN), &plan, &wan, &lan, &facts)
        .map_err(|e| format!("ports-plan write failed: {e}"))?;
    let mut msg = format!(
        "Ports plan written to {MANAGED_DROPIN}: workers={}{} (link={}Mbps cores={} queues={} driver={driver})",
        out.workers,
        if out.overridden { " (override)" } else { "" },
        facts.link_mbps,
        facts.physical_cores,
        facts.nic_queue_count
    );
    if !facts.topology_ok {
        msg.push_str(" [WARN: cpu topology unavailable -- logical count used]");
    }
    Ok((out, msg))
}

/// Write `content` to `path` ATOMICALLY (temp + rename) so a concurrent reader
/// (the bridge unit's EnvironmentFile glob) never sees a partial file.
pub fn write_managed_dropin_at(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("conf.tmp");
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path)
}

/// Gather -> derive -> write the managed drop-in. The operator's explicit
/// `workers` override WINS over the derived default; the override landing in a
/// known-bad region still carries the LOUD warning (never silent).
pub fn write_managed_ports_plan_at(
    path: &Path,
    plan: &PortPlan,
    wan_bdf: &str,
    lan_bdf: &str,
    facts: &PortsPlanFacts,
) -> std::io::Result<ManagedPlanOutcome> {
    let workers = facts.workers;
    let content = managed_env_block_with_workers(plan, wan_bdf, lan_bdf, facts, workers);
    write_managed_dropin_at(path, &content)?;
    Ok(ManagedPlanOutcome {
        workers,
        overridden: facts.workers_source == WorkersSource::Override,
        source: facts.workers_source,
    })
}

/// The managed bridge.env.d drop-in path (site keys never touch the packaged
/// bridge.env — the conffile-prompt trap). Upgrades REWRITE this file.
pub const MANAGED_DROPIN: &str = "/etc/libreqos/bridge.env.d/50-ports.conf";

/// The bind-mode env key value ("none" for the mlx5 in-place class).
pub fn bind_mode_key(plan: &PortPlan) -> &'static str {
    match plan.bind_mode {
        None => "none",
        Some(BindMode::Vfio) => "vfio",
        Some(BindMode::VfioNoiommu) => "vfio-noiommu",
        Some(BindMode::Uio) => "uio",
    }
}

/// The engine-class env key value.
pub fn engine_key(plan: &PortPlan) -> &'static str {
    match plan.engine {
        Engine::Pci => "pci",
        Engine::NetVirtio => "net_virtio",
    }
}

/// The MANAGED bridge.env.d block (E2 condition 4): written by the installer,
/// rewritten on upgrade, and OVERRIDDEN by any operator drop-in (the unit's
/// later `EnvironmentFile=` glob wins). Contains ONLY derived keys.
pub fn managed_env_block(
    plan: &PortPlan,
    wan_bdf: &str,
    lan_bdf: &str,
    link_mbps: u64,
    physical_cores: u32,
    nic_queue_count: u32,
) -> String {
    let workers = derived_workers(link_mbps, physical_cores, nic_queue_count);
    managed_env_block_with_workers(
        plan,
        wan_bdf,
        lan_bdf,
        &PortsPlanFacts {
            link_mbps,
            physical_cores,
            nic_queue_count,
            workers,
            workers_source: WorkersSource::Derived,
            worker_adoption: WorkerAdoption::None,
            topology_ok: true,
        },
        workers,
    )
}

/// The worker/queue-aware block builder (the override-aware entry point).
pub fn managed_env_block_with_workers(
    plan: &PortPlan,
    wan_bdf: &str,
    lan_bdf: &str,
    facts: &PortsPlanFacts,
    workers: u32,
) -> String {
    let mut out = String::new();
    let cfg_path = lqos_config::config_path();
    out.push_str(&format!(
        "# Managed by lqos_setup ports-plan from {cfg_path}.\n"
    ));
    out.push_str(&format!(
        "# Do not hand-edit; regenerate with: sudo lqos_setup ports-plan \
         --wan {wan_bdf} --lan {lan_bdf}\n"
    ));
    out.push_str("# Operator worker overrides belong in lqos.conf ([queues] workers).\n");
    out.push_str("# Drop-ins sorted after this one still win.\n");
    out.push_str(&format!("LIBREQOS_DPDK_ENGINE={}\n", engine_key(plan)));
    out.push_str(&format!(
        "LIBREQOS_DPDK_EAL_ARGS={}\n",
        eal_args_for_baked(
            wan_bdf,
            lan_bdf,
            bundled_eal_pmd_dir(Path::new(BUNDLED_DPDK_LIB_DIR)).as_deref()
        )
    ));
    out.push_str(&format!("LQOS_DPDK_WAN_BDF={wan_bdf}\n"));
    out.push_str(&format!("LQOS_DPDK_LAN_BDF={lan_bdf}\n"));
    // lqos-74t.4.1: `LIBREQOS_DPDK_SWAP_PORTS` is RETIRED — the explicit WAN/LAN
    // BDF pair above + the WAN-first EAL order ARE the orientation (and the
    // runtime's no-explicit-pair default now matches this order).
    out.push_str(&format!(
        "LIBREQOS_DPDK_BIND_MODE={}\n",
        bind_mode_key(plan)
    ));
    // Worker/queue auto-derivation (WAS fold): workers from the WAS rule; the
    // queues track the NIC/VM queue FACT (not the workers) — two quantities,
    // two rules. Never silent: a low-band count also emits the WARN.
    let link_mbps = facts.link_mbps;
    let physical_cores = facts.physical_cores;
    let nic_queue_count = facts.nic_queue_count;
    let queues = nic_queue_count.max(1);

    // `lqos-74t.160`: the only new note is the one-line adoption note on a write.
    if facts.worker_adoption == WorkerAdoption::Adopted {
        out.push_str(&format!(
            "# NOTE: adopted legacy W={workers} into [queues] workers.\n"
        ));
    }

 // Loud beyond-envelope line (the implementer (2)): the min may cap BELOW the
    // table's W via the cores OR the queues, and cell 2's delivered at a
    // capped W is UNMEASURED (state it, never imply the table).
    if beyond_measured_envelope(link_mbps) {
        out.push_str("# WARNING: link demand exceeds the measured SHAPED envelope (cell 2 top W=16 = 42.65G); ");
        out.push_str("the count is capped and the delivered rate at this point is UNMEASURED.\n");
    }
    match count_bound(link_mbps, physical_cores, nic_queue_count) {
        CountBound::Cores => out.push_str(
            "# WARNING: the PHYSICAL cores (minus the reserve) cannot field the link's SHAPED need -- \
             the count is capped BELOW the measured ceiling and the delivered rate at it is UNMEASURED.\n",
        ),
        CountBound::Queues => out.push_str(
            "# WARNING: the NIC/VM queue count caps the worker count BELOW the measured ceiling -- \
             the delivered rate at it is UNMEASURED.\n",
        ),
        CountBound::MeasuredCeiling => {}
    }
    if demand_in_measured_gap(link_mbps) {
        out.push_str("# NOTE: the link demand falls BETWEEN measured ceiling points; the derived W is the next MEASURED point. ");
        out.push_str("The true optimum may be a LOWER, unmeasured W (2/3/5/6/7/9..15) -- extrapolated, not computed.\n");
    }
    if let Some(warn) = low_worker_warning(link_mbps, workers) {
        // LOUD + PERSISTENT: the warning lives in the managed block so it is
        // visible to the operator on every read, not just at install time.
        for line in warn.lines() {
            out.push_str("# ");
            out.push_str(line);
            out.push('\n');
        }
    }
    if let Some(warn) = virtio_worker_warning(plan.class, workers) {
        // `lqos-74t.42`: low-band FIRST, then the virtio memory warning (the
        // two are W-axis complements). Deterministic/idempotent: fixed text,
        // no state, so a re-derive writes byte-identical lines.
        for line in warn.lines() {
            out.push_str("# ");
            out.push_str(line);
            out.push('\n');
        }
    }
 // (the implementer (1)) Emit DISTINCT MANAGED keys, NOT an APP_ARGS blob: the blob
    // would shadow a packaged-bridge.env operator APP_ARGS (later-wins) and make
    // the precedence undetectable. The wrapper MERGES these; an operator
    // APP_ARGS (a DIFFERENT file) is identifiable by provenance.
    out.push_str(&format!("LIBREQOS_DPDK_WORKERS={workers}\n"));
    out.push_str(&format!("LIBREQOS_DPDK_RX_QUEUES={queues}\n"));
    out.push_str(&format!("LIBREQOS_DPDK_TX_QUEUES={queues}\n"));
    if plan.class == PortClass::Virtio {
        out.push_str("LIBREQOS_DPDK_BIND_ID=1af4 1041\n");
    }
    out
}

/// The DPDK engine the bridge runs (`LIBREQOS_DPDK_ENGINE`-class).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    /// The PCI bus (`-a <bdf>`), the `mlx5`/`i40e` PMDs.
    Pci,
    /// The virtio vdev PMD (`net_virtio`), for the VM class.
    NetVirtio,
}

/// The full decision for a detected class.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortPlan {
    /// The detected class.
    pub class: PortClass,
    /// The bridge engine path.
    pub engine: Engine,
    /// The PMD the bridge loads.
    pub pmd: &'static str,
    /// The kernel-driver binding, or `None` when the NIC stays in place
    /// (mlx5 in-place — no unbind, so the kernel path keeps working too).
    pub bind_mode: Option<BindMode>,
    /// Hugepages (2 MiB) the DPDK pool needs. The jackrabbit recipe.
    pub hugepages_2m: u32,
}

/// The default hugepage reservation (the jackrabbit/jackrabbit-class 1G+ host).
/// SIZE is derived: `default_hugepages_2m()` returns this unless the operator
/// sets `LIBREQOS_DPDK_HUGEPAGES_2M` (the sizing hook, E2 condition 1).
pub const DEFAULT_HUGEPAGES_2M: u32 = 3072;

/// The hugepage sizing hook (E2 condition 1): the default, overridable via
/// `LIBREQOS_DPDK_HUGEPAGES_2M` so a host can size its own reservation.
pub fn default_hugepages_2m() -> u32 {
    std::env::var("LIBREQOS_DPDK_HUGEPAGES_2M")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(DEFAULT_HUGEPAGES_2M)
}

/// The DPDK PMD for a kernel driver (A0). DRIVER-SPECIFIC — never a class
/// hardcode: an X520 is `ixgbe` (NOT `i40e`), `e1000e` is the `e1000` PMD, and
/// `igb` covers i210/i350.
pub fn pmd_for_driver(driver: &str) -> Option<&'static str> {
    Some(match driver {
        "mlx5_core" => "mlx5",
        "mlx4_core" => "mlx4",
        "i40e" => "i40e",
        "ixgbe" => "ixgbe",
        "ice" => "ice",
        "igb" => "igb",
        "e1000e" => "e1000",
        "virtio_net" | "virtio-pci" => "net_virtio",
        "vmxnet3" => "vmxnet3",
        "ena" => "ena",
        "bnxt_en" => "bnxt",
        "qede" => "qede",
        "sfc" => "sfc",
        _ => return None,
    })
}

/// Decide the plan for a class. `iommu_present` is READ from the host: the PCI
/// class uses plain SAFE `vfio-pci` when an IOMMU exists and only falls back
/// to unsafe-noiommu (vfio) / uio when it does not — never a blanket unsafe
/// (a hardening regression). The virtio class always uses `uio_pci_generic`
/// (vfio-pci REFUSES on no-IOMMU VMs; the noiommu param is `vfio`'s).
pub fn plan(driver: &str, iommu_present: bool) -> Option<PortPlan> {
    let class = PortClass::detect(driver)?;
    let pmd = pmd_for_driver(driver)?;
    let hugepages_2m = default_hugepages_2m();
    Some(match class {
        PortClass::Mlx5 => PortPlan {
            class,
            engine: Engine::Pci,
            pmd,
            bind_mode: None,
            hugepages_2m,
        },
        PortClass::Intel => PortPlan {
            class,
            engine: Engine::Pci,
            pmd,
            bind_mode: Some(if iommu_present {
                BindMode::Vfio
            } else {
                BindMode::VfioNoiommu
            }),
            hugepages_2m,
        },
        PortClass::Virtio => PortPlan {
            class,
            engine: Engine::NetVirtio,
            pmd,
            bind_mode: Some(BindMode::Uio),
            hugepages_2m,
        },
    })
}

/// Read the kernel driver a netdev is bound to (`/sys/class/net/<dev>/device
/// /driver`), the fact the detector keys on. `net_sysfs` is `/sys/class/net`
/// in production (a fake tree in tests).
pub fn netdev_driver(net_sysfs: &Path, dev: &str) -> Option<String> {
    std::fs::read_link(net_sysfs.join(dev).join("device/driver"))
        .ok()?
        .file_name()
        .and_then(|n| n.to_str())
        .map(ToOwned::to_owned)
}

/// True when `value` is a PCI BDF of the form `DDDD:BB:DD.F` (domain 4 hex,
/// bus 2, device 2, function 0–7). The persisted `bridge.env` pair and the
/// engine's `interface_topology.device` must never carry a bus name such as
/// the virtio `virtioN` (the v2→v3 VM blocker).
pub fn is_pci_bdf(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 12 {
        return false;
    }
    let hex = |b: u8| b.is_ascii_hexdigit();
    hex(bytes[0])
        && hex(bytes[1])
        && hex(bytes[2])
        && hex(bytes[3])
        && bytes[4] == b':'
        && hex(bytes[5])
        && hex(bytes[6])
        && bytes[7] == b':'
        && hex(bytes[8])
        && hex(bytes[9])
        && bytes[10] == b'.'
        && (b'0'..=b'7').contains(&bytes[11])
}

/// The PCI BDF of a netdev, resolving the virtio-bus indirection.
///
/// `/sys/class/net/<dev>/device` resolves to the PCI device for a PCI netdev,
/// but to the virtio **bus** device (`.../0000:00:13.0/virtio2`) for a virtio
/// netdev. Taking the basename there yields `virtio2`, which is not a path
/// under `/sys/bus/pci/devices/` and cannot be bound. Resolve the canonical
/// path; if its basename is `virtioN`, use the PCI parent.
///
/// ⚠ The netdev→BDF fact exists ONLY while the NIC is still on its kernel
/// driver. The setup MUST resolve it BEFORE any bind and PERSIST it
/// (`LQOS_DPDK_WAN_BDF` / `LQOS_DPDK_LAN_BDF`), because after the bind the
/// netdev is gone.
pub fn pci_bdf_of_netdev(net_sysfs: &Path, dev: &str) -> Option<String> {
    let dev = dev.trim();
    if dev.is_empty() {
        return None;
    }
    let target = std::fs::canonicalize(net_sysfs.join(dev).join("device")).ok()?;
    let name = target.file_name()?.to_str()?;
    if is_pci_bdf(name) {
        return Some(name.to_ascii_lowercase());
    }
    if name.starts_with("virtio") {
        let parent = target.parent()?.file_name()?.to_str()?;
        if is_pci_bdf(parent) {
            return Some(parent.to_ascii_lowercase());
        }
    }
    None
}

/// The PCI BDF of a netdev (the shared [`pci_bdf_of_netdev`] resolver).
pub fn netdev_bdf(net_sysfs: &Path, dev: &str) -> Option<String> {
    pci_bdf_of_netdev(net_sysfs, dev)
}

/// The PCI vendor/device IDs of a BDF, when the device exists.
pub fn pci_vendor_device(bdf: &str) -> Option<(u16, u16)> {
    pci_vendor_device_at(Path::new("/sys"), bdf)
}

/// [`pci_vendor_device`] over an explicit sysfs root (the fake-sysfs oracle).
pub fn pci_vendor_device_at(sysfs: &Path, bdf: &str) -> Option<(u16, u16)> {
    if !is_pci_bdf(bdf) {
        return None;
    }
    let dir = sysfs.join("bus/pci/devices").join(bdf);
    let read = |name: &str| {
        let raw = std::fs::read_to_string(dir.join(name)).ok()?;
        u16::from_str_radix(raw.trim().trim_start_matches("0x"), 16).ok()
    };
    Some((read("vendor")?, read("device")?))
}

/// True for the **transitional** virtio NIC (`1af4:1000`). It is not the
/// packaged bind target: QEMU `disable-legacy=on` produces the modern
/// `1af4:1041` that the `net_virtio` PMD drives. A named refusal beats a
/// cryptic bind/PMD failure later.
pub fn is_transitional_virtio_bdf(bdf: &str) -> bool {
    is_transitional_virtio_bdf_at(Path::new("/sys"), bdf)
}

/// The named class error for a virtio BDF that is not the packaged modern
/// target (`1af4:1041`). `None` for a non-virtio or a modern-virtio device.
pub fn virtio_class_error_at(sysfs: &Path, bdf: &str) -> Option<String> {
    match pci_vendor_device_at(sysfs, bdf) {
        Some((0x1af4, 0x1041)) | None => None,
        Some((0x1af4, 0x1000)) => Some(format!(
            "{bdf} is a transitional virtio NIC (1af4:1000); the packaged net_virtio \
             target is 1af4:1041 — set disable-legacy=on on the VM NIC"
        )),
        Some((0x1af4, device)) => Some(format!(
            "{bdf} is virtio 1af4:{device:04x}, not the packaged modern 1af4:1041 \
             target — use a modern virtio NIC (disable-legacy=on on the VM)"
        )),
        Some(_) => None,
    }
}

/// [`is_transitional_virtio_bdf`] over an explicit sysfs root (the oracle).
pub fn is_transitional_virtio_bdf_at(sysfs: &Path, bdf: &str) -> bool {
    matches!(pci_vendor_device_at(sysfs, bdf), Some(ids) if is_transitional_virtio_ids(ids))
}

/// The pure ID rule behind [`is_transitional_virtio_bdf`]: QEMU's legacy virtio
/// NIC is vendor `1af4`, device `1000`. Modern virtio 1.0 is `1af4:1041`.
pub fn is_transitional_virtio_ids((vendor, device): (u16, u16)) -> bool {
    vendor == 0x1af4 && device == 0x1000
}

/// Validate a persisted WAN/LAN pair before a bind/restore action. A named
/// refusal is the only safe response to a bus name (`virtioN`), a duplicate
/// function, or the transitional virtio class — never a guessed rebind.
pub fn validate_bind_pair(wan: &str, lan: &str) -> Result<(), String> {
    validate_bind_pair_at(Path::new("/sys"), wan, lan)
}

/// [`validate_bind_pair`] over an explicit sysfs root (the oracle).
///
/// A persisted BDF must be a PRESENT PCI function with the Ethernet class;
/// shape alone is not enough (a stale/renamed device otherwise reaches the
/// bind and fails cryptically, or a non-Ethernet function is bound).
pub fn validate_bind_pair_at(sysfs: &Path, wan: &str, lan: &str) -> Result<(), String> {
    let wan = wan.trim();
    let lan = lan.trim();
    for (role, bdf) in [("WAN", wan), ("LAN", lan)] {
        validate_dataplane_bdf_at(sysfs, role, bdf)?;
    }
    if wan.eq_ignore_ascii_case(lan) {
        return Err(format!(
            "WAN and LAN are the same PCI function {wan}; a two-port bridge needs a \
             distinct pair"
        ));
    }
    Ok(())
}

/// The shared existence + Ethernet-class + virtio-class rule for one persisted
/// dataplane BDF.
fn validate_dataplane_bdf_at(sysfs: &Path, role: &str, bdf: &str) -> Result<(), String> {
    if !is_pci_bdf(bdf) {
        return Err(format!(
            "{role} '{bdf}' is not a PCI BDF; bridge.env must carry the pre-bind \
             PCI function (run setup BEFORE the bind and regenerate the pair)"
        ));
    }
    if !sysfs.join("bus/pci/devices").join(bdf).exists() {
        return Err(format!(
            "{role} {bdf} is a PCI BDF but the device is not present; the NIC was \
             removed/renamed — re-run setup BEFORE the bind"
        ));
    }
    if !pci_class_is_ethernet_at(sysfs, bdf) {
        return Err(format!(
            "{role} {bdf} is not an Ethernet controller (PCI class 0x020000); refusing \
             to bind it as a dataplane port"
        ));
    }
    if let Some(error) = virtio_class_error_at(sysfs, bdf) {
        return Err(format!("{role} {error}"));
    }
    Ok(())
}

/// True when the PCI function's class is an Ethernet controller (`0x020000`).
pub fn pci_class_is_ethernet_at(sysfs: &Path, bdf: &str) -> bool {
    std::fs::read_to_string(sysfs.join("bus/pci/devices").join(bdf).join("class"))
        .map(|class| class.trim().starts_with("0x020000"))
        .unwrap_or(false)
}

/// Validate the ONE persisted dataplane device for an on-a-stick trunk.
///
/// The same PCI BDF is deliberately written to BOTH `LQOS_DPDK_WAN_BDF` and
/// `LQOS_DPDK_LAN_BDF` (there is no new key; `WAN == LAN` is the on-a-stick
/// invariant). The class rule is the `.37.8` one: a modern virtio NIC must be
/// `1af4:1041` (transitional `1af4:1000` is refused); a native class (the
/// mlx5/i40e family) is accepted here and gated by the derived plan.
pub fn validate_single_device(bdf: &str) -> Result<(), String> {
    validate_single_device_at(Path::new("/sys"), bdf)
}

/// [`validate_single_device`] over an explicit sysfs root (the oracle).
pub fn validate_single_device_at(sysfs: &Path, bdf: &str) -> Result<(), String> {
    let bdf = bdf.trim();
    validate_dataplane_bdf_at(sysfs, "single-interface", bdf)
}

/// The generated EAL args. The `pmds` directory is ALWAYS appended (§C): the
/// wrapper must never rely on the caller getting it right, and the pinned
/// build needs the closure dir. The WAN is listed FIRST, the LAN second.
///
/// ⚠ The WAN-first order IS the orientation (lqos-74t.4.1): the retired
/// `LIBREQOS_DPDK_SWAP_PORTS` hatch is gone, and the runtime's no-explicit-pair
/// positional default now aligns with this order. Reordering here without moving
/// that default silently mis-orients the classifier (the shared host trap).
/// The private PMD dir the package ships and the 25.11 meson prefix bakes.
pub const PRIVATE_PMD_PATH: &str = "/usr/libexec/libreqos/pmds";

/// The bundled DPDK runtime dir (the private EAL lives here).
pub const BUNDLED_DPDK_LIB_DIR: &str = "/usr/libexec/libreqos/lib";

fn eal_attach_args(wan_bdf: &str, lan_bdf: &str) -> String {
    // `lqos-74t.37.7`: a single-interface (on-a-stick) box persists the SAME
    // BDF in both keys; EAL must not be asked to attach one device twice.
    if wan_bdf.trim() == lan_bdf.trim() {
        format!("-a {}", wan_bdf.trim())
    } else {
        format!("-a {wan_bdf} -a {lan_bdf}")
    }
}

/// The legacy/transitional EAL args: an explicit `-d <private>` because the
/// bundled EAL's baked scan dir is the distro path (23.11 artifacts and the
/// transitional F1 rewrite).
pub fn eal_args(wan_bdf: &str, lan_bdf: &str) -> String {
    format!(
        "-d {PRIVATE_PMD_PATH} {}",
        eal_attach_args(wan_bdf, lan_bdf)
    )
}

/// The installed-artifact EAL args. The MANAGED args ALWAYS carry
/// `-d <private>`: a regenerated `50-ports.conf` must never drop the PMD scan
/// (the 1st-lqos-01 X520/ixgbe class). The wrapper is the duplicate guard: it
/// strips this one `-d` when the bundled EAL bakes the same private dir, and
/// keeps it for an older/foreign baked path. `baked_pmd_dir` is retained for
/// the pure test surface but no longer suppresses `-d`.
pub fn eal_args_for_baked(
    wan_bdf: &str,
    lan_bdf: &str,
    _baked_pmd_dir: Option<&str>,
) -> String {
    eal_args(wan_bdf, lan_bdf)
}

/// True when `path` ends at a `pmds` or `pmds-<abi>` component (the engine's
/// component-boundary rule): `/usr/libexec/libreqos/pmds` and
/// `.../pmds-24.0` match; `.../pmds/child`, `.../pmds_expected`, and
/// `private-pmds` do not.
fn is_pmds_component(path: &str) -> bool {
    let Some(last) = path.rsplit('/').next() else {
        return false;
    };
    if last == "pmds" {
        return true;
    }
    last.strip_prefix("pmds-").is_some_and(|version| {
        !version.is_empty()
            && version
                .chars()
                .all(|c| c.is_ascii_digit() || c == '.')
    })
}

/// Read the baked PMD scan dir from a bundled EAL's bytes. `None` when the
/// bytes carry no `pmds` component (a non-DPDK or stripped library).
pub fn baked_pmd_dir_from_bytes(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    for token in text.split(|c: char| {
        !(c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '.' | '-'))
    }) {
        if token.starts_with('/') && is_pmds_component(token) {
            return Some(token.to_string());
        }
    }
    None
}

/// The baked PMD scan dir of the installed bundled EAL, when readable.
pub fn bundled_eal_pmd_dir(lib_dir: &Path) -> Option<String> {
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(lib_dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("librte_eal.so"))
        })
        .collect();
    files.sort();
    for file in files {
        if let Ok(bytes) = std::fs::read(&file)
            && let Some(baked) = baked_pmd_dir_from_bytes(&bytes)
        {
            return Some(baked);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_the_three_supported_classes() {
        assert_eq!(PortClass::detect("mlx5_core"), Some(PortClass::Mlx5));
        assert_eq!(PortClass::detect("i40e"), Some(PortClass::Intel));
        assert_eq!(PortClass::detect("ixgbe"), Some(PortClass::Intel));
        assert_eq!(PortClass::detect("virtio_net"), Some(PortClass::Virtio));
        // ice/igb/e1000e are the community class, not Tier 1
        assert_eq!(PortClass::detect("ice"), None);
        assert_eq!(PortClass::detect("igb"), None);
    }

    #[test]
    fn refuses_an_unknown_class() {
        assert_eq!(PortClass::detect("e1000e"), None);
        assert_eq!(PortClass::detect("bonding"), None);
    }

    #[test]
    fn a0_tier1_is_the_listed_set() {
        assert_eq!(
            NicTier::classify("ixgbe", true),
            NicTier::Tier1(PortClass::Intel)
        );
        assert_eq!(
            NicTier::classify("i40e", true),
            NicTier::Tier1(PortClass::Intel)
        );
        assert_eq!(
            NicTier::classify("mlx5_core", true),
            NicTier::Tier1(PortClass::Mlx5)
        );
        assert_eq!(
            NicTier::classify("virtio_net", true),
            NicTier::Tier1(PortClass::Virtio)
        );
    }

    #[test]
    fn a0_tier2_community_pmd_proceeds_with_a_notice() {
        assert_eq!(
            NicTier::classify("ice", true),
            NicTier::Tier2 {
                class: PortClass::Intel,
                pmd: "ice"
            }
        );
        assert_eq!(
            NicTier::classify("vmxnet3", true),
            NicTier::Tier2 {
                class: PortClass::Intel,
                pmd: "vmxnet3"
            }
        );
        assert!(NicTier::classify("qede", true).class().is_some());
    }

    #[test]
    fn a0_tier3_is_a_netdev_with_no_pmd() {
        assert_eq!(NicTier::classify("r8169", true), NicTier::Tier3);
        // e1000e is a known DPDK PMD (Tier 2), not Tier 3
        assert_eq!(
            NicTier::classify("e1000e", true),
            NicTier::Tier2 {
                class: PortClass::Intel,
                pmd: "e1000"
            }
        );
    }

    #[test]
    fn a0_refuses_only_no_path_or_bond() {
        // a bond is a topology refusal
        assert_eq!(
            NicTier::classify("bonding", true),
            NicTier::Refuse(RefuseReason::Bonded)
        );
        assert_eq!(
            NicTier::classify("bond0", true),
            NicTier::Refuse(RefuseReason::Bonded)
        );
        // no PMD AND no netdev => no dataplane path
        assert_eq!(
            NicTier::classify("r8169", false),
            NicTier::Refuse(RefuseReason::NoDataplanePath)
        );
        assert!(NicTier::classify("r8169", true).class().is_none());
    }

    #[test]
    fn tier2_binding_is_per_pmd_with_family_overrides() {
        // the community PCI class defaults to the IOMMU-aware vfio rule
        assert_eq!(tier2_bind("ice", true), Some(BindMode::Vfio));
        assert_eq!(tier2_bind("ice", false), Some(BindMode::VfioNoiommu));
        assert_eq!(tier2_bind("qede", true), Some(BindMode::Vfio));
        // the mlx5 family is in-place (never bound)
        assert_eq!(tier2_bind("mlx5", true), None);
        assert_eq!(tier2_bind("mlx4", true), None);
        // virtio uses uio
        assert_eq!(tier2_bind("net_virtio", false), Some(BindMode::Uio));
    }

    #[test]
    fn tier3_capability_states_shape_vs_forward() {
        assert!(CompatEngine::AfXdpShaping.provides_shaping());
        assert!(!CompatEngine::AfPacketPassthrough.provides_shaping());
    }

    #[test]
    fn tier12_on_compat_is_a_hard_assert_and_cleaned_on_upgrade() {
        let t1 = NicTier::classify("mlx5_core", true);
        assert!(assert_tier12_not_compat(t1, true).is_err());
        assert!(assert_tier12_not_compat(t1, false).is_ok());
        let t3 = NicTier::classify("r8169", true);
        assert!(assert_tier12_not_compat(t3, true).is_ok());
        assert!(purge_stale_compat_for(t1));
        assert!(!purge_stale_compat_for(t3));
    }

    #[test]
    fn tier2_pmd_is_driver_specific_and_never_planned_as_a_class() {
        // an mlx4 (a ConnectX) is Tier 2 but must be treated IN-PLACE, not vfio
        assert_eq!(
            NicTier::classify("mlx4_core", true).class(),
            Some(PortClass::Intel)
        );
        assert_eq!(tier2_bind("mlx4", true), None);
        assert_eq!(tier2_bind("bnxt", true), Some(BindMode::Vfio));
        assert_eq!(pmd_for_driver("bnxt_en"), Some("bnxt"));
        assert_eq!(pmd_for_driver("ena"), Some("ena"));
    }

    #[test]
    fn shaped_ceiling_table_and_the_small_job_rule() {
        // the smallest measured W whose delivered meets the demand
        assert_eq!(shaped_ceiling(1_000), 1); // 1.07G covers a 1G link (SMALL JOB = W1)
        assert_eq!(shaped_ceiling(5_000), 4); // 6.09G covers 5G
        assert_eq!(shaped_ceiling(10_000), 8); // 6.09G does NOT; 21.18G does
        assert_eq!(shaped_ceiling(100_000), W_MAX_PLACEHOLDER); // beyond the table
    }

    #[test]
    fn derived_workers_min_of_ceiling_cores_and_queues() {
        // 1G/8c/8q: the table picks W=1 (the small-job rule, no special case)
        assert_eq!(derived_workers(1_000, 8, 8), 1);
        // 10G/8c/8q: target 40, the cores-2=6 bound it
        assert_eq!(derived_workers(10_000, 8, 8), 6);
        // 100G/16c/16q: target 400, the cores-2=14 bound it
        assert_eq!(derived_workers(100_000, 16, 16), 14);
        // 10G/4c: the cores-2=2 bound it (the undersized box)
        assert_eq!(derived_workers(10_000, 4, 8), 2);
        // the card matrix
        assert_eq!(derived_workers(2_500, 12, 32), 10);
        assert_eq!(derived_workers(2_500, 14, 32), 10);
        assert_eq!(derived_workers(2_500, 8, 32), 6);
        assert_eq!(derived_workers(2_500, 14, 6), 6);
        assert_eq!(derived_workers(2_500, 14, 4), 4);
        assert_eq!(derived_workers(1_200, 14, 32), 5);
        assert_eq!(derived_workers(6_500, 14, 32), 12);
        assert_eq!(derived_workers(10_000, 8, 32), 6);
        assert_eq!(derived_workers(25_000, 14, 32), 12);
        // asymmetric down/up is a gather_facts concern; the sizing link is max
        assert_eq!(derived_workers(1_070, 14, 32), 1); // B inactive
        assert_eq!(derived_workers(1_071, 14, 32), 5); // B active
        // the queues bound too
        assert_eq!(derived_workers(100_000, 16, 2), 2);
    }

    #[test]
    fn was_rule_bands_and_cores_reserve() {
        assert_eq!(rate_workers(1_000), 1);
        assert_eq!(rate_workers(5_000), 2);
        assert_eq!(rate_workers(20_000), 4);
        assert_eq!(rate_workers(100_000), 8);
        assert_eq!(rate_workers(400_000), 16);
        // the WAS examples
        assert_eq!(was_workers(1_000, 4), 1);
        assert_eq!(was_workers(10_000, 8), 4);
        assert_eq!(was_workers(1_000, 8), 1); // never split a small job
        assert_eq!(was_workers(10_000, 4), 2); // cores-2=2, the box cannot field the link
        assert_eq!(was_workers(10_000, 16), 4); // extra cores = headroom
    }

    #[test]
    fn count_bound_distinguishes_the_caps() {
        // pre-existing table rule (unchanged warnings): 8c/10G -> Cores
        assert_eq!(count_bound(10_000, 8, 8), CountBound::Cores);
        // 100G/16c/2q -> Queues
        assert_eq!(count_bound(100_000, 16, 2), CountBound::Queues);
        // 1G/8c/8q -> no cap
        assert_eq!(count_bound(1_000, 8, 8), CountBound::MeasuredCeiling);
        // 14c/2.5G/12c/1.2G -> the table ceiling is met
        assert_eq!(count_bound(2_500, 14, 32), CountBound::MeasuredCeiling);
        assert_eq!(count_bound(2_500, 12, 32), CountBound::MeasuredCeiling);
        assert_eq!(count_bound(1_200, 14, 32), CountBound::MeasuredCeiling);
        // 14c/6.5G q8 -> the table ceiling is met (no new warning)
        assert_eq!(count_bound(6_500, 14, 8), CountBound::MeasuredCeiling);
        // 25G/14c -> Cores (table 16, cores-2=12)
        assert_eq!(count_bound(25_000, 14, 32), CountBound::Cores);
        // tie: cores == queues below the table -> Cores
        assert_eq!(count_bound(2_500, 4, 4), CountBound::Cores);
        // the two loud lines are present and distinct, byte-identical to beta16
        let m = plan("mlx5_core", true).unwrap();
        let c = managed_env_block(&m, "0000:01:00.0", "0000:01:00.1", 10_000, 8, 8);
        assert!(c.contains("the PHYSICAL cores (minus the reserve)"), "{c}");
        let q = managed_env_block(&m, "0000:01:00.0", "0000:01:00.1", 100_000, 16, 2);
        assert!(q.contains("the NIC/VM queue count caps the worker count"), "{q}");
    }

    #[test]
    fn physical_core_count_keys_by_package_and_warns_without_topology() {
        let root = std::env::temp_dir().join(format!("lqos-cpu-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // 2 packages x 4 cores x 2 threads = 16 logical -> 8 PHYSICAL cores
        for cpu in 0..16 {
            let dir = root.join(format!("cpu{cpu}"));
            std::fs::create_dir_all(dir.join("topology")).unwrap();
            let pkg = cpu / 8;
            let core = (cpu % 8) / 2;
            std::fs::write(dir.join("topology/physical_package_id"), format!("{pkg}\n")).unwrap();
            std::fs::write(dir.join("topology/core_id"), format!("{core}\n")).unwrap();
        }
        assert_eq!(physical_core_count_at(&root), (8, true));
        // two SOCKETS each 8 cores -> 16 (the bug that collapsed to 8 is gone)
        let root2 = std::env::temp_dir().join(format!("lqos-cpu2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root2);
        for cpu in 0..16 {
            let dir = root2.join(format!("cpu{cpu}"));
            std::fs::create_dir_all(dir.join("topology")).unwrap();
            std::fs::write(
                dir.join("topology/physical_package_id"),
                format!("{}\n", cpu / 8),
            )
            .unwrap();
            std::fs::write(dir.join("topology/core_id"), format!("{}\n", cpu % 8)).unwrap();
        }
        assert_eq!(physical_core_count_at(&root2), (16, true));
        // missing topology -> the flag is FALSE (the caller WARNs), count = logical
        let root3 = root.join("notopo");
        std::fs::create_dir_all(root3.join("cpu0")).unwrap();
        std::fs::write(root3.join("cpu0/something"), "x").unwrap();
        let (n, ok) = physical_core_count_at(&root3);
        assert!(!ok, "missing topology must flag");
        assert!(n >= 1);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&root2);
    }

    #[test]
    fn physical_core_count_is_sane_and_iommu_probe_is_total() {
        // on any real runner this is >= 1 and <= the logical CPU count
        let p = physical_core_count();
        assert!(p >= 1);
        // iommu_present must not panic on any host (no /sys/kernel/iommu_groups)
        let _ = iommu_present();
    }

    #[test]
    fn write_managed_ports_plan_lands_the_derived_or_overridden_workers() {
        let root = std::env::temp_dir().join(format!("lqos-plan-{}", std::process::id()));
        let path = root.join("bridge.env.d/50-ports.conf");
        let _ = std::fs::remove_dir_all(&root);
        let plan = plan("mlx5_core", true).unwrap();
        // 1G link -> W=1; the file lands ATOMICALLY carrying the derived count
        let facts = PortsPlanFacts {
            link_mbps: 1_000,
            physical_cores: 8,
            nic_queue_count: 8,
            workers: 1,
            workers_source: WorkersSource::Derived,
            worker_adoption: WorkerAdoption::None,
            topology_ok: true,
        };
        let out = write_managed_ports_plan_at(&path, &plan, "0000:01:00.0", "0000:01:00.1", &facts)
            .unwrap();
        assert_eq!(out.workers, 1);
        assert!(!out.overridden);
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("LIBREQOS_DPDK_WORKERS=1"), "{body}");
        assert!(body.contains("LIBREQOS_DPDK_RX_QUEUES=8"), "{body}");
        assert!(body.contains("LQOS_DPDK_WAN_BDF=0000:01:00.0"), "{body}");
        // no temp file left behind (the rename consumed it)
        assert!(!path.with_extension("conf.tmp").exists());
        // the operator's lqos.conf override WINS
        let ov = PortsPlanFacts {
            workers: 9,
            workers_source: WorkersSource::Override,
            ..facts
        };
        let out2 =
            write_managed_ports_plan_at(&path, &plan, "0000:01:00.0", "0000:01:00.1", &ov).unwrap();
        assert_eq!(out2.workers, 9);
        assert!(out2.overridden);
        let body2 = std::fs::read_to_string(&path).unwrap();
        assert!(body2.contains("LIBREQOS_DPDK_WORKERS=9"), "{body2}");
        // rewriting is idempotent (upgrade-safe)
        assert!(!body2.contains("LIBREQOS_DPDK_WORKERS=1\n"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn beyond_envelope_is_loud_and_no_interpolation_happens() {
        assert!(!beyond_measured_envelope(42_650));
        assert!(beyond_measured_envelope(100_000));
        // NO interpolation: a demand in a gap takes the NEXT MEASURED W (4, not a
        // computed 2/3)
        assert_eq!(shaped_ceiling(2_000), 4);
        assert_eq!(shaped_ceiling(7_000), 8);
        // the gap rule: strictly-between demands are flagged; exact points are not
        assert!(demand_in_measured_gap(2_000));
        assert!(demand_in_measured_gap(7_000));
        assert!(!demand_in_measured_gap(6_090));
        assert!(!demand_in_measured_gap(1_070));
        assert!(!demand_in_measured_gap(100_000));
        // the block carries the loud beyond-envelope line
        let m = plan("mlx5_core", true).unwrap();
        let b = managed_env_block(&m, "0000:01:00.0", "0000:01:00.1", 100_000, 64, 64);
        assert!(b.contains("exceeds the measured SHAPED envelope"), "{b}");
    }

    #[test]
    fn low_band_warns_only_when_the_link_demand_exceeds_the_w1_ceiling() {
        // a 1G link at W=1 is CORRECT (the W=1 ceiling IS its capacity) -> no warn
        assert!(low_worker_warning(1_000, 1).is_none());
        // a demand above the W=1 ceiling in the low band IS pathological
        assert!(low_worker_warning(10_000, 1).is_some());
        assert!(low_worker_warning(10_000, 2).is_some());
        assert!(low_worker_warning(10_000, 4).is_none());
        assert!(low_worker_warning(10_000, 1)
            .unwrap()
            .ends_with("set LIBREQOS_DPDK_WORKERS explicitly."));
        // the env writer calls the WAS rule + tracks the NIC queue fact
        let m = plan("mlx5_core", true).unwrap();
        let b = managed_env_block(&m, "0000:01:00.0", "0000:01:00.1", 10_000, 8, 8);
        // target 40, cores-2=6 binds -> 6 + the cores-cap WARNING
        assert!(b.contains("LIBREQOS_DPDK_WORKERS=6"), "{b}");
        assert!(b.contains("LIBREQOS_DPDK_RX_QUEUES=8"), "{b}");
        assert!(b.contains("LIBREQOS_DPDK_TX_QUEUES=8"), "{b}");
        assert!(b.contains("PHYSICAL cores"), "{b}");
        // a box whose ceiling the cores fully satisfy carries NO warning
        let ok = managed_env_block(&m, "0000:01:00.0", "0000:01:00.1", 1_000, 8, 8);
        assert!(!ok.contains("WARNING"), "{ok}");
        // an UNDERSIZED box (demand 10G but cores-2 = 2, W=2 in the low band)
        // carries the LOUD persistent warning inside the block
        let lo = managed_env_block(&m, "0000:01:00.0", "0000:01:00.1", 10_000, 4, 8);
        assert!(lo.contains("LIBREQOS_DPDK_WORKERS=2"), "{lo}");
        assert!(lo.contains("LIBREQOS_DPDK_RX_QUEUES=8"), "{lo}");
        assert!(lo.contains("# WARNING"), "{lo}");
        // and a 1G box does NOT (W=1 is its correct count)
        let small = managed_env_block(&m, "0000:01:00.0", "0000:01:00.1", 1_000, 8, 8);
        assert!(!small.contains("WARNING"), "{small}");
    }

    /// `lqos-74t.42`: `virtio_worker_warning` fires only above 2 workers and is
    /// the W-axis complement of `low_worker_warning` (no count double-warns).
    #[test]
    fn virtio_worker_warning_is_the_low_band_complement() {
        assert!(virtio_worker_warning(PortClass::Virtio, 4).is_some());
        assert!(virtio_worker_warning(PortClass::Virtio, 3).is_some());
        assert!(virtio_worker_warning(PortClass::Virtio, 2).is_none());
        assert!(virtio_worker_warning(PortClass::Mlx5, 4).is_none());
        assert!(virtio_worker_warning(PortClass::Intel, 8).is_none());

        // The 995/995 VM (link = down + up = 1990): W<=2 => low-band warns and
        // virtio does not; W=4 => virtio warns and low-band does not.
        assert!(low_worker_warning(1_990, 1).is_some());
        assert!(virtio_worker_warning(PortClass::Virtio, 1).is_none());
        assert!(virtio_worker_warning(PortClass::Virtio, 4).is_some());
        assert!(low_worker_warning(1_990, 4).is_none());

        // A true 1G VM (link <= 1070, derive W<=2): NEITHER warns.
        assert!(low_worker_warning(1_000, 2).is_none());
        assert!(virtio_worker_warning(PortClass::Virtio, 2).is_none());
    }

    /// `lqos-74t.42`: the managed block carries the virtio warning for a VM that
    /// derives >2 workers, and the block is deterministic/idempotent.
    #[test]
    fn virtio_warning_lands_in_the_block_and_is_idempotent() {
        let v = plan("virtio_net", false).unwrap();
        let first = managed_env_block(&v, "0000:00:12.0", "0000:00:13.0", 10_000, 64, 64);
        let second = managed_env_block(&v, "0000:00:12.0", "0000:00:13.0", 10_000, 64, 64);
        assert_eq!(first, second, "the managed block must be idempotent");
        assert!(first.contains("virtio/hypervisor"), "{first}");
        // A 1G virtio VM derives W=1 => no virtio warning.
        let small = managed_env_block(&v, "0000:00:12.0", "0000:00:13.0", 1_000, 8, 8);
        assert!(!small.contains("virtio/hypervisor"), "{small}");
    }

    #[test]
    fn managed_block_carries_only_derived_keys_and_virtio_bind_id() {
        let v = plan("virtio_net", false).unwrap();
        let b = managed_env_block(&v, "0000:00:12.0", "0000:00:13.0", 1_000, 8, 8);
        assert!(b.contains("LIBREQOS_DPDK_ENGINE=net_virtio"), "{b}");
        assert!(b.contains("LQOS_DPDK_WAN_BDF=0000:00:12.0"), "{b}");
        assert!(
            !b.contains("LIBREQOS_DPDK_SWAP_PORTS"),
            "the retired SWAP env must not be emitted: {b}"
        );
        assert!(b.contains("LIBREQOS_DPDK_BIND_MODE=uio"), "{b}");
        assert!(b.contains("LIBREQOS_DPDK_BIND_ID=1af4 1041"), "{b}");
        // On-a-stick: one device in both keys => exactly ONE `-a`.
        let s = managed_env_block(&v, "0000:00:13.0", "0000:00:13.0", 1_000, 8, 1);
        assert!(
            s.contains("LIBREQOS_DPDK_EAL_ARGS=-d /usr/libexec/libreqos/pmds -a 0000:00:13.0"),
            "{s}"
        );
        assert_eq!(s.matches("-a 0000:00:13.0").count(), 1, "{s}");
        // the mlx5 in-place class: no bind, so no bind id
        let m = plan("mlx5_core", true).unwrap();
        let mb = managed_env_block(&m, "0000:01:00.0", "0000:01:00.1", 10_000, 8, 1);
        assert!(mb.contains("LIBREQOS_DPDK_BIND_MODE=none"), "{mb}");
        assert!(!mb.contains("BIND_ID"), "{mb}");
        assert!(mb.contains("LIBREQOS_DPDK_ENGINE=pci"), "{mb}");
    }

    #[test]
    fn a0_multiqueue_must_equal_vcpus() {
        assert!(multiqueue_matches(8, 8));
        assert!(!multiqueue_matches(8, 4));
        assert!(!multiqueue_matches(1, 4));
    }

    #[test]
    fn mlx5_stays_in_place_and_uses_pci() {
        let p = plan("mlx5_core", true).unwrap();
        assert_eq!(p.engine, Engine::Pci);
        assert_eq!(p.pmd, "mlx5");
        assert_eq!(p.bind_mode, None); // in-place: no unbind
        // (1) the Intel PMD is DRIVER-specific: an X520 is ixgbe, NOT i40e
        assert_eq!(plan("ixgbe", true).unwrap().pmd, "ixgbe");
        assert_eq!(plan("i40e", true).unwrap().pmd, "i40e");
        // (3) e1000e is the e1000 PMD, igb is i210/i350
        assert_eq!(pmd_for_driver("e1000e"), Some("e1000"));
        assert_eq!(pmd_for_driver("igb"), Some("igb"));
    }

    #[test]
    fn intel_is_iommu_aware_never_blanket_unsafe() {
        // IOMMU present => the SAFE vfio-pci.
        assert_eq!(plan("i40e", true).unwrap().bind_mode, Some(BindMode::Vfio));
        // IOMMU absent => unsafe-noiommu is the supported fallback.
        assert_eq!(
            plan("i40e", false).unwrap().bind_mode,
            Some(BindMode::VfioNoiommu)
        );
    }

    #[test]
    fn virtio_uses_uio_and_the_net_virtio_pmd() {
        let p = plan("virtio_net", false).unwrap();
        assert_eq!(p.engine, Engine::NetVirtio);
        assert_eq!(p.pmd, "net_virtio");
        assert_eq!(p.bind_mode, Some(BindMode::Uio));
    }

    #[test]
    fn eal_always_appends_the_pmds_dir_wan_first() {
        let args = eal_args("0000:00:12.0", "0000:00:13.0");
        assert!(args.starts_with("-d /usr/libexec/libreqos/pmds "), "{args}");
        assert!(args.contains("-a 0000:00:12.0 -a 0000:00:13.0"), "{args}");
    }

    #[test]
    fn the_plan_accepts_one_device_as_a_single_interface_trunk() {
        assert_eq!(
            plan_bdf_pair(
                Some("0000:00:13.0".into()),
                Some("0000:00:14.0".into())
            )
            .unwrap(),
            ("0000:00:13.0".into(), "0000:00:14.0".into())
        );
        assert_eq!(
            plan_bdf_pair(Some("0000:00:13.0".into()), None).unwrap(),
            ("0000:00:13.0".into(), "0000:00:13.0".into())
        );
        assert!(plan_bdf_pair(None, None).is_err());
    }

    #[test]
    fn the_managed_eal_args_always_emit_the_private_d_for_the_wrapper_to_dedupe() {
        // The managed 50-ports.conf must always carry -d (1st-lqos-01 X520);
        // the staging wrapper strips it when the bundled EAL bakes the same dir.
        assert_eq!(
            eal_args_for_baked("0000:00:13.0", "0000:00:14.0", Some(PRIVATE_PMD_PATH)),
            "-d /usr/libexec/libreqos/pmds -a 0000:00:13.0 -a 0000:00:14.0"
        );
        // A transitional/older artifact behaves the same: always explicit -d.
        assert!(
            eal_args_for_baked("0000:00:13.0", "0000:00:14.0", Some("/usr/lib/x86_64-linux-gnu/dpdk/pmds-24.0"))
                .contains("-d /usr/libexec/libreqos/pmds")
        );
        assert!(
            eal_args_for_baked("0000:00:13.0", "0000:00:14.0", None)
                .contains("-d /usr/libexec/libreqos/pmds")
        );
    }

    #[test]
    fn the_baked_pmd_scanner_obeys_the_component_boundary() {
        let probe = |text: &str| baked_pmd_dir_from_bytes(text.as_bytes());
        assert_eq!(
            probe("junk\0/usr/libexec/libreqos/pmds\0").as_deref(),
            Some("/usr/libexec/libreqos/pmds")
        );
        assert_eq!(
            probe("DPDK_PLUGIN_PATH=/usr/lib/x86_64-linux-gnu/dpdk/pmds-24.0\0").as_deref(),
            Some("/usr/lib/x86_64-linux-gnu/dpdk/pmds-24.0")
        );
        assert_eq!(probe("junk\0/usr/lib/pmds/child\0"), None);
        assert_eq!(probe("junk\0/usr/lib/pmds_expected\0"), None);
        assert_eq!(probe("junk\0/usr/lib/private-pmds\0"), None);
    }

    #[test]
    fn eal_attaches_a_single_interface_bdf_once() {
        let args = eal_args("0000:00:13.0", "0000:00:13.0");
        assert_eq!(args, "-d /usr/libexec/libreqos/pmds -a 0000:00:13.0");
    }

    #[test]
    fn bdf_shape_is_exact_and_lowercase_safe() {
        assert!(is_pci_bdf("0000:00:13.0"));
        assert!(is_pci_bdf("0000:ff:1f.7"));
        assert!(is_pci_bdf("ABcd:00:13.0"));
        assert!(!is_pci_bdf("virtio2"));
        assert!(!is_pci_bdf("00:13.0"));
        assert!(!is_pci_bdf("0000:00:13.8"));
        assert!(!is_pci_bdf("0000:00:13"));
        assert!(!is_pci_bdf(""));
    }

    /// A virtio netdev's `device` link resolves to `.../0000:00:13.0/virtio2`;
    /// the resolver must return the PCI parent, never `virtio2`.
    #[test]
    fn virtio_bus_device_resolves_to_its_pci_parent() {
        let root =
            std::env::temp_dir().join(format!("lqos-portclass-virtio-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let net = root.join("class/net");
        std::fs::create_dir_all(net.join("ens19")).unwrap();
        let pci = root.join("bus/pci/devices/0000:00:13.0");
        std::fs::create_dir_all(pci.join("virtio2")).unwrap();
        std::os::unix::fs::symlink(pci.join("virtio2"), net.join("ens19/device")).unwrap();

        assert_eq!(
            pci_bdf_of_netdev(&net, "ens19").as_deref(),
            Some("0000:00:13.0")
        );
        // A bare virtio bus name is never a persisted device.
        assert!(!is_pci_bdf("virtio2"));
        assert_eq!(pci_bdf_of_netdev(&net, "ens99"), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_single_device_allows_the_same_bdf_but_not_the_legacy_virtio() {
        let root =
            std::env::temp_dir().join(format!("lqos-portclass-single-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let pci = root.join("bus/pci/devices");
        for (bdf, vendor, device) in [
            ("0000:00:13.0", "0x1af4", "0x1041"),
            ("0000:00:14.0", "0x1af4", "0x1000"),
            ("0000:00:16.0", "0x1af4", "0x1001"),
            ("0000:00:15.0", "0x15b3", "0x101d"),
        ] {
            let dir = pci.join(bdf);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("vendor"), format!("{vendor}\n")).unwrap();
            std::fs::write(dir.join("device"), format!("{device}\n")).unwrap();
            std::fs::write(dir.join("class"), "0x020000\n").unwrap();
        }
        // Shape + modern virtio + a native mlx5 BDF are accepted.
        assert!(validate_single_device_at(&root, "0000:00:13.0").is_ok());
        assert!(validate_single_device_at(&root, "0000:00:15.0").is_ok());
        // Transitional (or any non-1041 virtio) is refused, even if persisted.
        assert!(validate_single_device_at(&root, "0000:00:14.0").is_err());
        assert!(validate_single_device_at(&root, "0000:00:16.0").is_err());
        // A bus name is never a device; a shape-valid but ABSENT BDF is
        // refused too (never bind a stale device).
        assert!(validate_single_device_at(&root, "virtio2").is_err());
        assert!(validate_single_device_at(&root, "0000:00:99.0").is_err());
        // The on-a-stick invariant itself is WAN == LAN; the two-port
        // validator keeps rejecting it.
        assert!(validate_single_device_at(&root, "0000:00:13.0").is_ok());
        assert!(validate_bind_pair_at(&root, "0000:00:13.0", "0000:00:13.0").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_bind_pair_is_shape_and_device_checked_before_any_sysfs_write() {
        let root = std::env::temp_dir().join(format!(
            "lqos-portclass-pair-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let pci = root.join("bus/pci/devices");
        for (bdf, class) in [
            ("0000:00:13.0", "0x020000"),
            ("0000:00:14.0", "0x020000"),
            ("0000:00:17.0", "0x030000"),
        ] {
            let dir = pci.join(bdf);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("class"), format!("{class}\n")).unwrap();
        }
        assert!(validate_bind_pair_at(&root, "0000:00:13.0", "0000:00:14.0").is_ok());
        // A bus name / duplicate / bad shape / absent device / non-Ethernet.
        assert!(validate_bind_pair_at(&root, "virtio2", "virtio3").is_err());
        assert!(validate_bind_pair_at(&root, "0000:00:13.0", "0000:00:13.0").is_err());
        assert!(validate_bind_pair_at(&root, "0000:00:13.8", "0000:00:14.0").is_err());
        assert!(validate_bind_pair_at(&root, "0000:00:13.0", "0000:00:99.0").is_err());
        assert!(validate_bind_pair_at(&root, "0000:00:13.0", "0000:00:17.0").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn only_the_modern_virtio_device_is_a_bind_target() {
        // `1af4:1000` is transitional and must be rejected; `1af4:1041` is the
        // packaged `net_virtio` target (docs/virtio_vm_v2v3_upgrade_card.md).
        assert!(is_transitional_virtio_ids((0x1af4, 0x1000)));
        assert!(!is_transitional_virtio_ids((0x1af4, 0x1041)));
        assert!(!is_transitional_virtio_ids((0x15b3, 0x101d)));
        // A bus name must never reach the PCI lookup.
        assert!(!is_transitional_virtio_bdf("virtio2"));
    }

    #[test]
    fn netdev_driver_and_bdf_read_the_sysfs_links() {
        let root = std::env::temp_dir().join(format!("lqos-portclass-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // Mirror real sysfs: the netdev's `device` -> the PCI dev dir, and the
        // `driver` symlink lives INSIDE the PCI dev dir pointing at the driver.
        let net = root.join("class/net");
        std::fs::create_dir_all(net.join("ens18")).unwrap();
        let drv = root.join("bus/pci/drivers/virtio-pci");
        std::fs::create_dir_all(&drv).unwrap();
        let pci = root.join("bus/pci/devices/0000:00:12.0");
        std::fs::create_dir_all(&pci).unwrap();
        std::os::unix::fs::symlink(&drv, pci.join("driver")).unwrap();
        std::os::unix::fs::symlink(&pci, net.join("ens18/device")).unwrap();

        assert_eq!(netdev_bdf(&net, "ens18").as_deref(), Some("0000:00:12.0"));
        assert_eq!(netdev_driver(&net, "ens18").as_deref(), Some("virtio-pci"));
        assert_eq!(netdev_bdf(&net, "ens99"), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn worker_source_is_explicit_and_higher_only_before_the_stamp() {
        // config key wins over every legacy surface (including B)
        let (w, s, a) = resolve_worker_source(Some(8), Some(4), Some(4), false, 10);
        assert_eq!((w, s, a), (8, WorkersSource::Override, WorkerAdoption::None));
        // main env matrix: W4 -> IgnoredLower, W10 -> IgnoredEqual, W12 -> Adopted
        let (w, s, a) = resolve_worker_source(None, Some(4), None, false, 10);
        assert_eq!((w, s, a), (10, WorkersSource::Derived, WorkerAdoption::IgnoredLower));
        let (w, s, a) = resolve_worker_source(None, Some(10), None, false, 10);
        assert_eq!((w, s, a), (10, WorkersSource::Derived, WorkerAdoption::IgnoredEqual));
        let (w, s, a) = resolve_worker_source(None, Some(12), None, false, 10);
        assert_eq!((w, s, a), (12, WorkersSource::LegacyEnv, WorkerAdoption::Adopted));
        // after the stamp, main/managed are never read: stale W4 -> Derived 10
        let (w, s, a) = resolve_worker_source(None, Some(4), Some(8), true, 10);
        assert_eq!((w, s, a), (10, WorkersSource::Derived, WorkerAdoption::None));
        // managed matrix: W8 -> IgnoredLower, W10 -> IgnoredEqual, W12 -> Adopted
        let (w, s, a) = resolve_worker_source(None, None, Some(8), false, 10);
        assert_eq!((w, s, a), (10, WorkersSource::Derived, WorkerAdoption::IgnoredLower));
        let (w, s, a) = resolve_worker_source(None, None, Some(10), false, 10);
        assert_eq!((w, s, a), (10, WorkersSource::Derived, WorkerAdoption::IgnoredEqual));
        let (w, s, a) = resolve_worker_source(None, None, Some(12), false, 10);
        assert_eq!((w, s, a), (12, WorkersSource::LegacyEnv, WorkerAdoption::Adopted));
        // main env wins over managed when both exist
        let (w, s, a) = resolve_worker_source(None, Some(12), Some(8), false, 10);
        assert_eq!((w, s, a), (12, WorkersSource::LegacyEnv, WorkerAdoption::Adopted));
        // 0 is not a valid worker override
        let (w, s, a) = resolve_worker_source(Some(0), Some(4), Some(8), false, 10);
        assert_eq!((w, s, a), (10, WorkersSource::Derived, WorkerAdoption::IgnoredLower));
    }

    #[test]
    fn bound_queue_fact_prefers_persisted_then_managed_then_live() {
        // explicit wins
        assert_eq!(
            pick_queue_count(Some(20), Some(32), Some(8), Some(4)),
            Ok(20)
        );
        // post-bind: live is gone, persisted STATE supplies the fact
        assert_eq!(
            pick_queue_count(None, Some(32), Some(8), None),
            Ok(32)
        );
        // no STATE: the managed RX_QUEUES is the one-time fallback
        assert_eq!(pick_queue_count(None, None, Some(8), None), Ok(8));
        // nothing at all is the named error, never a silent 1
        assert!(pick_queue_count(None, None, None, None).is_err());
        // N5b: managed W12 + RX_QUEUES=32, 14c, 2.5G -> B10, adopt W12
        let derived = derived_workers(2_500, 14, 32);
        assert_eq!(derived, 10);
        let (w, s, a) = resolve_worker_source(None, None, Some(12), false, derived);
        assert_eq!((w, s, a), (12, WorkersSource::LegacyEnv, WorkerAdoption::Adopted));
        // live managed W8 vs B10 is ignored (lower)
        let (w, s, a) = resolve_worker_source(None, None, Some(8), false, derived);
        assert_eq!((w, s, a), (10, WorkersSource::Derived, WorkerAdoption::IgnoredLower));
    }

    #[test]
    fn managed_header_names_the_plan_writer_and_never_rerun_setup() {
        let m = plan("mlx5_core", true).unwrap();
        let b = managed_env_block(&m, "0000:01:00.0", "0000:01:00.1", 1_000, 8, 8);
        assert!(
            b.contains("sudo lqos_setup ports-plan --wan 0000:01:00.0 --lan 0000:01:00.1"),
            "{b}"
        );
        assert!(b.contains("Managed by lqos_setup ports-plan from "), "{b}");
        assert!(b.contains(lqos_config::config_path().as_str()), "{b}");
        assert!(!b.contains("rerun libreqos-setup"), "{b}");
    }

    #[test]
    fn b_count_matrix_and_warning_text_regression() {
        let m = plan("mlx5_core", true).unwrap();
        // target reached: no new warning
        let b = managed_env_block(&m, "0000:01:00.0", "0000:01:00.1", 2_500, 14, 32);
        assert!(b.contains("LIBREQOS_DPDK_WORKERS=10"), "{b}");
        assert!(!b.contains("PHYSICAL cores"), "{b}");
        assert!(!b.contains("queue count caps"), "{b}");
        let b12 = managed_env_block(&m, "0000:01:00.0", "0000:01:00.1", 2_500, 12, 32);
        assert!(b12.contains("LIBREQOS_DPDK_WORKERS=10"), "{b12}");
        // B raises 8c/6c and the table-bound q8/6.5G: no new warning
        let c8 = managed_env_block(&m, "0000:01:00.0", "0000:01:00.1", 2_500, 8, 32);
        assert!(c8.contains("LIBREQOS_DPDK_WORKERS=6"), "{c8}");
        assert!(!c8.contains("PHYSICAL cores"), "{c8}");
        let c6 = managed_env_block(&m, "0000:01:00.0", "0000:01:00.1", 2_500, 6, 32);
        assert!(c6.contains("LIBREQOS_DPDK_WORKERS=4"), "{c6}");
        let q8 = managed_env_block(&m, "0000:01:00.0", "0000:01:00.1", 6_500, 14, 8);
        assert!(q8.contains("LIBREQOS_DPDK_WORKERS=8"), "{q8}");
        // hard caps keep the beta16 text
        let c10 = managed_env_block(&m, "0000:01:00.0", "0000:01:00.1", 10_000, 8, 8);
        assert!(c10.contains("LIBREQOS_DPDK_WORKERS=6"), "{c10}");
        assert!(c10.contains("the PHYSICAL cores (minus the reserve)"), "{c10}");
        let c25 = managed_env_block(&m, "0000:01:00.0", "0000:01:00.1", 25_000, 14, 32);
        assert!(c25.contains("LIBREQOS_DPDK_WORKERS=12"), "{c25}");
        assert!(c25.contains("the PHYSICAL cores (minus the reserve)"), "{c25}");
        // 1.2G/14c and 1G: count only, no warning
        let bw = managed_env_block(&m, "0000:01:00.0", "0000:01:00.1", 1_200, 14, 32);
        assert!(bw.contains("LIBREQOS_DPDK_WORKERS=5"), "{bw}");
        assert!(!bw.contains("WARNING"), "{bw}");
        let small = managed_env_block(&m, "0000:01:00.0", "0000:01:00.1", 1_000, 14, 32);
        assert!(small.contains("LIBREQOS_DPDK_WORKERS=1"), "{small}");
        assert!(!small.contains("WARNING"), "{small}");
        // 1070/1071 boundary
        let b1070 = managed_env_block(&m, "0000:01:00.0", "0000:01:00.1", 1_070, 14, 32);
        assert!(b1070.contains("LIBREQOS_DPDK_WORKERS=1"), "{b1070}");
        let b1071 = managed_env_block(&m, "0000:01:00.0", "0000:01:00.1", 1_071, 14, 32);
        assert!(b1071.contains("LIBREQOS_DPDK_WORKERS=5"), "{b1071}");
        // no override/legacy notes; the only note is an adoption write
        let facts = PortsPlanFacts {
            link_mbps: 2_500,
            physical_cores: 14,
            nic_queue_count: 32,
            workers: 8,
            workers_source: WorkersSource::Override,
            worker_adoption: WorkerAdoption::None,
            topology_ok: true,
        };
        let ov = managed_env_block_with_workers(&m, "0000:01:00.0", "0000:01:00.1", &facts, 8);
        assert!(!ov.contains("explicit operator override"), "{ov}");
        assert!(!ov.contains("adopted legacy"), "{ov}");
        // beta16 warning strings restored byte-for-byte
        assert!(low_worker_warning(10_000, 1)
            .unwrap()
            .ends_with("set LIBREQOS_DPDK_WORKERS explicitly."));
        assert!(virtio_worker_warning(PortClass::Virtio, 4)
            .unwrap()
            .contains("(or LIBREQOS_DPDK_WORKERS)"));
    }

    fn gather_fixture(name: &str) -> (PathBuf, GatherEnv) {
        let root = std::env::temp_dir().join(format!(
            "lqos-gather-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let env = GatherEnv {
            main_env_path: root.join("bridge.env"),
            managed_path: root.join("50-ports.conf"),
            state_path: root.join("dpdk-bind-originals.env"),
            conf_path: root.join("lqos.conf"),
            stamp_path: root.join("worker-override-migrated"),
            physical_cores_override: Some(14),
        };
        std::fs::write(
            &env.conf_path,
            "[queues]\ndownlink_bandwidth_mbps = 2500\nuplink_bandwidth_mbps = 2500\n",
        )
        .unwrap();
        std::fs::write(&env.main_env_path, "").unwrap();
        std::fs::write(
            &env.managed_path,
            "LIBREQOS_DPDK_WORKERS=12\nLIBREQOS_DPDK_RX_QUEUES=32\n",
        )
        .unwrap();
        (root, env)
    }

    fn cfg_2500() -> lqos_config::Config {
        let mut cfg = lqos_config::Config::default();
        cfg.queues.downlink_bandwidth_mbps = 2_500.0;
        cfg.queues.uplink_bandwidth_mbps = 2_500.0;
        cfg.queues.workers = None;
        cfg
    }

    #[test]
    fn gather_facts_adopts_a_higher_managed_w12_once() {
        let (root, env) = gather_fixture("live-w12");
        let mut cfg = cfg_2500();
        let facts = gather_facts_with(&cfg, "0000:01:00.0", Some(32), true, &env).unwrap();
        assert_eq!(facts.workers, 12, "{facts:?}");
        assert_eq!(facts.workers_source, WorkersSource::LegacyEnv);
        assert_eq!(facts.worker_adoption, WorkerAdoption::Adopted);
        let conf = std::fs::read_to_string(&env.conf_path).unwrap();
        assert!(conf.contains("workers = 12"), "{conf}");
        let stamp = std::fs::read_to_string(&env.stamp_path).unwrap();
        assert!(stamp.contains("status=adopted"), "{stamp}");

        // second render: the config key is now the source; no re-adoption
        cfg.queues.workers = Some(12);
        let again = gather_facts_with(&cfg, "0000:01:00.0", Some(32), true, &env).unwrap();
        assert_eq!(again.workers_source, WorkersSource::Override);
        assert_eq!(again.worker_adoption, WorkerAdoption::None);
        assert_eq!(
            std::fs::read_to_string(&env.conf_path)
                .unwrap()
                .matches("workers = 12")
                .count(),
            1
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn gather_facts_ignores_a_lower_managed_w8_and_stamps_it() {
        let (root, env) = gather_fixture("live-w8-lower");
        std::fs::write(
            &env.managed_path,
            "LIBREQOS_DPDK_WORKERS=8\nLIBREQOS_DPDK_RX_QUEUES=32\n",
        )
        .unwrap();
        let cfg = cfg_2500();
        let facts = gather_facts_with(&cfg, "0000:01:00.0", Some(32), true, &env).unwrap();
        assert_eq!(facts.workers, 10, "{facts:?}");
        assert_eq!(facts.workers_source, WorkersSource::Derived);
        assert_eq!(facts.worker_adoption, WorkerAdoption::IgnoredLower);
        assert!(!std::fs::read_to_string(&env.conf_path).unwrap().contains("workers ="));
        let stamp = std::fs::read_to_string(&env.stamp_path).unwrap();
        assert!(stamp.contains("status=ignored-lower"), "{stamp}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn gather_facts_dry_run_does_not_write_the_config_or_stamp() {
        let (root, env) = gather_fixture("dry-run");
        let cfg = cfg_2500();
        let facts = gather_facts_with(&cfg, "0000:01:00.0", Some(32), false, &env).unwrap();
        assert_eq!(facts.workers, 12);
        assert_eq!(facts.workers_source, WorkersSource::LegacyEnv);
        // dry-run must not claim the adoption/write
        assert_eq!(facts.worker_adoption, WorkerAdoption::None);
        assert!(!std::fs::read_to_string(&env.conf_path).unwrap().contains("workers ="));
        assert!(!env.stamp_path.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn gather_facts_sizes_on_max_down_up_not_the_sum() {
        let (root, env) = gather_fixture("asymmetric");
        std::fs::write(&env.managed_path, "").unwrap();
        for (down, up) in [(2_500.0, 1_000.0), (1_000.0, 2_500.0)] {
            let mut cfg = cfg_2500();
            cfg.queues.downlink_bandwidth_mbps = down;
            cfg.queues.uplink_bandwidth_mbps = up;
            let facts = gather_facts_with(&cfg, "0000:01:00.0", Some(32), false, &env).unwrap();
            assert_eq!(facts.link_mbps, 2_500, "down={down} up={up}");
            assert_eq!(facts.workers, 10, "down={down} up={up}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn gather_facts_main_env_w4_is_ignored_lower() {
        let (root, env) = gather_fixture("main-w4");
        std::fs::write(&env.main_env_path, "LIBREQOS_DPDK_WORKERS=4\n").unwrap();
        let cfg = cfg_2500();
        let facts = gather_facts_with(&cfg, "0000:01:00.0", Some(32), true, &env).unwrap();
        assert_eq!(facts.workers, 10, "{facts:?}");
        assert_eq!(facts.workers_source, WorkersSource::Derived);
        assert_eq!(facts.worker_adoption, WorkerAdoption::IgnoredLower);
        assert!(!std::fs::read_to_string(&env.conf_path).unwrap().contains("workers ="));
        assert!(std::fs::read_to_string(&env.stamp_path)
            .unwrap()
            .contains("status=ignored-lower"));
        let _ = std::fs::remove_dir_all(&root);
    }
    #[test]
    fn warning_text_golden_is_beta16() {
        let m = plan("mlx5_core", true).unwrap();
        let c = managed_env_block(&m, "0000:01:00.0", "0000:01:00.1", 10_000, 8, 8);
        assert!(c.contains(
            "the PHYSICAL cores (minus the reserve) cannot field the link's SHAPED need"
        ), "{c}");
        let q = managed_env_block(&m, "0000:01:00.0", "0000:01:00.1", 100_000, 16, 2);
        assert!(q.contains(
            "the NIC/VM queue count caps the worker count BELOW the measured ceiling"
        ), "{q}");
        assert!(low_worker_warning(10_000, 1)
            .unwrap()
            .ends_with("set LIBREQOS_DPDK_WORKERS explicitly."));
        assert!(virtio_worker_warning(PortClass::Virtio, 4)
            .unwrap()
            .contains("(or LIBREQOS_DPDK_WORKERS)"));
    }

    #[test]
    fn adoption_note_is_positive_only_on_a_write() {
        let m = plan("mlx5_core", true).unwrap();
        let facts = PortsPlanFacts {
            link_mbps: 2_500,
            physical_cores: 14,
            nic_queue_count: 32,
            workers: 12,
            workers_source: WorkersSource::LegacyEnv,
            worker_adoption: WorkerAdoption::Adopted,
            topology_ok: true,
        };
        let b = managed_env_block_with_workers(&m, "0000:01:00.0", "0000:01:00.1", &facts, 12);
        assert!(b.contains("# NOTE: adopted legacy W=12 into [queues] workers."), "{b}");
        let ignored = PortsPlanFacts {
            worker_adoption: WorkerAdoption::IgnoredLower,
            ..facts
        };
        let b2 = managed_env_block_with_workers(&m, "0000:01:00.0", "0000:01:00.1", &ignored, 12);
        assert!(!b2.contains("adopted legacy"), "{b2}");
    }

    #[test]
    fn main_lower_is_ignored_even_when_managed_is_higher() {
        let (w, s, a) = resolve_worker_source(None, Some(4), Some(12), false, 10);
        assert_eq!((w, s, a), (10, WorkersSource::Derived, WorkerAdoption::IgnoredLower));
    }

    #[test]
    fn warning_uses_the_max_down_up_link() {
        let (root, env) = gather_fixture("warning-max");
        std::fs::write(&env.managed_path, "").unwrap();
        let mut cfg = cfg_2500();
        cfg.queues.downlink_bandwidth_mbps = 1_000.0;
        cfg.queues.uplink_bandwidth_mbps = 2_500.0;
        let facts = gather_facts_with(&cfg, "0000:01:00.0", Some(32), false, &env).unwrap();
        assert_eq!(facts.link_mbps, 2_500);
        assert!(low_worker_warning(facts.link_mbps, 2).is_some());
        assert!(low_worker_warning(1_000, 2).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

}
