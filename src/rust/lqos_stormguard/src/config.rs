use crate::STORMGUARD_STATS;
use allocative::Allocative;
use lqos_bus::TcHandle;
use lqos_overrides::{NetworkAdjustment, OverrideFile, OverrideLayer, OverrideStore};
use std::collections::{HashMap, HashSet};
use tracing::{debug, info, warn};

use lqos_config::StormguardStrategy;

#[derive(Allocative, Clone)]
pub struct WatchingSite {
    /// Stable canonical node id, when the watched site resolved one. The
    /// persisted clamp is keyed by this id (rung-3 1a); `name` is the
    /// display-only fallback for legacy id-less topologies.
    pub node_id: Option<String>,
    pub name: String,
    pub max_download_mbps: u64,
    pub max_upload_mbps: u64,
    pub min_download_mbps: u64,
    pub min_upload_mbps: u64,
    pub dependent_nodes: Vec<WatchingSiteDependency>,
    pub current_download_mbps: u64,
    pub current_upload_mbps: u64,
}

#[derive(Allocative, Clone)]
pub struct WatchingSiteDependency {
    pub name: String,
    pub class_id: TcHandle,
    pub original_min_download_mbps: u64,
    pub original_min_upload_mbps: u64,
    pub original_max_download_mbps: u64,
    pub original_max_upload_mbps: u64,
}

pub struct StormguardConfig {
    pub sites: HashMap<String, WatchingSite>,
    // K3-B: kernel interface selection; no reader remains after the kernel
    // strategy arms were deleted. Retained for config-shape stability.
    #[allow(dead_code)]
    pub download_interface: String,
    #[allow(dead_code)]
    pub upload_interface: String,
    pub dry_run: bool,
    pub log_filename: Option<String>,
    pub strategy: StormguardStrategy,
    pub increase_fast_multiplier: f64,
    pub increase_multiplier: f64,
    pub decrease_multiplier: f64,
    pub decrease_fast_multiplier: f64,
    pub increase_fast_cooldown_seconds: f32,
    pub increase_cooldown_seconds: f32,
    pub decrease_cooldown_seconds: f32,
    pub decrease_fast_cooldown_seconds: f32,
    pub circuit_fallback_enabled: bool,
    pub circuit_fallback_sqm: String,
    pub delay_threshold_ms: f32,
    pub delay_threshold_ratio: f32,
    pub baseline_alpha_up: f32,
    pub baseline_alpha_down: f32,
    pub probe_interval_seconds: f32,
    pub min_throughput_mbps_for_rtt: f32,
    pub active_ping_target: String,
    pub active_ping_interval_seconds: f32,
    pub active_ping_weight: f32,
    pub active_ping_timeout_seconds: f32,
    /// K1b §4.6 pin 1: TTL (seconds) stamped on machine-authored site clamps.
    pub override_ttl_seconds: u64,
    /// K1b §4.8: bound (seconds) on waiting for a clamp's minted generation.
    pub publish_wait_seconds: u64,
}

impl StormguardConfig {
    pub fn is_empty(&self) -> bool {
        self.sites.is_empty()
    }
}

pub fn configure() -> anyhow::Result<StormguardConfig> {
    debug!("Configuring LibreQoS StormGuard...");
    let config = lqos_config::load_config()?;

    let Some(sg_config) = &config.stormguard else {
        debug!("StormGuard is not enabled in the configuration.");
        return Err(anyhow::anyhow!(
            "StormGuard is not enabled in the configuration."
        ));
    };
    if !sg_config.enabled {
        debug!("StormGuard is not enabled in the configuration.");
        return Err(anyhow::anyhow!(
            "StormGuard is not enabled in the configuration."
        ));
    }

    let persisted_site_overrides = if sg_config.dry_run {
        OverrideFile::default()
    } else {
        load_stormguard_site_overrides(&config)
    };
    let sites = get_sites_from_canonical(&config, sg_config, &persisted_site_overrides);

    let result = StormguardConfig {
        sites,
        download_interface: config.isp_interface().clone(),
        upload_interface: config.internet_interface().clone(),
        dry_run: sg_config.dry_run,
        log_filename: sg_config.log_file.clone(),
        strategy: sg_config.strategy,
        increase_fast_multiplier: sg_config.increase_fast_multiplier as f64,
        increase_multiplier: sg_config.increase_multiplier as f64,
        decrease_multiplier: sg_config.decrease_multiplier as f64,
        decrease_fast_multiplier: sg_config.decrease_fast_multiplier as f64,
        increase_fast_cooldown_seconds: sg_config.increase_fast_cooldown_seconds,
        increase_cooldown_seconds: sg_config.increase_cooldown_seconds,
        decrease_cooldown_seconds: sg_config.decrease_cooldown_seconds,
        decrease_fast_cooldown_seconds: sg_config.decrease_fast_cooldown_seconds,
        circuit_fallback_enabled: sg_config.circuit_fallback_enabled,
        circuit_fallback_sqm: sg_config.circuit_fallback_sqm.trim().to_ascii_lowercase(),
        delay_threshold_ms: sg_config.delay_threshold_ms,
        delay_threshold_ratio: sg_config.delay_threshold_ratio,
        baseline_alpha_up: sg_config.baseline_alpha_up,
        baseline_alpha_down: sg_config.baseline_alpha_down,
        probe_interval_seconds: sg_config.probe_interval_seconds,
        min_throughput_mbps_for_rtt: sg_config.min_throughput_mbps_for_rtt,
        active_ping_target: sg_config.active_ping_target.clone(),
        active_ping_interval_seconds: sg_config.active_ping_interval_seconds,
        active_ping_weight: sg_config.active_ping_weight,
        active_ping_timeout_seconds: sg_config.active_ping_timeout_seconds,
        override_ttl_seconds: sg_config.override_ttl_seconds,
        publish_wait_seconds: sg_config.publish_wait_seconds,
    };

    Ok(result)
}

/// Loads the persisted StormGuard layer, running the one-time rung-3 1a
/// name→id migration in the same pass. Name-only `AdjustSiteSpeed` entries
/// whose site name resolves to a canonical node id are rewritten to carry the
/// id and the layer is persisted (with a `.pre-id-migration.bak` backup);
/// entries whose name no longer resolves are left name-only and reported by
/// the stale-target WARN at the site fold. Lapsed machine entries are filtered
/// out of the returned view without rewriting the layer (the periodic sweep
/// owns that).
fn load_stormguard_site_overrides(config: &lqos_config::Config) -> OverrideFile {
    let mut overrides = match OverrideStore::load_layer(OverrideLayer::Stormguard) {
        Ok(overrides) => overrides,
        Err(error) => {
            warn!("Unable to load StormGuard override layer; starting from planned rates: {error}");
            return OverrideFile::default();
        }
    };

    migrate_stormguard_site_overrides_to_node_ids(config, &mut overrides);

    // K1b §4.6 pin 1 startup filter: a restart must not resurrect a clamp
    // whose TTL lapsed while the process was down.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    overrides.remove_lapsed_site_speed_overrides(now);
    overrides
}

/// Rung-3 1a: rewrites persisted name-only site-speed clamps to stable node
/// ids using the canonical topology, backing up the pre-migration file once.
///
/// Side effects: reads the canonical topology state and may back up + persist
/// the StormGuard override layer.
fn migrate_stormguard_site_overrides_to_node_ids(
    config: &lqos_config::Config,
    overrides: &mut OverrideFile,
) {
    let node_ids_by_name = match canonical_node_ids_by_name(config) {
        Ok(node_ids_by_name) => node_ids_by_name,
        Err(error) => {
            warn!(
                "StormGuard could not read canonical topology to migrate name-keyed site clamps; leaving them name-keyed: {error}"
            );
            return;
        }
    };
    if !overrides.migrate_site_bandwidth_overrides_to_node_ids(&node_ids_by_name) {
        return;
    }
    if let Err(error) = backup_stormguard_layer(config) {
        warn!("StormGuard could not back up the override layer before id migration: {error}");
    }
    if let Err(error) = OverrideStore::save_layer(OverrideLayer::Stormguard, overrides) {
        warn!("StormGuard could not persist the migrated site-clamp layer: {error}");
    } else {
        info!("StormGuard migrated persisted site-speed clamps from names to node ids");
    }
}

/// The canonical node-name → node-id map used by the one-time site-clamp
/// migration. Duplicate names keep the first id seen (canonical validation
/// already rejects duplicate ids).
///
/// Side effects: reads the canonical topology state.
fn canonical_node_ids_by_name(
    config: &lqos_config::Config,
) -> anyhow::Result<HashMap<String, String>> {
    let canonical = lqos_config::TopologyCanonicalStateFile::load_with_legacy_fallback(config)?;
    let mut node_ids_by_name: HashMap<String, String> = HashMap::new();
    for node in &canonical.nodes {
        let name = node.node_name.trim();
        let node_id = node.node_id.trim();
        if name.is_empty() || node_id.is_empty() {
            continue;
        }
        node_ids_by_name
            .entry(name.to_string())
            .or_insert_with(|| node_id.to_string());
    }
    Ok(node_ids_by_name)
}

/// Writes a one-shot `.pre-id-migration.bak` copy of the StormGuard layer,
/// preserving the pre-migration name-keyed form. A prior backup is never
/// overwritten.
fn backup_stormguard_layer(config: &lqos_config::Config) -> std::io::Result<()> {
    let path =
        OverrideStore::path_for_layer_config(config, OverrideLayer::Stormguard);
    if !path.exists() {
        return Ok(());
    }
    let backup = path.with_extension("json.pre-id-migration.bak");
    if backup.exists() {
        return Ok(());
    }
    std::fs::copy(&path, &backup)?;
    Ok(())
}

/// True for canonical nodes StormGuard may watch as sites: a site-typed node.
/// Mirrors the queue structure's site selection, but from the canonical model —
/// the un-clamped source (K1b card §1.2; the post-fold tree self-ratchets a
/// clamp into its own "planned" ceiling).
fn is_canonical_site(node: &lqos_config::TopologyCanonicalNode) -> bool {
    node.node_kind.is_empty() || node.node_kind.eq_ignore_ascii_case("site")
}

fn get_sites_from_canonical(
    config: &lqos_config::Config,
    sg_config: &lqos_config::StormguardConfig,
    persisted_site_overrides: &OverrideFile,
) -> HashMap<String, WatchingSite> {
    let canonical =
        match lqos_config::TopologyCanonicalStateFile::load_with_legacy_fallback(config) {
            Ok(state) => state,
            Err(error) => {
                warn!(
                    "StormGuard could not load canonical topology state; no sites watched: {error}"
                );
                return HashMap::new();
            }
        };

    let mut selected: Vec<String> = if sg_config.all_sites {
        canonical
            .nodes
            .iter()
            .filter(|node| is_canonical_site(node))
            .map(|node| node.node_name.clone())
            .collect()
    } else {
        sg_config.targets.clone()
    };

    let excluded: HashSet<&str> = sg_config
        .exclude_sites
        .iter()
        .map(|site| site.as_str())
        .collect();

    selected.retain(|site| !excluded.contains(site.as_str()));
    selected.sort();
    selected.dedup();

    let nodes_by_name: HashMap<&str, &lqos_config::TopologyCanonicalNode> = canonical
        .nodes
        .iter()
        .map(|node| (node.node_name.as_str(), node))
        .collect();

    let mut sites = HashMap::new();
    {
        let mut lock = STORMGUARD_STATS.lock();
        lock.clear();
    }

    for target in selected {
        let Some(node) = nodes_by_name.get(target.as_str()).copied() else {
            warn!("StormGuard target '{target}' is absent from the canonical topology; skipping");
            continue;
        };
        let max_down = node
            .rate_input
            .intrinsic_download_mbps
            .or(node.rate_input.legacy_imported_download_mbps)
            .unwrap_or(0);
        let max_up = node
            .rate_input
            .intrinsic_upload_mbps
            .or(node.rate_input.legacy_imported_upload_mbps)
            .unwrap_or(0);
        let min_down = (max_down as f32 * sg_config.minimum_download_percentage) as u64;
        let min_up = (max_up as f32 * sg_config.minimum_upload_percentage) as u64;
        let node_id = node.node_id.trim();
        let node_id = (!node_id.is_empty()).then(|| node_id.to_string());
        // Rung-3 1a: the persisted clamp joins by stable node id first, falling
        // back to the site name only for legacy id-less topologies or
        // pre-migration name-only entries.
        let persisted = persisted_site_overrides
            .find_site_bandwidth_override(node_id.as_deref(), &node.node_name)
            .and_then(|adj| match adj {
                NetworkAdjustment::AdjustSiteSpeed {
                    download_bandwidth_mbps,
                    upload_bandwidth_mbps,
                    ..
                } => Some((*download_bandwidth_mbps, *upload_bandwidth_mbps)),
                _ => None,
            })
            .unwrap_or((None, None));
        let current_download_mbps = persisted
            .0
            .map(|mbps| mbps.max(0.0) as u64)
            .unwrap_or(max_down)
            .clamp(min_down, max_down);
        let current_upload_mbps = persisted
            .1
            .map(|mbps| mbps.max(0.0) as u64)
            .unwrap_or(max_up)
            .clamp(min_up, max_up);

        // K1b card §1.2: the kernel-arm class handles die on the bridge path;
        // they are only meaningful where the bakery's queue structure exists.
        // K3-B retires the queue structure; there are no kernel dependents.
        let dependent_nodes = Vec::new();

        let site = WatchingSite {
            node_id,
            name: target.to_owned(),
            max_download_mbps: max_down,
            max_upload_mbps: max_up,
            min_download_mbps: min_down,
            min_upload_mbps: min_up,
            dependent_nodes,
            current_download_mbps,
            current_upload_mbps,
        };
        sites.insert(target.to_owned(), site);
        {
            let mut lock = STORMGUARD_STATS.lock();
            lock.push((
                target.to_owned(),
                current_download_mbps,
                current_upload_mbps,
            ));
        }
    }
    warn_stale_persisted_site_overrides(persisted_site_overrides, &sites);
    sites
}

/// T4 amendment 2 (t4_legacy_display_names_derivation.md §5.2): a
/// persisted StormGuard site-speed entry can match no rolled-out site
/// after a tree change and would silently never be read again.
/// Rung-3 1a: an id-carrying entry is matched by its stable node id;
/// a legacy name-only entry is matched by name (the display-only
/// fallback). Returns the display identity of every unmatched entry so
/// the caller can name them — the loud-miss doctrine.
fn stale_persisted_site_overrides(
    persisted_site_overrides: &OverrideFile,
    sites: &HashMap<String, WatchingSite>,
) -> Vec<String> {
    let node_ids: HashSet<&str> = sites
        .values()
        .filter_map(|site| site.node_id.as_deref())
        .collect();
    let names: HashSet<&str> = sites.values().map(|site| site.name.as_str()).collect();
    persisted_site_overrides
        .network_adjustments()
        .iter()
        .filter_map(|adj| match adj {
            NetworkAdjustment::AdjustSiteSpeed { node_id, site_name, .. } => {
                let id = node_id.as_deref().map(str::trim).filter(|id| !id.is_empty());
                let matched = match id {
                    Some(id) => node_ids.contains(id),
                    None => names.contains(site_name.as_str()),
                };
                if matched {
                    None
                } else {
                    Some(id.unwrap_or(site_name).to_string())
                }
            }
            _ => None,
        })
        .collect()
}

fn warn_stale_persisted_site_overrides(
    persisted_site_overrides: &OverrideFile,
    sites: &HashMap<String, WatchingSite>,
) {
    let stale = stale_persisted_site_overrides(persisted_site_overrides, sites);
    if !stale.is_empty() {
        warn!(
            "StormGuard persisted site-speed override(s) match no current site (stale after a naming change, ignored): {:?}",
            stale
        );
    }
}

#[cfg(test)]
mod t4_stale_override_tests {
    use super::*;

    fn watching_site(name: &str) -> WatchingSite {
        watching_site_with_id(name, None)
    }

    fn watching_site_with_id(name: &str, node_id: Option<&str>) -> WatchingSite {
        WatchingSite {
            node_id: node_id.map(str::to_string),
            name: name.to_string(),
            max_download_mbps: 1_000,
            max_upload_mbps: 500,
            min_download_mbps: 100,
            min_upload_mbps: 50,
            dependent_nodes: Vec::new(),
            current_download_mbps: 1_000,
            current_upload_mbps: 500,
        }
    }

    /// T4 amendment 2 gate: persisted name-only entries whose site
    /// name matches no rolled-out site are REPORTED (the loud-miss
    /// doctrine) while live entries stay unflagged. This is the
    /// round-trip stale shape: an artifact-id-era or re-keyed name
    /// survives in lqos_overrides.stormguard.json after the flip.
    #[test]
    fn stale_persisted_site_overrides_are_detected_live_names_pass() {
        let mut persisted = OverrideFile::default();
        persisted.set_site_bandwidth_override(
            None,
            "s10-a-ap0".to_string(),
            Some(940.0),
            Some(470.0),
        );
        persisted.set_site_bandwidth_override(
            None,
            "tower-1 [3f2a91c0]".to_string(),
            None,
            None,
        );

        let mut sites: HashMap<String, WatchingSite> = HashMap::new();
        sites.insert("s10-a-ap0".to_string(), watching_site("s10-a-ap0"));

        let mut stale = stale_persisted_site_overrides(&persisted, &sites);
        stale.sort_unstable();
        assert_eq!(stale, vec!["tower-1 [3f2a91c0]"]);

        // The round-trip: once the rolled-out sites carry the renamed
        // world, the persisted entry is live again and nothing is
        // flagged.
        sites.insert(
            "tower-1 [3f2a91c0]".to_string(),
            watching_site("tower-1 [3f2a91c0]"),
        );
        assert!(stale_persisted_site_overrides(&persisted, &sites).is_empty());
    }

    /// 1a: an id-carrying entry is live by id even when its stored display
    /// name has gone stale after a rename, and an id-carrying entry whose id
    /// no longer exists is reported by the loud unknown-id WARN.
    #[test]
    fn stale_persisted_site_overrides_join_by_id_across_renames() {
        let mut persisted = OverrideFile::default();
        persisted.set_site_bandwidth_override(
            Some("node-1".to_string()),
            "old-name".to_string(),
            Some(800.0),
            None,
        );
        persisted.set_site_bandwidth_override(
            Some("node-gone".to_string()),
            "gone".to_string(),
            None,
            None,
        );

        let sites: HashMap<String, WatchingSite> = HashMap::from([(
            "new-name".to_string(),
            watching_site_with_id("new-name", Some("node-1")),
        )]);

        let mut stale = stale_persisted_site_overrides(&persisted, &sites);
        stale.sort_unstable();
        assert_eq!(stale, vec!["node-gone"]);
    }
}

#[cfg(test)]
mod startup_decoupling_tests {
    use super::*;
    use lqos_config::{
        Config, StormguardConfig as GlobalStormguardConfig, TopologyCanonicalNode,
        TopologyCanonicalRateInput, TopologyCanonicalRateInputSource, TopologyCanonicalStateFile,
    };

    fn temp_config(name: &str) -> Config {
        let dir = std::env::temp_dir().join(format!(
            "lqos-sg-decouple-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("state").join("topology"))
            .expect("temp state dir should create");
        let mut config = Config::default();
        config.lqos_directory = dir.to_string_lossy().to_string();
        config.state_directory = None;
        config
    }

    fn write_canonical_site(config: &Config, node_name: &str, download: u64, upload: u64) {
        let state = TopologyCanonicalStateFile {
            source: "test".to_string(),
            nodes: vec![TopologyCanonicalNode {
                node_id: "node-1".to_string(),
                node_name: node_name.to_string(),
                node_kind: "Site".to_string(),
                rate_input: TopologyCanonicalRateInput {
                    intrinsic_download_mbps: Some(download),
                    intrinsic_upload_mbps: Some(upload),
                    source: TopologyCanonicalRateInputSource::OperatorOverride,
                    ..Default::default()
                },
                ..Default::default()
            }],
            ..Default::default()
        };
        let path = config.topology_state_file_path("topology_canonical_state.json");
        std::fs::write(
            path,
            serde_json::to_string_pretty(&state).expect("canonical should serialize"),
        )
        .expect("canonical should write");
    }

    /// K1b startup decoupling card §2 gate 1 (HIGH-1): the watched-site source
    /// must be the UN-CLAMPED canonical rate. A persisted StormGuard clamp must
    /// not ratchet its own "planned" ceiling (which would make restore target
    /// the clamp). Fails against any post-fold source.
    #[test]
    fn canonical_site_rates_are_unclamped_by_a_stormguard_layer_entry() {
        let config = temp_config("parity");
        write_canonical_site(&config, "site-1", 1000, 500);

        let mut persisted = OverrideFile::default();
        persisted.set_site_bandwidth_override(
            None,
            "site-1".to_string(),
            Some(800.0),
            Some(400.0),
        );
        let sg = GlobalStormguardConfig {
            enabled: true,
            dry_run: false,
            targets: vec!["site-1".to_string()],
            ..Default::default()
        };

        let sites = get_sites_from_canonical(&config, &sg, &persisted);
        let site = sites.get("site-1").expect("site should be watched");
        assert_eq!(
            site.max_download_mbps, 1000,
            "planned download ceiling is canonical/un-clamped"
        );
        assert_eq!(
            site.max_upload_mbps, 500,
            "planned upload ceiling is canonical/un-clamped"
        );
        assert_eq!(
            site.current_download_mbps, 800,
            "current rate reflects the persisted clamp"
        );
        assert_eq!(site.current_upload_mbps, 400);
    }

    /// 1a rename-preserves-the-clamp: the persisted clamp is joined by stable
    /// node id, so a site whose display name changed still re-reads its clamp
    /// (a name-keyed join would silently lose it).
    #[test]
    fn id_keyed_site_clamp_survives_a_rename() {
        let config = temp_config("rename");
        write_canonical_site(&config, "renamed-site", 1000, 500);

        let mut persisted = OverrideFile::default();
        persisted.set_site_bandwidth_override(
            Some("node-1".to_string()),
            "old-site-name".to_string(),
            Some(700.0),
            Some(350.0),
        );
        let sg = GlobalStormguardConfig {
            enabled: true,
            dry_run: false,
            targets: vec!["renamed-site".to_string()],
            ..Default::default()
        };

        let sites = get_sites_from_canonical(&config, &sg, &persisted);
        let site = sites.get("renamed-site").expect("site should be watched");
        assert_eq!(site.node_id.as_deref(), Some("node-1"));
        assert_eq!(
            site.current_download_mbps, 700,
            "id-keyed clamp survives the rename"
        );
        assert_eq!(site.current_upload_mbps, 350);
    }

    /// K1b startup decoupling card §2 gate 6: `all_sites` selects site-typed
    /// canonical nodes.
    #[test]
    fn all_sites_selects_site_typed_canonical_nodes() {
        let config = temp_config("allsites");
        write_canonical_site(&config, "site-1", 1000, 500);
        let sg = GlobalStormguardConfig {
            enabled: true,
            dry_run: false,
            all_sites: true,
            ..Default::default()
        };
        let sites = get_sites_from_canonical(&config, &sg, &OverrideFile::default());
        assert!(sites.contains_key("site-1"));
    }

    /// 1a "old-name files load + migrate": a legacy name-only clamp resolves
    /// through the canonical name→id map and is rewritten to carry the id.
    #[test]
    fn canonical_name_to_id_map_drives_the_legacy_migration() {
        let config = temp_config("namemigrate");
        write_canonical_site(&config, "site-1", 1000, 500);
        let node_ids_by_name =
            canonical_node_ids_by_name(&config).expect("canonical should read");
        assert_eq!(node_ids_by_name.get("site-1").map(String::as_str), Some("node-1"));

        let mut persisted = OverrideFile::default();
        persisted.set_site_bandwidth_override(
            None,
            "site-1".to_string(),
            Some(800.0),
            Some(400.0),
        );
        assert!(persisted.migrate_site_bandwidth_overrides_to_node_ids(&node_ids_by_name));
        assert!(matches!(
            persisted.find_site_bandwidth_override(Some("node-1"), "site-1"),
            Some(NetworkAdjustment::AdjustSiteSpeed {
                node_id: Some(node_id),
                download_bandwidth_mbps: Some(download),
                ..
            }) if node_id == "node-1" && *download == 800.0
        ));
    }
}
