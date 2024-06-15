use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use rocket::fs::NamedFile;
use rocket::serde::Deserialize;
use rocket::serde::json::Json;
use lqos_config::load_config;
use lqos_support_tool::{run_sanity_checks, SanityChecks};
use crate::auth_guard::AuthGuard;

#[get("/api/sanity")]
pub async fn run_sanity_check(
    _auth: AuthGuard,
) -> Json<SanityChecks> {
    let mut status = run_sanity_checks().unwrap();
    status.results.sort_by(|a,b| a.success.cmp(&b.success));
    Json(status)
}

#[derive(Deserialize, Clone)]
#[serde(crate = "rocket::serde")]
pub struct SupportMetadata {
    name: String,
    comment: String,
}

#[post("/api/gatherSupport", data="<info>")]
pub async fn gather_support_data(
    info: Json<SupportMetadata>
) -> NamedFile {
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let filename = format!("/tmp/libreqos_{}.support", timestamp);
    let path = Path::new(&filename);

    let lts_key = if let Ok(cfg) = load_config() {
        cfg.long_term_stats.license_key.unwrap_or("None".to_string())
    } else {
        "None".to_string()
    };
    if let Ok(dump) = lqos_support_tool::gather_all_support_info(&info.name, &info.comment, &lts_key) {
        std::fs::write(&path, dump.serialize_and_compress().unwrap()).unwrap();
    }

    NamedFile::open(path).await.unwrap()
}

#[post("/api/submitSupport", data="<info>")]
pub async fn submit_support_data(
    info: Json<SupportMetadata>
) -> String {
    let lts_key = if let Ok(cfg) = load_config() {
        cfg.long_term_stats.license_key.unwrap_or("None".to_string())
    } else {
        "None".to_string()
    };
    if let Ok(dump) = lqos_support_tool::gather_all_support_info(&info.name, &info.comment, &lts_key) {
        lqos_support_tool::submit_to_network(dump);
        "Your support submission has been sent".to_string()
    } else {
        "Something went wrong".to_string()
    }

}