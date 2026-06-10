//! Tests for in-memory accounting session tracking.

use super::*;
use crate::{
    DynamicCircuitCommandSink, DynamicCircuitIntent, DynamicCircuitRemovalReason, MikrotikRateLimit,
};
use std::net::{Ipv4Addr, Ipv6Addr};

#[test]
fn start_and_interim_update_create_or_refresh_sessions_by_nas_and_session_id() {
    let mut store = AccountingSessionStore::new();
    let start = complete_event(
        AcctStatusType::Start,
        "nas-a",
        "session-1",
        Ipv4Addr::new(198, 51, 100, 10),
    );

    let update = store.apply_event_with_mapping(start, DynamicCircuitMapping::Ready);
    let expected_key = nas_session_key("nas-a", "session-1");

    assert_eq!(
        update,
        AccountingSessionUpdate::SessionUpdated {
            key: expected_key.clone(),
            state: AccountingSessionState::Active,
        }
    );
    assert_eq!(store.len(), 1);
    let created = store.session(&expected_key).unwrap();
    assert_eq!(created.pending_reasons, Vec::new());
    assert_eq!(
        created.latest_event.status_type,
        Some(AcctStatusType::Start)
    );
    assert_eq!(
        created.latest_event.nas_identifier.as_deref(),
        Some("nas-a")
    );
    assert_eq!(
        created.latest_event.framed_ip_address,
        Some(Ipv4Addr::new(198, 51, 100, 10))
    );

    let mut interim = complete_event(
        AcctStatusType::InterimUpdate,
        "nas-a",
        "session-1",
        Ipv4Addr::new(198, 51, 100, 11),
    );
    interim.user_name = Some("updated-subscriber".to_string());
    store.apply_event_with_mapping(interim, DynamicCircuitMapping::Ready);

    let refreshed = store.session(&expected_key).unwrap();
    assert_eq!(store.len(), 1);
    assert_eq!(refreshed.state, AccountingSessionState::Active);
    assert_eq!(
        refreshed.latest_event.status_type,
        Some(AcctStatusType::InterimUpdate)
    );
    assert_eq!(
        refreshed.latest_event.framed_ip_address,
        Some(Ipv4Addr::new(198, 51, 100, 11))
    );
    assert_eq!(
        refreshed.latest_event.user_name.as_deref(),
        Some("updated-subscriber")
    );

    let mut sparse_store = AccountingSessionStore::new();
    let sparse_key = nas_session_key("nas-sparse", "session-sparse");
    let mut sparse_start = complete_event(
        AcctStatusType::Start,
        "nas-sparse",
        "session-sparse",
        Ipv4Addr::new(198, 51, 100, 13),
    );
    sparse_start.class = vec![b"class-data".to_vec()];
    sparse_store.apply_event_with_mapping(sparse_start, DynamicCircuitMapping::Ready);
    sparse_store.apply_event_with_mapping(
        minimal_session_event(
            AcctStatusType::InterimUpdate,
            "nas-sparse",
            "session-sparse",
        ),
        DynamicCircuitMapping::Ready,
    );
    let sparse_session = sparse_store.session(&sparse_key).unwrap();
    assert_eq!(
        sparse_session.latest_event.status_type,
        Some(AcctStatusType::InterimUpdate)
    );
    assert_eq!(
        sparse_session.latest_event.framed_ip_address,
        Some(Ipv4Addr::new(198, 51, 100, 13))
    );
    assert!(!sparse_session.latest_event.mikrotik_rate_limits.is_empty());
    assert_eq!(
        sparse_session.latest_event.class,
        vec![b"class-data".to_vec()]
    );
    assert_eq!(sparse_session.pending_reasons, Vec::new());

    store.apply_event_with_mapping(
        complete_event(
            AcctStatusType::Start,
            "nas-b",
            "session-1",
            Ipv4Addr::new(198, 51, 100, 12),
        ),
        DynamicCircuitMapping::Ready,
    );
    assert_eq!(store.len(), 2);

    let mut interim_only_store = AccountingSessionStore::new();
    interim_only_store.apply_event_with_mapping(
        complete_event(
            AcctStatusType::InterimUpdate,
            "nas-c",
            "session-9",
            Ipv4Addr::new(198, 51, 100, 19),
        ),
        DynamicCircuitMapping::Ready,
    );
    assert_eq!(interim_only_store.len(), 1);
    assert_eq!(
        interim_only_store
            .session(&nas_session_key("nas-c", "session-9"))
            .unwrap()
            .state,
        AccountingSessionState::Active
    );

    let nas_ip = Ipv4Addr::new(192, 0, 2, 50);
    let mut mixed_identity_store = AccountingSessionStore::new();
    let mut start_by_ip = complete_event(
        AcctStatusType::Start,
        "ignored-by-test",
        "session-mixed",
        Ipv4Addr::new(198, 51, 100, 50),
    );
    start_by_ip.nas_identifier = None;
    start_by_ip.nas_ip_address = Some(nas_ip);
    let ip_key = AccountingSessionKey::NasSession {
        nas: NasIdentity::Ipv4(nas_ip),
        acct_session_id: "session-mixed".to_string(),
    };
    mixed_identity_store.apply_event_with_mapping(start_by_ip, DynamicCircuitMapping::Ready);

    let mut interim_with_identifier = complete_event(
        AcctStatusType::InterimUpdate,
        "nas-with-new-identifier",
        "session-mixed",
        Ipv4Addr::new(198, 51, 100, 51),
    );
    interim_with_identifier.nas_ip_address = Some(nas_ip);
    let mixed_update = mixed_identity_store
        .apply_event_with_mapping(interim_with_identifier, DynamicCircuitMapping::Ready);

    assert_eq!(
        mixed_update,
        AccountingSessionUpdate::SessionUpdated {
            key: ip_key.clone(),
            state: AccountingSessionState::Active,
        }
    );
    assert_eq!(mixed_identity_store.len(), 1);
    assert_eq!(
        mixed_identity_store
            .session(&ip_key)
            .unwrap()
            .latest_event
            .nas_identifier
            .as_deref(),
        Some("nas-with-new-identifier")
    );

    let pending_collision_ip = Ipv4Addr::new(192, 0, 2, 53);
    let mut pending_collision_store = AccountingSessionStore::new();
    pending_collision_store.apply_event(AccountingEvent {
        status_type: Some(AcctStatusType::Start),
        nas_identifier: Some("unrelated-pending".to_string()),
        ..AccountingEvent::default()
    });
    let mut pending_collision_start = complete_event(
        AcctStatusType::Start,
        "ignored-by-test",
        "session-pending-collision",
        Ipv4Addr::new(198, 51, 100, 58),
    );
    pending_collision_start.nas_identifier = None;
    pending_collision_start.nas_ip_address = Some(pending_collision_ip);
    pending_collision_store
        .apply_event_with_mapping(pending_collision_start, DynamicCircuitMapping::Ready);
    let pending_collision_key = AccountingSessionKey::NasSession {
        nas: NasIdentity::Ipv4(pending_collision_ip),
        acct_session_id: "session-pending-collision".to_string(),
    };
    let mut pending_collision_interim = complete_event(
        AcctStatusType::InterimUpdate,
        "nas-pending-collision",
        "session-pending-collision",
        Ipv4Addr::new(198, 51, 100, 59),
    );
    pending_collision_interim.nas_ip_address = Some(pending_collision_ip);
    pending_collision_store
        .apply_event_with_mapping(pending_collision_interim, DynamicCircuitMapping::Ready);
    assert_eq!(pending_collision_store.len(), 2);
    assert_eq!(
        pending_collision_store
            .session(&pending_collision_key)
            .unwrap()
            .latest_event
            .nas_identifier
            .as_deref(),
        Some("nas-pending-collision")
    );

    let mut pending_to_keyed_store = AccountingSessionStore::new();
    let mut missing_nas = complete_event(
        AcctStatusType::Start,
        "ignored-by-test",
        "session-late-nas",
        Ipv4Addr::new(198, 51, 100, 52),
    );
    missing_nas.nas_identifier = None;
    pending_to_keyed_store.apply_event_with_mapping(missing_nas, DynamicCircuitMapping::Ready);
    let late_nas_key = nas_session_key("nas-late", "session-late-nas");
    pending_to_keyed_store.apply_event_with_mapping(
        complete_event(
            AcctStatusType::InterimUpdate,
            "nas-late",
            "session-late-nas",
            Ipv4Addr::new(198, 51, 100, 53),
        ),
        DynamicCircuitMapping::Ready,
    );
    assert_eq!(pending_to_keyed_store.len(), 1);
    assert_eq!(
        pending_to_keyed_store
            .session(&late_nas_key)
            .unwrap()
            .latest_event
            .status_type,
        Some(AcctStatusType::InterimUpdate)
    );

    let split_nas_ip = Ipv4Addr::new(192, 0, 2, 51);
    let split_key = nas_session_key("nas-split", "session-split");
    let mut split_store = AccountingSessionStore::new();
    split_store.apply_event_with_mapping(
        complete_event(
            AcctStatusType::Start,
            "nas-split",
            "session-split",
            Ipv4Addr::new(198, 51, 100, 54),
        ),
        DynamicCircuitMapping::Ready,
    );
    let mut ip_only_interim = complete_event(
        AcctStatusType::InterimUpdate,
        "ignored-by-test",
        "session-split",
        Ipv4Addr::new(198, 51, 100, 55),
    );
    ip_only_interim.nas_identifier = None;
    ip_only_interim.nas_ip_address = Some(split_nas_ip);
    split_store.apply_event_with_mapping(ip_only_interim, DynamicCircuitMapping::Ready);
    assert_eq!(split_store.len(), 2);

    let mut combined_identity_interim = complete_event(
        AcctStatusType::InterimUpdate,
        "nas-split",
        "session-split",
        Ipv4Addr::new(198, 51, 100, 56),
    );
    combined_identity_interim.nas_ip_address = Some(split_nas_ip);
    split_store.apply_event_with_mapping(combined_identity_interim, DynamicCircuitMapping::Ready);
    assert_eq!(split_store.len(), 1);
    assert_eq!(
        split_store
            .session(&split_key)
            .unwrap()
            .latest_event
            .framed_ip_address,
        Some(Ipv4Addr::new(198, 51, 100, 56))
    );

    let retained_ipv6: Ipv6Addr = "2001:db8::60".parse().unwrap();
    let mut split_with_extra_alias = AccountingSessionStore::new();
    split_with_extra_alias.apply_event_with_mapping(
        complete_event(
            AcctStatusType::Start,
            "nas-extra-alias",
            "session-extra-alias",
            Ipv4Addr::new(198, 51, 100, 60),
        ),
        DynamicCircuitMapping::Ready,
    );
    let alternate_key = AccountingSessionKey::NasSession {
        nas: NasIdentity::Ipv4(split_nas_ip),
        acct_session_id: "session-extra-alias".to_string(),
    };
    split_with_extra_alias.sessions.insert(
        alternate_key,
        AccountingSession {
            state: AccountingSessionState::Active,
            latest_event: AccountingEvent {
                status_type: Some(AcctStatusType::InterimUpdate),
                acct_session_id: Some("session-extra-alias".to_string()),
                nas_ip_address: Some(split_nas_ip),
                nas_ipv6_address: Some(retained_ipv6),
                ..AccountingEvent::default()
            },
            known_nas_identities: vec![
                NasIdentity::Ipv4(split_nas_ip),
                NasIdentity::Ipv6(retained_ipv6),
            ],
            active_dynamic_circuit_ids: Vec::new(),
            pending_reasons: Vec::new(),
        },
    );
    let mut extra_alias_merge = complete_event(
        AcctStatusType::InterimUpdate,
        "nas-extra-alias",
        "session-extra-alias",
        Ipv4Addr::new(198, 51, 100, 61),
    );
    extra_alias_merge.nas_ip_address = Some(split_nas_ip);
    split_with_extra_alias
        .apply_event_with_mapping(extra_alias_merge, DynamicCircuitMapping::Ready);
    let extra_alias_key = nas_session_key("nas-extra-alias", "session-extra-alias");
    split_with_extra_alias.apply_event(AccountingEvent {
        status_type: Some(AcctStatusType::AccountingOff),
        nas_ipv6_address: Some(retained_ipv6),
        ..AccountingEvent::default()
    });
    assert_eq!(
        split_with_extra_alias
            .session(&extra_alias_key)
            .unwrap()
            .state,
        AccountingSessionState::Stale(NasResetStatus::AccountingOff)
    );

    let rich_alternate_ip = Ipv4Addr::new(192, 0, 2, 61);
    let mut rich_alternate_store = AccountingSessionStore::new();
    let rich_target_key = nas_session_key("nas-rich-merge", "session-rich-merge");
    rich_alternate_store.apply_event(minimal_session_event(
        AcctStatusType::Start,
        "nas-rich-merge",
        "session-rich-merge",
    ));
    let rich_alternate_key = AccountingSessionKey::NasSession {
        nas: NasIdentity::Ipv4(rich_alternate_ip),
        acct_session_id: "session-rich-merge".to_string(),
    };
    let mut rich_alternate_event = complete_event(
        AcctStatusType::InterimUpdate,
        "ignored-by-test",
        "session-rich-merge",
        Ipv4Addr::new(198, 51, 100, 62),
    );
    rich_alternate_event.nas_identifier = None;
    rich_alternate_event.nas_ip_address = Some(rich_alternate_ip);
    rich_alternate_store.sessions.insert(
        rich_alternate_key,
        AccountingSession {
            state: AccountingSessionState::Active,
            latest_event: rich_alternate_event,
            known_nas_identities: vec![NasIdentity::Ipv4(rich_alternate_ip)],
            active_dynamic_circuit_ids: Vec::new(),
            pending_reasons: Vec::new(),
        },
    );
    let mut sparse_bridge = minimal_session_event(
        AcctStatusType::InterimUpdate,
        "nas-rich-merge",
        "session-rich-merge",
    );
    sparse_bridge.nas_ip_address = Some(rich_alternate_ip);
    rich_alternate_store.apply_event_with_mapping(sparse_bridge, DynamicCircuitMapping::Ready);

    let rich_merged = rich_alternate_store.session(&rich_target_key).unwrap();
    assert_eq!(rich_alternate_store.len(), 1);
    assert_eq!(
        rich_merged.latest_event.framed_ip_address,
        Some(Ipv4Addr::new(198, 51, 100, 62))
    );
    assert!(!rich_merged.latest_event.mikrotik_rate_limits.is_empty());
    assert_eq!(rich_merged.pending_reasons, Vec::new());

    let ambiguous_ip = Ipv4Addr::new(192, 0, 2, 52);
    let mut ambiguous_retained_store = AccountingSessionStore::new();
    let (ambiguous_key_a, ambiguous_session_a) =
        retained_session_with_known_ip("nas-ambiguous-a", "shared-session", ambiguous_ip);
    let (ambiguous_key_b, ambiguous_session_b) =
        retained_session_with_known_ip("nas-ambiguous-b", "shared-session", ambiguous_ip);
    ambiguous_retained_store
        .sessions
        .insert(ambiguous_key_a.clone(), ambiguous_session_a);
    ambiguous_retained_store
        .sessions
        .insert(ambiguous_key_b.clone(), ambiguous_session_b);
    let mut ambiguous_interim = complete_event(
        AcctStatusType::InterimUpdate,
        "ignored-by-test",
        "shared-session",
        Ipv4Addr::new(198, 51, 100, 57),
    );
    ambiguous_interim.nas_identifier = None;
    ambiguous_interim.nas_ip_address = Some(ambiguous_ip);
    ambiguous_retained_store
        .apply_event_with_mapping(ambiguous_interim, DynamicCircuitMapping::Ready);

    assert_eq!(ambiguous_retained_store.len(), 3);
    assert_eq!(
        ambiguous_retained_store
            .session(&ambiguous_key_a)
            .unwrap()
            .state,
        AccountingSessionState::Active
    );
    assert_eq!(
        ambiguous_retained_store
            .session(&ambiguous_key_b)
            .unwrap()
            .state,
        AccountingSessionState::Active
    );
}

#[test]
fn stop_marks_session_stopped_and_duplicate_stop_is_harmless() {
    let mut store = AccountingSessionStore::new();
    let key = nas_session_key("nas-a", "session-1");
    store.apply_event_with_mapping(
        complete_event(
            AcctStatusType::Start,
            "nas-a",
            "session-1",
            Ipv4Addr::new(198, 51, 100, 20),
        ),
        DynamicCircuitMapping::Ready,
    );

    let stop = minimal_session_event(AcctStatusType::Stop, "nas-a", "session-1");
    let first_stop = store.apply_event(stop.clone());
    let second_stop = store.apply_event(stop);

    assert_eq!(
        first_stop,
        AccountingSessionUpdate::SessionUpdated {
            key: key.clone(),
            state: AccountingSessionState::Stopped,
        }
    );
    assert_eq!(second_stop, first_stop);
    assert_eq!(store.len(), 1);
    let session = store.session(&key).unwrap();
    assert_eq!(session.state, AccountingSessionState::Stopped);
    assert_eq!(session.latest_event.status_type, Some(AcctStatusType::Stop));
    assert_eq!(session.latest_event.framed_ip_address, None);

    let mut pending_stop_store = AccountingSessionStore::new();
    let pending_stop = AccountingEvent {
        status_type: Some(AcctStatusType::Stop),
        nas_identifier: Some("nas-pending".to_string()),
        user_name: Some("subscriber-without-session-id".to_string()),
        ..AccountingEvent::default()
    };
    let first_pending_stop = pending_stop_store.apply_event(pending_stop.clone());
    let second_pending_stop = pending_stop_store.apply_event(pending_stop);

    let AccountingSessionUpdate::SessionUpdated {
        key: pending_stop_key,
        state: pending_stop_state,
    } = first_pending_stop.clone()
    else {
        panic!("expected pending stop update, got {first_pending_stop:?}");
    };

    assert_eq!(pending_stop_state, AccountingSessionState::Stopped);
    assert_eq!(second_pending_stop, first_pending_stop);
    assert_eq!(pending_stop_store.len(), 1);
    let pending_session = pending_stop_store.session(&pending_stop_key).unwrap();
    assert_eq!(pending_session.state, AccountingSessionState::Stopped);
    assert_eq!(
        pending_session.latest_event.status_type,
        Some(AcctStatusType::Stop)
    );

    let mut fallback_stop_store = AccountingSessionStore::new();
    let fallback_key = nas_session_key("nas-fallback", "session-fallback");
    fallback_stop_store.apply_event_with_mapping(
        complete_event(
            AcctStatusType::Start,
            "nas-fallback",
            "session-fallback",
            Ipv4Addr::new(198, 51, 100, 21),
        ),
        DynamicCircuitMapping::Ready,
    );
    let fallback_stop = AccountingEvent {
        status_type: Some(AcctStatusType::Stop),
        nas_identifier: Some("nas-fallback".to_string()),
        user_name: Some("subscriber".to_string()),
        ..AccountingEvent::default()
    };
    let fallback_update = fallback_stop_store.apply_event(fallback_stop.clone());
    let duplicate_fallback_update = fallback_stop_store.apply_event(fallback_stop);

    assert_eq!(
        fallback_update,
        AccountingSessionUpdate::SessionUpdated {
            key: fallback_key.clone(),
            state: AccountingSessionState::Stopped,
        }
    );
    assert_eq!(duplicate_fallback_update, fallback_update);
    assert_eq!(fallback_stop_store.len(), 1);
    assert_eq!(
        fallback_stop_store.session(&fallback_key).unwrap().state,
        AccountingSessionState::Stopped
    );
}

#[test]
fn accounting_on_and_off_mark_only_matching_nas_sessions_stale() {
    let mut store = AccountingSessionStore::new();
    let nas_a_session_1 = nas_session_key("nas-a", "session-1");
    let nas_a_session_2 = nas_session_key("nas-a", "session-2");
    let nas_b_session = nas_session_key("nas-b", "session-1");

    for (nas, session_id, last_octet) in [
        ("nas-a", "session-1", 30),
        ("nas-a", "session-2", 31),
        ("nas-b", "session-1", 32),
    ] {
        store.apply_event_with_mapping(
            complete_event(
                AcctStatusType::Start,
                nas,
                session_id,
                Ipv4Addr::new(198, 51, 100, last_octet),
            ),
            DynamicCircuitMapping::Ready,
        );
    }

    let accounting_off = store.apply_event(reset_event(AcctStatusType::AccountingOff, "nas-a"));

    assert_eq!(
        accounting_off,
        AccountingSessionUpdate::NasSessionsMarkedStale {
            nas: NasIdentity::Identifier("nas-a".to_string()),
            reset: NasResetStatus::AccountingOff,
            marked_count: 2,
        }
    );
    assert_eq!(
        store.session(&nas_a_session_1).unwrap().state,
        AccountingSessionState::Stale(NasResetStatus::AccountingOff)
    );
    assert_eq!(
        store.session(&nas_a_session_2).unwrap().state,
        AccountingSessionState::Stale(NasResetStatus::AccountingOff)
    );
    assert_eq!(
        store.session(&nas_b_session).unwrap().state,
        AccountingSessionState::Active
    );

    let accounting_on = store.apply_event(reset_event(AcctStatusType::AccountingOn, "nas-b"));

    assert_eq!(
        accounting_on,
        AccountingSessionUpdate::NasSessionsMarkedStale {
            nas: NasIdentity::Identifier("nas-b".to_string()),
            reset: NasResetStatus::AccountingOn,
            marked_count: 1,
        }
    );
    assert_eq!(
        store.session(&nas_b_session).unwrap().state,
        AccountingSessionState::Stale(NasResetStatus::AccountingOn)
    );

    let nas_c_ip = Ipv4Addr::new(192, 0, 2, 60);
    let nas_c_session = nas_session_key("nas-c", "session-1");
    let mut nas_c_start = complete_event(
        AcctStatusType::Start,
        "nas-c",
        "session-1",
        Ipv4Addr::new(198, 51, 100, 33),
    );
    nas_c_start.nas_ip_address = Some(nas_c_ip);
    store.apply_event_with_mapping(nas_c_start, DynamicCircuitMapping::Ready);
    store.apply_event_with_mapping(
        complete_event(
            AcctStatusType::InterimUpdate,
            "nas-c",
            "session-1",
            Ipv4Addr::new(198, 51, 100, 34),
        ),
        DynamicCircuitMapping::Ready,
    );

    let accounting_off_by_old_ip = store.apply_event(AccountingEvent {
        status_type: Some(AcctStatusType::AccountingOff),
        nas_ip_address: Some(nas_c_ip),
        ..AccountingEvent::default()
    });

    assert_eq!(
        accounting_off_by_old_ip,
        AccountingSessionUpdate::NasSessionsMarkedStale {
            nas: NasIdentity::Ipv4(nas_c_ip),
            reset: NasResetStatus::AccountingOff,
            marked_count: 1,
        }
    );
    assert_eq!(
        store.session(&nas_c_session).unwrap().state,
        AccountingSessionState::Stale(NasResetStatus::AccountingOff)
    );
}

#[test]
fn pending_reasons_are_retained_for_unshapeable_sessions() {
    let mut store = AccountingSessionStore::new();
    let missing_session_id = AccountingEvent {
        status_type: Some(AcctStatusType::Start),
        nas_identifier: Some("nas-a".to_string()),
        ..AccountingEvent::default()
    };

    let update = store.apply_event(missing_session_id);
    let AccountingSessionUpdate::SessionUpdated { key, state } = update else {
        panic!("expected pending session update, got {update:?}");
    };

    assert_eq!(
        key,
        AccountingSessionKey::Pending {
            fingerprint: PendingSessionFingerprint {
                nas: Some(NasIdentity::Identifier("nas-a".to_string())),
                acct_session_id: None,
                user_name: None,
                calling_station_id: None,
                nas_port_id: None,
                nas_port: None,
            },
        }
    );
    assert_eq!(state, AccountingSessionState::Active);
    assert_eq!(
        store.session(&key).unwrap().pending_reasons,
        vec![
            PendingSessionReason::MissingSessionId,
            PendingSessionReason::MissingIpAddress,
            PendingSessionReason::MissingRate,
            PendingSessionReason::MissingParent,
        ]
    );

    let missing_nas = AccountingEvent {
        status_type: Some(AcctStatusType::Start),
        acct_session_id: Some("session-without-nas".to_string()),
        framed_ip_address: Some(Ipv4Addr::new(198, 51, 100, 40)),
        mikrotik_rate_limits: vec![rate_limit()],
        ..AccountingEvent::default()
    };
    let missing_nas_update =
        store.apply_event_with_mapping(missing_nas, DynamicCircuitMapping::Ready);
    let AccountingSessionUpdate::SessionUpdated {
        key: missing_nas_key,
        ..
    } = missing_nas_update
    else {
        panic!("expected missing-NAS pending session update, got {missing_nas_update:?}");
    };
    assert_eq!(
        store.session(&missing_nas_key).unwrap().pending_reasons,
        vec![PendingSessionReason::MissingNasIdentity]
    );

    let ambiguous_mapping_key = nas_session_key("nas-b", "session-ambiguous");
    store.apply_event_with_mapping(
        complete_event(
            AcctStatusType::Start,
            "nas-b",
            "session-ambiguous",
            Ipv4Addr::new(198, 51, 100, 41),
        ),
        DynamicCircuitMapping::Ambiguous,
    );
    assert_eq!(
        store
            .session(&ambiguous_mapping_key)
            .unwrap()
            .pending_reasons,
        vec![PendingSessionReason::AmbiguousMapping]
    );

    let mut route_only = minimal_session_event(AcctStatusType::Start, "nas-route", "session-route");
    route_only.framed_routes = vec!["198.51.100.0/24 192.0.2.1 1".to_string()];
    route_only.mikrotik_rate_limits = vec![rate_limit()];
    let route_key = nas_session_key("nas-route", "session-route");
    store.apply_event_with_mapping(route_only, DynamicCircuitMapping::Ready);
    assert_eq!(
        store.session(&route_key).unwrap().pending_reasons,
        Vec::new()
    );

    let mut pending_promotion_store = AccountingSessionStore::new();
    let mut missing_id = AccountingEvent {
        status_type: Some(AcctStatusType::Start),
        nas_identifier: Some("nas-promotion".to_string()),
        user_name: Some("subscriber-late-session-id".to_string()),
        ..AccountingEvent::default()
    };
    pending_promotion_store.apply_event(missing_id.clone());
    missing_id.status_type = Some(AcctStatusType::InterimUpdate);
    missing_id.acct_session_id = Some("promoted-session".to_string());
    missing_id.framed_ip_address = Some(Ipv4Addr::new(198, 51, 100, 42));
    missing_id.mikrotik_rate_limits = vec![rate_limit()];
    let promoted_key = nas_session_key("nas-promotion", "promoted-session");
    pending_promotion_store.apply_event_with_mapping(missing_id, DynamicCircuitMapping::Ready);
    assert_eq!(pending_promotion_store.len(), 1);
    assert_eq!(
        pending_promotion_store
            .session(&promoted_key)
            .unwrap()
            .pending_reasons,
        Vec::new()
    );

    let mut delayed_pending_store = AccountingSessionStore::new();
    let delayed_pending_ip = Ipv4Addr::new(192, 0, 2, 72);
    delayed_pending_store.apply_event(AccountingEvent {
        status_type: Some(AcctStatusType::Start),
        nas_ip_address: Some(delayed_pending_ip),
        user_name: Some("delayed-pending-user".to_string()),
        ..AccountingEvent::default()
    });
    delayed_pending_store.apply_event_with_mapping(
        minimal_session_event(
            AcctStatusType::Start,
            "nas-delayed-pending",
            "delayed-pending-session",
        ),
        DynamicCircuitMapping::Ready,
    );
    assert_eq!(delayed_pending_store.len(), 2);
    let mut delayed_rich_event = complete_event(
        AcctStatusType::InterimUpdate,
        "nas-delayed-pending",
        "delayed-pending-session",
        Ipv4Addr::new(198, 51, 100, 46),
    );
    delayed_rich_event.nas_ip_address = Some(delayed_pending_ip);
    delayed_rich_event.user_name = Some("delayed-pending-user".to_string());
    let delayed_key = nas_session_key("nas-delayed-pending", "delayed-pending-session");
    delayed_pending_store
        .apply_event_with_mapping(delayed_rich_event, DynamicCircuitMapping::Ready);
    assert_eq!(delayed_pending_store.len(), 1);
    assert_eq!(
        delayed_pending_store
            .session(&delayed_key)
            .unwrap()
            .latest_event
            .framed_ip_address,
        Some(Ipv4Addr::new(198, 51, 100, 46))
    );

    let mut delayed_alternate_store = AccountingSessionStore::new();
    let delayed_alternate_ip = Ipv4Addr::new(192, 0, 2, 73);
    delayed_alternate_store.apply_event(AccountingEvent {
        status_type: Some(AcctStatusType::Start),
        nas_ip_address: Some(delayed_alternate_ip),
        user_name: Some("delayed-alternate-user".to_string()),
        ..AccountingEvent::default()
    });
    let mut sparse_ip_keyed = minimal_session_event(
        AcctStatusType::Start,
        "ignored-by-test",
        "delayed-alternate-session",
    );
    sparse_ip_keyed.nas_identifier = None;
    sparse_ip_keyed.nas_ip_address = Some(delayed_alternate_ip);
    delayed_alternate_store.apply_event_with_mapping(sparse_ip_keyed, DynamicCircuitMapping::Ready);
    assert_eq!(delayed_alternate_store.len(), 2);
    let mut full_alternate_bridge = complete_event(
        AcctStatusType::InterimUpdate,
        "nas-delayed-alternate",
        "delayed-alternate-session",
        Ipv4Addr::new(198, 51, 100, 47),
    );
    full_alternate_bridge.nas_ip_address = Some(delayed_alternate_ip);
    full_alternate_bridge.user_name = Some("delayed-alternate-user".to_string());
    let delayed_alternate_key = AccountingSessionKey::NasSession {
        nas: NasIdentity::Ipv4(delayed_alternate_ip),
        acct_session_id: "delayed-alternate-session".to_string(),
    };
    delayed_alternate_store
        .apply_event_with_mapping(full_alternate_bridge, DynamicCircuitMapping::Ready);
    assert_eq!(delayed_alternate_store.len(), 1);
    assert_eq!(
        delayed_alternate_store
            .session(&delayed_alternate_key)
            .unwrap()
            .latest_event
            .framed_ip_address,
        Some(Ipv4Addr::new(198, 51, 100, 47))
    );

    let mut three_way_store = AccountingSessionStore::new();
    let three_way_ip = Ipv4Addr::new(192, 0, 2, 74);
    three_way_store.apply_event(AccountingEvent {
        status_type: Some(AcctStatusType::Start),
        nas_ip_address: Some(three_way_ip),
        user_name: Some("three-way-user".to_string()),
        ..AccountingEvent::default()
    });
    three_way_store.apply_event(minimal_session_event(
        AcctStatusType::Start,
        "nas-three-way",
        "three-way-session",
    ));
    let mut three_way_ip_keyed = minimal_session_event(
        AcctStatusType::InterimUpdate,
        "ignored-by-test",
        "three-way-session",
    );
    three_way_ip_keyed.nas_identifier = None;
    three_way_ip_keyed.nas_ip_address = Some(three_way_ip);
    three_way_store.apply_event_with_mapping(three_way_ip_keyed, DynamicCircuitMapping::Ready);
    assert_eq!(three_way_store.len(), 3);
    let mut three_way_bridge = complete_event(
        AcctStatusType::InterimUpdate,
        "nas-three-way",
        "three-way-session",
        Ipv4Addr::new(198, 51, 100, 48),
    );
    three_way_bridge.nas_ip_address = Some(three_way_ip);
    three_way_bridge.user_name = Some("three-way-user".to_string());
    let three_way_key = nas_session_key("nas-three-way", "three-way-session");
    three_way_store.apply_event_with_mapping(three_way_bridge, DynamicCircuitMapping::Ready);

    assert_eq!(three_way_store.len(), 1);
    assert_eq!(
        three_way_store
            .session(&three_way_key)
            .unwrap()
            .latest_event
            .framed_ip_address,
        Some(Ipv4Addr::new(198, 51, 100, 48))
    );

    let mut ip_pending_store = AccountingSessionStore::new();
    let pending_nas_ip = Ipv4Addr::new(192, 0, 2, 70);
    ip_pending_store.apply_event(AccountingEvent {
        status_type: Some(AcctStatusType::Start),
        nas_ip_address: Some(pending_nas_ip),
        user_name: Some("ip-only-pending-user".to_string()),
        ..AccountingEvent::default()
    });
    let mut later_with_identifier = complete_event(
        AcctStatusType::InterimUpdate,
        "nas-from-later-identifier",
        "ip-promoted-session",
        Ipv4Addr::new(198, 51, 100, 43),
    );
    later_with_identifier.nas_ip_address = Some(pending_nas_ip);
    later_with_identifier.user_name = Some("ip-only-pending-user".to_string());
    let ip_promoted_key = nas_session_key("nas-from-later-identifier", "ip-promoted-session");
    ip_pending_store.apply_event_with_mapping(later_with_identifier, DynamicCircuitMapping::Ready);
    assert_eq!(ip_pending_store.len(), 1);
    assert!(ip_pending_store.session(&ip_promoted_key).is_some());

    let mut pending_alias_store = AccountingSessionStore::new();
    let pending_alias_ip = Ipv4Addr::new(192, 0, 2, 71);
    let pending_alias_ipv6: Ipv6Addr = "2001:db8::71".parse().unwrap();
    pending_alias_store.apply_event(AccountingEvent {
        status_type: Some(AcctStatusType::Start),
        nas_ip_address: Some(pending_alias_ip),
        nas_ipv6_address: Some(pending_alias_ipv6),
        user_name: Some("pending-alias-user".to_string()),
        ..AccountingEvent::default()
    });
    let mut pending_alias_promotion = complete_event(
        AcctStatusType::InterimUpdate,
        "nas-pending-alias",
        "pending-alias-session",
        Ipv4Addr::new(198, 51, 100, 44),
    );
    pending_alias_promotion.nas_ip_address = Some(pending_alias_ip);
    pending_alias_promotion.user_name = Some("pending-alias-user".to_string());
    let pending_alias_key = nas_session_key("nas-pending-alias", "pending-alias-session");
    pending_alias_store
        .apply_event_with_mapping(pending_alias_promotion, DynamicCircuitMapping::Ready);
    pending_alias_store.apply_event(AccountingEvent {
        status_type: Some(AcctStatusType::AccountingOff),
        nas_ipv6_address: Some(pending_alias_ipv6),
        ..AccountingEvent::default()
    });
    assert_eq!(
        pending_alias_store
            .session(&pending_alias_key)
            .unwrap()
            .state,
        AccountingSessionState::Stale(NasResetStatus::AccountingOff)
    );

    let mut conflicting_unknown_nas_store = AccountingSessionStore::new();
    conflicting_unknown_nas_store.apply_event(AccountingEvent {
        status_type: Some(AcctStatusType::Start),
        acct_session_id: Some("unknown-nas-session".to_string()),
        user_name: Some("original-user".to_string()),
        ..AccountingEvent::default()
    });
    conflicting_unknown_nas_store.apply_event_with_mapping(
        complete_event(
            AcctStatusType::InterimUpdate,
            "nas-conflicting-user",
            "unknown-nas-session",
            Ipv4Addr::new(198, 51, 100, 45),
        ),
        DynamicCircuitMapping::Ready,
    );
    assert_eq!(conflicting_unknown_nas_store.len(), 2);
    assert!(
        conflicting_unknown_nas_store
            .session(&nas_session_key(
                "nas-conflicting-user",
                "unknown-nas-session"
            ))
            .is_some()
    );

    let mut ambiguous_pending_store = AccountingSessionStore::new();
    for calling_station_id in ["00:00:00:00:00:01", "00:00:00:00:00:02"] {
        ambiguous_pending_store.apply_event(AccountingEvent {
            status_type: Some(AcctStatusType::Start),
            nas_identifier: Some("nas-ambiguous-pending".to_string()),
            user_name: Some("shared-pending-user".to_string()),
            calling_station_id: Some(calling_station_id.to_string()),
            ..AccountingEvent::default()
        });
    }
    let mut ambiguous_resolution = minimal_session_event(
        AcctStatusType::InterimUpdate,
        "nas-ambiguous-pending",
        "resolved-session",
    );
    ambiguous_resolution.user_name = Some("shared-pending-user".to_string());
    ambiguous_pending_store.apply_event(ambiguous_resolution);

    assert_eq!(ambiguous_pending_store.len(), 3);
    assert!(
        ambiguous_pending_store
            .session(&nas_session_key(
                "nas-ambiguous-pending",
                "resolved-session"
            ))
            .is_some()
    );
}

#[test]
fn dynamic_circuit_sink_receives_session_lifecycle_intents() {
    let mut store = AccountingSessionStore::new();
    let mut sink = FakeDynamicCircuitSink::default();
    let key = nas_session_key("nas-command", "session-command");
    let circuit_id = key.dynamic_circuit_id().unwrap();

    store.apply_event_with_mapping_and_commands(
        complete_event(
            AcctStatusType::Start,
            "nas-command",
            "session-command",
            Ipv4Addr::new(198, 51, 100, 70),
        ),
        DynamicCircuitMapping::Ready,
        &mut sink,
    );
    assert_eq!(sink.intents.len(), 1);
    assert_eq!(sink.intents[0].circuit_id(), circuit_id);
    let DynamicCircuitIntent::CreateDynamicCircuit(create) = &sink.intents[0] else {
        panic!("expected create intent, got {:?}", sink.intents[0]);
    };
    assert_eq!(create.session_key, key);

    store.apply_event_with_mapping_and_commands(
        complete_event(
            AcctStatusType::InterimUpdate,
            "nas-command",
            "session-command",
            Ipv4Addr::new(198, 51, 100, 71),
        ),
        DynamicCircuitMapping::Ready,
        &mut sink,
    );
    assert_eq!(sink.intents.len(), 2);
    assert_eq!(sink.intents[1].circuit_id(), circuit_id);
    let DynamicCircuitIntent::UpdateDynamicCircuit(update) = &sink.intents[1] else {
        panic!("expected update intent, got {:?}", sink.intents[1]);
    };
    assert_eq!(update.session_key, key);
    assert_eq!(
        update.event.framed_ip_address,
        Some(Ipv4Addr::new(198, 51, 100, 71))
    );

    store.apply_event_with_mapping_and_commands(
        minimal_session_event(AcctStatusType::Stop, "nas-command", "session-command"),
        DynamicCircuitMapping::Ready,
        &mut sink,
    );
    assert_eq!(sink.intents.len(), 3);
    assert_eq!(sink.intents[2].circuit_id(), circuit_id);
    let DynamicCircuitIntent::RemoveDynamicCircuit(stop) = &sink.intents[2] else {
        panic!("expected stop removal intent, got {:?}", sink.intents[2]);
    };
    assert_eq!(stop.session_key, key);
    assert_eq!(stop.reason, DynamicCircuitRemovalReason::Stop);
    assert!(
        store
            .session(&key)
            .unwrap()
            .active_dynamic_circuit_ids
            .is_empty()
    );
    store.apply_event_with_mapping_and_commands(
        minimal_session_event(AcctStatusType::Stop, "nas-command", "session-command"),
        DynamicCircuitMapping::Ready,
        &mut sink,
    );
    assert_eq!(sink.intents.len(), 3);

    let mut expiry_store = AccountingSessionStore::new();
    let mut expiry_sink = FakeDynamicCircuitSink::default();
    let expiry_key = nas_session_key("nas-expiry", "session-expiry");
    let expiry_circuit_id = expiry_key.dynamic_circuit_id().unwrap();
    expiry_store.apply_event_with_mapping_and_commands(
        complete_event(
            AcctStatusType::Start,
            "nas-expiry",
            "session-expiry",
            Ipv4Addr::new(198, 51, 100, 72),
        ),
        DynamicCircuitMapping::Ready,
        &mut expiry_sink,
    );
    assert!(
        expiry_store
            .expire_session_with_commands(&expiry_key, &mut expiry_sink)
            .is_some()
    );
    assert_eq!(expiry_sink.intents.len(), 2);
    assert_eq!(expiry_sink.intents[1].circuit_id(), expiry_circuit_id);
    let DynamicCircuitIntent::RemoveDynamicCircuit(expiry) = &expiry_sink.intents[1] else {
        panic!(
            "expected expiry removal intent, got {:?}",
            expiry_sink.intents[1]
        );
    };
    assert_eq!(expiry.session_key, expiry_key);
    assert_eq!(expiry.reason, DynamicCircuitRemovalReason::Expired);

    let mut unshapeable_store = AccountingSessionStore::new();
    let mut unshapeable_sink = FakeDynamicCircuitSink::default();
    unshapeable_store.apply_event_with_mapping_and_commands(
        minimal_session_event(
            AcctStatusType::Start,
            "nas-pending-command",
            "session-pending",
        ),
        DynamicCircuitMapping::MissingParent,
        &mut unshapeable_sink,
    );
    assert!(unshapeable_sink.intents.is_empty());
    unshapeable_store.apply_event_with_mapping_and_commands(
        minimal_session_event(
            AcctStatusType::Stop,
            "nas-pending-command",
            "session-pending",
        ),
        DynamicCircuitMapping::Ready,
        &mut unshapeable_sink,
    );
    assert!(unshapeable_sink.intents.is_empty());

    let never_emitted_expiry_key = nas_session_key("nas-never-emitted", "session-expiry");
    unshapeable_store.apply_event_with_mapping_and_commands(
        minimal_session_event(AcctStatusType::Start, "nas-never-emitted", "session-expiry"),
        DynamicCircuitMapping::MissingParent,
        &mut unshapeable_sink,
    );
    assert!(
        unshapeable_store
            .expire_session_with_commands(&never_emitted_expiry_key, &mut unshapeable_sink)
            .is_some()
    );
    assert!(unshapeable_sink.intents.is_empty());
}

#[test]
fn dynamic_circuit_sink_creates_when_pending_session_becomes_shapeable() {
    let mut store = AccountingSessionStore::new();
    let mut sink = FakeDynamicCircuitSink::default();
    let key = nas_session_key("nas-late-command", "session-late-command");
    let circuit_id = key.dynamic_circuit_id().unwrap();

    store.apply_event_with_mapping_and_commands(
        minimal_session_event(
            AcctStatusType::Start,
            "nas-late-command",
            "session-late-command",
        ),
        DynamicCircuitMapping::MissingParent,
        &mut sink,
    );
    assert!(sink.intents.is_empty());

    store.apply_event_with_mapping_and_commands(
        complete_event(
            AcctStatusType::InterimUpdate,
            "nas-late-command",
            "session-late-command",
            Ipv4Addr::new(198, 51, 100, 73),
        ),
        DynamicCircuitMapping::Ready,
        &mut sink,
    );

    assert_eq!(sink.intents.len(), 1);
    assert_eq!(sink.intents[0].circuit_id(), circuit_id);
    let DynamicCircuitIntent::CreateDynamicCircuit(create) = &sink.intents[0] else {
        panic!("expected create intent, got {:?}", sink.intents[0]);
    };
    assert_eq!(create.session_key, key);
    assert_eq!(
        store.session(&key).unwrap().active_dynamic_circuit_ids,
        vec![circuit_id]
    );
}

#[test]
fn dynamic_circuit_sink_removes_when_shapeable_session_becomes_pending() {
    let mut store = AccountingSessionStore::new();
    let mut sink = FakeDynamicCircuitSink::default();
    let key = nas_session_key("nas-removal-command", "session-removal-command");
    let circuit_id = key.dynamic_circuit_id().unwrap();

    store.apply_event_with_mapping_and_commands(
        complete_event(
            AcctStatusType::Start,
            "nas-removal-command",
            "session-removal-command",
            Ipv4Addr::new(198, 51, 100, 74),
        ),
        DynamicCircuitMapping::Ready,
        &mut sink,
    );
    store.apply_event_with_mapping_and_commands(
        minimal_session_event(
            AcctStatusType::InterimUpdate,
            "nas-removal-command",
            "session-removal-command",
        ),
        DynamicCircuitMapping::MissingParent,
        &mut sink,
    );

    assert_eq!(sink.intents.len(), 2);
    assert_eq!(sink.intents[1].circuit_id(), circuit_id);
    let DynamicCircuitIntent::RemoveDynamicCircuit(removal) = &sink.intents[1] else {
        panic!(
            "expected no-longer-shapeable removal, got {:?}",
            sink.intents[1]
        );
    };
    assert_eq!(removal.session_key, key);
    assert_eq!(
        removal.reason,
        DynamicCircuitRemovalReason::NoLongerShapeable
    );
    assert!(
        store
            .session(&key)
            .unwrap()
            .active_dynamic_circuit_ids
            .is_empty()
    );
}

#[test]
fn dynamic_circuit_sink_removes_rekeyed_promoted_session_ids() {
    let mut store = AccountingSessionStore::new();
    let mut sink = FakeDynamicCircuitSink::default();
    let key = nas_session_key("nas-rekey-command", "session-rekey-command");
    let circuit_id = key.dynamic_circuit_id().unwrap();

    store.apply_event_with_mapping_and_commands(
        complete_event(
            AcctStatusType::Start,
            "nas-rekey-command",
            "session-rekey-command",
            Ipv4Addr::new(198, 51, 100, 75),
        ),
        DynamicCircuitMapping::Ready,
        &mut sink,
    );

    let alternate_nas_ip = Ipv4Addr::new(192, 0, 2, 80);
    let alternate_key = AccountingSessionKey::NasSession {
        nas: NasIdentity::Ipv4(alternate_nas_ip),
        acct_session_id: "session-rekey-command".to_string(),
    };
    let alternate_circuit_id = alternate_key.dynamic_circuit_id().unwrap();
    let mut alternate_start = complete_event(
        AcctStatusType::Start,
        "ignored-by-test",
        "session-rekey-command",
        Ipv4Addr::new(198, 51, 100, 76),
    );
    alternate_start.nas_identifier = None;
    alternate_start.nas_ip_address = Some(alternate_nas_ip);
    store.apply_event_with_mapping_and_commands(
        alternate_start,
        DynamicCircuitMapping::Ready,
        &mut sink,
    );

    let mut bridge = complete_event(
        AcctStatusType::InterimUpdate,
        "nas-rekey-command",
        "session-rekey-command",
        Ipv4Addr::new(198, 51, 100, 77),
    );
    bridge.nas_ip_address = Some(alternate_nas_ip);
    store.apply_event_with_mapping_and_commands(bridge, DynamicCircuitMapping::Ready, &mut sink);

    assert_eq!(store.len(), 1);
    assert_eq!(sink.intents.len(), 4);
    assert_eq!(sink.intents[2].circuit_id(), circuit_id);
    let DynamicCircuitIntent::UpdateDynamicCircuit(update) = &sink.intents[2] else {
        panic!("expected update intent, got {:?}", sink.intents[2]);
    };
    assert_eq!(update.session_key, key);
    assert_eq!(sink.intents[3].circuit_id(), alternate_circuit_id);
    let DynamicCircuitIntent::RemoveDynamicCircuit(removal) = &sink.intents[3] else {
        panic!("expected rekeyed removal, got {:?}", sink.intents[3]);
    };
    assert_eq!(removal.session_key, key);
    assert_eq!(removal.reason, DynamicCircuitRemovalReason::Rekeyed);
    assert_eq!(
        store.session(&key).unwrap().active_dynamic_circuit_ids,
        vec![circuit_id]
    );
}

#[test]
fn dynamic_circuit_sink_removes_active_circuits_on_nas_reset() {
    let mut store = AccountingSessionStore::new();
    let mut sink = FakeDynamicCircuitSink::default();
    let key = nas_session_key("nas-reset-command", "session-reset-command");
    let circuit_id = key.dynamic_circuit_id().unwrap();
    let pending_key = nas_session_key("nas-reset-command", "session-reset-pending");
    let unrelated_key = nas_session_key("nas-reset-other", "session-reset-other");
    let unrelated_circuit_id = unrelated_key.dynamic_circuit_id().unwrap();

    store.apply_event_with_mapping_and_commands(
        complete_event(
            AcctStatusType::Start,
            "nas-reset-command",
            "session-reset-command",
            Ipv4Addr::new(198, 51, 100, 78),
        ),
        DynamicCircuitMapping::Ready,
        &mut sink,
    );
    store.apply_event_with_mapping_and_commands(
        minimal_session_event(
            AcctStatusType::Start,
            "nas-reset-command",
            "session-reset-pending",
        ),
        DynamicCircuitMapping::MissingParent,
        &mut sink,
    );
    store.apply_event_with_mapping_and_commands(
        complete_event(
            AcctStatusType::Start,
            "nas-reset-other",
            "session-reset-other",
            Ipv4Addr::new(198, 51, 100, 79),
        ),
        DynamicCircuitMapping::Ready,
        &mut sink,
    );
    store.apply_event_with_mapping_and_commands(
        reset_event(AcctStatusType::AccountingOff, "nas-reset-command"),
        DynamicCircuitMapping::Ready,
        &mut sink,
    );

    assert_eq!(sink.intents.len(), 3);
    assert_eq!(sink.intents[2].circuit_id(), circuit_id);
    let DynamicCircuitIntent::RemoveDynamicCircuit(removal) = &sink.intents[2] else {
        panic!("expected NAS reset removal, got {:?}", sink.intents[2]);
    };
    assert_eq!(removal.session_key, key);
    assert_eq!(
        removal.reason,
        DynamicCircuitRemovalReason::NasReset(NasResetStatus::AccountingOff)
    );
    assert_eq!(
        store.session(&key).unwrap().state,
        AccountingSessionState::Stale(NasResetStatus::AccountingOff)
    );
    assert!(
        store
            .session(&key)
            .unwrap()
            .active_dynamic_circuit_ids
            .is_empty()
    );
    assert_eq!(
        store.session(&pending_key).unwrap().state,
        AccountingSessionState::Stale(NasResetStatus::AccountingOff)
    );
    assert!(
        store
            .session(&pending_key)
            .unwrap()
            .active_dynamic_circuit_ids
            .is_empty()
    );
    assert_eq!(
        store.session(&unrelated_key).unwrap().state,
        AccountingSessionState::Active
    );
    assert_eq!(
        store
            .session(&unrelated_key)
            .unwrap()
            .active_dynamic_circuit_ids,
        vec![unrelated_circuit_id]
    );
}

#[derive(Default)]
struct FakeDynamicCircuitSink {
    intents: Vec<DynamicCircuitIntent>,
}

impl DynamicCircuitCommandSink for FakeDynamicCircuitSink {
    fn emit(&mut self, intent: DynamicCircuitIntent) {
        self.intents.push(intent);
    }
}

fn complete_event(
    status_type: AcctStatusType,
    nas_identifier: &str,
    acct_session_id: &str,
    framed_ip_address: Ipv4Addr,
) -> AccountingEvent {
    AccountingEvent {
        status_type: Some(status_type),
        acct_session_id: Some(acct_session_id.to_string()),
        nas_identifier: Some(nas_identifier.to_string()),
        user_name: Some("subscriber".to_string()),
        framed_ip_address: Some(framed_ip_address),
        mikrotik_rate_limits: vec![rate_limit()],
        ..AccountingEvent::default()
    }
}

fn minimal_session_event(
    status_type: AcctStatusType,
    nas_identifier: &str,
    acct_session_id: &str,
) -> AccountingEvent {
    AccountingEvent {
        status_type: Some(status_type),
        acct_session_id: Some(acct_session_id.to_string()),
        nas_identifier: Some(nas_identifier.to_string()),
        ..AccountingEvent::default()
    }
}

fn reset_event(status_type: AcctStatusType, nas_identifier: &str) -> AccountingEvent {
    AccountingEvent {
        status_type: Some(status_type),
        nas_identifier: Some(nas_identifier.to_string()),
        ..AccountingEvent::default()
    }
}

fn nas_session_key(nas_identifier: &str, acct_session_id: &str) -> AccountingSessionKey {
    AccountingSessionKey::NasSession {
        nas: NasIdentity::Identifier(nas_identifier.to_string()),
        acct_session_id: acct_session_id.to_string(),
    }
}

fn retained_session_with_known_ip(
    nas_identifier: &str,
    acct_session_id: &str,
    nas_ip_address: Ipv4Addr,
) -> (AccountingSessionKey, AccountingSession) {
    (
        nas_session_key(nas_identifier, acct_session_id),
        AccountingSession {
            state: AccountingSessionState::Active,
            latest_event: minimal_session_event(
                AcctStatusType::Start,
                nas_identifier,
                acct_session_id,
            ),
            known_nas_identities: vec![
                NasIdentity::Identifier(nas_identifier.to_string()),
                NasIdentity::Ipv4(nas_ip_address),
            ],
            active_dynamic_circuit_ids: Vec::new(),
            pending_reasons: Vec::new(),
        },
    )
}

fn rate_limit() -> MikrotikRateLimit {
    MikrotikRateLimit {
        original: "10M/25M".to_string(),
        nas_rx_bps: 10_000_000,
        nas_tx_bps: 25_000_000,
        upload_bps: 10_000_000,
        download_bps: 25_000_000,
    }
}
