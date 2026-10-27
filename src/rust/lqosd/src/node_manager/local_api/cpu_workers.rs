use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use lqos_bus::{BusRequest, BusResponse, LibreqosBusClient};
use serde_json::{Value, json};
use std::sync::OnceLock;

/// The shaping-CPU topology, computed once at daemon startup (off the request
/// path). `None` = not initialized yet; the endpoint reports topology
/// unavailable instead of guessing.
static CPU_TOPOLOGY: OnceLock<Option<Value>> = OnceLock::new();

/// `lqos-74t.118.3`: initialize the CPU topology section at startup from the
/// same detector the public API uses.
pub(crate) fn init_topology(config: &lqos_config::Config) {
    let detection = lqos_config::detect_shaping_cpus(config);
    let _ = CPU_TOPOLOGY.set(Some(topology_json(detection)));
}

fn topology_source_str(source: lqos_config::ShapingCpuSource) -> &'static str {
    match source {
        lqos_config::ShapingCpuSource::ExclusionDisabled => "exclusion_disabled",
        lqos_config::ShapingCpuSource::CoreTypeSysfs => "core_type_sysfs",
        lqos_config::ShapingCpuSource::CpuCapacitySysfs => "cpu_capacity_sysfs",
        lqos_config::ShapingCpuSource::CpuCoreSysfs => "cpu_core_sysfs",
        lqos_config::ShapingCpuSource::PossibleMinusAtomSysfs => "possible_minus_atom_sysfs",
        lqos_config::ShapingCpuSource::CpuidHybrid => "cpuid_hybrid",
        lqos_config::ShapingCpuSource::MaxFrequencyHeuristic => "max_frequency_heuristic",
        lqos_config::ShapingCpuSource::FallbackAllPossible => "fallback_all_possible",
        lqos_config::ShapingCpuSource::FallbackAvailableParallelism => {
            "fallback_available_parallelism"
        }
    }
}

fn topology_json(detection: lqos_config::ShapingCpuDetection) -> Value {
    json!({
        "available": true,
        "reason": null,
        "exclude_efficiency_cores": detection.exclude_efficiency_cores,
        "source": topology_source_str(detection.source),
        "from_cache": detection.from_cache,
        "has_hybrid_split": detection.has_hybrid_split,
        "possible": detection.possible,
        "performance": detection.performance,
        "efficiency": detection.efficiency,
        "shaping": detection.shaping,
        "detail": detection.detail,
    })
}

fn unavailable_topology(reason: &str) -> Value {
    json!({
        "available": false,
        "reason": reason,
        "exclude_efficiency_cores": null,
        "source": null,
        "from_cache": null,
        "has_hybrid_split": null,
        "possible": null,
        "performance": null,
        "efficiency": null,
        "shaping": null,
        "detail": null,
    })
}

fn topology_section() -> Value {
    CPU_TOPOLOGY
        .get()
        .and_then(|value| value.clone())
        .unwrap_or_else(|| unavailable_topology("topology_pending"))
}

/// Build the CPU-usage section from the daemon's 1 Hz sampler. A 0 CPU count
/// means the sampler has not ticked yet (unavailable, not zeros); a length
/// mismatch means the CPU-id join is unproven.
fn cpu_usage_section(snapshot: &crate::system_stats::CpuUsageSnapshot) -> Value {
    if snapshot.num_cpus == 0 {
        return json!({
            "available": false,
            "reason": "system_monitor_unavailable",
            "num_cpus": null,
            "per_cpu": null,
        });
    }
    if snapshot.cpu_usage.len() != snapshot.cpu_ids.len() {
        return json!({
            "available": false,
            "reason": "cpu_id_mapping_unproven",
            "num_cpus": snapshot.num_cpus,
            "per_cpu": null,
        });
    }
    let per_cpu: Vec<Value> = snapshot
        .cpu_usage
        .iter()
        .zip(snapshot.cpu_ids.iter())
        .map(|(percent, cpu_id)| {
            json!({
                "cpu_id": if *cpu_id == u32::MAX {
                    Value::Null
                } else {
                    json!(cpu_id)
                },
                "percent": percent,
            })
        })
        .collect();
    json!({
        "available": true,
        "reason": null,
        "num_cpus": snapshot.num_cpus,
        "per_cpu": per_cpu,
    })
}

fn cpu_usage_json() -> Value {
    cpu_usage_section(&crate::system_stats::cpu_usage_snapshot())
}

fn section_available(section: &Value) -> bool {
    section.get("available").and_then(Value::as_bool) == Some(true)
}

fn section_reason(section: &Value, fallback: &str) -> String {
    section
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or(fallback)
        .to_string()
}

fn push_reason(reasons: &mut Vec<String>, available: bool, section: &Value, fallback: &str) {
    if available {
        return;
    }
    let reason = section_reason(section, fallback);
    if !reasons.contains(&reason) {
        reasons.push(reason);
    }
}

/// Assemble the exact public API shape from the raw `GetCpuWorkers` document
/// plus the daemon-local CPU sources. Pure and unit-testable: no timestamps are
/// invented, and a missing raw section becomes an explicit unavailable object.
pub(crate) fn assemble_cpu_workers(raw: &Value, cpu_usage: Value, topology: Value) -> Value {
    let bridge = raw
        .get("bridge")
        .cloned()
        .unwrap_or_else(|| json!({"available": false, "reason": "bus_payload_incomplete"}));
    let workers = raw
        .get("workers")
        .cloned()
        .unwrap_or_else(|| json!({"available": false, "reason": "bus_payload_incomplete"}));
    let ownership = raw
        .get("ownership")
        .cloned()
        .unwrap_or_else(|| json!({"available": false, "reason": "bus_payload_incomplete"}));
    let moves = raw
        .get("moves")
        .cloned()
        .unwrap_or_else(|| json!({"available": false, "reason": "bus_payload_incomplete"}));

    let sections = [
        (&cpu_usage, "cpu_usage_unavailable"),
        (&topology, "cpu_topology_unavailable"),
        (&bridge, "bridge_unavailable"),
        (&workers, "workers_unavailable"),
        (&ownership, "ownership_unavailable"),
        (&moves, "moves_unavailable"),
    ];
    let mut unavailable_reasons = Vec::new();
    let mut all_available = true;
    let mut any_available = false;
    for (section, fallback) in sections {
        let available = section_available(section);
        all_available &= available;
        any_available |= available;
        push_reason(&mut unavailable_reasons, available, section, fallback);
    }
    let state = if all_available {
        "ok"
    } else if any_available {
        "partial"
    } else {
        "unavailable"
    };
    let generated_at_unix = raw
        .get("generated_at_unix")
        .cloned()
        .unwrap_or(Value::Null);

    json!({
        "state": state,
        "unavailable_reasons": unavailable_reasons,
        "generated_at_unix": generated_at_unix,
        "cpu_usage": cpu_usage,
        "cpu_topology": topology,
        "bridge": bridge,
        "workers": workers,
        "ownership": ownership,
        "moves": moves,
    })
}

/// `GET /local-api/cpu-workers` — the same-origin browser surface for the CPU &
/// Workers page. It reuses the daemon's `GetCpuWorkers` bus request and
/// assembles the SAME shape as the public `/cpu_workers` endpoint; the session
/// gate is the node-manager `auth_layer`, not an API key.
pub async fn get_cpu_workers() -> Result<Response, (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "lqosd bus unavailable".to_string(),
        )
    })?;
    let responses = client
        .request(vec![BusRequest::GetCpuWorkers])
        .await
        .map_err(|_| {
            (
                StatusCode::BAD_GATEWAY,
                "cpu-workers request failed".to_string(),
            )
        })?;
    for response in responses {
        if let BusResponse::CpuWorkers(document) = response {
            let raw = serde_json::from_str::<Value>(&document).map_err(|_| {
                (
                    StatusCode::BAD_GATEWAY,
                    "cpu-workers payload invalid".to_string(),
                )
            })?;
            let assembled = assemble_cpu_workers(&raw, cpu_usage_json(), topology_section());
            let mut response = Json(assembled).into_response();
            response.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-store"),
            );
            return Ok(response);
        }
    }
    Err((
        StatusCode::BAD_GATEWAY,
        "unexpected bus response for cpu-workers".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn raw_bus() -> Value {
        json!({
            "schema": 1,
            "generated_at_unix": 123,
            "bridge": {"available": true, "reason": null, "state": "healthy"},
            "workers": {
                "available": true,
                "reason": null,
                "schema": 1,
                "rx_queue_count": 1,
                "tx_queue_count": 1,
                "counter_support": "unsupported",
                "rows": [{
                    "worker_id": 0,
                    "lcore_id": null,
                    "numa_node": null,
                    "rx_queue_ids": null,
                    "tx_queue_id": null,
                    "shared_tx_queue": null,
                    "circuit_count": 0,
                    "solely_owned_node_count": 0,
                    "counters": null,
                    "throughput": null,
                    "throughput_support": "unsupported",
                    "drops": null,
                    "drops_support": "unsupported",
                }],
            },
            "ownership": {
                "available": true,
                "reason": null,
                "generation": "gen",
                "circuits": [
                    {"circuit_id": "c1", "parent_node_id": "root", "name": "Circuit One", "worker_id": 0},
                    {"circuit_id": "c2", "parent_node_id": "root", "name": null, "worker_id": null},
                ],
                "nodes": [
                    {"node_id": "root", "parent_node_id": null, "name": "Root", "worker_ids": [0], "crossing": false},
                    {"node_id": "site:1", "parent_node_id": "root", "name": null, "worker_ids": [0], "crossing": false},
                ],
                "crossings": [],
                "unowned_circuits": 1,
            },
            "moves": {
                "available": false,
                "reason": "no_history_source_v1",
                "source": null,
                "since_unix": null,
                "items": null,
            },
        })
    }

    fn cpu() -> Value {
        json!({
            "available": true,
            "reason": null,
            "num_cpus": 1,
            "per_cpu": [{"cpu_id": 0, "percent": 10}],
        })
    }

    fn topo() -> Value {
        json!({
            "available": true,
            "reason": null,
            "source": "exclusion_disabled",
            "exclude_efficiency_cores": false,
            "from_cache": false,
            "has_hybrid_split": false,
            "possible": [0],
            "performance": [0],
            "efficiency": [],
            "shaping": [0],
            "detail": "Efficiency-core exclusion disabled",
        })
    }

    #[test]
    fn local_response_has_the_exact_assembled_shape() {
        let assembled = assemble_cpu_workers(&raw_bus(), cpu(), topo());
        let keys: BTreeSet<&str> = assembled
            .as_object()
            .expect("object")
            .keys()
            .map(String::as_str)
            .collect();
        let expected: BTreeSet<&str> = [
            "state",
            "unavailable_reasons",
            "generated_at_unix",
            "cpu_usage",
            "cpu_topology",
            "bridge",
            "workers",
            "ownership",
            "moves",
        ]
        .into_iter()
        .collect();
        assert_eq!(keys, expected, "the local route must expose the public shape");
        assert!(
            assembled.get("schema").is_none(),
            "the raw bus schema marker must not leak into the page shape"
        );
        assert_eq!(
            assembled["state"].as_str(),
            Some("partial"),
            "moves v1 is unavailable"
        );
        assert!(
            assembled["unavailable_reasons"]
                .as_array()
                .expect("reasons")
                .iter()
                .any(|reason| reason.as_str() == Some("no_history_source_v1"))
        );
        assert_eq!(assembled["workers"]["counter_support"].as_str(), Some("unsupported"));
        assert!(assembled["workers"]["rows"][0]["throughput"].is_null());
        assert_eq!(
            assembled["workers"]["rows"][0]["throughput_support"].as_str(),
            Some("unsupported")
        );
        assert_eq!(
            assembled["ownership"]["circuits"][0]["name"].as_str(),
            Some("Circuit One")
        );
        assert!(
            assembled["ownership"]["circuits"][1]["name"].is_null(),
            "an absent name is null, never an empty string"
        );
        assert_eq!(assembled["ownership"]["nodes"][0]["name"].as_str(), Some("Root"));
        assert!(assembled["ownership"]["nodes"][1]["name"].is_null());
        assert_eq!(assembled["generated_at_unix"].as_u64(), Some(123));
        assert!(!assembled.to_string().contains("\"throughput\":0"));
    }

    #[test]
    fn missing_raw_sections_are_unavailable_not_zero() {
        let assembled = assemble_cpu_workers(&json!({}), cpu(), topo());
        assert_eq!(assembled["state"].as_str(), Some("partial"));
        assert_eq!(assembled["workers"]["available"].as_bool(), Some(false));
        assert_eq!(
            assembled["workers"]["reason"].as_str(),
            Some("bus_payload_incomplete")
        );
        assert!(assembled["workers"].get("rows").is_none());
        assert_eq!(assembled["ownership"]["available"].as_bool(), Some(false));
        assert!(assembled.get("schema").is_none());
    }

    #[test]
    fn cpu_usage_length_mismatch_is_unavailable() {
        let snapshot = crate::system_stats::CpuUsageSnapshot {
            cpu_usage: vec![1, 2],
            cpu_ids: vec![0],
            num_cpus: 2,
        };
        let section = cpu_usage_section(&snapshot);
        assert_eq!(section["available"].as_bool(), Some(false));
        assert_eq!(section["reason"].as_str(), Some("cpu_id_mapping_unproven"));
        assert!(section["per_cpu"].is_null());
    }

    #[test]
    fn topology_source_is_explicit_snake_case() {
        assert_eq!(
            topology_source_str(lqos_config::ShapingCpuSource::ExclusionDisabled),
            "exclusion_disabled"
        );
        assert_eq!(
            topology_source_str(lqos_config::ShapingCpuSource::CoreTypeSysfs),
            "core_type_sysfs"
        );
    }
}
