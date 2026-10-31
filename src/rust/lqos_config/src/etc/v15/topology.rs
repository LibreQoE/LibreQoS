use allocative::Allocative;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

const ALL_COMPILE_MODES: &[&str] = &["flat", "ap_only", "ap_site", "full"];
const SONAR_COMPILE_MODES: &[&str] = &["flat", "full"];
const FULL_COMPILE_MODE: &[&str] = &["full"];

/// One active topology source.
///
/// The enum is the single authority for who produces the topology; the legacy
/// `enable_*` flags are migrated to it and normalised on save.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize, Allocative)]
#[serde(rename_all = "snake_case")]
pub enum TopologySource {
    Uisp,
    Splynx,
    Netzur,
    Visp,
    Sonar,
    Powercode,
    Wispgate,
    ShapedDevices,
    LibreqosNetwork,
}

impl TopologySource {
    /// Every accepted source, in card order.
    pub const ALL: [Self; 9] = [
        Self::Uisp,
        Self::Splynx,
        Self::Netzur,
        Self::Visp,
        Self::Sonar,
        Self::Powercode,
        Self::Wispgate,
        Self::ShapedDevices,
        Self::LibreqosNetwork,
    ];

    /// The seven integration sources (the two file sources are excluded).
    pub const INTEGRATION_SOURCES: [Self; 7] = [
        Self::Uisp,
        Self::Splynx,
        Self::Netzur,
        Self::Visp,
        Self::Sonar,
        Self::Powercode,
        Self::Wispgate,
    ];

    /// The serialized snake_case value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Uisp => "uisp",
            Self::Splynx => "splynx",
            Self::Netzur => "netzur",
            Self::Visp => "visp",
            Self::Sonar => "sonar",
            Self::Powercode => "powercode",
            Self::Wispgate => "wispgate",
            Self::ShapedDevices => "shaped_devices",
            Self::LibreqosNetwork => "libreqos_network",
        }
    }

    /// Operator-facing display name (card §6.1).
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Uisp => "UISP",
            Self::Splynx => "Splynx",
            Self::Netzur => "Netzur",
            Self::Visp => "VISP",
            Self::Sonar => "Sonar",
            Self::Powercode => "Powercode",
            Self::Wispgate => "WispGate",
            Self::ShapedDevices => "Shaped devices",
            Self::LibreqosNetwork => "LibreQoS network",
        }
    }

    /// True for the seven integrations; false for the two file sources.
    pub fn is_integration(self) -> bool {
        !matches!(self, Self::ShapedDevices | Self::LibreqosNetwork)
    }

    /// The example section for an integration source (used by the example
    /// parse test); the two file sources return `None`.
    pub fn integration_section(self) -> Option<&'static str> {
        match self {
            Self::Uisp => Some("uisp_integration"),
            Self::Splynx => Some("splynx_integration"),
            Self::Netzur => Some("netzur_integration"),
            Self::Visp => Some("visp_integration"),
            Self::Sonar => Some("sonar_integration"),
            Self::Powercode => Some("powercode_integration"),
            Self::Wispgate => Some("wispgate_integration"),
            Self::ShapedDevices | Self::LibreqosNetwork => None,
        }
    }

    /// The matching legacy `enable_*` key, if this source has one.
    pub fn enable_flag_key(self) -> Option<&'static str> {
        match self {
            Self::Uisp => Some("enable_uisp"),
            Self::Splynx => Some("enable_splynx"),
            Self::Netzur => Some("enable_netzur"),
            Self::Visp => Some("enable_visp"),
            Self::Sonar => Some("enable_sonar"),
            Self::Powercode => Some("enable_powercode"),
            Self::Wispgate => Some("enable_wispgate"),
            Self::ShapedDevices | Self::LibreqosNetwork => None,
        }
    }

    /// The compile modes accepted by this source.
    pub fn supported_compile_modes(self) -> &'static [&'static str] {
        match self {
            Self::Uisp
            | Self::Splynx
            | Self::ShapedDevices
            | Self::LibreqosNetwork => ALL_COMPILE_MODES,
            Self::Sonar => SONAR_COMPILE_MODES,
            Self::Netzur | Self::Visp | Self::Powercode | Self::Wispgate => FULL_COMPILE_MODE,
        }
    }

    /// Case-insensitive parse of the operator-facing value.
    pub fn from_value(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "uisp" => Some(Self::Uisp),
            "splynx" => Some(Self::Splynx),
            "netzur" => Some(Self::Netzur),
            "visp" => Some(Self::Visp),
            "sonar" => Some(Self::Sonar),
            "powercode" => Some(Self::Powercode),
            "wispgate" => Some(Self::Wispgate),
            "shaped_devices" => Some(Self::ShapedDevices),
            "libreqos_network" => Some(Self::LibreqosNetwork),
            _ => None,
        }
    }

    /// Accepted values for fail-loud messages.
    pub fn accepted_list() -> &'static str {
        "uisp, splynx, netzur, visp, sonar, powercode, wispgate, shaped_devices, libreqos_network"
    }
}

/// Deserializes `[topology].source`: trim + ASCII-lowercase, empty -> `None`,
/// unknown -> a typed error naming the accepted set.
pub fn deserialize_topology_source<'de, D>(
    deserializer: D,
) -> Result<Option<TopologySource>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<String>::deserialize(deserializer)?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.trim().is_empty() {
        return Ok(None);
    }
    TopologySource::from_value(&raw)
        .map(Some)
        .ok_or_else(|| {
            D::Error::custom(format!(
                "unknown topology source '{}'; accepted values: {}",
                raw.trim(),
                TopologySource::accepted_list()
            ))
        })
}

/// Shared topology compiler settings used by `lqos_topology`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Allocative)]
pub struct TopologyConfig {
    /// Selected topology source. Absent/empty before the one-time migration.
    #[serde(default, deserialize_with = "deserialize_topology_source")]
    pub source: Option<TopologySource>,

    /// Selected topology compile mode.
    ///
    /// Empty means "use the default (`full`)" during the transition period.
    #[serde(default)]
    pub compile_mode: String,
}

/// How the current `topology_source` value came to be.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Allocative)]
#[serde(rename_all = "snake_case")]
pub enum TopologySourceOrigin {
    /// No source is configured (degraded row-8 conflict).
    #[default]
    Missing,
    /// The source was already present in `lqos.conf` at load.
    Config,
    /// This load migrated the source from the legacy flags or the D1 default.
    Migrated,
}

impl TopologySourceOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Config => "config",
            Self::Migrated => "migrated",
        }
    }
}

/// A loud, degraded source conflict (D11).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Allocative)]
pub struct TopologySourceConflict {
    /// The selected source, when one was retained (rows 13/14); `null` on row 8.
    #[serde(default)]
    pub source: Option<TopologySource>,
    /// The enabled legacy flag names that caused the conflict.
    #[serde(default)]
    pub enabled: Vec<String>,
    /// The exact operator-facing conflict message.
    #[serde(default)]
    pub message: String,
}

/// Runtime-only topology-source state. `Config` carries this with
/// `#[serde(skip)]`; it is never written to `lqos.conf`.
#[derive(Clone, Debug, Default, PartialEq, Allocative)]
pub struct TopologySourceRuntime {
    pub conflict: Option<TopologySourceConflict>,
    pub origin: TopologySourceOrigin,
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
        "uisp" | "splynx" | "shaped_devices" | "libreqos_network" => {
            matches!(mode, "flat" | "ap_only" | "ap_site" | "full")
        }
        "sonar" => matches!(mode, "flat" | "full"),
        _ => matches!(mode, "full"),
    }
}

/// Cross-field validation: `source` x `compile_mode`.
///
/// An empty mode resolves to `full`; an unknown mode is always rejected, even
/// when no source is selected yet.
pub fn validate_topology_selection(
    source: Option<TopologySource>,
    compile_mode: &str,
) -> Result<(), String> {
    let mode = if compile_mode.trim().is_empty() {
        "full"
    } else {
        normalize_topology_compile_mode(compile_mode).ok_or_else(|| {
            "topology.compile_mode must be one of flat, ap_only, ap_site, or full".to_string()
        })?
    };
    let Some(source) = source else {
        return Ok(());
    };
    if !source.supported_compile_modes().contains(&mode) {
        return Err(format!(
            "topology.compile_mode '{mode}' is not supported by topology source '{}'; supported: {}",
            source.as_str(),
            source.supported_compile_modes().join(", ")
        ));
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Deserialize)]
    struct SourceWrapper {
        #[serde(default, deserialize_with = "deserialize_topology_source")]
        source: Option<TopologySource>,
    }

    #[test]
    fn all_nine_sources_parse_round_trip() {
        for source in TopologySource::ALL {
            let parsed: SourceWrapper =
                toml::from_str(&format!("source = \"{}\"", source.as_str())).unwrap();
            assert_eq!(parsed.source, Some(source));
            assert_eq!(TopologySource::from_value(source.as_str()), Some(source));
        }
    }

    #[test]
    fn source_deserializer_trims_lowercases_and_treats_empty_as_absent() {
        let parsed: SourceWrapper = toml::from_str("source = \"  UISP  \"").unwrap();
        assert_eq!(parsed.source, Some(TopologySource::Uisp));
        let parsed: SourceWrapper = toml::from_str("source = \"   \"").unwrap();
        assert_eq!(parsed.source, None);
        let parsed: SourceWrapper = toml::from_str("").unwrap();
        assert_eq!(parsed.source, None);
    }

    #[test]
    fn source_deserializer_rejects_unknown_with_accepted_set() {
        let error = toml::from_str::<SourceWrapper>("source = \"nope\"").unwrap_err();
        let text = error.to_string();
        assert!(text.contains("unknown topology source 'nope'"), "{text}");
        assert!(text.contains("shaped_devices"), "{text}");
        assert!(text.contains("libreqos_network"), "{text}");
    }

    #[test]
    fn compile_mode_support_matrix_matches_the_card() {
        for source in TopologySource::ALL {
            assert!(validate_topology_selection(Some(source), "").is_ok());
            assert!(validate_topology_selection(Some(source), "full").is_ok());
        }
        for source in TopologySource::INTEGRATION_SOURCES {
            let allowed = source.supported_compile_modes();
            for mode in ["flat", "ap_only", "ap_site", "full"] {
                assert_eq!(
                    validate_topology_selection(Some(source), mode).is_ok(),
                    allowed.contains(&mode),
                    "{source:?} {mode}"
                );
            }
        }
        assert!(validate_topology_selection(Some(TopologySource::Visp), "flat").is_err());
        assert!(validate_topology_selection(Some(TopologySource::Splynx), "ap_only").is_ok());
        assert!(validate_topology_selection(None, "nope").is_err());
    }

    #[test]
    fn file_sources_have_no_legacy_flag() {
        for source in [TopologySource::ShapedDevices, TopologySource::LibreqosNetwork] {
            assert!(!source.is_integration());
            assert_eq!(source.enable_flag_key(), None);
        }
        assert_eq!(
            TopologySource::Wispgate.enable_flag_key(),
            Some("enable_wispgate")
        );
    }
}
