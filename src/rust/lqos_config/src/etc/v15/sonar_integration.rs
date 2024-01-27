use serde::{Serialize, Deserialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SonarIntegration {
    pub enable_sonar: bool,
    pub sonar_api_url: String,
    pub sonar_api_key: String,
    pub snmp_community: String,
    // TODO: It isn't clear what types `sonar_api_key,sonar_airmax_ap_model_ids,sonar_active_status_ids,sonar_ltu_ap_model_ids`
    // are supposed to be. 
}

impl Default for SonarIntegration {
    fn default() -> Self {
        SonarIntegration {
            enable_sonar: false,
            sonar_api_url: "".to_string(),
            sonar_api_key: "".to_string(),
            snmp_community: "public".to_string(),
        }
    }
}