mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::json;
use serial_test::serial;
use tower::ServiceExt;

fn restore_env(name: &str, old: Option<String>) {
    unsafe {
        match old {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }
}

#[tokio::test]
#[serial]
async fn preserved_config_without_use_xdp_bridge_loads_and_builds_router() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_path = temp.path().join("lqos.conf");
    let state_dir = temp.path().join("state");
    std::fs::create_dir_all(state_dir.join("topology")).expect("create state/topology");

    let example = common::lqos_example();
    let config = example
        .replace(
            "lqos_directory = \"/opt/libreqos/src\"",
            &format!("lqos_directory = \"{}\"", temp.path().display()),
        )
        .replace(
            "state_directory = \"/opt/libreqos/state\"",
            &format!("state_directory = \"{}\"", state_dir.display()),
        )
        .lines()
        .filter(|line| !line.trim_start().starts_with("use_xdp_bridge"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    assert!(
        !config.contains("use_xdp_bridge"),
        "fixture must not carry the key"
    );
    std::fs::write(&config_path, config).expect("write preserved config");

    let old_config = std::env::var("LQOS_CONFIG").ok();
    let old_directory = std::env::var("LQOS_DIRECTORY").ok();
    unsafe {
        std::env::set_var("LQOS_CONFIG", &config_path);
        std::env::set_var("LQOS_DIRECTORY", temp.path());
    }
    lqos_config::clear_cached_config();
    let loaded = lqos_config::load_config();
    let migrated = std::fs::read_to_string(&config_path);
    let backup_exists = temp
        .path()
        .join("lqos.conf.bridge_use_xdp_bridge_backup")
        .exists();

    // The API start path: mount the data surface under the preserved config and
    // prove a request is served.
    let app = lqos_api::api_routes(common::api_state());
    let request = Request::builder()
        .method(Method::GET)
        .uri("/version")
        .body(Body::empty())
        .expect("request");
    let response = app.oneshot(request).await.expect("router response");
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("response body")
        .to_bytes();
    let parsed: serde_json::Value = serde_json::from_slice(&body).expect("version json");
    assert_eq!(parsed["schema_version"], json!(1));

    restore_env("LQOS_CONFIG", old_config);
    restore_env("LQOS_DIRECTORY", old_directory);
    lqos_config::clear_cached_config();

    assert_eq!(status, StatusCode::OK);
    loaded.expect("preserved config without use_xdp_bridge must load");
    let migrated = migrated.expect("read migrated config");
    assert!(
        migrated.contains("use_xdp_bridge"),
        "v3 config load must add the external-consumer compat key"
    );
    assert!(backup_exists, "the pre-migration config must be backed up");
}
