use serde::{Serialize, Deserialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct UispIntegration {
    pub enable_uisp: bool,
    pub token: String,
    pub url: String,
    pub site: String,
    pub strategy: String,
    pub suspended_strategy: String,
    pub airmax_capacity: f32,
    pub ltu_capacity: f32,
    pub exclude_sites: Vec<String>,
    pub ipv6_with_mikrotik: bool,
    pub bandwidth_overhead_factor: f32,
    pub commit_bandwidth_multiplier: f32,
    pub exception_cpes: Vec<ExceptionCpe>,
    pub use_ptmp_as_parent: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
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
            airmax_capacity: 0.0,
            ltu_capacity: 0.0,
            exclude_sites: vec![],
            ipv6_with_mikrotik: false,
            bandwidth_overhead_factor: 1.0,
            commit_bandwidth_multiplier: 1.0,
            exception_cpes: vec![],
            use_ptmp_as_parent: false,
        }
    }
}