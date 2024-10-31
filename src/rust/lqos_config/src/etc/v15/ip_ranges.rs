use serde::{Serialize, Deserialize};

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct IpRanges {
    pub ignore_subnets: Vec<String>,
    pub allow_subnets: Vec<String>,
}

impl Default for IpRanges {
    fn default() -> Self {
        Self {
            ignore_subnets: vec![],
            allow_subnets: vec![
                "172.16.0.0/12".to_string(),
                "10.0.0.0/8".to_string(),
                "100.64.0.0/10".to_string(),
                "192.168.0.0/16".to_string(),                
            ],
        }
    }
}