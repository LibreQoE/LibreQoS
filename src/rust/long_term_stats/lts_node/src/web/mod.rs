//! The webserver listens on port 9127, but it is intended that this only
//! listen on localhost and have a reverse proxy in front of it. The proxy
//! should provide HTTPS.

use axum::{
    response::Html,
    routing::get,
    Router,
};
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
