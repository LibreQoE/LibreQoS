use crate::errors::UispIntegrationError;
use crate::uisp_types::{UispDataLink, UispSite, UispSiteType};
use lqos_config::Config;
use tracing::{error, info, warn};

/// Looks to identify the root site for the site tree.
/// If the "site" is defined in the configuration, it will try to use it.
/// If the site is defined but does not exist, it will search for an Internet-connected site
/// and try to use that.
/// If it still hasn't found one, and there are multiple Internet connected sites - it will insert
/// a fake root and use that instead. I'm not sure that's a great idea.
pub fn find_root_site(
    config: &Config,
    sites: &mut Vec<UispSite>,
    data_links: &[UispDataLink],
) -> Result<String, UispIntegrationError> {
    let mut root_site_name = config.uisp_integration.site.clone();
    if root_site_name.is_empty() {
        warn!("Root site name isn't specified in /etc/lqos.conf - we'll try and figure it out");
        root_site_name = handle_multiple_internet_connected_sites(sites, data_links)?;
    } else {
        info!("Using root UISP site from /etc/lqos.conf: {root_site_name}");

        if !sites.iter().any(|s| s.name == root_site_name) {
            error!("Site {root_site_name} (from /etc/lqos.conf) not found in the UISP sites list");
            return Err(UispIntegrationError::NoRootSite);
        } else {
            info!("{root_site_name} found in the sites list.");
        }
    }

    Ok(root_site_name)
}

fn handle_multiple_internet_connected_sites(
    sites: &mut Vec<UispSite>,
    data_links: &[UispDataLink],
) -> Result<String, UispIntegrationError> {
    let mut root_site_name = String::new();
    let mut candidates = Vec::new();

    data_links.iter().filter(|l| !l.can_delete).for_each(|l| {
        candidates.push(l.from_site_name.clone());
    });

    if candidates.is_empty() {
        error!("Unable to find a root site in the sites/data-links.");
        return Err(UispIntegrationError::NoRootSite);
    } else if candidates.len() == 1 {
        info!(
            "Found only one site with an Internet connection: {root_site_name}, using it as root"
        );
        root_site_name = candidates[0].clone();
    } else {
        warn!("Multiple Internet links detected. Will create an 'Internet' root node");
        root_site_name = "INSERTED_INTERNET".to_string();
        sites.push(UispSite {
            id: "ROOT-001".to_string(),
            name: "INSERTED_INTERNET".to_string(),
            site_type: UispSiteType::Root,
            ..Default::default()
        })
    }

    Ok(root_site_name)
}

/// Sets the root site in the site list.
/// If there are multiple root sites, it will return an error.
/// 
/// # Arguments
/// * `sites` - The list of sites
/// * `root_site` - The name of the root site
pub fn set_root_site(sites: &mut [UispSite], root_site: &str) -> Result<(), UispIntegrationError> {
    if let Some(root) = sites.iter_mut().find(|s| s.name == root_site) {
        root.site_type = UispSiteType::Root;
    }
    let number_of_roots = sites
        .iter()
        .filter(|s| s.site_type == UispSiteType::Root)
        .count();
    if number_of_roots > 1 {
        error!("More than one root present in the tree! That's not going to work. Bailing.");
        return Err(UispIntegrationError::NoRootSite);
    } else {
        info!("Single root tagged in the tree");
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_known_root() {
        let mut cfg = Config::default();
        cfg.uisp_integration.enable_uisp = true;
        cfg.uisp_integration.site = "TEST".to_string();
        let mut sites = vec![UispSite {
            id: "TEST".to_string(),
            name: "TEST".to_string(),
            site_type: UispSiteType::Site,
            ..Default::default()
        }];
        let data_links = vec![];
        let result = find_root_site(&cfg, &mut sites, &data_links);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "TEST");
    }

    #[test]
    fn fail_find_a_known_root() {
        let mut cfg = Config::default();
        cfg.uisp_integration.enable_uisp = true;
        cfg.uisp_integration.site = "DOES NOT EXIST".to_string();
        let mut sites = vec![UispSite {
            id: "TEST".to_string(),
            name: "TEST".to_string(),
            site_type: UispSiteType::Site,
            ..Default::default()
        }];
        let data_links = vec![];
        let result = find_root_site(&cfg, &mut sites, &data_links);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), UispIntegrationError::NoRootSite);
    }

    #[test]
    fn find_single_root_from_data_links() {
        let mut cfg = Config::default();
        cfg.uisp_integration.enable_uisp = true;
        cfg.uisp_integration.site = String::new();

        let mut sites = vec![UispSite {
            id: "TEST".to_string(),
            name: "TEST".to_string(),
            site_type: UispSiteType::Site,
            ..Default::default()
        }];
        let data_links = vec![UispDataLink {
            id: "".to_string(),
            from_site_id: "TEST".to_string(),
            from_site_name: "TEST".to_string(),
            to_site_id: "".to_string(),
            to_site_name: "".to_string(),
            can_delete: false,
        }];
        let result = find_root_site(&cfg, &mut sites, &data_links);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "TEST");
    }

    #[test]
    fn test_inserted_internet() {
        let mut cfg = Config::default();
        cfg.uisp_integration.enable_uisp = true;
        cfg.uisp_integration.site = String::new();

        let mut sites = vec![
            UispSite {
                id: "TEST".to_string(),
                name: "TEST".to_string(),
                site_type: UispSiteType::Site,
                ..Default::default()
            },
            UispSite {
                id: "TEST2".to_string(),
                name: "TEST2".to_string(),
                site_type: UispSiteType::Site,
                ..Default::default()
            },
        ];
        let data_links = vec![
            UispDataLink {
                id: "".to_string(),
                from_site_id: "".to_string(),
                to_site_id: "TEST".to_string(),
                from_site_name: "".to_string(),
                to_site_name: "TEST".to_string(),
                can_delete: false,
            },
            UispDataLink {
                id: "".to_string(),
                from_site_id: "".to_string(),
                to_site_id: "TEST2".to_string(),
                from_site_name: "".to_string(),
                to_site_name: "TEST2".to_string(),
                can_delete: false,
            },
        ];
        let result = find_root_site(&cfg, &mut sites, &data_links);
        assert!(result.is_ok());
        assert!(sites.iter().any(|s| s.name == "INSERTED_INTERNET"));
    }
}
