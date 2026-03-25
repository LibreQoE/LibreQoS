use crate::node_manager::ws::messages::{
    BakeryActivityEntry, BakeryCapacityInterfaceData, BakeryPreflightData,
    BakeryQueueDistributionData, BakeryRuntimeOperationHeadlineData, BakeryRuntimeOperationsData,
    BakeryStatusData, BakeryStatusState, WsResponse,
};
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use crate::shaped_devices_tracker::NETWORK_JSON;
use lqos_bakery::{
    BakeryActivityEntry as BakeryActivitySnapshot, BakeryApplyType, BakeryMode,
    BakeryPreflightSnapshot, BakeryRuntimeNodeOperationAction, BakeryRuntimeNodeOperationStatus,
    BakeryStatusSnapshot,
};
use lqos_utils::hash_to_i64;
use std::sync::Arc;

fn mode_to_string(mode: BakeryMode) -> String {
    match mode {
        BakeryMode::Idle => "Idle",
        BakeryMode::ApplyingFullReload => "ApplyingFullReload",
        BakeryMode::ApplyingLiveChange => "ApplyingLiveChange",
    }
    .to_string()
}

fn apply_type_to_string(apply_type: BakeryApplyType) -> String {
    match apply_type {
        BakeryApplyType::None => "None",
        BakeryApplyType::FullReload => "FullReload",
        BakeryApplyType::LiveChange => "LiveChange",
    }
    .to_string()
}

fn runtime_action_to_string(action: BakeryRuntimeNodeOperationAction) -> String {
    match action {
        BakeryRuntimeNodeOperationAction::Virtualize => "Virtualize",
        BakeryRuntimeNodeOperationAction::Restore => "Restore",
    }
    .to_string()
}

fn runtime_status_to_string(status: BakeryRuntimeNodeOperationStatus) -> String {
    match status {
        BakeryRuntimeNodeOperationStatus::Submitted => "Submitted",
        BakeryRuntimeNodeOperationStatus::Deferred => "Deferred",
        BakeryRuntimeNodeOperationStatus::Applying => "Applying",
        BakeryRuntimeNodeOperationStatus::AppliedAwaitingCleanup => "AppliedAwaitingCleanup",
        BakeryRuntimeNodeOperationStatus::Completed => "Completed",
        BakeryRuntimeNodeOperationStatus::Failed => "Failed",
        BakeryRuntimeNodeOperationStatus::Dirty => "Dirty",
    }
    .to_string()
}

fn map_preflight(snapshot: BakeryPreflightSnapshot) -> BakeryPreflightData {
    BakeryPreflightData {
        ok: snapshot.ok,
        message: snapshot.message,
        safe_budget: snapshot.safe_budget,
        hard_limit: snapshot.hard_limit,
        estimated_total_memory_bytes: snapshot.estimated_total_memory_bytes,
        memory_available_bytes: snapshot.memory_available_bytes,
        memory_guard_min_available_bytes: snapshot.memory_guard_min_available_bytes,
        memory_ok: snapshot.memory_ok,
        interfaces: snapshot
            .interfaces
            .into_iter()
            .map(|entry| BakeryCapacityInterfaceData {
                name: entry.name,
                planned_qdiscs: entry.planned_qdiscs,
                infra_qdiscs: entry.infra_qdiscs,
                cake_qdiscs: entry.cake_qdiscs,
                fq_codel_qdiscs: entry.fq_codel_qdiscs,
                estimated_memory_bytes: entry.estimated_memory_bytes,
            })
            .collect(),
    }
}

fn resolve_site_name(site_hash: i64) -> Option<String> {
    let reader = NETWORK_JSON.read();
    reader
        .get_nodes_when_ready()
        .iter()
        .find_map(|node| (hash_to_i64(&node.name) == site_hash).then(|| node.name.clone()))
}

fn extract_site_hash_from_summary(summary: &str) -> Option<i64> {
    let lower = summary.to_ascii_lowercase();
    let site_index = lower.find("site ")?;
    let rest = &summary[site_index + 5..];
    let digits: String = rest
        .chars()
        .take_while(|ch| *ch == '-' || ch.is_ascii_digit())
        .collect();
    (!digits.is_empty()).then_some(digits)?.parse().ok()
}

fn map_status(snapshot: BakeryStatusSnapshot) -> BakeryStatusData {
    BakeryStatusData {
        current_state: BakeryStatusState {
            active_circuits: snapshot.active_circuits,
            mode: mode_to_string(snapshot.mode),
            current_action_started_unix: snapshot.current_action_started_unix,
            current_apply_phase: snapshot.current_apply_phase,
            current_apply_total_tc_commands: snapshot.current_apply_total_tc_commands,
            current_apply_completed_tc_commands: snapshot.current_apply_completed_tc_commands,
            current_apply_total_chunks: snapshot.current_apply_total_chunks,
            current_apply_completed_chunks: snapshot.current_apply_completed_chunks,
            last_success_unix: snapshot.last_success_unix,
            last_full_reload_success_unix: snapshot.last_full_reload_success_unix,
            last_failure_unix: snapshot.last_failure_unix,
            last_failure_summary: snapshot.last_failure_summary,
            last_apply_type: apply_type_to_string(snapshot.last_apply_type),
            last_total_tc_commands: snapshot.last_total_tc_commands,
            last_class_commands: snapshot.last_class_commands,
            last_qdisc_commands: snapshot.last_qdisc_commands,
            last_build_duration_ms: snapshot.last_build_duration_ms,
            last_apply_duration_ms: snapshot.last_apply_duration_ms,
            runtime_operations: BakeryRuntimeOperationsData {
                submitted_count: snapshot.runtime_operations.submitted_count,
                deferred_count: snapshot.runtime_operations.deferred_count,
                applying_count: snapshot.runtime_operations.applying_count,
                awaiting_cleanup_count: snapshot.runtime_operations.awaiting_cleanup_count,
                failed_count: snapshot.runtime_operations.failed_count,
                dirty_count: snapshot.runtime_operations.dirty_count,
                latest: snapshot.runtime_operations.latest.map(|entry| {
                    BakeryRuntimeOperationHeadlineData {
                        operation_id: entry.operation_id,
                        site_hash: entry.site_hash,
                        site_name: resolve_site_name(entry.site_hash),
                        action: runtime_action_to_string(entry.action),
                        status: runtime_status_to_string(entry.status),
                        attempt_count: entry.attempt_count,
                        updated_at_unix: entry.updated_at_unix,
                        next_retry_at_unix: entry.next_retry_at_unix,
                        last_error: entry.last_error,
                    }
                }),
            },
            reload_required: snapshot.reload_required,
            reload_required_reason: snapshot.reload_required_reason,
            dirty_subtree_count: snapshot.dirty_subtree_count,
            queue_distribution: snapshot
                .queue_distribution
                .into_iter()
                .map(|entry| BakeryQueueDistributionData {
                    queue: entry.queue,
                    top_level_site_count: entry.top_level_site_count,
                    site_count: entry.site_count,
                    circuit_count: entry.circuit_count,
                    download_mbps: entry.download_mbps,
                    upload_mbps: entry.upload_mbps,
                })
                .collect(),
            preflight: snapshot.preflight.map(map_preflight),
        },
    }
}

fn map_activity(entry: BakeryActivitySnapshot) -> BakeryActivityEntry {
    let site_name = extract_site_hash_from_summary(&entry.summary).and_then(resolve_site_name);
    BakeryActivityEntry {
        ts: entry.ts,
        event: entry.event,
        status: entry.status,
        summary: entry.summary,
        site_name,
    }
}

pub async fn bakery_status(pubsub: Arc<PubSub>) {
    if !pubsub
        .is_channel_alive(PublishedChannels::BakeryStatus)
        .await
    {
        return;
    }

    let data = map_status(lqos_bakery::bakery_status_snapshot());
    let msg = WsResponse::BakeryStatus { data };
    pubsub.send(PublishedChannels::BakeryStatus, msg).await;
}

pub async fn bakery_activity(pubsub: Arc<PubSub>) {
    if !pubsub
        .is_channel_alive(PublishedChannels::BakeryActivity)
        .await
    {
        return;
    }

    let data = lqos_bakery::bakery_activity_snapshot()
        .into_iter()
        .map(map_activity)
        .collect();
    let msg = WsResponse::BakeryActivity { data };
    pubsub.send(PublishedChannels::BakeryActivity, msg).await;
}
