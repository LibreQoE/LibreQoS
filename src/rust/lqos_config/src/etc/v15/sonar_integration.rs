use serde::{Serialize, Deserialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SonarIntegration {
    pub enable_sonar: bool,
    pub sonar_api_url: String,
    pub sonar_api_key: String,
    pub snmp_community: String,
    pub airmax_model_ids: Vec<String>,
    pub ltu_model_ids: Vec<String>,
    pub active_status_ids: Vec<String>,
}

impl Default for SonarIntegration {
    fn default() -> Self {
        SonarIntegration {
            enable_sonar: false,
            sonar_api_url: "".to_string(),
            sonar_api_key: "".to_string(),
            snmp_community: "public".to_string(),
            airmax_model_ids: vec![],
            ltu_model_ids: vec![],
            active_status_ids: vec![],
        }
    }
}