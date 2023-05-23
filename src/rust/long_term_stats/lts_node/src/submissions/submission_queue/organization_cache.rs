use std::collections::HashMap;
use once_cell::sync::Lazy;
use pgdb::{OrganizationDetails, sqlx::{Pool, Postgres}};
use tokio::sync::RwLock;

static ORG_CACHE: Lazy<RwLock<HashMap<String, OrganizationDetails>>> = Lazy::new(|| {
    RwLock::new(HashMap::new())
});

pub async fn get_org_details(cnn: &Pool<Postgres>, key: &str) -> Option<OrganizationDetails> {
    { // Safety scope - lock is dropped on exit
        let cache = ORG_CACHE.read().await;
        if let Some(org) = cache.get(key) {
            return Some(org.clone());
        }
    }
    // We can be certain that we don't have a dangling lock now.
    // Upgrade to a write lock and try to fetch the org details.
    let mut cache = ORG_CACHE.write().await;
    if let Ok(org) = pgdb::get_organization(cnn, key).await {
        cache.insert(key.to_string(), org.clone());
        return Some(org);
    }
    None
}