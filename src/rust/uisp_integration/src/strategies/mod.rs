mod common;
mod full2;
mod legacy_bandwidth_overrides;
mod legacy_mikrotik;
mod legacy_parse;
mod legacy_routes_override;
mod legacy_uisp_fetch;

use crate::blackboard;
use crate::errors::UispIntegrationError;
use crate::ethernet_advisory::write_ethernet_advisories;
use crate::ip_ranges::IpRanges;
use lqos_bus::BlackboardSystem;
use lqos_config::{Config, circuit_anchors_path};
use lqos_topology_compile::{
    CompiledTopologyBundle, TopologyCompileMode, TopologyCompiledShapingFile, TopologyImportFile,
    compile_topology,
};
use std::io::ErrorKind;
use std::sync::Arc;
use tracing::{error, info, warn};

fn compile_mode_for_strategy(strategy: &str) -> Result<TopologyCompileMode, UispIntegrationError> {
    match strategy {
        "flat" => Ok(TopologyCompileMode::Flat),
        "ap_only" => Ok(TopologyCompileMode::ApOnly),
        "ap_site" => Ok(TopologyCompileMode::ApSite),
        "full" => Ok(TopologyCompileMode::Full),
        "full2" => Ok(TopologyCompileMode::Full2),
        _ => Err(UispIntegrationError::UnknownIntegrationStrategy),
    }
}

fn write_compiled_outputs(
    config: &Config,
    topology_import: &TopologyImportFile,
    compiled_shaping: &TopologyCompiledShapingFile,
    compiled: CompiledTopologyBundle,
) -> Result<(), UispIntegrationError> {
    topology_import.save(config).map_err(|e| {
        error!("Unable to write topology_import.json: {e:?}");
        UispIntegrationError::WriteNetJson
    })?;
    compiled_shaping.save(config).map_err(|e| {
        error!("Unable to write topology_compiled_shaping.json: {e:?}");
        UispIntegrationError::WriteNetJson
    })?;
    compiled.parent_candidates.save(config).map_err(|e| {
        error!("Unable to write topology parent candidates snapshot: {e:?}");
        UispIntegrationError::WriteNetJson
    })?;
    compiled.editor.save(config).map_err(|e| {
        error!("Unable to write topology editor state snapshot: {e:?}");
        UispIntegrationError::WriteNetJson
    })?;
    compiled.canonical.save(config).map_err(|e| {
        error!("Unable to write topology canonical state snapshot: {e:?}");
        UispIntegrationError::WriteNetJson
    })?;
    let anchors_path = circuit_anchors_path(config);
    if let Err(err) = std::fs::remove_file(&anchors_path)
        && err.kind() != ErrorKind::NotFound
    {
        warn!(
            "Unable to remove stale duplicate circuit_anchors.json at {}: {err:?}",
            anchors_path.display()
        );
    }
    write_ethernet_advisories(config, &compiled.ethernet_advisories)?;
    Ok(())
}

/// Builds the network using the selected strategy.
pub async fn build_with_strategy(
    config: Arc<Config>,
    ip_ranges: IpRanges,
) -> Result<(), UispIntegrationError> {
    let strategy = config.resolved_topology_compile_mode_for_uisp().to_string();
    blackboard(BlackboardSystem::System, "UISP", strategy.as_str()).await;
    let mode = compile_mode_for_strategy(&strategy)?;
    info!("Strategy selected: {strategy}");
    let imported = full2::build_imported_full2_bundle(config.clone(), ip_ranges).await?;
    let topology_import = TopologyImportFile::from_imported_bundle(&imported, strategy.clone());
    let compiled = compile_topology(imported, mode).map_err(|e| {
        error!("Unable to compile UISP topology for mode '{strategy}': {e:?}");
        UispIntegrationError::WriteNetJson
    })?;
    let compiled_shaping = TopologyCompiledShapingFile::from_compiled(&compiled, strategy.clone());
    write_compiled_outputs(
        config.as_ref(),
        &topology_import,
        &compiled_shaping,
        compiled,
    )
}
