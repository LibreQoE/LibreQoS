use crate::node_manager::ws::messages::{
    BakeryActivityEntry, BakeryCapacityInterfaceData, BakeryPreflightData, BakeryStatusData,
    BakeryStatusState, WsResponse,
};
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use lqos_bakery::{
    BakeryActivityEntry as BakeryActivitySnapshot, BakeryApplyType, BakeryMode,
    BakeryPreflightSnapshot, BakeryStatusSnapshot,
};
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
            last_failure_unix: snapshot.last_failure_unix,
            last_failure_summary: snapshot.last_failure_summary,
            last_apply_type: apply_type_to_string(snapshot.last_apply_type),
            last_total_tc_commands: snapshot.last_total_tc_commands,
            last_class_commands: snapshot.last_class_commands,
            last_qdisc_commands: snapshot.last_qdisc_commands,
            last_build_duration_ms: snapshot.last_build_duration_ms,
            last_apply_duration_ms: snapshot.last_apply_duration_ms,
            preflight: snapshot.preflight.map(map_preflight),
        },
    }
}

fn map_activity(entry: BakeryActivitySnapshot) -> BakeryActivityEntry {
    BakeryActivityEntry {
        ts: entry.ts,
        event: entry.event,
        status: entry.status,
        summary: entry.summary,
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
