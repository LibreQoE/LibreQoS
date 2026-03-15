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

    pub fn network_json_id(&self) -> String {
        match self {
            GraphMapping::Root { id, .. } | GraphMapping::Site { id, .. } => {
                format!("uisp:site:{id}")
            }
            GraphMapping::GeneratedSite { name } => {
                format!("libreqos:generated:uisp:site:{}", slugify_generated_name(name))
            }
            GraphMapping::AccessPoint { id, .. } => format!("uisp:device:{id}"),
        }
    }
}

fn slugify_generated_name(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    let mut last_was_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }
    slug.trim_matches('-').to_string()
}

#[cfg(test)]
mod test {
    use super::GraphMapping;

    #[test]
    fn emits_generic_ids_for_real_and_generated_nodes() {
        let root = GraphMapping::Root {
            name: "Main".to_string(),
            id: "site-1".to_string(),
        };
        let ap = GraphMapping::AccessPoint {
            name: "AP".to_string(),
            id: "device-1".to_string(),
            site_name: "Main".to_string(),
            download_mbps: 100,
            upload_mbps: 100,
        };
        let generated = GraphMapping::GeneratedSite {
            name: "Orphans".to_string(),
        };

        assert_eq!(root.network_json_id(), "uisp:site:site-1");
        assert_eq!(ap.network_json_id(), "uisp:device:device-1");
        assert_eq!(
            generated.network_json_id(),
            "libreqos:generated:uisp:site:orphans"
        );
    }
}
