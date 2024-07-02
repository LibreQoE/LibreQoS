mod dashboard_themes;
mod version_check;
mod device_counts;

use axum::Router;
use axum::routing::{get, post};

pub fn local_api() -> Router {
    Router::new()
        .route("/dashletThemes", get(dashboard_themes::list_themes))
        .route("/dashletSave", post(dashboard_themes::save_theme))
        .route("/dashletDelete", post(dashboard_themes::delete_theme))
        .route("/dashletGet", post(dashboard_themes::get_theme))
        .route("/versionCheck", get(version_check::version_check))
        .route("/deviceCount", get(device_counts::count_users))
}