use std::{collections::HashMap, sync::RwLock};
use once_cell::sync::Lazy;
use sqlx::{Pool, Postgres};
use crate::{OrganizationDetails, get_organization};

static ORG_CACHE: Lazy<RwLock<HashMap<String, OrganizationDetails>>> = Lazy::new(|| {
    RwLock::new(HashMap::new())
});

pub async fn get_org_details(cnn: &Pool<Postgres>, key: &str) -> Option<OrganizationDetails> {
    { // Safety scope - lock is dropped on exit
        let cache = ORG_CACHE.read().unwrap();
        if let Some(org) = cache.get(key) {
            return Some(org.clone());
        }
    }
    // We can be certain that we don't have a dangling lock now.
    // Upgrade to a write lock and try to fetch the org details.
    if let Ok(org) = get_organization(cnn, key).await {
        let mut cache = ORG_CACHE.write().unwrap();
        cache.insert(key.to_string(), org.clone());
        return Some(org);
    }
    None
}