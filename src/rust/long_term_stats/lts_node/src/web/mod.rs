//! The webserver listens on port 9127, but it is intended that this only
//! listen on localhost and have a reverse proxy in front of it. The proxy
//! should provide HTTPS.
mod wss;
use crate::web::wss::ws_handler;
use axum::body::StreamBody;
use axum::http::{header, HeaderMap};
use axum::response::IntoResponse;
use axum::{response::Html, routing::get, Router};
use pgdb::sqlx::Pool;
use pgdb::sqlx::Postgres;
use tokio_util::io::ReaderStream;
use tower_http::trace::TraceLayer;
use tower_http::trace::DefaultMakeSpan;

const JS_BUNDLE: &str = include_str!("../../web/app.js");
const JS_MAP: &str = include_str!("../../web/app.js.map");
const CSS: &str = include_str!("../../web/style.css");
const CSS_MAP: &str = include_str!("../../web/style.css.map");
const HTML_MAIN: &str = include_str!("../../web/main.html");
const WASM_BODY: &[u8] = include_bytes!("../../web/wasm_pipe_bg.wasm");

pub async fn webserver(cnn: Pool<Postgres>) {
    let app = Router::new()
        .route("/", get(index_page))
        .route("/app.js", get(js_bundle))
        .route("/app.js.map", get(js_map))
        .route("/style.css", get(css))
        .route("/style.css.map", get(css_map))
        .route("/ws", get(ws_handler))
        .route("/wasm_pipe_bg.wasm", get(wasm_file))
        .with_state(cnn)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(true)),
        );

    tracing::info!("Listening for web traffic on 0.0.0.0:9127");
    axum::Server::bind(&"0.0.0.0:9127".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn index_page() -> Html<String> {
    Html(HTML_MAIN.to_string())
}

async fn js_bundle() -> axum::response::Response<String> {
    axum::response::Response::builder()
        .header("Content-Type", "text/javascript")
        .body(JS_BUNDLE.to_string())
        .unwrap()
}

async fn js_map() -> axum::response::Response<String> {
    axum::response::Response::builder()
        .header("Content-Type", "text/json")
        .body(JS_MAP.to_string())
        .unwrap()
}

async fn css() -> axum::response::Response<String> {
    axum::response::Response::builder()
        .header("Content-Type", "text/css")
        .body(CSS.to_string())
        .unwrap()
}

async fn css_map() -> axum::response::Response<String> {
    axum::response::Response::builder()
        .header("Content-Type", "text/json")
        .body(CSS_MAP.to_string())
        .unwrap()
}

async fn wasm_file() -> impl IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/wasm"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        header::HeaderValue::from_static("attachment; filename=wasm_pipe_bg.wasm"),
    );
    axum::response::Response::builder()
        .header(header::CONTENT_TYPE, header::HeaderValue::from_static("application/wasm"))
        .header(header::CONTENT_DISPOSITION, header::HeaderValue::from_static("attachment; filename=wasm_pipe_bg.wasm"))
        .body(StreamBody::new(ReaderStream::new(WASM_BODY)))
        .unwrap()
}
