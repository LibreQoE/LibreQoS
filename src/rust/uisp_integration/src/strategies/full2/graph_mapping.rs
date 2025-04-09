use serde::Serialize;

#[derive(Debug, Ord, PartialOrd, Eq, PartialEq, Clone, Serialize)]
pub enum GraphMapping {
    Root {
        name: String,
        id: String,
    },
    Site {
        name: String,
        id: String,
    },
    GeneratedSite {
        name: String,
    },
    AccessPoint {
        name: String,
        id: String,
        site_name: String,
        download_mbps: u64,
        upload_mbps: u64,
    },
}

impl GraphMapping {
    pub fn name(&self) -> String {
        match self {
            GraphMapping::Root { name, .. } => name.clone(),
            GraphMapping::Site { name, .. } => name.clone(),
            GraphMapping::GeneratedSite { name } => name.clone(),
            GraphMapping::AccessPoint { name, .. } => name.clone(),
        }
    }
}
