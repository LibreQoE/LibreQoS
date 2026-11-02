mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use serial_test::serial;
use tower::ServiceExt;

#[tokio::test]
#[serial]
async fn health_returns_500_when_bus_missing() {
    let _env = common::env();

    // If a developer runs tests on a real node with lqosd running, the bus will exist.
    if std::path::Path::new(lqos_bus::BUS_SOCKET_PATH).exists() {
        return;
    }

    let app = common::router();

    let req = Request::builder()
        .method(Method::GET)
        .uri("/health")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&body);
    assert!(
        text.contains("Failed to connect to bus"),
        "unexpected /health error message: {text}"
    );
}
