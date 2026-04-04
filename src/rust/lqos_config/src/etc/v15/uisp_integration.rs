use allocative::Allocative;
use serde::{Deserialize, Serialize};

fn default_airmax_flexible_frame_download_ratio() -> f32 {
    0.8
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
    pub use_ptmp_as_parent: bool,
    #[serde(default = "default_ignore_calculated_capacity")]
    pub ignore_calculated_capacity: bool,
    pub insecure_ssl: Option<bool>,

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
    }
}
