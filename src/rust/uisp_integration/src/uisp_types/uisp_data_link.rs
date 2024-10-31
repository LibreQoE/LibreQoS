use uisp::DataLink;

/// Shortened/Flattened version of the UISP DataLink type.
pub struct UispDataLink {
    pub id: String,
    pub from_site_id: String,
    pub to_site_id: String,
    pub from_site_name: String,
    pub to_site_name: String,
    pub can_delete: bool,
}

impl UispDataLink {
    /// Converts a UISP DataLink into a UispDataLink.
    /// 
    /// # Arguments
    /// * `value` - The UISP DataLink to convert
    pub fn from_uisp(value: &DataLink) -> Option<Self> {
        let mut from_site_id = String::new();
        let mut to_site_id = String::new();
        let mut to_site_name = String::new();
        let from_site_name = String::new();

        // Obvious Site Links
        if let Some(from_site) = &value.from.site {
            from_site_id = from_site.identification.id.clone();
            to_site_id = from_site.identification.name.clone();
        }
        if let Some(to_site) = &value.to.site {
            to_site_id = to_site.identification.id.clone();
            to_site_name = to_site.identification.name.clone();
        }

        // Remove any links with no site targets
        if from_site_id.is_empty() || to_site_id.is_empty() {
            return None;
        }

        // Remove any links that go to themselves
        if from_site_id == to_site_id {
            return None;
        }

        Some(Self {
            id: value.id.clone(),
            from_site_id,
            to_site_id,
            from_site_name,
            to_site_name,
            can_delete: value.can_delete,
        })
    }
}
