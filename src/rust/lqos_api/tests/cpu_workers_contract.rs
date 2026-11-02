mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use lqos_api::api::{
    AutobalanceSection, BridgeSection, CircuitOwner, CpuTopologySection, CpuUsageRow,
    CpuUsageSection, CpuWorkersBus, CpuWorkersResponse, DeferredMaintenanceSection, MovesSection,
    NodeOwners, OwnershipSection, WorkerRow, WorkersSection, assemble_response,
};
use serial_test::serial;
use tower::ServiceExt;

fn cpu_section() -> CpuUsageSection {
    CpuUsageSection {
        available: true,
        reason: None,
        num_cpus: Some(2),
        per_cpu: Some(vec![
            CpuUsageRow {
                cpu_id: Some(0),
                percent: 10,
            },
            CpuUsageRow {
                cpu_id: None,
                percent: 20,
            },
        ]),
    }
}

fn topology_section() -> CpuTopologySection {
    CpuTopologySection {
        available: true,
        reason: None,
        exclude_efficiency_cores: Some(false),
        source: Some("exclusion_disabled".to_string()),
        from_cache: Some(false),
        has_hybrid_split: Some(false),
        possible: Some(vec![0, 1]),
        performance: Some(vec![0, 1]),
        efficiency: Some(vec![]),
        shaping: Some(vec![0, 1]),
        detail: Some("Efficiency-core exclusion disabled".to_string()),
    }
}

fn bridge() -> BridgeSection {
    BridgeSection {
        available: true,
        reason: None,
        state: Some("healthy".to_string()),
        applied_generation: Some("gen".to_string()),
        desired_generation: Some("gen".to_string()),
        applied_provenance: Some("BASE_ADOPT".to_string()),
        applied_diverged: Some(false),
        pending: None,
        last_reject: None,
        deferred_maintenance: None,
        autobalance: AutobalanceSection {
            configured: Some(true),
            runtime: None,
            reason: Some("log_only".to_string()),
        },
    }
}

fn empty_ownership() -> OwnershipSection {
    OwnershipSection {
        available: true,
        reason: None,
        generation: Some("gen".to_string()),
        circuits: Some(vec![]),
        nodes: Some(vec![]),
        crossings: Some(vec![]),
        unowned_circuits: Some(0),
    }
}

fn empty_workers() -> WorkersSection {
    WorkersSection {
        available: true,
        reason: None,
        schema: Some(1),
        rx_queue_count: Some(0),
        tx_queue_count: Some(0),
        counter_support: Some("unsupported".to_string()),
        rows: Some(vec![]),
    }
}

#[test]
fn absent_bus_is_partial_and_uses_null_payloads() {
    let response = assemble_response(1, cpu_section(), topology_section(), None);
    assert_eq!(response.state, "partial");
    assert!(!response.workers.available);
    assert!(response.workers.rows.is_none());
    assert!(!response.ownership.available);
    assert!(response.ownership.circuits.is_none());
    assert!(!response.moves.available);
    assert!(response.moves.items.is_none());
}

#[test]
fn present_empty_is_supported_not_unavailable() {
    let bus = CpuWorkersBus {
        schema: 1,
        generated_at_unix: 1,
        bridge: bridge(),
        workers: empty_workers(),
        ownership: empty_ownership(),
        moves: MovesSection {
            available: false,
            reason: Some("no_history_source_v1".to_string()),
            source: None,
            since_unix: None,
            items: None,
        },
    };
    let response = assemble_response(1, cpu_section(), topology_section(), Some(bus));
    assert_eq!(response.state, "partial", "moves are unavailable in v1");
    assert!(
        response
            .workers
            .rows
            .as_deref()
            .is_some_and(|rows| rows.is_empty())
    );
    assert!(
        response
            .ownership
            .circuits
            .as_deref()
            .is_some_and(|circuits| circuits.is_empty())
    );
}

#[test]
fn worker_nulls_and_unsupported_counters_stay_null() {
    let workers = WorkersSection {
        available: true,
        reason: None,
        schema: Some(1),
        rx_queue_count: Some(1),
        tx_queue_count: Some(1),
        counter_support: Some("unsupported".to_string()),
        rows: Some(vec![WorkerRow {
            worker_id: 0,
            lcore_id: None,
            numa_node: None,
            rx_queue_ids: None,
            tx_queue_id: None,
            shared_tx_queue: None,
            circuit_count: Some(1),
            solely_owned_node_count: Some(0),
            counters: None,
            throughput: None,
            throughput_support: Some("unsupported".to_string()),
            drops: None,
            drops_support: Some("unsupported".to_string()),
        }]),
    };
    let ownership = OwnershipSection {
        available: true,
        reason: None,
        generation: Some("gen".to_string()),
        circuits: Some(vec![
            CircuitOwner {
                circuit_id: "c1".to_string(),
                parent_node_id: "site:1".to_string(),
                name: Some("Circuit One".to_string()),
                worker_id: Some(0),
            },
            CircuitOwner {
                circuit_id: "c2".to_string(),
                parent_node_id: "site:1".to_string(),
                name: None,
                worker_id: None,
            },
        ]),
        nodes: Some(vec![NodeOwners {
            node_id: "site:1".to_string(),
            parent_node_id: Some("root".to_string()),
            name: Some("Site One".to_string()),
            worker_ids: vec![0, 1],
            crossing: true,
        }]),
        crossings: Some(vec!["site:1".to_string()]),
        unowned_circuits: Some(1),
    };
    let bus = CpuWorkersBus {
        schema: 1,
        generated_at_unix: 1,
        bridge: bridge(),
        workers,
        ownership,
        moves: MovesSection {
            available: false,
            reason: Some("no_history_source_v1".to_string()),
            source: None,
            since_unix: None,
            items: None,
        },
    };
    let response = assemble_response(1, cpu_section(), topology_section(), Some(bus));
    let value = serde_json::to_value(&response).expect("serialize response");
    let row = &value["workers"]["rows"][0];
    assert!(row["lcore_id"].is_null());
    assert!(row["numa_node"].is_null());
    assert!(row["rx_queue_ids"].is_null());
    assert!(row["tx_queue_id"].is_null());
    assert!(row["counters"].is_null());
    assert!(row["throughput"].is_null());
    assert_eq!(row["throughput_support"].as_str(), Some("unsupported"));
    assert!(row["drops"].is_null());
    assert_eq!(row["drops_support"].as_str(), Some("unsupported"));
    assert_eq!(
        row["solely_owned_node_count"].as_u64(),
        Some(0),
        "a crossing node is not solely owned"
    );
    assert_eq!(
        value["workers"]["counter_support"].as_str(),
        Some("unsupported")
    );
    assert_eq!(value["ownership"]["unowned_circuits"].as_u64(), Some(1));
    assert_eq!(
        value["ownership"]["circuits"][0]["name"].as_str(),
        Some("Circuit One")
    );
    assert!(
        value["ownership"]["circuits"][1]["name"].is_null(),
        "an absent name is null, never an empty string"
    );
    assert_eq!(
        value["ownership"]["nodes"][0]["name"].as_str(),
        Some("Site One")
    );
    assert!(value["ownership"]["circuits"][1]["worker_id"].is_null());
    assert_eq!(
        value["ownership"]["nodes"][0]["crossing"].as_bool(),
        Some(true)
    );
    assert_eq!(value["ownership"]["crossings"][0].as_str(), Some("site:1"));
    assert!(!value.to_string().contains("\"throughput\":0"));
}

#[test]
fn cpu_id_join_never_assumes_vector_index() {
    let response = assemble_response(1, cpu_section(), topology_section(), None);
    let value = serde_json::to_value(&response).unwrap();
    assert_eq!(value["cpu_usage"]["per_cpu"][0]["cpu_id"].as_u64(), Some(0));
    assert!(value["cpu_usage"]["per_cpu"][1]["cpu_id"].is_null());
}

#[tokio::test]
#[serial]
async fn route_is_200_partial_without_a_live_bus() {
    let _env = common::env();
    let app = common::router();

    let request = Request::builder()
        .method(Method::GET)
        .uri("/cpu_workers")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let parsed: CpuWorkersResponse =
        serde_json::from_slice(&body).expect("parse CpuWorkersResponse");
    // The no-bus fixture has ONE source (the local system monitor) and five
    // unavailable ones, so the assembled state is exactly "partial" — the same
    // shape `absent_bus_is_partial_and_never_zeros` pins at the unit level.
    assert_eq!(
        parsed.state, "partial",
        "no live bus: cpu/topology only, so the response is partial"
    );
    assert!(
        parsed.cpu_usage.available,
        "the local system monitor must answer"
    );
    assert!(!parsed.workers.available);
    assert!(parsed.workers.rows.is_none());
    assert!(!parsed.ownership.available);
    assert!(parsed.ownership.circuits.is_none());
    assert!(!parsed.moves.available);
    assert!(parsed.moves.items.is_none());
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(value["workers"]["rows"].is_null());
    assert!(value["ownership"]["circuits"].is_null());
    assert!(value["moves"]["items"].is_null());
    assert!(!value.to_string().contains("\"throughput\":0"));
}

#[test]
fn unavailable_bridge_keeps_optional_fields_null() {
    let bus = CpuWorkersBus {
        schema: 1,
        generated_at_unix: 1,
        bridge: BridgeSection::unavailable("no_bridge_report"),
        workers: empty_workers(),
        ownership: empty_ownership(),
        moves: MovesSection {
            available: false,
            reason: Some("no_history_source_v1".to_string()),
            source: None,
            since_unix: None,
            items: None,
        },
    };
    let response = assemble_response(1, cpu_section(), topology_section(), Some(bus));
    let value = serde_json::to_value(&response).unwrap();
    assert!(value["bridge"]["applied_diverged"].is_null());
    assert!(value["bridge"]["deferred_maintenance"].is_null());
    assert!(value["bridge"]["state"].is_null());
}

#[test]
fn deferred_maintenance_is_a_typed_object_not_json_text() {
    let mut bridge = bridge();
    bridge.deferred_maintenance = Some(DeferredMaintenanceSection {
        class: "Baseline".to_string(),
        first_deferred_unix: 1,
        held_ops: 2,
        forced: false,
    });
    let bus = CpuWorkersBus {
        schema: 1,
        generated_at_unix: 1,
        bridge,
        workers: empty_workers(),
        ownership: empty_ownership(),
        moves: MovesSection {
            available: false,
            reason: Some("no_history_source_v1".to_string()),
            source: None,
            since_unix: None,
            items: None,
        },
    };
    let response = assemble_response(1, cpu_section(), topology_section(), Some(bus));
    let value = serde_json::to_value(&response).unwrap();
    assert_eq!(
        value["bridge"]["deferred_maintenance"]["class"].as_str(),
        Some("Baseline")
    );
    assert!(value["bridge"]["deferred_maintenance"].is_object());
}

/// Test-only OpenAPI dump for the commit step (the drift test compares the
/// committed JSON; this writes the generated document when explicitly asked).
#[test]
#[ignore]
fn dump_openapi_for_commit() {
    if let Ok(path) = std::env::var("LQOS_OPENAPI_OUT") {
        use utoipa::OpenApi;
        let generated = lqos_api::ApiDoc::openapi();
        std::fs::write(
            path,
            serde_json::to_string_pretty(&generated).expect("serialize openapi"),
        )
        .expect("write openapi");
    }
}
