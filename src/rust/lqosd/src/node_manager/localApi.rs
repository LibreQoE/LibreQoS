mod dashboard_themes;

use axum::Router;
use axum::routing::{get, post};

pub fn local_api() -> Router {
    Router::new()
        .route("/dashletThemes", get(dashboard_themes::list_themes))
        .route("/dashletSave", post(dashboard_themes::save_theme))
        .route("/dashletDelete", post(dashboard_themes::delete_theme))
        .route("/dashletGet", post(dashboard_themes::get_theme))
}