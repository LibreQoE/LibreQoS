mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use serial_test::serial;
use tower::ServiceExt;

async fn get(app: &axum::Router, uri: &str) -> (StatusCode, Vec<u8>) {
    let builder = Request::builder().method(Method::GET).uri(uri);
    let resp = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let body = resp
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, body)
}

#[tokio::test]
#[serial]
async fn version_is_inventory_readable() {
    let _env = common::env();
    let app = common::router();

    let (status, body) = get(&app, "/version").await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let parsed: serde_json::Value = serde_json::from_slice(&body).expect("version is json");
    assert_eq!(parsed["schema_version"], 1);
    assert_eq!(parsed["lqos_api"]["version"], "0.2.0");
    let build_rev = parsed["lqos_api"]["build_rev"]
        .as_str()
        .expect("lqos_api.build_rev");
    assert!(
        build_rev == "unknown"
            || (build_rev.len() == 12
                && build_rev
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))),
        "unexpected build rev {build_rev:?}"
    );
    let generated_at = parsed["generated_at"].as_str().expect("generated_at");
    assert!(generated_at.ends_with('Z'), "{generated_at:?}");
    let unavailable = parsed["unavailable"].as_array().expect("unavailable");
    let keys: Vec<&str> = unavailable
        .iter()
        .filter_map(|value| value.as_str())
        .collect();
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    assert_eq!(keys, sorted, "unavailable must be sorted");
}
