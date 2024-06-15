use crate::errors::UispIntegrationError;
use std::fmt::{Display, Formatter};
use tracing::error;
use uisp::Site;

/// Defines the types of sites found in the UISP Tree
#[derive(Debug, PartialEq)]
pub enum UispSiteType {
    Site,
    Client,
    ClientWithChildren,
    AccessPoint,
    Root,
    SquashDeleted,
}

impl Display for UispSiteType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match &self {
            Self::Site => write!(f, "Site"),
            Self::Client => write!(f, "Client"),
            Self::ClientWithChildren => write!(f, "GeneratedNode"),
            Self::AccessPoint => write!(f, "AP"),
            Self::Root => write!(f, "Root"),
            Self::SquashDeleted => write!(f, "SquashDeleted"),
        }
    }
}

impl UispSiteType {
    /// Converts a UISP site record into a UispSiteType
    pub fn from_uisp_record(site: &Site) -> Result<Self, UispIntegrationError> {
        if let Some(id) = &site.identification {
            if let Some(t) = &id.site_type {
                return match t.as_str() {
                    "site" => Ok(Self::Site),
                    "endpoint" => Ok(Self::Client),
                    _ => {
                        error!("Unknown site type: {t}");
                        Err(UispIntegrationError::UnknownSiteType)
                    }
                };
            }
        }
        Err(UispIntegrationError::UnknownSiteType)
    }
}
