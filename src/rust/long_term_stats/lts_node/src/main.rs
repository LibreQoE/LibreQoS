use axum::{
    response::Html,
    routing::{get, post},
    Router, Json,
};
use lqos_bus::long_term_stats::StatsSubmission;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Start the logger
    env_logger::init_from_env(
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "warn"),
    );

    if let Err(e) = tokio::spawn(server()).await {
        log::error!("Server exited with error: {}", e);
    }

    Ok(())
}

async fn server() {
    let app = Router::new()
        .route("/", get(index_page))
        .route("/submit", post(on_submission));

    log::info!("Listening for web traffic on 0.0.0.0:9127");
    axum::Server::bind(&"0.0.0.0:9127".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn index_page() -> Html<String> {
    Html("Hello, World!".to_string())
}

async fn on_submission(Json(payload): Json<StatsSubmission>) -> Html<String> {
    log::info!("Submission arrived");
    println!("{payload:#?}");
    Html("Hello, World!".to_string())
}
