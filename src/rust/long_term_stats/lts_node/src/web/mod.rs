use axum::{
    response::Html,
    routing::{get, post},
    Router, Json,
};
use lqos_bus::long_term_stats::StatsSubmission;

pub async fn webserver() {
    let app = Router::new()
        .route("/", get(index_page));

    log::info!("Listening for web traffic on 0.0.0.0:9127");
    axum::Server::bind(&"0.0.0.0:9127".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn index_page() -> Html<String> {
    Html("Hello, World!".to_string())
}
