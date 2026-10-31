//! Persistent bootstrap/setup state for first-run setup workflows.

use anyhow::{Context, Result};
use lqos_config::{Config, UserRole};
use nix::{
    ifaddrs::getifaddrs,
    sys::socket::{AddressFamily, SockaddrLike},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

const BOOTSTRAP_STATE_VERSION: u32 = 1;
const DEFAULT_LQOS_DIRECTORY: &str = "/opt/libreqos/src";
const DEFAULT_STATE_DIRECTORY: &str = "/opt/libreqos/state";
const TOKEN_TTL: Duration = Duration::from_secs(2 * 60 * 60);

/// High-level setup status visible to operators.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SetupStatus {
    /// Setup is not complete yet.
    Incomplete,
    /// Setup completed and LibreQoS can hand off to runtime services.
    Complete,
}

impl SetupStatus {
    /// Human-readable label for operator-facing status output.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Incomplete => "Setup Incomplete",
            Self::Complete => "Setup Complete",
        }
    }
}

/// Current bootstrap token metadata for the temporary setup surface.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BootstrapToken {
    /// Opaque setup token.
    pub token: String,
    /// Expiration time in Unix seconds.
    pub expires_at_unix: u64,
}

/// Persistent first-run setup state stored under the LibreQoS state directory.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BootstrapState {
    /// On-disk schema version for setup state.
    pub version: u32,
    /// Whether setup has been completed.
    pub setup_complete: bool,
    /// Whether an admin user exists.
    pub first_admin_exists: bool,
    /// Current setup token.
    pub token: BootstrapToken,
}

impl Default for BootstrapState {
    fn default() -> Self {
        Self {
            version: BOOTSTRAP_STATE_VERSION,
            setup_complete: false,
            first_admin_exists: false,
            token: new_bootstrap_token(),
        }
    }
}

/// Computed setup status snapshot for CLI surfaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusSnapshot {
    /// Synced bootstrap state.
    pub state: BootstrapState,
    /// Operator-visible status derived for the current snapshot.
    pub display_status: SetupStatus,
    /// Hostname and IP hints for the operator.
    pub host_hints: Vec<String>,
    /// Whether `/etc/lqos.conf` currently parses.
    pub config_loads: bool,
}

/// Service mode the package postinst should activate after installation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostinstAction {
    /// Start the runtime services and keep setup inside `lqosd`.
    ActivateRuntime,
    /// Start the dedicated setup service for a true first-run install.
    ActivateSetup,
}

impl PostinstAction {
    /// Stable token printed for package postinst shell logic.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ActivateRuntime => "activate_runtime",
            Self::ActivateSetup => "activate_setup",
        }
    }
}

#[derive(Default, Deserialize)]
struct AuthUsersFile {
    #[serde(default)]
    users: Vec<AuthUserEntry>,
}

#[derive(Deserialize)]
struct AuthUserEntry {
    role: UserRole,
}

#[derive(Clone, Debug)]
struct RuntimePaths {
    lqos_directory: PathBuf,
    state_directory: PathBuf,
}

#[derive(Clone, Debug)]
struct LiveInputs {
    config_loads: bool,
    first_admin_exists: bool,
}

fn current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn new_bootstrap_token() -> BootstrapToken {
    BootstrapToken {
        token: Uuid::new_v4().simple().to_string(),
        expires_at_unix: current_unix_seconds() + TOKEN_TTL.as_secs(),
    }
}

fn runtime_paths() -> RuntimePaths {
    if let Ok(config) = lqos_config::load_config() {
        return RuntimePaths {
            lqos_directory: PathBuf::from(&config.lqos_directory),
            state_directory: config.resolved_state_directory(),
        };
    }

    RuntimePaths {
        lqos_directory: PathBuf::from(DEFAULT_LQOS_DIRECTORY),
        state_directory: PathBuf::from(DEFAULT_STATE_DIRECTORY),
    }
}

/// Returns the active or default LibreQoS runtime directory used by first-run setup.
pub fn runtime_lqos_directory() -> PathBuf {
    runtime_paths().lqos_directory
}

fn bootstrap_state_path() -> PathBuf {
    runtime_paths()
        .state_directory
        .join("setup")
        .join("bootstrap_state.json")
}

fn completion_report_path() -> PathBuf {
    runtime_paths()
        .state_directory
        .join("setup")
        .join("completion_report.txt")
}

fn auth_file_has_admin(lqos_directory: &Path) -> bool {
    let path = lqos_directory.join("lqusers.toml");
    let Ok(raw) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(users) = toml::from_str::<AuthUsersFile>(&raw) else {
        return false;
    };
    users.users.iter().any(|user| user.role == UserRole::Admin)
}

fn collect_live_inputs() -> LiveInputs {
    let paths = runtime_paths();
    LiveInputs {
        config_loads: lqos_config::load_config().is_ok(),
        first_admin_exists: auth_file_has_admin(&paths.lqos_directory),
    }
}

fn display_status(setup_complete: bool) -> SetupStatus {
    if !setup_complete {
        SetupStatus::Incomplete
    } else {
        SetupStatus::Complete
    }
}

fn legacy_runtime_install_detected() -> bool {
    Path::new(LEGACY_CONFIG_PATH).exists()
}

/// The legacy v2 configuration path, present on every v2 install.
pub const LEGACY_CONFIG_PATH: &str = "/etc/lqos.conf";
/// The canonical v3 runtime configuration path the units pin as `LQOS_CONFIG`.
pub const RUNTIME_CONFIG_PATH: &str = lqos_config::DEFAULT_CONFIG_PATH;
/// The bridge runtime-config path when no config dir can be resolved.
pub const DEFAULT_BRIDGE_CONF_PATH: &str = "/etc/libreqos/bridge.conf";
/// Must equal `bridge_rs::config::CONFIG_SCHEMA_VERSION`.
pub const BRIDGE_CONF_SCHEMA_VERSION: u32 = 1;

/// A recorded legacy-config migration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigMigration {
    /// The legacy path the copy came from (never modified).
    pub from: PathBuf,
    /// The runtime path the config was copied to.
    pub to: PathBuf,
}

/// Migrate a legacy `/etc/lqos.conf` to the canonical v3 path when the v3 path
/// is absent. Copy-only: the original is never moved or removed. A v3 install
/// activates the runtime services on legacy detection, and those units read
/// [`RUNTIME_CONFIG_PATH`], so without this copy `lqosd` crash-loops on a box
/// whose only config is the legacy path. Idempotent: a present v3 config (or an
/// absent legacy config) is a no-op.
pub fn migrate_legacy_config() -> Result<Option<ConfigMigration>> {
    migrate_legacy_config_at(
        Path::new(LEGACY_CONFIG_PATH),
        Path::new(RUNTIME_CONFIG_PATH),
    )
}

/// The testable surface for [`migrate_legacy_config`].
pub fn migrate_legacy_config_at(legacy: &Path, runtime: &Path) -> Result<Option<ConfigMigration>> {
    if runtime.exists() || !legacy.exists() {
        return Ok(None);
    }
    if let Some(parent) = runtime.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Unable to create {}", parent.display()))?;
    }
    fs::copy(legacy, runtime).with_context(|| {
        format!(
            "Unable to migrate the legacy configuration {} to {}",
            legacy.display(),
            runtime.display()
        )
    })?;
    Ok(Some(ConfigMigration {
        from: legacy.to_path_buf(),
        to: runtime.to_path_buf(),
    }))
}

/// The bridge runtime-config path: the `bridge.conf` sibling of the resolved
/// configuration (`/etc/libreqos/bridge.conf` by default), which the shipped
/// `bridge.env` points `LIBREQOS_DPDK_CONFIG_FILE` at.
pub fn bridge_conf_path() -> PathBuf {
    Path::new(&lqos_config::config_path())
        .parent()
        .map(|dir| dir.join("bridge.conf"))
        .unwrap_or_else(|| PathBuf::from(DEFAULT_BRIDGE_CONF_PATH))
}

/// Ensure the typed-JSON `bridge.conf` exists, writing the minimal default when
/// absent. `bridge_rs` refuses to start when `LIBREQOS_DPDK_CONFIG_FILE` names
/// a missing file, and the setup/legacy activation path is its producer (the
/// env example + the support bundle stay readers).
///
/// §8 UPGRADE MIGRATION: the typed `circuit_capacity` was DELETED (the emitter
/// now DERIVES the capacity) and the bridge's `deny_unknown_fields` rejects the
/// stale key — leaving it in place would block every upgrade ("Refusing to
/// start: invalid runtime config"). An existing file has ONLY that key removed;
/// every other field is preserved verbatim. Idempotent: a file without the
/// legacy key is never rewritten.
pub fn ensure_bridge_conf() -> Result<Option<PathBuf>> {
    let config = lqos_config::load_config()
        .map(|config| (*config).clone())
        .unwrap_or_default();
    ensure_bridge_conf_with(&config)
}

/// Ensure the typed runtime config AND its `[single_interface]` topology, in
/// one call so every producer gets both (the setup success path and the
/// v2→v3 activation path). A topology failure is LOUD: the caller must not
/// write a minimal `{"schema_version":1}` and let the bridge die at start.
pub fn ensure_bridge_conf_with(config: &Config) -> Result<Option<PathBuf>> {
    let path = bridge_conf_path();
    let wrote_conf = ensure_bridge_conf_at(&path)?;
    let wrote_topology = ensure_bridge_interface_topology_at(&path, config)?;
    Ok(wrote_conf.or(wrote_topology))
}

/// A `[single_interface]` trunk topology for the engine's `interface_topology`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StickTopology {
    /// The trunk NIC's PCI BDF (never a virtio bus name).
    pub device: String,
    /// The LAN/network VLAN (the engine maps it to Upload).
    pub lan_vlan: u16,
    /// The WAN/internet VLAN (the engine maps it to Download).
    pub wan_vlan: u16,
}

fn single_vlan(value: u32, which: &str) -> Result<u16, String> {
    u16::try_from(value)
        .ok()
        .filter(|vlan| (1..=4094).contains(vlan))
        .ok_or_else(|| {
            format!(
                "[single_interface] {which} = {value} is not a usable VLAN ID; the engine \
                 accepts 1..=4094 (untagged/0 is dropped by design)"
            )
        })
}

/// The persisted single-interface BDF. Both keys carry the SAME PCI function
/// (the on-a-stick invariant); two DIFFERENT BDFs under a `[single_interface]`
/// config are ambiguous and fail loud rather than guessing a trunk.
fn persisted_single_bdf() -> Result<Option<String>, String> {
    let wan = bridge_env_value(ENV_WAN_BDF).filter(|v| !v.trim().is_empty());
    let lan = bridge_env_value(ENV_LAN_BDF).filter(|v| !v.trim().is_empty());
    match (wan, lan) {
        (Some(wan), Some(lan)) if wan.eq_ignore_ascii_case(&lan) => Ok(Some(wan)),
        (Some(wan), Some(lan)) => Err(format!(
            "bridge.env carries two DIFFERENT BDFs ({wan} / {lan}) but the config is \
             [single_interface]; the on-a-stick trunk expects the SAME BDF in both keys — \
             unbind and re-run setup to repair the pair"
        )),
        (Some(wan), None) => Ok(Some(wan)),
        (None, Some(lan)) => Ok(Some(lan)),
        (None, None) => Ok(None),
    }
}

/// Derive the engine topology from `[single_interface]`. `None` for a
/// `[bridge]`/unset config (the explicit WAN/LAN BDF pair is its orientation).
fn derive_single_interface_topology_at(
    sysfs: &Path,
    net_sysfs: &Path,
    config: &Config,
    recorded_device: Option<&str>,
) -> Result<Option<StickTopology>, String> {
    let Some(single) = config.single_interface.as_ref() else {
        return Ok(None);
    };
    let lan_vlan = single_vlan(single.network_vlan, "network_vlan")?;
    let wan_vlan = single_vlan(single.internet_vlan, "internet_vlan")?;
    if lan_vlan == wan_vlan {
        return Err(format!(
            "[single_interface] network_vlan and internet_vlan are both {lan_vlan}; the \
             on-a-stick trunk needs two DISTINCT VLANs"
        ));
    }
    let device = match dpdk_device_name_at(net_sysfs, &single.interface) {
        Some(device) => Some(device),
        None => match recorded_device.filter(|d| crate::port_class::is_pci_bdf(d)) {
            Some(recorded) => Some(recorded.to_string()),
            None => persisted_single_bdf()?,
        },
    };
    let Some(device) = device else {
        return Err(format!(
            "[single_interface] interface '{}' has no resolvable PCI BDF (the netdev is \
             absent and no persisted BDF exists); run setup BEFORE the DPDK bind, or \
             'lqos_setup bind-nics --unbind' and re-run setup",
            single.interface
        ));
    };
    crate::port_class::validate_single_device_at(sysfs, &device).map_err(|error| {
        format!(
            "[single_interface] interface '{}': {error}",
            single.interface
        )
    })?;
    Ok(Some(StickTopology {
        device,
        lan_vlan,
        wan_vlan,
    }))
}

fn stick_topology_json(topology: &StickTopology) -> serde_json::Value {
    serde_json::json!({
        "mode": "stick",
        "device": topology.device,
        "lan_vlan": topology.lan_vlan,
        "wan_vlan": topology.wan_vlan,
    })
}

fn existing_stick_matches(existing: &serde_json::Value, desired: &StickTopology) -> bool {
    existing.get("mode").and_then(|v| v.as_str()) == Some("stick")
        && existing
            .get("device")
            .and_then(|v| v.as_str())
            .is_some_and(|d| d.eq_ignore_ascii_case(&desired.device))
        && existing.get("lan_vlan").and_then(|v| v.as_u64()) == Some(desired.lan_vlan as u64)
        && existing.get("wan_vlan").and_then(|v| v.as_u64()) == Some(desired.wan_vlan as u64)
}

/// Write the `[single_interface]` topology into `bridge.conf`, preserving every
/// other key. Idempotent. A different, VALID existing topology is a loud
/// conflict (never silently overwritten); a non-PCI device is repaired (it is
/// never a deliberate engine topology).
pub fn ensure_bridge_interface_topology_at(
    path: &Path,
    config: &Config,
) -> Result<Option<PathBuf>> {
    ensure_bridge_interface_topology_at_with(
        path,
        config,
        Path::new("/sys"),
        Path::new("/sys/class/net"),
    )
}

/// The testable surface for [`ensure_bridge_interface_topology_at`].
pub fn ensure_bridge_interface_topology_at_with(
    path: &Path,
    config: &Config,
    sysfs: &Path,
    net_sysfs: &Path,
) -> Result<Option<PathBuf>> {
    if config.single_interface.is_none() {
        return Ok(None);
    }
    let mut value: serde_json::Value = if path.exists() {
        let body = fs::read_to_string(path)
            .with_context(|| format!("Unable to read {}", path.display()))?;
        serde_json::from_str(&body).with_context(|| {
            format!(
                "{} is not valid bridge JSON; refusing to rewrite an operator edit \
                 (the bridge fails closed on it)",
                path.display()
            )
        })?
    } else {
        serde_json::json!({ "schema_version": BRIDGE_CONF_SCHEMA_VERSION })
    };
    let object = value.as_object_mut().ok_or_else(|| {
        anyhow::anyhow!(
            "{} is not a JSON object; refusing to rewrite an operator edit",
            path.display()
        )
    })?;
    // A JSON `null` is the engine's equivalent of absent; treat it as absent
    // (a prior tool may have serialized an `Option::None`).
    let existing = object
        .get("interface_topology")
        .cloned()
        .filter(|topology| !topology.is_null());
    let recorded = existing
        .as_ref()
        .and_then(|topology| topology.get("device"))
        .and_then(|device| device.as_str());
    let Some(desired) = derive_single_interface_topology_at(sysfs, net_sysfs, config, recorded)
        .map_err(anyhow::Error::msg)?
    else {
        return Ok(None);
    };
    if let Some(existing) = existing.as_ref() {
        if existing_stick_matches(existing, &desired) {
            return Ok(None);
        }
        let existing_device = existing.get("device").and_then(|v| v.as_str());
        let repairable = existing.get("mode").and_then(|v| v.as_str()) == Some("stick")
            && existing_device
                .map(|d| !crate::port_class::is_pci_bdf(d))
                .unwrap_or(true);
        if !repairable {
            anyhow::bail!(
                "{} already carries a DIFFERENT interface_topology ({existing}); the \
                 [single_interface] config wants mode=stick device={} lan_vlan={} \
                 wan_vlan={} — refusing to overwrite a deliberate topology",
                path.display(),
                desired.device,
                desired.lan_vlan,
                desired.wan_vlan
            );
        }
    }
    if !object.contains_key("schema_version") {
        object.insert(
            "schema_version".to_string(),
            serde_json::json!(BRIDGE_CONF_SCHEMA_VERSION),
        );
    }
    object.insert(
        "interface_topology".to_string(),
        stick_topology_json(&desired),
    );
    crate::port_class::write_managed_dropin_at(path, &format!("{value}\n"))
        .with_context(|| format!("Unable to write the topology to {}", path.display()))?;
    Ok(Some(path.to_path_buf()))
}

/// The testable surface for [`ensure_bridge_conf`].
pub fn ensure_bridge_conf_at(path: &Path) -> Result<Option<PathBuf>> {
    if path.exists() {
        let body = fs::read_to_string(path)
            .with_context(|| format!("Unable to read {}", path.display()))?;
        let mut value: serde_json::Value = match serde_json::from_str(&body) {
            Ok(value) => value,
            // A hand-edited or torn file is left untouched: the bridge fails
            // LOUD on it, and setup must never destroy an operator's edit.
            Err(_) => return Ok(None),
        };
        let removed = value
            .as_object_mut()
            .is_some_and(|object| object.remove("circuit_capacity").is_some());
        if !removed {
            return Ok(None);
        }
        // Atomic (temp + rename) so the bridge unit never reads a partial file.
        crate::port_class::write_managed_dropin_at(path, &format!("{value}\n"))
            .with_context(|| format!("Unable to migrate {}", path.display()))?;
        return Ok(Some(path.to_path_buf()));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Unable to create {}", parent.display()))?;
    }
    let body = format!("{{\"schema_version\":{BRIDGE_CONF_SCHEMA_VERSION}}}\n");
    crate::port_class::write_managed_dropin_at(path, &body)
        .with_context(|| format!("Unable to write the default {}", path.display()))?;
    Ok(Some(path.to_path_buf()))
}

/// The shipped bridge environment file (EnvironmentFile= on the bridge unit).
pub const BRIDGE_ENV_PATH: &str = "/etc/libreqos/bridge.env";
/// The explicit WAN device env key (a DPDK PCI BDF).
pub const ENV_WAN_BDF: &str = "LQOS_DPDK_WAN_BDF";
/// The explicit LAN device env key (a DPDK PCI BDF).
pub const ENV_LAN_BDF: &str = "LQOS_DPDK_LAN_BDF";

/// The explicit dataplane WAN/LAN device pair (DPDK PCI BDFs).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BridgeBdfPair {
    /// The WAN (internet-facing) PCI BDF.
    pub wan: String,
    /// The LAN (network-facing) PCI BDF.
    pub lan: String,
}

/// Resolve a configured interface to its DPDK device name: the PCI BDF from
/// `/sys/class/net/<iface>/device`, through the ONE virtio-aware resolver
/// (`port_class::pci_bdf_of_netdev`). Only valid BEFORE the DPDK bind — after
/// it the kernel netdev (and this symlink) is gone, which is why the setup/bind
/// moment owns the translation.
///
/// `lqos-74t.37.8` B2: the old basename read returned `virtioN` on a VM. A bus
/// name is never a DPDK device name; reject it rather than persist it.
/// Resolve one netdev to its PCI BDF over an explicit `/sys/class/net` root
/// (production is `/sys/class/net`; tests use a fake tree).
fn dpdk_device_name_at(net_sysfs: &Path, iface: &str) -> Option<String> {
    crate::port_class::pci_bdf_of_netdev(net_sysfs, iface)
}

/// The pure shape/identity rules for a persisted WAN/LAN pair. The two sides
/// must be distinct PCI BDFs; the engine's orientation depends on it.
fn validate_bridge_bdf_pair(pair: &BridgeBdfPair) -> Result<(), String> {
    if !crate::port_class::is_pci_bdf(&pair.wan) || !crate::port_class::is_pci_bdf(&pair.lan) {
        return Err(format!(
            "the WAN/LAN pair is not a PCI BDF pair ({} / {}); never persist a bus name",
            pair.wan, pair.lan
        ));
    }
    if pair.wan.eq_ignore_ascii_case(&pair.lan) {
        return Err(format!(
            "the WAN and LAN interfaces resolve to the same PCI function {}; the two-port \
             bridge needs a distinct pair",
            pair.wan
        ));
    }
    Ok(())
}

/// Resolve the explicit WAN/LAN pair, fail-loud and named. Kept separate from
/// [`derive_bridge_bdf_pair`] so the activation path can tell the operator
/// exactly which side is unresolvable.
fn resolve_bridge_bdf_pair(config: &lqos_config::Config) -> Result<BridgeBdfPair, String> {
    resolve_bridge_bdf_pair_at(Path::new("/sys"), config)
}

/// [`resolve_bridge_bdf_pair`] over an explicit sysfs root (the oracle).
fn resolve_bridge_bdf_pair_at(
    sysfs: &Path,
    config: &lqos_config::Config,
) -> Result<BridgeBdfPair, String> {
    let bridge = config.bridge.as_ref().ok_or("no [bridge] section")?;
    let net_sysfs = sysfs.join("class/net");
    let wan = dpdk_device_name_at(&net_sysfs, &bridge.to_internet).ok_or_else(|| {
        format!(
            "cannot resolve a PCI BDF for [bridge] to_internet '{}' (a netdev is required \
             BEFORE the DPDK bind; after the bind only the persisted pair is available)",
            bridge.to_internet
        )
    })?;
    let lan = dpdk_device_name_at(&net_sysfs, &bridge.to_network).ok_or_else(|| {
        format!(
            "cannot resolve a PCI BDF for [bridge] to_network '{}' (a netdev is required \
             BEFORE the DPDK bind; after the bind only the persisted pair is available)",
            bridge.to_network
        )
    })?;
    let pair = BridgeBdfPair { wan, lan };
    validate_bridge_bdf_pair(&pair)?;
    for (role, bdf) in [("WAN", &pair.wan), ("LAN", &pair.lan)] {
        if let Some(error) = crate::port_class::virtio_class_error_at(sysfs, bdf) {
            return Err(format!("{role} {error}"));
        }
    }
    Ok(pair)
}

/// Derive the explicit WAN/LAN BDF pair from `/etc/libreqos/lqos.conf`'s
/// `[bridge] to_internet`/`to_network`. The setup/bind step is the ONLY holder
/// of the netdev->DPDK mapping (the netdev vanishes after the bind). `None`
/// when the pair cannot be resolved (already bound, or not a PCI pair).
pub fn derive_bridge_bdf_pair(config: &lqos_config::Config) -> Option<BridgeBdfPair> {
    resolve_bridge_bdf_pair(config).ok()
}

/// Resolve + write the explicit WAN/LAN BDF pair into the shipped `bridge.env`
/// (the bridge consumes the named pair; explicit beats positional ordering).
/// Idempotent: an already-matching pair is left untouched.
///
/// `lqos-74t.37.8` B2 pins: a post-bind boot cannot re-derive the BDF (the
/// netdev is gone), so a VALID persisted pair is the fact and is left alone.
/// A non-PCI or transitional persisted pair FAILS LOUD — never guess and never
/// keep a `virtioN` that the bind cannot resolve.
pub fn ensure_bridge_env_bdf_pair(config: &lqos_config::Config) -> Result<Option<PathBuf>> {
    // The one-port arm (lqos-74t.37.7): persist the SAME PCI BDF in BOTH env
    // keys. There is no new key/schema; `WAN == LAN` is the on-a-stick
    // invariant (the bind script's second bind is idempotent, and eal_args
    // emits one `-a`). The two-port validator stays strict.
    if let Some(single) = config.single_interface.as_ref() {
        let bdf = match dpdk_device_name_at(Path::new("/sys/class/net"), &single.interface) {
            Some(bdf) => bdf,
            None => persisted_single_bdf()
                .map_err(anyhow::Error::msg)?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "[single_interface] interface '{}' has no resolvable PCI BDF and \
                     bridge.env has no persisted BDF; run setup BEFORE the DPDK bind",
                        single.interface
                    )
                })?,
        };
        crate::port_class::validate_single_device(&bdf)
            .map_err(|error| anyhow::anyhow!("[single_interface] {error}"))?;
        return ensure_bridge_env_bdf_pair_at(
            Path::new(BRIDGE_ENV_PATH),
            &BridgeBdfPair {
                wan: bdf.clone(),
                lan: bdf,
            },
        );
    }
    let Some(_bridge) = config.bridge.as_ref() else {
        return Ok(None);
    };
    match resolve_bridge_bdf_pair(config) {
        Ok(pair) => ensure_bridge_env_bdf_pair_at(Path::new(BRIDGE_ENV_PATH), &pair),
        Err(err) => {
            if let Some(existing) = bridge_env_bdf_pair() {
                // A persisted pair is the post-bind fact, but it must still be
                // two PRESENT Ethernet functions (shape + class), distinct,
                // and a supported virtio class. Never bind a stale device.
                crate::port_class::validate_bind_pair(&existing.wan, &existing.lan)
                    .map_err(|e| anyhow::anyhow!("{err}; the persisted pair is invalid: {e}"))?;
                return Ok(None);
            }
            anyhow::bail!(
                "{err}; no persisted BDF pair in {BRIDGE_ENV_PATH} either — run setup \
                 BEFORE the DPDK bind so the netdev→PCI-BDF fact is captured"
            )
        }
    }
}

/// The env key recording the kernel driver to restore on rollback.
pub const ENV_ORIGINAL_DRIVER: &str = "LQOS_DPDK_ORIGINAL_DRIVER";

/// The per-port bind-originals state file written by the packaged bind unit
/// (`libreqos-bind-dpdk-ports.sh`). The ONE precedence for the removal/rollback
/// readers is: explicit arg -> setup env per-port -> this STATE -> singular.
pub const BIND_ORIGINALS_STATE: &str = "/var/lib/libreqos-bridge/dpdk-bind-originals.env";

/// Read a per-port original driver from the STATE file, if present.
pub fn state_original_driver(which: &str) -> Option<String> {
    state_value(&format!("LQOS_DPDK_ORIG_{which}_DRIVER"))
}

/// Read a `KEY=value` from the bind-originals STATE file. FIRST-wins: the
/// record keeps the first observation (the true pre-bind fact), and every
/// reader must take the same one. Used for the per-port drivers AND the
/// pre-bind BDFs (the rollback fact when bridge.env is stale).
pub fn state_value(key: &str) -> Option<String> {
    state_value_at(Path::new(BIND_ORIGINALS_STATE), key)
}

/// The testable [`state_value`] over an explicit path.
pub fn state_value_at(path: &Path, key: &str) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let needle = format!("{key}=");
    text.lines()
        .find_map(|line| line.strip_prefix(&needle))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Per-port pre-bind kernel drivers (the restore prefers these; the singular
/// `ENV_ORIGINAL_DRIVER` is the fallback for a same-driver host). The WAN/LAN
/// may differ (e.g. a mixed-NIC box), so the pair must not be collapsed.
pub const ENV_ORIG_WAN_DRIVER: &str = "LQOS_DPDK_ORIG_WAN_DRIVER";
/// See [`ENV_ORIG_WAN_DRIVER`].
pub const ENV_ORIG_LAN_DRIVER: &str = "LQOS_DPDK_ORIG_LAN_DRIVER";

/// The managed/operator drop-in dir (the unit's LATER `EnvironmentFile=`
/// glob). A drop-in wins over the packaged `bridge.env`.
pub const BRIDGE_ENV_DROPIN_DIR: &str = "/etc/libreqos/bridge.env.d";

/// Read a `KEY=value` from one env-style file (first match wins).
pub(crate) fn env_file_value(path: &Path, key: &str) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let prefix = format!("{key}=");
    text.lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix(&prefix)?.trim();
            (!rest.is_empty()).then(|| rest.to_string())
        })
        .last()
}

/// Read a `KEY=value` from the packaged **`/etc/libreqos/bridge.env` only**.
///
/// This is the `WorkersSource::LegacyEnv` runtime source (lqos-74t.160): the
/// managed `bridge.env.d/50-ports.conf` must never freeze a derived worker
/// count as an override. The one-time migration may read the managed file
/// through its own explicit path, never through this reader.
pub fn bridge_env_value_main(key: &str) -> Option<String> {
    bridge_env_value_main_at(Path::new(BRIDGE_ENV_PATH), key)
}

/// The testable [`bridge_env_value_main`] over an explicit path.
pub fn bridge_env_value_main_at(path: &Path, key: &str) -> Option<String> {
    env_file_value(path, key)
}

/// Read a `KEY=value` from `bridge.env` + `bridge.env.d/*.conf` in systemd's
/// order (lexical; the last definition wins). The bash bind script already
/// reads this way — the Rust readers must agree, or a stale main-file pair
/// can shadow the managed drop-in (the `.9` rollback trap).
///
/// NOTE: worker provenance must use [`bridge_env_value_main`]; this reader is
/// for the BDF/driver/path facts that legitimately live in drop-ins.
pub fn bridge_env_value(key: &str) -> Option<String> {
    let mut value = env_file_value(Path::new(BRIDGE_ENV_PATH), key);
    if let Ok(entries) = fs::read_dir(BRIDGE_ENV_DROPIN_DIR) {
        let mut dropins: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "conf"))
            .collect();
        dropins.sort();
        for file in dropins {
            if let Some(found) = env_file_value(&file, key) {
                value = Some(found);
            }
        }
    }
    value
}

/// The setup-written WAN/LAN BDF pair from `bridge.env`, when present.
pub fn bridge_env_bdf_pair() -> Option<BridgeBdfPair> {
    Some(BridgeBdfPair {
        wan: bridge_env_value(ENV_WAN_BDF)?,
        lan: bridge_env_value(ENV_LAN_BDF)?,
    })
}

/// Set/replace a `KEY=value` in the shipped `bridge.env`, leaving other lines.
pub fn set_bridge_env_value(key: &str, value: &str) -> Result<()> {
    set_bridge_env_value_at(Path::new(BRIDGE_ENV_PATH), key, value)
}

/// The testable surface for [`set_bridge_env_value`].
pub fn set_bridge_env_value_at(path: &Path, key: &str, value: &str) -> Result<()> {
    let existing = fs::read_to_string(path).unwrap_or_default();
    let prefix = format!("{key}=");
    let mut out = String::new();
    let mut replaced = false;
    for line in existing.lines() {
        if line.trim_start().starts_with(&prefix) {
            if replaced {
                continue;
            }
            out.push_str(&format!("{key}={value}\n"));
            replaced = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !replaced {
        out.push_str(&format!("{key}={value}\n"));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Unable to create {}", parent.display()))?;
    }
    fs::write(path, out).with_context(|| format!("Unable to write {key} to {}", path.display()))?;
    Ok(())
}

/// The testable surface for [`ensure_bridge_env_bdf_pair`].
pub fn ensure_bridge_env_bdf_pair_at(path: &Path, pair: &BridgeBdfPair) -> Result<Option<PathBuf>> {
    let existing = fs::read_to_string(path).unwrap_or_default();
    let desired = [
        (ENV_WAN_BDF, pair.wan.as_str()),
        (ENV_LAN_BDF, pair.lan.as_str()),
    ];
    if desired.iter().all(|(key, value)| {
        let want = format!("{key}={value}");
        existing
            .lines()
            .any(|line| line.trim_start() == want.as_str())
    }) {
        return Ok(None);
    }

    let wan_prefix = format!("{ENV_WAN_BDF}=");
    let lan_prefix = format!("{ENV_LAN_BDF}=");
    let mut out = String::new();
    for line in existing.lines() {
        let line = line.trim_start();
        if line.starts_with(wan_prefix.as_str()) || line.starts_with(lan_prefix.as_str()) {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.push_str(
        "# Dataplane WAN/LAN PCI BDFs derived from /etc/libreqos/lqos.conf's pair\n\
         # (written by setup at the bind moment; explicit beats positional).\n\
         # On-a-stick ([single_interface]): both keys carry the SAME trunk BDF.\n",
    );
    out.push_str(&format!("{ENV_WAN_BDF}={}\n", pair.wan));
    out.push_str(&format!("{ENV_LAN_BDF}={}\n", pair.lan));

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Unable to create {}", parent.display()))?;
    }
    fs::write(path, out)
        .with_context(|| format!("Unable to write the BDF pair to {}", path.display()))?;
    Ok(Some(path.to_path_buf()))
}

fn classify_postinst_action_from_inputs(
    runtime_ready: bool,
    legacy_runtime_install: bool,
) -> PostinstAction {
    if runtime_ready || legacy_runtime_install {
        PostinstAction::ActivateRuntime
    } else {
        PostinstAction::ActivateSetup
    }
}

/// The v3 runtime predicate. `lqos-74t.37.10`: the Noble systemd hotfix is a
/// **v2** artifact and is deliberately NOT an input here (nor in the status
/// snapshot or TUI). A host that still reports `hotfix::status().required`
/// starts v3 normally; the hotfix commands remain only as legacy v2 recovery.
fn v3_runtime_ready(setup_complete: bool, first_admin_exists: bool, config_loads: bool) -> bool {
    setup_complete && first_admin_exists && config_loads
}

fn detect_host_hints() -> Vec<String> {
    let mut hints = BTreeSet::new();

    if let Ok(hostname) = fs::read_to_string("/etc/hostname") {
        let hostname = hostname.trim();
        if !hostname.is_empty() {
            hints.insert(hostname.to_string());
        }
    }

    if let Ok(ifaddrs) = getifaddrs() {
        for iface in ifaddrs {
            if iface.interface_name == "lo" {
                continue;
            }
            let Some(address) = iface.address else {
                continue;
            };
            if address.family() != Some(AddressFamily::Inet) {
                continue;
            }
            let Some(inet) = address.as_sockaddr_in() else {
                continue;
            };
            let ip = inet.ip();
            if !ip.is_loopback() {
                hints.insert(ip.to_string());
            }
        }
    }

    if hints.is_empty() {
        hints.insert("127.0.0.1".to_string());
    }

    hints.into_iter().collect()
}

fn hydrate_state(mut state: BootstrapState, refresh_expired_token: bool) -> BootstrapState {
    if refresh_expired_token && state.token.expires_at_unix <= current_unix_seconds() {
        state.token = new_bootstrap_token();
    }
    state.version = BOOTSTRAP_STATE_VERSION;
    state
}

fn read_persisted_state() -> Result<BootstrapState> {
    let path = bootstrap_state_path();
    if !path.exists() {
        return Ok(BootstrapState::default());
    }
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("Unable to read bootstrap state {}", path.display()))?;
    serde_json::from_str::<BootstrapState>(&raw)
        .with_context(|| format!("Unable to parse bootstrap state {}", path.display()))
}

fn bootstrap_state_from_live(live: &LiveInputs) -> BootstrapState {
    BootstrapState {
        setup_complete: live.config_loads && live.first_admin_exists,
        first_admin_exists: live.first_admin_exists,
        ..BootstrapState::default()
    }
}

/// Loads persisted bootstrap state, creating it if needed and syncing derived fields.
fn load_or_create_state() -> Result<BootstrapState> {
    let path = bootstrap_state_path();
    let live = collect_live_inputs();
    let loaded = if path.exists() {
        read_persisted_state()?
    } else {
        bootstrap_state_from_live(&live)
    };
    let synced = hydrate_state(loaded, true);
    if let Some(parent) = path.parent()
        && let Err(err) = fs::create_dir_all(parent)
    {
        if path.exists() {
            return Err(err).with_context(|| {
                format!(
                    "Unable to create setup state directory {}",
                    parent.display()
                )
            });
        }
        return Ok(synced);
    }

    if let Err(err) = save_state(&synced)
        && path.exists()
    {
        return Err(err);
    }
    Ok(synced)
}

/// Persists bootstrap state to disk.
pub fn save_state(state: &BootstrapState) -> Result<()> {
    let path = bootstrap_state_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Unable to create setup state directory {}",
                parent.display()
            )
        })?;
    }
    let raw = serde_json::to_string_pretty(state).context("Unable to serialize bootstrap state")?;
    fs::write(&path, raw)
        .with_context(|| format!("Unable to write bootstrap state {}", path.display()))?;
    Ok(())
}

/// Stores the most recent setup completion report for the setup web UI.
pub fn store_setup_completion_report(report: &str) -> Result<()> {
    let path = completion_report_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Unable to create setup state directory {}",
                parent.display()
            )
        })?;
    }
    fs::write(&path, report)
        .with_context(|| format!("Unable to write setup completion report {}", path.display()))?;
    Ok(())
}

/// Loads the most recent setup completion report for the setup web UI.
pub fn load_setup_completion_report() -> Result<String> {
    let path = completion_report_path();
    fs::read_to_string(&path)
        .with_context(|| format!("Unable to read setup completion report {}", path.display()))
}

/// Records that setup successfully committed configuration.
pub fn record_setup_success(config: &Config) -> Result<BootstrapState> {
    let mut state = load_or_create_state()?;
    let live = LiveInputs {
        config_loads: true,
        first_admin_exists: auth_file_has_admin(Path::new(&config.lqos_directory)),
    };
    state.setup_complete = live.first_admin_exists;
    state.first_admin_exists = live.first_admin_exists;
    state = hydrate_state(state, true);
    save_state(&state)?;
    Ok(state)
}

/// Returns true when an admin user exists for the current or default runtime directory.
pub fn first_admin_exists() -> bool {
    auth_file_has_admin(&runtime_paths().lqos_directory)
}

/// Builds a current status snapshot for operator-facing CLI output.
pub fn status_snapshot() -> Result<StatusSnapshot> {
    let live = collect_live_inputs();
    let mut state = if bootstrap_state_path().exists() {
        hydrate_state(read_persisted_state()?, false)
    } else {
        bootstrap_state_from_live(&live)
    };
    state.first_admin_exists = live.first_admin_exists;
    let host_hints = detect_host_hints();
    let display_status = display_status(state.setup_complete);

    Ok(StatusSnapshot {
        state,
        display_status,
        host_hints,
        config_loads: live.config_loads,
    })
}

/// Renders a text report for `lqos_setup status`.
pub fn render_status_report() -> Result<String> {
    let snapshot = status_snapshot()?;
    let mut report = format!(
        "lqos_setup status\n\nStatus: {}",
        snapshot.display_status.as_str()
    );
    report.push_str("\nSource: persisted setup state plus current runtime diagnostics");

    report.push_str(&format!(
        "\n- setup_complete: {}",
        yes_no(snapshot.state.setup_complete)
    ));
    report.push_str(&format!(
        "\n- config_loads: {}",
        yes_no(snapshot.config_loads)
    ));
    report.push_str(&format!(
        "\n- first_admin_exists: {}",
        yes_no(snapshot.state.first_admin_exists)
    ));

    if !snapshot.state.setup_complete {
        report.push_str("\n\nHost hints:");
        for host in &snapshot.host_hints {
            report.push_str(&format!("\n- {host}"));
        }
        report.push_str("\n\nRun `lqos_setup tui` to complete the interactive setup.");
    }

    Ok(report)
}

/// Returns true when setup is complete enough for runtime services to start.
///
/// Side effects: none.
pub fn runtime_services_should_start() -> Result<bool> {
    let live = collect_live_inputs();
    let state = if bootstrap_state_path().exists() {
        hydrate_state(read_persisted_state()?, false)
    } else {
        bootstrap_state_from_live(&live)
    };
    Ok(v3_runtime_ready(
        state.setup_complete,
        live.first_admin_exists,
        live.config_loads,
    ))
}

/// Classifies which service mode package postinst should activate.
pub fn classify_postinst_action() -> Result<PostinstAction> {
    let runtime_ready = runtime_services_should_start()?;
    let legacy_runtime_install = legacy_runtime_install_detected();
    Ok(classify_postinst_action_from_inputs(
        runtime_ready,
        legacy_runtime_install,
    ))
}

/// Returns true when first-run setup has not yet been completed.
pub fn setup_is_incomplete() -> Result<bool> {
    let live = collect_live_inputs();
    let state = if bootstrap_state_path().exists() {
        hydrate_state(read_persisted_state()?, false)
    } else {
        bootstrap_state_from_live(&live)
    };
    Ok(!state.setup_complete || !live.first_admin_exists)
}

/// Validates a setup token against the persisted bootstrap state.
///
/// Side effects: none.
pub fn validate_setup_token(token: &str) -> Result<bool> {
    let state = if bootstrap_state_path().exists() {
        read_persisted_state()?
    } else {
        return Ok(false);
    };
    if state.setup_complete {
        return Ok(false);
    }
    Ok(state.token.expires_at_unix > current_unix_seconds() && state.token.token == token)
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

#[cfg(test)]
mod tests {
    use super::{
        BridgeBdfPair, PostinstAction, TOKEN_TTL, classify_postinst_action_from_inputs,
        current_unix_seconds, ensure_bridge_conf_at, ensure_bridge_env_bdf_pair_at,
        ensure_bridge_interface_topology_at_with, migrate_legacy_config_at, new_bootstrap_token,
        resolve_bridge_bdf_pair_at, set_bridge_env_value_at, v3_runtime_ready,
        validate_bridge_bdf_pair,
    };
    use lqos_config::{BridgeConfig, Config, SingleInterfaceConfig};
    use std::path::Path;

    fn scratch(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("lqos-boot-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    /// Mirror the real virtio sysfs shape: the netdev's `device` link points
    /// at `<pci>/virtioN`, whose PCI parent is the actual function.
    fn fake_virtio(root: &Path, iface: &str, bdf: &str, device_id: u16) {
        let net = root.join("class/net").join(iface);
        std::fs::create_dir_all(&net).unwrap();
        let pci = root.join("bus/pci/devices").join(bdf);
        std::fs::create_dir_all(pci.join("virtio2")).unwrap();
        std::fs::write(pci.join("vendor"), "0x1af4\n").unwrap();
        std::fs::write(pci.join("device"), format!("0x{device_id:04x}\n")).unwrap();
        std::fs::write(pci.join("class"), "0x020000\n").unwrap();
        std::os::unix::fs::symlink(pci.join("virtio2"), net.join("device")).unwrap();
    }

    fn bridge_config(wan: &str, lan: &str) -> Config {
        Config {
            bridge: Some(BridgeConfig {
                to_internet: wan.to_string(),
                to_network: lan.to_string(),
                mtu: None,
            }),
            single_interface: None,
            ..Config::default()
        }
    }

    #[test]
    fn a_virtio_bridge_pair_resolves_to_pci_bdfs() {
        let root = scratch("virtio-pair");
        fake_virtio(&root, "ens19", "0000:00:13.0", 0x1041);
        fake_virtio(&root, "ens20", "0000:00:14.0", 0x1041);
        let pair = resolve_bridge_bdf_pair_at(&root, &bridge_config("ens19", "ens20"))
            .expect("resolves the modern virtio pair");
        assert_eq!(pair.wan, "0000:00:13.0");
        assert_eq!(pair.lan, "0000:00:14.0");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_transitional_virtio_port_is_named_and_rejected() {
        let root = scratch("virtio-legacy");
        fake_virtio(&root, "ens19", "0000:00:13.0", 0x1041);
        fake_virtio(&root, "ens20", "0000:00:14.0", 0x1000);
        let error = resolve_bridge_bdf_pair_at(&root, &bridge_config("ens19", "ens20"))
            .expect_err("the transitional NIC must not be persisted");
        assert!(error.contains("1af4:1000"), "{error}");
        assert!(error.contains("disable-legacy=on"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unresolvable_netdev_is_never_guessed() {
        let root = scratch("virtio-missing");
        std::fs::create_dir_all(root.join("class/net")).unwrap();
        let error = resolve_bridge_bdf_pair_at(&root, &bridge_config("ens19", "ens20"))
            .expect_err("a missing netdev must fail loud");
        assert!(error.contains("ens19"), "{error}");
        assert!(error.contains("BEFORE the DPDK bind"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    fn single_config(iface: &str, internet_vlan: u32, network_vlan: u32) -> Config {
        Config {
            bridge: None,
            single_interface: Some(SingleInterfaceConfig {
                interface: iface.to_string(),
                internet_vlan,
                network_vlan,
                mtu: None,
            }),
            ..Config::default()
        }
    }

    #[test]
    fn single_interface_writes_the_exact_stick_topology_and_keeps_other_keys() {
        let root = scratch("single-topology");
        fake_virtio(&root, "ens19", "0000:00:13.0", 0x1041);
        let path = root.join("bridge.conf");
        std::fs::write(&path, "{\"schema_version\":1,\"queue_mode\":\"shape\"}\n").unwrap();
        let config = single_config("ens19", 100, 200);
        let written = ensure_bridge_interface_topology_at_with(
            &path,
            &config,
            &root,
            &root.join("class/net"),
        )
        .expect("writes")
        .expect("changed");
        assert_eq!(written, path);
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            value
                .pointer("/interface_topology/mode")
                .and_then(|v| v.as_str()),
            Some("stick")
        );
        assert_eq!(
            value
                .pointer("/interface_topology/device")
                .and_then(|v| v.as_str()),
            Some("0000:00:13.0")
        );
        assert_eq!(
            value
                .pointer("/interface_topology/lan_vlan")
                .and_then(|v| v.as_u64()),
            Some(200),
            "network_vlan is the LAN side"
        );
        assert_eq!(
            value
                .pointer("/interface_topology/wan_vlan")
                .and_then(|v| v.as_u64()),
            Some(100),
            "internet_vlan is the WAN side"
        );
        assert_eq!(
            value.get("queue_mode").and_then(|v| v.as_str()),
            Some("shape"),
            "every other bridge.conf key is preserved"
        );
        // Idempotent: a correct topology is not rewritten.
        assert!(
            ensure_bridge_interface_topology_at_with(
                &path,
                &config,
                &root,
                &root.join("class/net"),
            )
            .unwrap()
            .is_none()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn single_interface_repairs_a_bus_name_but_refuses_a_deliberate_conflict() {
        let root = scratch("single-conflict");
        fake_virtio(&root, "ens19", "0000:00:13.0", 0x1041);
        let path = root.join("bridge.conf");
        std::fs::write(
            &path,
            "{\"schema_version\":1,\"interface_topology\":{\"mode\":\"stick\",\
             \"device\":\"virtio2\",\"lan_vlan\":200,\"wan_vlan\":100}}\n",
        )
        .unwrap();
        let config = single_config("ens19", 100, 200);
        // A non-PCI device is never a deliberate topology: repair it.
        assert!(
            ensure_bridge_interface_topology_at_with(
                &path,
                &config,
                &root,
                &root.join("class/net"),
            )
            .unwrap()
            .is_some()
        );
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            value
                .pointer("/interface_topology/device")
                .and_then(|v| v.as_str()),
            Some("0000:00:13.0")
        );
        // A valid but DIFFERENT topology is a loud conflict.
        std::fs::write(
            &path,
            "{\"schema_version\":1,\"interface_topology\":{\"mode\":\"stick\",\
             \"device\":\"0000:00:14.0\",\"lan_vlan\":200,\"wan_vlan\":100}}\n",
        )
        .unwrap();
        assert!(
            ensure_bridge_interface_topology_at_with(
                &path,
                &config,
                &root,
                &root.join("class/net"),
            )
            .is_err()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn single_interface_rejects_an_invalid_vlan_and_never_guesses_a_bdf() {
        let root = scratch("single-invalid");
        fake_virtio(&root, "ens19", "0000:00:13.0", 0x1041);
        let path = root.join("bridge.conf");
        std::fs::write(&path, "{\"schema_version\":1}\n").unwrap();
        let equal = single_config("ens19", 100, 100);
        assert!(
            ensure_bridge_interface_topology_at_with(
                &path,
                &equal,
                &root,
                &root.join("class/net"),
            )
            .is_err()
        );
        let untagged = single_config("ens19", 0, 200);
        assert!(
            ensure_bridge_interface_topology_at_with(
                &path,
                &untagged,
                &root,
                &root.join("class/net"),
            )
            .is_err()
        );
        // The netdev is gone and no persisted BDF exists -> named failure.
        std::fs::remove_dir_all(root.join("class/net")).unwrap();
        let config = single_config("ens19", 100, 200);
        let error = ensure_bridge_interface_topology_at_with(
            &path,
            &config,
            &root,
            &root.join("class/net"),
        )
        .expect_err("must fail loud");
        assert!(
            error.to_string().contains("no resolvable PCI BDF"),
            "{error}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_single_interface_pair_persists_the_same_bdf_in_both_keys() {
        let root = scratch("single-bdf-pair");
        let path = root.join("etc/libreqos/bridge.env");
        let pair = BridgeBdfPair {
            wan: "0000:00:13.0".to_string(),
            lan: "0000:00:13.0".to_string(),
        };
        ensure_bridge_env_bdf_pair_at(&path, &pair)
            .unwrap()
            .expect("writes");
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("LQOS_DPDK_WAN_BDF=0000:00:13.0"), "{body}");
        assert!(body.contains("LQOS_DPDK_LAN_BDF=0000:00:13.0"), "{body}");
        // The bind script's second bind is idempotent; the env write is too.
        assert!(
            ensure_bridge_env_bdf_pair_at(&path, &pair)
                .unwrap()
                .is_none()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_bus_name_is_never_a_valid_bridge_pair() {
        let bus_names = BridgeBdfPair {
            wan: "virtio2".to_string(),
            lan: "virtio3".to_string(),
        };
        assert!(validate_bridge_bdf_pair(&bus_names).is_err());

        let same = BridgeBdfPair {
            wan: "0000:00:13.0".to_string(),
            lan: "0000:00:13.0".to_string(),
        };
        assert!(validate_bridge_bdf_pair(&same).is_err());

        let ok = BridgeBdfPair {
            wan: "0000:00:13.0".to_string(),
            lan: "0000:00:14.0".to_string(),
        };
        assert!(validate_bridge_bdf_pair(&ok).is_ok());
    }

    #[test]
    fn the_bdf_pair_is_written_and_kept() {
        let root = scratch("bridge-env-bdf");
        let path = root.join("etc/libreqos/bridge.env");
        let pair = BridgeBdfPair {
            wan: "0000:00:10.0".to_string(),
            lan: "0000:00:10.1".to_string(),
        };
        ensure_bridge_env_bdf_pair_at(&path, &pair)
            .unwrap()
            .expect("writes");
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("LQOS_DPDK_WAN_BDF=0000:00:10.0"), "{body}");
        assert!(body.contains("LQOS_DPDK_LAN_BDF=0000:00:10.1"), "{body}");
        // Idempotent: a matching pair is left untouched.
        assert!(
            ensure_bridge_env_bdf_pair_at(&path, &pair)
                .unwrap()
                .is_none()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_bdf_pair_is_updated_in_place_and_other_lines_kept() {
        let root = scratch("bridge-env-bdf-update");
        let path = root.join("etc/libreqos/bridge.env");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "KEEP=1\nLQOS_DPDK_WAN_BDF=old\nLQOS_DPDK_LAN_BDF=old2\nTAIL=2\n",
        )
        .unwrap();
        let pair = BridgeBdfPair {
            wan: "0000:00:10.0".to_string(),
            lan: "0000:00:10.1".to_string(),
        };
        ensure_bridge_env_bdf_pair_at(&path, &pair)
            .unwrap()
            .expect("writes");
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("LQOS_DPDK_WAN_BDF=0000:00:10.0"), "{body}");
        assert!(!body.contains("=old"), "{body}");
        assert!(body.contains("KEEP=1") && body.contains("TAIL=2"), "{body}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_bridge_env_value_is_set_replaced_and_kept() {
        let root = scratch("bridge-env-set");
        let path = root.join("etc/libreqos/bridge.env");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "KEEP=1\n").unwrap();
        set_bridge_env_value_at(&path, "LQOS_DPDK_ORIGINAL_DRIVER", "i40e").unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("LQOS_DPDK_ORIGINAL_DRIVER=i40e"), "{body}");
        assert!(body.contains("KEEP=1"), "{body}");
        // Replace in place.
        set_bridge_env_value_at(&path, "LQOS_DPDK_ORIGINAL_DRIVER", "ice").unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("LQOS_DPDK_ORIGINAL_DRIVER=ice"), "{body}");
        assert!(!body.contains("=i40e"), "{body}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_legacy_config_migrates_without_touching_the_original() {
        let root = scratch("migrate");
        let legacy = root.join("etc/lqos.conf");
        let runtime = root.join("etc/libreqos/lqos.conf");
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, "[bridge]\nto_internet = \"ens19\"\n").unwrap();

        let migration = migrate_legacy_config_at(&legacy, &runtime)
            .unwrap()
            .expect("migrates");
        assert_eq!(migration.from, legacy);
        assert_eq!(migration.to, runtime);
        assert!(runtime.exists() && legacy.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_present_v3_config_is_never_overwritten() {
        let root = scratch("migrate-idempotent");
        let legacy = root.join("etc/lqos.conf");
        let runtime = root.join("etc/libreqos/lqos.conf");
        std::fs::create_dir_all(runtime.parent().unwrap()).unwrap();
        std::fs::write(&legacy, "legacy = true\n").unwrap();
        std::fs::write(&runtime, "runtime = true\n").unwrap();
        assert!(
            migrate_legacy_config_at(&legacy, &runtime)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            std::fs::read_to_string(&runtime).unwrap(),
            "runtime = true\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn bridge_conf_default_omits_the_deleted_capacity_key() {
        let root = scratch("bridge-conf");
        let path = root.join("etc/libreqos/bridge.conf");
        let written = ensure_bridge_conf_at(&path).unwrap().expect("writes");
        assert_eq!(written, path);
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("\"schema_version\":1"), "{body}");
        assert!(
            !body.contains("circuit_capacity"),
            "the §8 deleted key must never be written: {body}"
        );
        // Idempotent: a current file is never rewritten.
        assert!(ensure_bridge_conf_at(&path).unwrap().is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// §8 upgrade: an EXISTING `bridge.conf` carrying the deleted
    /// `circuit_capacity` must MIGRATE (drop that key, keep the rest) so the
    /// bridge's `deny_unknown_fields` cannot block the upgrade.
    #[test]
    fn bridge_conf_migrates_the_deleted_capacity_key_and_keeps_the_rest() {
        let root = scratch("bridge-conf-migrate");
        let path = root.join("etc/libreqos/bridge.conf");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "{\"schema_version\":1,\"circuit_capacity\":4096,\"queue_mode\":\"shape\"}\n",
        )
        .unwrap();
        let migrated = ensure_bridge_conf_at(&path).unwrap().expect("migrates");
        assert_eq!(migrated, path);
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            value.get("schema_version").and_then(|v| v.as_u64()),
            Some(1)
        );
        assert_eq!(
            value.get("queue_mode").and_then(|v| v.as_str()),
            Some("shape")
        );
        assert!(
            value.get("circuit_capacity").is_none(),
            "the legacy key must be dropped"
        );
        // Idempotent: a second pass is a no-op.
        assert!(ensure_bridge_conf_at(&path).unwrap().is_none());
        // An unparsable operator edit is left alone (never clobbered).
        std::fs::write(&path, "not json").unwrap();
        assert!(ensure_bridge_conf_at(&path).unwrap().is_none());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "not json");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn bootstrap_tokens_expire_in_future() {
        let now = current_unix_seconds();
        let token = new_bootstrap_token();
        assert!(!token.token.is_empty());
        assert!(token.expires_at_unix >= now + TOKEN_TTL.as_secs() - 1);
    }

    #[test]
    fn a_required_legacy_hotfix_state_does_not_block_v3() {
        // lqos-74t.37.10: the v3 predicate has NO hotfix input. A host whose
        // legacy v2 recovery surface still reports "required" starts normally.
        assert!(v3_runtime_ready(true, true, true));
        assert!(!v3_runtime_ready(false, true, true));
    }

    #[test]
    fn postinst_action_keeps_existing_runtime_installs_on_runtime_services() {
        assert_eq!(
            classify_postinst_action_from_inputs(false, true),
            PostinstAction::ActivateRuntime
        );
    }

    #[test]
    fn postinst_action_only_activates_setup_for_true_first_run() {
        assert_eq!(
            classify_postinst_action_from_inputs(false, false),
            PostinstAction::ActivateSetup
        );
    }
}
