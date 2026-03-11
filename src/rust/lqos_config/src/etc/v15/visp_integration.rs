use allocative::Allocative;
use serde::{Deserialize, Serialize};

fn default_token_url() -> String {
    "https://data.visp.net/token".to_string()
}

fn default_graphql_url() -> String {
    "https://integrations.visp.net/graphql".to_string()
}

fn default_timeout_secs() -> u64 {
    20
}

fn default_true() -> bool {
    true
}

fn default_strategy() -> String {
    "flat".to_string()
}

fn default_service_type_allowlist() -> Vec<String> {
    vec![
        // Internet access service types we can resolve to IP + speed.
        "ServiceTypeWifi".to_string(),
        "ServiceTypeOtherConnection".to_string(),
        "ServiceTypeFiberCircuit".to_string(),
        "ServiceTypeCable".to_string(),
        "ServiceTypeDsl".to_string(),
        "ServiceTypeAttDsl".to_string(),
        "ServiceTypeBpl".to_string(),
        // LTE is supported only when an IP can be resolved. Keep in allowlist
        // so operators can use it, but the integration will skip LTE services
        // without an IP.
        "ServiceTypeLte".to_string(),
    ]
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Allocative)]
pub struct VispIntegration {
    pub enable_visp: bool,

    pub client_id: String,
    pub client_secret: String,

    pub username: String,
    pub password: String,

    #[serde(default = "default_token_url")]
    pub token_url: String,
    #[serde(default = "default_graphql_url")]
    pub graphql_url: String,

    /// VISP tenant/ISP identifier. If not set, the integration will use the
    /// first `ispId` returned in the token payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isp_id: Option<i64>,

    /// Optional header override for multi-tenant installs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<i64>,

    /// Optional RADIUS domain for `onlineUsers(domain: ...)` enrichment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub online_users_domain: Option<String>,

    #[serde(default = "default_strategy")]
    pub strategy: String,

    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,

    #[serde(default = "default_true")]
    pub verify_tls: bool,

    #[serde(default = "default_service_type_allowlist")]
    pub service_type_allowlist: Vec<String>,
}

impl Default for VispIntegration {
    fn default() -> Self {
        Self {
            enable_visp: false,
            client_id: "".to_string(),
            client_secret: "".to_string(),
            username: "".to_string(),
            password: "".to_string(),
            token_url: default_token_url(),
            graphql_url: default_graphql_url(),
            isp_id: None,
            tenant_id: None,
            online_users_domain: None,
            strategy: default_strategy(),
            timeout_secs: default_timeout_secs(),
            verify_tls: true,
            service_type_allowlist: default_service_type_allowlist(),
        }
    }
}
