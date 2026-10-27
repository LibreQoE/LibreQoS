use allocative::Allocative;
use serde::{Deserialize, Serialize};

/// Shared topology compiler settings used by `lqos_topology`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Allocative)]
pub struct TopologyConfig {
    /// Selected topology compile mode.
    ///
    /// Empty means "use legacy per-integration fallback" during the transition period.
    #[serde(default)]
    pub compile_mode: String,
}

/// Normalizes operator-facing topology compile mode strings.
///
/// `full2` is accepted as a legacy UISP alias and maps to `full`.
pub fn normalize_topology_compile_mode(mode: &str) -> Option<&'static str> {
    match mode.trim().to_ascii_lowercase().as_str() {
        "flat" => Some("flat"),
        "ap_only" => Some("ap_only"),
        "ap_site" => Some("ap_site"),
        "full" => Some("full"),
        "full2" => Some("full"),
        _ => None,
    }
}

/// Returns true when `mode` is supported by the named integration.
pub fn integration_supports_topology_compile_mode(integration: &str, mode: &str) -> bool {
    let Some(mode) = normalize_topology_compile_mode(mode) else {
        return false;
    };
    match integration.trim().to_ascii_lowercase().as_str() {
        "uisp" | "splynx" => matches!(mode, "flat" | "ap_only" | "ap_site" | "full"),
        "sonar" => matches!(mode, "flat" | "full"),
        _ => matches!(mode, "full"),
    }
}

/// Resolves one operator-facing compile mode for a specific integration, returning `None`
/// when the requested mode is unsupported.
pub fn normalize_supported_topology_compile_mode(
    integration: &str,
    mode: &str,
) -> Option<&'static str> {
    let mode = normalize_topology_compile_mode(mode)?;
    integration_supports_topology_compile_mode(integration, mode).then_some(mode)
}
