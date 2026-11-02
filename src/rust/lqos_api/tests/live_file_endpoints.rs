mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use serial_test::serial;
use tower::ServiceExt;

async fn send_json(
    app: &axum::Router,
    method: Method,
    uri: &str,
    body: &serde_json::Value,
) -> (StatusCode, Vec<u8>) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let body_bytes = resp
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, body_bytes)
}

async fn send_empty(app: &axum::Router, method: Method, uri: &str) -> (StatusCode, Vec<u8>) {
    let builder = Request::builder().method(method).uri(uri);
    let resp = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let body_bytes = resp
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, body_bytes)
}

fn circuit_request(circuit_id: &str, device_id: &str, parent_node: &str) -> serde_json::Value {
    serde_json::json!({
        "circuit_id": circuit_id,
        "circuit_name": "Test Circuit",
        "device_id": device_id,
        "device_name": "Device 1",
        "parent_node": parent_node,
        "mac": "",
        "ipv4_list": ["100.64.0.1"],
        "ipv6_list": [],
        "download_min_mbps": 1.0,
        "upload_min_mbps": 1.0,
        "download_max_mbps": 10.0,
        "upload_max_mbps": 5.0,
        "comment": "test",
        "sqm_override": null
    })
}

fn write_operator_view() -> std::path::PathBuf {
    let config = lqos_config::load_config().expect("load config");
    let path = lqos_config::topology_operator_view_path(config.as_ref());
    std::fs::create_dir_all(path.parent().expect("operator view parent"))
        .expect("create operator view parent");
    let view = serde_json::json!({
        "schema_version": 1,
        "source_generation": "test-generation",
        "nodes": [
            {
                "id": "site:ap1",
                "name": "AP1",
                "parent_id": null,
                "type": "site"
            },
            {
                "id": "site:ambiguous-a",
                "name": "Ambiguous",
                "parent_id": null,
                "type": "site"
            },
            {
                "id": "site:ambiguous-b",
                "name": "Ambiguous",
                "parent_id": null,
                "type": "site"
            }
        ],
        "ambiguous_base_names": ["Ambiguous"]
    });
    let bytes = serde_json::to_vec_pretty(&view).expect("serialize operator view");
    std::fs::write(&path, bytes).expect("write operator view");
    path
}

#[tokio::test]
#[serial]
async fn live_upsert_circuit_resolves_flat_operator_view_parent() {
    let _env = common::env();
    common::reset_shaped_devices();
    let app = common::router();
    let view_path = write_operator_view();
    let view_before = std::fs::read(&view_path).expect("read operator view");

    let request = circuit_request("test_circuit_1", "dev1", "AP1");
    let (status, body) = send_json(
        &app,
        Method::POST,
        "/shaped_devices/circuit?reload=false",
        &request,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "response body: {}",
        String::from_utf8_lossy(&body)
    );
    let resp: lqos_api::LiveUpdateResponse =
        serde_json::from_slice(&body).expect("parse LiveUpdateResponse");
    assert!(resp.changed);
    assert!(!resp.reload_requested);

    let view_after = std::fs::read(&view_path).expect("read operator view");
    assert_eq!(
        view_before, view_after,
        "circuit upsert must not mutate the operator view"
    );

    let missing = circuit_request("test_circuit_missing", "dev-missing", "NoSuchParent");
    let (status, body) = send_json(
        &app,
        Method::POST,
        "/shaped_devices/circuit?reload=false",
        &missing,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "response body: {}",
        String::from_utf8_lossy(&body)
    );
    assert!(
        String::from_utf8_lossy(&body).contains("Parent node not found in network.json"),
        "unexpected body: {}",
        String::from_utf8_lossy(&body)
    );

    let ambiguous = circuit_request("test_circuit_ambiguous", "dev-ambiguous", "Ambiguous");
    let (status, body) = send_json(
        &app,
        Method::POST,
        "/shaped_devices/circuit?reload=false",
        &ambiguous,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "response body: {}",
        String::from_utf8_lossy(&body)
    );
    assert!(
        String::from_utf8_lossy(&body).contains("ambiguous"),
        "unexpected body: {}",
        String::from_utf8_lossy(&body)
    );
}

fn write_canonical_state(path: &std::path::Path, ingress_kind: &str, source: &str) {
    let state = serde_json::json!({
        "schema_version": 1,
        "source": source,
        "ingress_kind": ingress_kind,
        "nodes": []
    });
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&state).expect("serialize canonical state"),
    )
    .expect("write canonical state");
}

fn write_topology_import(path: &std::path::Path, source: &str) {
    let imported = serde_json::json!({
        "source": source,
        "imported": { "source": source }
    });
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&imported).expect("serialize topology import"),
    )
    .expect("write topology import");
}

fn clear_topology_fixtures(canonical: &std::path::Path, import: &std::path::Path) {
    let _ = std::fs::remove_file(canonical);
    let _ = std::fs::remove_file(import);
}

async fn assert_stub_routes(
    app: &axum::Router,
    routes: &[(Method, &str)],
    expected: &str,
    source: Option<&str>,
    view_path: &std::path::Path,
    view_before: &[u8],
) {
    for (method, uri) in routes {
        let (status, body) = send_json(app, method.clone(), uri, &serde_json::json!({})).await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "{uri}: {}",
            String::from_utf8_lossy(&body)
        );
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains(expected), "{uri}: {text}");
        if let Some(source) = source {
            assert!(text.contains(source), "{uri}: {text}");
        }
        assert_eq!(
            view_before,
            std::fs::read(view_path).expect("read operator view"),
            "operator view changed by {uri}"
        );
    }
}

#[tokio::test]
#[serial]
async fn legacy_topology_writes_are_409_stubs() {
    let _env = common::env();
    let app = common::router();
    let view_path = write_operator_view();
    let view_before = std::fs::read(&view_path).expect("read operator view");

    let config = lqos_config::load_config().expect("load config");
    let canonical_path =
        config.topology_state_file_path(lqos_config::TOPOLOGY_CANONICAL_STATE_FILENAME);
    let import_path = config.topology_state_file_path(lqos_config::TOPOLOGY_IMPORT_FILENAME);
    let routes = [
        (Method::POST, "/network_json/update"),
        (Method::POST, "/network_json/node"),
        (Method::DELETE, "/network_json/node/AP1"),
        (Method::POST, "/network/reparent_site"),
    ];

    // Legacy canonical state: the editor owns it.
    clear_topology_fixtures(&canonical_path, &import_path);
    write_canonical_state(
        &canonical_path,
        "legacy_network_json",
        "legacy/network.json",
    );
    assert_stub_routes(
        &app,
        &routes,
        "topology_edit_requires_editor",
        None,
        &view_path,
        &view_before,
    )
    .await;

    // No body extractor: an absent body is still the ownership 409, not 415.
    let (status, body) = send_empty(&app, Method::POST, "/network_json/update").await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert!(
        String::from_utf8_lossy(&body).contains("topology_edit_requires_editor"),
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert_eq!(
        view_before,
        std::fs::read(&view_path).expect("read operator view"),
        "operator view changed by the no-body stub call"
    );

    // Native integration canonical state: the integration owns it.
    clear_topology_fixtures(&canonical_path, &import_path);
    write_canonical_state(&canonical_path, "native_integration", "uisp/full2");
    assert_stub_routes(
        &app,
        &routes,
        "topology_managed_by_integration",
        Some("uisp/full2"),
        &view_path,
        &view_before,
    )
    .await;

    // Legacy import and no canonical: the editor owns it.
    clear_topology_fixtures(&canonical_path, &import_path);
    write_topology_import(&import_path, "legacy/network.json");
    assert_stub_routes(
        &app,
        &routes,
        "topology_edit_requires_editor",
        None,
        &view_path,
        &view_before,
    )
    .await;

    // Neither artifact: the editor owns it.
    clear_topology_fixtures(&canonical_path, &import_path);
    assert_stub_routes(
        &app,
        &routes,
        "topology_edit_requires_editor",
        None,
        &view_path,
        &view_before,
    )
    .await;

    // Unparseable canonical state: refuse with the editor message.
    clear_topology_fixtures(&canonical_path, &import_path);
    std::fs::write(&canonical_path, b"{not valid json").expect("write bad canonical state");
    assert_stub_routes(
        &app,
        &routes,
        "topology_edit_requires_editor",
        None,
        &view_path,
        &view_before,
    )
    .await;

    // Precedence: either integration source blocks. A non-legacy import wins
    // even when a legacy canonical state is also present.
    clear_topology_fixtures(&canonical_path, &import_path);
    write_canonical_state(
        &canonical_path,
        "legacy_network_json",
        "legacy/network.json",
    );
    write_topology_import(&import_path, "uisp/full2");
    assert_stub_routes(
        &app,
        &routes,
        "topology_managed_by_integration",
        Some("uisp/full2"),
        &view_path,
        &view_before,
    )
    .await;
}
