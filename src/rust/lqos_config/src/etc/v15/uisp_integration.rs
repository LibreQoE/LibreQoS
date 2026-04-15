use allocative::Allocative;
use serde::{Deserialize, Serialize};

fn default_airmax_flexible_frame_download_ratio() -> f32 {
    0.8
}

fn default_infrastructure_transport_caps_enabled() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Allocative)]
pub struct UispIntegration {
    pub enable_uisp: bool,
    pub token: String,
    pub url: String,
    pub site: String,
    pub strategy: String,
    pub suspended_strategy: String,
    pub airmax_capacity: f32,
    #[serde(default = "default_airmax_flexible_frame_download_ratio")]
    pub airmax_flexible_frame_download_ratio: f32,
    pub ltu_capacity: f32,
    pub exclude_sites: Vec<String>,
    pub squash_sites: Option<Vec<String>>,
    pub ipv6_with_mikrotik: bool,
    pub bandwidth_overhead_factor: f32,
    pub commit_bandwidth_multiplier: f32,
    pub exception_cpes: Vec<ExceptionCpe>,
    /// Deprecated legacy importer-side PtMP-parent toggle. Existing values are ignored.
    #[serde(default, skip_serializing)]
    pub use_ptmp_as_parent: bool,
    #[serde(default = "default_ignore_calculated_capacity")]
    pub ignore_calculated_capacity: bool,
    #[serde(default = "default_infrastructure_transport_caps_enabled")]
    pub infrastructure_transport_caps_enabled: bool,
    pub insecure_ssl: Option<bool>,

    /// Deprecated legacy importer-side squashing toggle. Existing values are ignored.
    pub enable_squashing: Option<bool>,
    pub do_not_squash_sites: Option<Vec<String>>,
}

fn default_ignore_calculated_capacity() -> bool {
    false
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Allocative)]
pub struct ExceptionCpe {
    pub cpe: String,
    pub parent: String,
}

impl Default for UispIntegration {
    fn default() -> Self {
        UispIntegration {
            enable_uisp: false,
            token: "".to_string(),
            url: "".to_string(),
            site: "".to_string(),
            strategy: "".to_string(),
            suspended_strategy: "".to_string(),
            airmax_capacity: 1.0,
            airmax_flexible_frame_download_ratio: default_airmax_flexible_frame_download_ratio(),
            ltu_capacity: 1.0,
            exclude_sites: vec![],
            squash_sites: None,
            ipv6_with_mikrotik: false,
            bandwidth_overhead_factor: 1.0,
            commit_bandwidth_multiplier: 1.0,
            exception_cpes: vec![],
            use_ptmp_as_parent: false,
            ignore_calculated_capacity: false,
            infrastructure_transport_caps_enabled: default_infrastructure_transport_caps_enabled(),
            insecure_ssl: None,
            enable_squashing: None,
            do_not_squash_sites: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::UispIntegration;

    #[test]
    fn default_capacity_defaults_match_new_install_policy() {
        let config = UispIntegration::default();
        assert_eq!(config.airmax_capacity, 1.0);
        assert_eq!(config.airmax_flexible_frame_download_ratio, 0.8);
        assert_eq!(config.ltu_capacity, 1.0);
        assert!(config.infrastructure_transport_caps_enabled);
    }

    #[test]
    fn deprecated_ptmp_parent_flag_loads_but_does_not_serialize() {
        let config: UispIntegration = toml::from_str(
            r#"
enable_uisp = true
token = ""
url = ""
site = ""
strategy = ""
suspended_strategy = ""
airmax_capacity = 1.0
ltu_capacity = 1.0
exclude_sites = []
ipv6_with_mikrotik = false
bandwidth_overhead_factor = 1.0
commit_bandwidth_multiplier = 1.0
exception_cpes = []
use_ptmp_as_parent = true
"#,
        )
        .expect("deprecated use_ptmp_as_parent key should still deserialize");

        assert!(config.use_ptmp_as_parent);

        let serialized =
            toml::to_string(&config).expect("uisp integration config should serialize");
        assert!(!serialized.contains("use_ptmp_as_parent"));
    }
}
