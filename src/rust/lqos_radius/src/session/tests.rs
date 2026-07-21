//! Tests for in-memory accounting session tracking.

use super::*;
use crate::{
    DynamicCircuitCommandSink, DynamicCircuitIntent, DynamicCircuitRemovalReason, Ipv6Prefix,
    MikrotikRateLimit, ShapedDevicesMacMatcher,
};
use lqos_config::ShapedDevice;
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

    let update = store.apply_event_with_mapping(start, ready_mapping());
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
    store.apply_event_with_mapping(interim, ready_mapping());

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
    sparse_store.apply_event_with_mapping(sparse_start, ready_mapping());
    sparse_store.apply_event_with_mapping(
        minimal_session_event(
            AcctStatusType::InterimUpdate,
            "nas-sparse",
            "session-sparse",
        ),
        ready_mapping(),
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
        ready_mapping(),
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
        ready_mapping(),
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
    mixed_identity_store.apply_event_with_mapping(start_by_ip, ready_mapping());

    let mut interim_with_identifier = complete_event(
        AcctStatusType::InterimUpdate,
        "nas-with-new-identifier",
        "session-mixed",
        Ipv4Addr::new(198, 51, 100, 51),
    );
    interim_with_identifier.nas_ip_address = Some(nas_ip);
    let mixed_update =
        mixed_identity_store.apply_event_with_mapping(interim_with_identifier, ready_mapping());

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
    pending_collision_store.apply_event_with_mapping(pending_collision_start, ready_mapping());
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
    pending_collision_store.apply_event_with_mapping(pending_collision_interim, ready_mapping());
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
    pending_to_keyed_store.apply_event_with_mapping(missing_nas, ready_mapping());
    let late_nas_key = nas_session_key("nas-late", "session-late-nas");
    pending_to_keyed_store.apply_event_with_mapping(
        complete_event(
            AcctStatusType::InterimUpdate,
            "nas-late",
            "session-late-nas",
            Ipv4Addr::new(198, 51, 100, 53),
        ),
        ready_mapping(),
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
        ready_mapping(),
    );
    let mut ip_only_interim = complete_event(
        AcctStatusType::InterimUpdate,
        "ignored-by-test",
        "session-split",
        Ipv4Addr::new(198, 51, 100, 55),
    );
    ip_only_interim.nas_identifier = None;
    ip_only_interim.nas_ip_address = Some(split_nas_ip);
    split_store.apply_event_with_mapping(ip_only_interim, ready_mapping());
    assert_eq!(split_store.len(), 2);

    let mut combined_identity_interim = complete_event(
        AcctStatusType::InterimUpdate,
        "nas-split",
        "session-split",
        Ipv4Addr::new(198, 51, 100, 56),
    );
    combined_identity_interim.nas_ip_address = Some(split_nas_ip);
    split_store.apply_event_with_mapping(combined_identity_interim, ready_mapping());
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
        ready_mapping(),
    );
    let alternate_key = AccountingSessionKey::NasSession {
        nas: NasIdentity::Ipv4(split_nas_ip),
        acct_session_id: "session-extra-alias".to_string(),
    };
    insert_retained_session(
        &mut split_with_extra_alias,
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
            resolved_rate: None,
            resolved_shaped_device: None,
            active_dynamic_circuit_ids: Vec::new(),
            diagnostic_circuit_ids: Vec::new(),
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
    split_with_extra_alias.apply_event_with_mapping(extra_alias_merge, ready_mapping());
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
    insert_retained_session(
        &mut rich_alternate_store,
        rich_alternate_key,
        AccountingSession {
            state: AccountingSessionState::Active,
            latest_event: rich_alternate_event,
            known_nas_identities: vec![NasIdentity::Ipv4(rich_alternate_ip)],
            resolved_rate: None,
            resolved_shaped_device: None,
            active_dynamic_circuit_ids: Vec::new(),
            diagnostic_circuit_ids: Vec::new(),
            pending_reasons: Vec::new(),
        },
    );
    let mut sparse_bridge = minimal_session_event(
        AcctStatusType::InterimUpdate,
        "nas-rich-merge",
        "session-rich-merge",
    );
    sparse_bridge.nas_ip_address = Some(rich_alternate_ip);
    rich_alternate_store.apply_event_with_mapping(sparse_bridge, ready_mapping());

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
    insert_retained_session(
        &mut ambiguous_retained_store,
        ambiguous_key_a.clone(),
        ambiguous_session_a,
    );
    insert_retained_session(
        &mut ambiguous_retained_store,
        ambiguous_key_b.clone(),
        ambiguous_session_b,
    );
    let mut ambiguous_interim = complete_event(
        AcctStatusType::InterimUpdate,
        "ignored-by-test",
        "shared-session",
        Ipv4Addr::new(198, 51, 100, 57),
    );
    ambiguous_interim.nas_identifier = None;
    ambiguous_interim.nas_ip_address = Some(ambiguous_ip);
    ambiguous_retained_store.apply_event_with_mapping(ambiguous_interim, ready_mapping());

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
        ready_mapping(),
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
    assert_eq!(session.resolved_rate, None);
    assert!(session.resolved_shaped_device.is_none());

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
        ready_mapping(),
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
    assert!(
        fallback_stop_store
            .session(&fallback_key)
            .unwrap()
            .resolved_shaped_device
            .is_none()
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
            ready_mapping(),
        );
    }

    let accounting_off = store.apply_event(reset_event(AcctStatusType::AccountingOff, "nas-a"));

    assert_eq!(
        accounting_off,
        AccountingSessionUpdate::NasSessionsMarkedStale {
            nas: NasIdentity::Identifier("nas-a".to_string()),
            reset: NasResetStatus::AccountingOff,
            marked_count: 2,
            newly_stale_session_keys: HashSet::from([
                nas_a_session_1.clone(),
                nas_a_session_2.clone(),
            ]),
            stale_session_keys: HashSet::from([nas_a_session_1.clone(), nas_a_session_2.clone()]),
        }
    );
    assert_eq!(
        store.session(&nas_a_session_1).unwrap().state,
        AccountingSessionState::Stale(NasResetStatus::AccountingOff)
    );
    assert!(
        store
            .session(&nas_a_session_1)
            .unwrap()
            .resolved_shaped_device
            .is_none()
    );
    assert_eq!(
        store.session(&nas_a_session_2).unwrap().state,
        AccountingSessionState::Stale(NasResetStatus::AccountingOff)
    );
    assert!(
        store
            .session(&nas_a_session_2)
            .unwrap()
            .resolved_shaped_device
            .is_none()
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
            newly_stale_session_keys: HashSet::from([nas_b_session.clone()]),
            stale_session_keys: HashSet::from([nas_b_session.clone()]),
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
    store.apply_event_with_mapping(nas_c_start, ready_mapping());
    store.apply_event_with_mapping(
        complete_event(
            AcctStatusType::InterimUpdate,
            "nas-c",
            "session-1",
            Ipv4Addr::new(198, 51, 100, 34),
        ),
        ready_mapping(),
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
            newly_stale_session_keys: HashSet::from([nas_c_session.clone()]),
            stale_session_keys: HashSet::from([nas_c_session.clone()]),
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
    let missing_nas_update = store.apply_event_with_mapping(missing_nas, ready_mapping());
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
    route_only.user_name = Some("subscriber".to_string());
    route_only.framed_routes = vec!["198.51.100.0/24 192.0.2.1 1".to_string()];
    route_only.mikrotik_rate_limits = vec![rate_limit()];
    let route_key = nas_session_key("nas-route", "session-route");
    store.apply_event_with_mapping(route_only, ready_mapping());
    assert_eq!(
        store.session(&route_key).unwrap().pending_reasons,
        Vec::new()
    );
    assert_eq!(
        store
            .session(&route_key)
            .unwrap()
            .resolved_shaped_device
            .as_ref()
            .unwrap()
            .ipv4,
        vec![(Ipv4Addr::new(198, 51, 100, 0), 24)]
    );

    let mut framed_ipv6_prefix_only = minimal_session_event(
        AcctStatusType::Start,
        "nas-framed-ipv6-prefix",
        "session-framed-ipv6-prefix",
    );
    framed_ipv6_prefix_only.user_name = Some("subscriber".to_string());
    framed_ipv6_prefix_only.framed_ipv6_prefixes = vec![Ipv6Prefix {
        address: "2001:db8:200::".parse().unwrap(),
        prefix_len: 56,
    }];
    framed_ipv6_prefix_only.mikrotik_rate_limits = vec![rate_limit()];
    let framed_ipv6_prefix_key =
        nas_session_key("nas-framed-ipv6-prefix", "session-framed-ipv6-prefix");
    store.apply_event_with_mapping(framed_ipv6_prefix_only, ready_mapping());
    let framed_ipv6_prefix_session = store.session(&framed_ipv6_prefix_key).unwrap();
    assert_eq!(framed_ipv6_prefix_session.pending_reasons, Vec::new());
    let framed_ipv6_prefix_device = framed_ipv6_prefix_session
        .resolved_shaped_device
        .as_ref()
        .unwrap();
    assert!(framed_ipv6_prefix_device.ipv4.is_empty());
    assert_eq!(
        framed_ipv6_prefix_device.ipv6,
        vec![("2001:db8:200::".parse().unwrap(), 56)]
    );

    let mut invalid_ipv6_only = minimal_session_event(
        AcctStatusType::Start,
        "nas-invalid-ipv6",
        "session-invalid-ipv6",
    );
    invalid_ipv6_only.user_name = Some("subscriber".to_string());
    invalid_ipv6_only.framed_ipv6_address = Some(Ipv6Addr::UNSPECIFIED);
    invalid_ipv6_only.framed_ipv6_prefixes = vec![Ipv6Prefix {
        address: "2001:db8:bad::".parse().unwrap(),
        prefix_len: 0,
    }];
    invalid_ipv6_only.delegated_ipv6_prefixes = vec![Ipv6Prefix {
        address: "ff00::".parse().unwrap(),
        prefix_len: 8,
    }];
    invalid_ipv6_only.mikrotik_rate_limits = vec![rate_limit()];
    let invalid_ipv6_key = nas_session_key("nas-invalid-ipv6", "session-invalid-ipv6");
    store.apply_event_with_mapping(invalid_ipv6_only, ready_mapping());
    let invalid_ipv6_session = store.session(&invalid_ipv6_key).unwrap();
    assert_eq!(
        invalid_ipv6_session.pending_reasons,
        vec![PendingSessionReason::MissingIpAddress]
    );
    assert!(invalid_ipv6_session.resolved_shaped_device.is_none());

    let mut default_route_only = minimal_session_event(
        AcctStatusType::Start,
        "nas-default-route",
        "session-default-route",
    );
    default_route_only.user_name = Some("subscriber".to_string());
    default_route_only.framed_routes = vec!["0.0.0.0/0 192.0.2.1 1".to_string()];
    default_route_only.mikrotik_rate_limits = vec![rate_limit()];
    let default_route_key = nas_session_key("nas-default-route", "session-default-route");
    store.apply_event_with_mapping(default_route_only, ready_mapping());
    assert_eq!(
        store.session(&default_route_key).unwrap().pending_reasons,
        vec![PendingSessionReason::MissingIpAddress]
    );

    let mut default_netmask = minimal_session_event(
        AcctStatusType::Start,
        "nas-default-netmask",
        "session-default-netmask",
    );
    default_netmask.user_name = Some("subscriber".to_string());
    default_netmask.framed_ip_address = Some(Ipv4Addr::new(198, 51, 100, 44));
    default_netmask.framed_ip_netmask = Some(Ipv4Addr::UNSPECIFIED);
    default_netmask.mikrotik_rate_limits = vec![rate_limit()];
    let default_netmask_key = nas_session_key("nas-default-netmask", "session-default-netmask");
    store.apply_event_with_mapping(default_netmask, ready_mapping());
    assert_eq!(
        store.session(&default_netmask_key).unwrap().pending_reasons,
        Vec::new()
    );
    assert_eq!(
        store
            .session(&default_netmask_key)
            .unwrap()
            .resolved_shaped_device
            .as_ref()
            .unwrap()
            .ipv4,
        vec![(Ipv4Addr::new(198, 51, 100, 44), 32)]
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
    pending_promotion_store.apply_event_with_mapping(missing_id, ready_mapping());
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
        ready_mapping(),
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
    delayed_pending_store.apply_event_with_mapping(delayed_rich_event, ready_mapping());
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
    delayed_alternate_store.apply_event_with_mapping(sparse_ip_keyed, ready_mapping());
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
    delayed_alternate_store.apply_event_with_mapping(full_alternate_bridge, ready_mapping());
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
    three_way_store.apply_event_with_mapping(three_way_ip_keyed, ready_mapping());
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
    three_way_store.apply_event_with_mapping(three_way_bridge, ready_mapping());

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
    ip_pending_store.apply_event_with_mapping(later_with_identifier, ready_mapping());
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
    pending_alias_store.apply_event_with_mapping(pending_alias_promotion, ready_mapping());
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
        ready_mapping(),
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
fn rate_resolution_prefers_packet_then_shaped_devices_then_fallback() {
    let fallback_rate = SessionRateProfile::new(5.0, 3.0, 25.0, 10.0).unwrap();
    let shaped_device_rate = SessionRateProfile::new(7.0, 4.0, 50.0, 20.0).unwrap();
    let shaped_device_and_fallback = SessionRateSources {
        shaped_device_profile: Some(shaped_device_rate),
        fallback_profile: Some(fallback_rate),
    };
    let mut no_rate_event = minimal_session_event(
        AcctStatusType::Start,
        "nas-fallback-rate",
        "session-fallback-rate",
    );
    no_rate_event.user_name = Some("subscriber".to_string());
    no_rate_event.framed_ip_address = Some(Ipv4Addr::new(198, 51, 100, 90));
    let key = nas_session_key("nas-fallback-rate", "session-fallback-rate");

    let mut no_rate_store = AccountingSessionStore::new();
    no_rate_store.apply_event_with_mapping_and_rate_sources(
        no_rate_event.clone(),
        ready_mapping(),
        SessionRateSources::default(),
    );
    let no_rate_session = no_rate_store.session(&key).unwrap();
    assert_eq!(no_rate_session.resolved_rate, None);
    assert_eq!(
        no_rate_session.pending_reasons,
        vec![PendingSessionReason::MissingRate]
    );

    let mut fallback_store = AccountingSessionStore::new();
    fallback_store.apply_event_with_mapping_and_rate_sources(
        no_rate_event.clone(),
        ready_mapping(),
        SessionRateSources {
            shaped_device_profile: None,
            fallback_profile: Some(fallback_rate),
        },
    );
    let fallback_session = fallback_store.session(&key).unwrap();
    assert_eq!(fallback_session.pending_reasons, Vec::new());
    assert_eq!(
        fallback_session.resolved_rate,
        Some(ResolvedSessionRate {
            source: SessionRateSource::Fallback,
            profile: fallback_rate,
        })
    );

    let mut shaped_device_rate_store = AccountingSessionStore::new();
    shaped_device_rate_store.apply_event_with_mapping_and_rate_sources(
        no_rate_event,
        ready_mapping(),
        shaped_device_and_fallback,
    );
    let shaped_device_session = shaped_device_rate_store.session(&key).unwrap();
    assert_eq!(shaped_device_session.pending_reasons, Vec::new());
    assert_eq!(
        shaped_device_session.resolved_rate,
        Some(ResolvedSessionRate {
            source: SessionRateSource::ShapedDevice,
            profile: shaped_device_rate,
        })
    );

    shaped_device_rate_store.apply_event_with_mapping_and_rate_sources(
        complete_event(
            AcctStatusType::InterimUpdate,
            "nas-fallback-rate",
            "session-fallback-rate",
            Ipv4Addr::new(198, 51, 100, 94),
        ),
        ready_mapping(),
        shaped_device_and_fallback,
    );
    assert_eq!(
        shaped_device_rate_store
            .session(&key)
            .unwrap()
            .resolved_rate,
        Some(ResolvedSessionRate {
            source: SessionRateSource::Packet,
            profile: SessionRateProfile::new(25.0, 10.0, 25.0, 10.0).unwrap(),
        })
    );

    let mut zero_packet_store = AccountingSessionStore::new();
    let zero_packet_key = nas_session_key("nas-zero-rate", "session-zero-rate");
    let mut zero_packet_rate = complete_event(
        AcctStatusType::Start,
        "nas-zero-rate",
        "session-zero-rate",
        Ipv4Addr::new(198, 51, 100, 92),
    );
    zero_packet_rate.mikrotik_rate_limits = vec![zero_rate_limit()];
    zero_packet_store.apply_event_with_mapping_and_rate_sources(
        zero_packet_rate,
        ready_mapping(),
        shaped_device_and_fallback,
    );
    assert_eq!(
        zero_packet_store
            .session(&zero_packet_key)
            .unwrap()
            .resolved_rate,
        Some(ResolvedSessionRate {
            source: SessionRateSource::ShapedDevice,
            profile: shaped_device_rate,
        })
    );

    let mut zero_packet_fallback_store = AccountingSessionStore::new();
    let zero_packet_fallback_key =
        nas_session_key("nas-zero-fallback-rate", "session-zero-fallback-rate");
    let mut zero_packet_fallback = complete_event(
        AcctStatusType::Start,
        "nas-zero-fallback-rate",
        "session-zero-fallback-rate",
        Ipv4Addr::new(198, 51, 100, 93),
    );
    zero_packet_fallback.mikrotik_rate_limits = vec![zero_rate_limit()];
    zero_packet_fallback_store.apply_event_with_mapping_and_rate_sources(
        zero_packet_fallback,
        ready_mapping(),
        SessionRateSources {
            shaped_device_profile: None,
            fallback_profile: Some(fallback_rate),
        },
    );
    assert_eq!(
        zero_packet_fallback_store
            .session(&zero_packet_fallback_key)
            .unwrap()
            .resolved_rate,
        Some(ResolvedSessionRate {
            source: SessionRateSource::Fallback,
            profile: fallback_rate,
        })
    );
}

#[test]
fn unique_mac_match_supplies_dynamic_circuit_metadata_with_radius_ips() {
    let matched_device = shaped_device("circuit-mac", "device-mac", "aa-bb-cc-dd-ee-ff");
    let matcher = ShapedDevicesMacMatcher::from_devices(std::slice::from_ref(&matched_device));
    let mut event = minimal_session_event(AcctStatusType::Start, "nas-mac", "session-mac");
    event.calling_station_id = Some("AABB.CCDD.EEFF".to_string());
    event.framed_ip_address = Some(Ipv4Addr::new(203, 0, 113, 10));
    event.framed_ip_netmask = Some(Ipv4Addr::new(255, 255, 255, 0));
    event.framed_ipv6_address = Some("2001:db8::10".parse().unwrap());
    event.mikrotik_rate_limits = vec![rate_limit()];
    event.delegated_ipv6_prefixes = vec![Ipv6Prefix {
        address: "2001:db8:100::".parse().unwrap(),
        prefix_len: 56,
    }];
    let resolution =
        DynamicCircuitResolution::from_shaped_devices_mac_match(matcher.match_event(&event), None);
    let mut store = AccountingSessionStore::new();
    let key = nas_session_key("nas-mac", "session-mac");

    store.apply_event_with_dynamic_circuit_resolution(event, resolution);

    let session = store.session(&key).unwrap();
    assert_eq!(session.pending_reasons, Vec::new());
    assert_eq!(
        session.resolved_rate,
        Some(ResolvedSessionRate {
            source: SessionRateSource::Packet,
            profile: SessionRateProfile::new(25.0, 10.0, 25.0, 10.0).unwrap(),
        })
    );
    let resolved_device = session.resolved_shaped_device.as_ref().unwrap();
    assert_eq!(resolved_device.circuit_id, matched_device.circuit_id);
    assert_eq!(resolved_device.circuit_name, matched_device.circuit_name);
    assert_eq!(resolved_device.device_id, matched_device.device_id);
    assert_eq!(resolved_device.device_name, matched_device.device_name);
    assert_eq!(resolved_device.parent_node, matched_device.parent_node);
    assert_eq!(
        resolved_device.parent_node_id,
        matched_device.parent_node_id
    );
    assert_eq!(
        resolved_device.anchor_node_id,
        matched_device.anchor_node_id
    );
    assert_resolved_hashes_refreshed(resolved_device);
    assert_eq!(resolved_device.sqm_override, matched_device.sqm_override);
    assert_eq!(resolved_device.download_min_mbps, 25.0);
    assert_eq!(resolved_device.upload_min_mbps, 10.0);
    assert_eq!(resolved_device.download_max_mbps, 25.0);
    assert_eq!(resolved_device.upload_max_mbps, 10.0);
    assert_eq!(
        resolved_device.ipv4,
        vec![(Ipv4Addr::new(203, 0, 113, 10), 24)]
    );
    assert_eq!(
        resolved_device.ipv6,
        vec![
            ("2001:db8::10".parse().unwrap(), 128),
            ("2001:db8:100::".parse().unwrap(), 56),
        ]
    );
    assert!(
        !resolved_device
            .ipv4
            .contains(&(Ipv4Addr::new(198, 51, 100, 200), 32))
    );

    let mut interim = complete_event(
        AcctStatusType::InterimUpdate,
        "nas-mac",
        "session-mac",
        Ipv4Addr::new(203, 0, 113, 16),
    );
    interim.calling_station_id = Some("aa-bb-cc-dd-ee-ff".to_string());
    interim.mikrotik_rate_limits = vec![MikrotikRateLimit {
        original: "12M/30M".to_string(),
        nas_rx_bps: 12_000_000,
        nas_tx_bps: 30_000_000,
        upload_bps: 12_000_000,
        download_bps: 30_000_000,
    }];
    let resolution = DynamicCircuitResolution::from_shaped_devices_mac_match(
        matcher.match_event(&interim),
        None,
    );

    store.apply_event_with_dynamic_circuit_resolution(interim, resolution);

    let refreshed_device = store
        .session(&key)
        .unwrap()
        .resolved_shaped_device
        .as_ref()
        .unwrap();
    assert_eq!(refreshed_device.circuit_id, matched_device.circuit_id);
    assert_eq!(refreshed_device.device_id, matched_device.device_id);
    assert_eq!(refreshed_device.parent_node, matched_device.parent_node);
    assert_eq!(
        refreshed_device.ipv4,
        vec![(Ipv4Addr::new(203, 0, 113, 16), 24)]
    );
    assert_eq!(refreshed_device.download_min_mbps, 30.0);
    assert_eq!(refreshed_device.upload_min_mbps, 12.0);
    assert_eq!(refreshed_device.download_max_mbps, 30.0);
    assert_eq!(refreshed_device.upload_max_mbps, 12.0);
}

#[test]
fn sparse_mac_start_promotes_when_later_update_adds_nas_identity() {
    let matched_device = shaped_device(
        "circuit-delayed-nas",
        "device-delayed-nas",
        "aa-bb-cc-dd-ee-ff",
    );
    let matcher = ShapedDevicesMacMatcher::from_devices(std::slice::from_ref(&matched_device));
    let mut store = AccountingSessionStore::new();
    let mut start = minimal_session_event(
        AcctStatusType::Start,
        "ignored-until-later",
        "session-delayed-nas",
    );
    start.nas_identifier = None;
    start.calling_station_id = Some("aa:bb:cc:dd:ee:ff".to_string());
    start.nas_port_id = Some("pppoe-port-1".to_string());

    store.apply_event_with_shaped_devices_mac_matcher(start, &matcher, None);
    assert_eq!(store.len(), 1);
    assert_eq!(
        store
            .sessions()
            .next()
            .unwrap()
            .1
            .pending_reasons
            .as_slice(),
        &[
            PendingSessionReason::MissingNasIdentity,
            PendingSessionReason::MissingIpAddress,
        ]
    );

    let mut interim = complete_event(
        AcctStatusType::InterimUpdate,
        "nas-delayed",
        "session-delayed-nas",
        Ipv4Addr::new(203, 0, 113, 17),
    );
    interim.calling_station_id = None;
    interim.nas_port_id = Some("pppoe-port-1".to_string());
    store.apply_event_with_shaped_devices_mac_matcher(interim, &matcher, None);

    let key = nas_session_key("nas-delayed", "session-delayed-nas");
    let session = store.session(&key).unwrap();
    assert_eq!(store.len(), 1);
    assert_eq!(
        session.latest_event.calling_station_id.as_deref(),
        Some("aa:bb:cc:dd:ee:ff")
    );
    assert_eq!(session.pending_reasons, Vec::new());
    let resolved_device = session.resolved_shaped_device.as_ref().unwrap();
    assert_eq!(resolved_device.circuit_id, matched_device.circuit_id);
    assert_eq!(resolved_device.device_id, matched_device.device_id);
    assert_eq!(
        resolved_device.ipv4,
        vec![(Ipv4Addr::new(203, 0, 113, 17), 32)]
    );
}

#[test]
fn session_id_without_nas_context_does_not_promote_unrelated_pending_session() {
    let matched_device = shaped_device(
        "circuit-unrelated-pending",
        "device-unrelated-pending",
        "aa-bb-cc-dd-ee-ff",
    );
    let matcher = ShapedDevicesMacMatcher::from_devices(std::slice::from_ref(&matched_device));
    let mut store = AccountingSessionStore::new();
    let mut pending = minimal_session_event(
        AcctStatusType::Start,
        "ignored-without-nas",
        "shared-session-id",
    );
    pending.nas_identifier = None;
    pending.calling_station_id = Some("aa:bb:cc:dd:ee:ff".to_string());
    pending.framed_ip_address = Some(Ipv4Addr::new(203, 0, 113, 88));
    pending.mikrotik_rate_limits = vec![rate_limit()];

    let pending_update = store.apply_event_with_shaped_devices_mac_matcher(pending, &matcher, None);
    let AccountingSessionUpdate::SessionUpdated {
        key: pending_key, ..
    } = pending_update
    else {
        panic!("expected pending session update, got {pending_update:?}");
    };

    let later = minimal_session_event(
        AcctStatusType::InterimUpdate,
        "nas-later",
        "shared-session-id",
    );
    let later = AccountingEvent {
        user_name: Some("different-subscriber".to_string()),
        ..later
    };
    let later_key = nas_session_key("nas-later", "shared-session-id");
    store.apply_event_with_mapping(later, ready_mapping());

    assert!(store.session(&pending_key).is_some());
    let later_session = store.session(&later_key).unwrap();
    assert_eq!(store.len(), 2);
    assert_eq!(
        later_session.pending_reasons,
        vec![
            PendingSessionReason::MissingIpAddress,
            PendingSessionReason::MissingRate,
        ]
    );
    assert!(later_session.resolved_shaped_device.is_none());
}

#[test]
fn unique_mac_match_uses_shaped_devices_rate_without_packet_rate() {
    let matched_device = shaped_device(
        "circuit-shaped-rate",
        "device-shaped-rate",
        "aa-bb-cc-dd-ee-ff",
    );
    let matcher = ShapedDevicesMacMatcher::from_devices(std::slice::from_ref(&matched_device));
    let mut event = minimal_session_event(
        AcctStatusType::Start,
        "nas-shaped-rate",
        "session-shaped-rate",
    );
    event.calling_station_id = Some("aa:bb:cc:dd:ee:ff".to_string());
    event.framed_ip_address = Some(Ipv4Addr::new(203, 0, 113, 11));
    event.mikrotik_rate_limits.clear();
    let resolution =
        DynamicCircuitResolution::from_shaped_devices_mac_match(matcher.match_event(&event), None);
    let mut store = AccountingSessionStore::new();
    let key = nas_session_key("nas-shaped-rate", "session-shaped-rate");

    store.apply_event_with_dynamic_circuit_resolution(event, resolution);

    let session = store.session(&key).unwrap();
    assert_eq!(session.pending_reasons, Vec::new());
    assert_eq!(
        session.resolved_rate,
        Some(ResolvedSessionRate {
            source: SessionRateSource::ShapedDevice,
            profile: SessionRateProfile::new(5.0, 2.0, 50.0, 20.0).unwrap(),
        })
    );
    let resolved_device = session.resolved_shaped_device.as_ref().unwrap();
    assert_eq!(resolved_device.circuit_id, matched_device.circuit_id);
    assert_eq!(resolved_device.device_id, matched_device.device_id);
    assert_eq!(
        resolved_device.ipv4,
        vec![(Ipv4Addr::new(203, 0, 113, 11), 32)]
    );
    assert_eq!(resolved_device.download_min_mbps, 5.0);
    assert_eq!(resolved_device.upload_min_mbps, 2.0);
    assert_eq!(resolved_device.download_max_mbps, 50.0);
    assert_eq!(resolved_device.upload_max_mbps, 20.0);
}

#[test]
fn active_mac_update_clears_resolved_shaped_device_when_match_disappears() {
    let matched_device = shaped_device(
        "circuit-cleared-match",
        "device-cleared-match",
        "aa-bb-cc-dd-ee-ff",
    );
    let matcher = ShapedDevicesMacMatcher::from_devices(std::slice::from_ref(&matched_device));
    let mut store = AccountingSessionStore::new();
    let key = nas_session_key("nas-cleared-match", "session-cleared-match");
    let mut start = complete_event(
        AcctStatusType::Start,
        "nas-cleared-match",
        "session-cleared-match",
        Ipv4Addr::new(203, 0, 113, 60),
    );
    start.calling_station_id = Some("aa:bb:cc:dd:ee:ff".to_string());

    store.apply_event_with_shaped_devices_mac_matcher(start, &matcher, None);

    assert!(
        store
            .session(&key)
            .unwrap()
            .resolved_shaped_device
            .is_some()
    );

    let mut interim = complete_event(
        AcctStatusType::InterimUpdate,
        "nas-cleared-match",
        "session-cleared-match",
        Ipv4Addr::new(203, 0, 113, 61),
    );
    interim.calling_station_id = Some("11:22:33:44:55:66".to_string());
    store.apply_event_with_shaped_devices_mac_matcher(interim, &matcher, None);

    let session = store.session(&key).unwrap();
    assert_eq!(
        session.pending_reasons,
        vec![PendingSessionReason::NoMacMatch]
    );
    assert_eq!(
        session.latest_event.calling_station_id.as_deref(),
        Some("11:22:33:44:55:66")
    );
    assert_eq!(
        session.latest_event.framed_ip_address,
        Some(Ipv4Addr::new(203, 0, 113, 61))
    );
    assert!(session.resolved_shaped_device.is_none());
}

#[test]
fn invalid_shaped_devices_rate_is_not_hidden_by_fallback_rate() {
    let fallback_rate = SessionRateProfile::new(4.0, 2.0, 40.0, 12.0).unwrap();
    for (label, matched_device) in [
        ("zero download minimum", {
            let mut device =
                shaped_device("circuit-bad-rate", "device-bad-rate", "aa-bb-cc-dd-ee-ff");
            device.download_min_mbps = 0.0;
            device
        }),
        ("non-finite upload maximum", {
            let mut device =
                shaped_device("circuit-bad-rate", "device-bad-rate", "aa-bb-cc-dd-ee-ff");
            device.upload_max_mbps = f32::INFINITY;
            device
        }),
        ("download minimum above maximum", {
            let mut device =
                shaped_device("circuit-bad-rate", "device-bad-rate", "aa-bb-cc-dd-ee-ff");
            device.download_min_mbps = device.download_max_mbps + 1.0;
            device
        }),
        ("upload minimum above maximum", {
            let mut device =
                shaped_device("circuit-bad-rate", "device-bad-rate", "aa-bb-cc-dd-ee-ff");
            device.upload_min_mbps = device.upload_max_mbps + 1.0;
            device
        }),
    ] {
        let matcher = ShapedDevicesMacMatcher::from_devices(std::slice::from_ref(&matched_device));
        let mut event =
            minimal_session_event(AcctStatusType::Start, "nas-bad-rate", "session-bad-rate");
        event.calling_station_id = Some("aa:bb:cc:dd:ee:ff".to_string());
        event.framed_ip_address = Some(Ipv4Addr::new(203, 0, 113, 12));
        let resolution = DynamicCircuitResolution::from_shaped_devices_mac_match(
            matcher.match_event(&event),
            Some(fallback_rate),
        );
        let mut store = AccountingSessionStore::new();
        let key = nas_session_key("nas-bad-rate", "session-bad-rate");

        store.apply_event_with_dynamic_circuit_resolution(event, resolution);

        let session = store.session(&key).unwrap();
        assert_eq!(session.resolved_rate, None, "{label}");
        assert_eq!(
            session.pending_reasons,
            vec![PendingSessionReason::MissingRate],
            "{label}"
        );
        assert!(session.resolved_shaped_device.is_none(), "{label}");
    }
}

#[test]
fn matched_shaped_device_without_device_id_stays_pending() {
    let mut matched_device = shaped_device(
        "circuit-missing-device",
        "device-missing-device",
        "aa-bb-cc-dd-ee-ff",
    );
    matched_device.device_id.clear();
    let matcher = ShapedDevicesMacMatcher::from_devices(std::slice::from_ref(&matched_device));
    let mut event = minimal_session_event(
        AcctStatusType::Start,
        "nas-missing-device",
        "session-missing-device",
    );
    event.calling_station_id = Some("aa:bb:cc:dd:ee:ff".to_string());
    event.framed_ip_address = Some(Ipv4Addr::new(203, 0, 113, 13));
    event.mikrotik_rate_limits = vec![rate_limit()];
    let resolution =
        DynamicCircuitResolution::from_shaped_devices_mac_match(matcher.match_event(&event), None);
    let mut store = AccountingSessionStore::new();
    let key = nas_session_key("nas-missing-device", "session-missing-device");

    store.apply_event_with_dynamic_circuit_resolution(event, resolution);

    let session = store.session(&key).unwrap();
    assert_eq!(
        session.pending_reasons,
        vec![PendingSessionReason::MissingDeviceIdentity]
    );
    assert!(session.resolved_shaped_device.is_none());
}

#[test]
fn matched_shaped_device_without_circuit_id_stays_pending() {
    let mut matched_device = shaped_device(
        "circuit-missing-circuit",
        "device-missing-circuit",
        "aa-bb-cc-dd-ee-ff",
    );
    matched_device.circuit_id.clear();
    let matcher = ShapedDevicesMacMatcher::from_devices(std::slice::from_ref(&matched_device));
    let mut event = minimal_session_event(
        AcctStatusType::Start,
        "nas-missing-circuit",
        "session-missing-circuit",
    );
    event.calling_station_id = Some("aa:bb:cc:dd:ee:ff".to_string());
    event.framed_ip_address = Some(Ipv4Addr::new(203, 0, 113, 14));
    event.mikrotik_rate_limits = vec![rate_limit()];
    let resolution =
        DynamicCircuitResolution::from_shaped_devices_mac_match(matcher.match_event(&event), None);
    let mut store = AccountingSessionStore::new();
    let key = nas_session_key("nas-missing-circuit", "session-missing-circuit");

    store.apply_event_with_dynamic_circuit_resolution(event, resolution);

    let session = store.session(&key).unwrap();
    assert_eq!(
        session.pending_reasons,
        vec![PendingSessionReason::MissingCircuitIdentity]
    );
    assert!(session.resolved_shaped_device.is_none());
}

#[test]
fn matched_shaped_device_without_parent_node_stays_pending() {
    let mut matched_device = shaped_device(
        "circuit-missing-parent",
        "device-missing-parent",
        "aa-bb-cc-dd-ee-ff",
    );
    matched_device.parent_node.clear();
    let matcher = ShapedDevicesMacMatcher::from_devices(std::slice::from_ref(&matched_device));
    let mut event = minimal_session_event(
        AcctStatusType::Start,
        "nas-missing-parent",
        "session-missing-parent",
    );
    event.calling_station_id = Some("aa:bb:cc:dd:ee:ff".to_string());
    event.framed_ip_address = Some(Ipv4Addr::new(203, 0, 113, 15));
    event.mikrotik_rate_limits = vec![rate_limit()];
    let resolution =
        DynamicCircuitResolution::from_shaped_devices_mac_match(matcher.match_event(&event), None);
    let mut store = AccountingSessionStore::new();
    let key = nas_session_key("nas-missing-parent", "session-missing-parent");

    store.apply_event_with_dynamic_circuit_resolution(event, resolution);

    let session = store.session(&key).unwrap();
    assert_eq!(
        session.pending_reasons,
        vec![PendingSessionReason::MissingParent]
    );
    assert!(session.resolved_shaped_device.is_none());
}

#[test]
fn missing_or_ambiguous_mac_match_stays_pending_without_resolved_shaped_device() {
    let fallback_rate = SessionRateProfile::new(4.0, 2.0, 40.0, 12.0).unwrap();
    let matcher = ShapedDevicesMacMatcher::from_devices(&[
        shaped_device("circuit-a", "device-a", "00:11:22:33:44:55"),
        shaped_device("circuit-b", "device-b", "0011.2233.4455"),
    ]);
    let mut store = AccountingSessionStore::new();

    let mut no_match_event = complete_event(
        AcctStatusType::Start,
        "nas-no-mac-match",
        "session-no-mac-match",
        Ipv4Addr::new(198, 51, 100, 81),
    );
    no_match_event.calling_station_id = Some("aa:bb:cc:dd:ee:ff".to_string());
    no_match_event.mikrotik_rate_limits.clear();
    let no_match_resolution = DynamicCircuitResolution::from_shaped_devices_mac_match(
        matcher.match_event(&no_match_event),
        Some(fallback_rate),
    );
    store.apply_event_with_dynamic_circuit_resolution(no_match_event, no_match_resolution);
    let no_match_session = store
        .session(&nas_session_key("nas-no-mac-match", "session-no-mac-match"))
        .unwrap();
    assert_eq!(
        no_match_session.resolved_rate,
        Some(ResolvedSessionRate {
            source: SessionRateSource::Fallback,
            profile: fallback_rate,
        })
    );
    assert_eq!(
        no_match_session.pending_reasons,
        vec![PendingSessionReason::NoMacMatch]
    );
    assert!(no_match_session.resolved_shaped_device.is_none());

    let mut ambiguous_event = complete_event(
        AcctStatusType::Start,
        "nas-ambiguous-mac",
        "session-ambiguous-mac",
        Ipv4Addr::new(198, 51, 100, 82),
    );
    ambiguous_event.calling_station_id = Some("00-11-22-33-44-55".to_string());
    ambiguous_event.mikrotik_rate_limits.clear();
    let ambiguous_resolution = DynamicCircuitResolution::from_shaped_devices_mac_match(
        matcher.match_event(&ambiguous_event),
        Some(fallback_rate),
    );
    store.apply_event_with_dynamic_circuit_resolution(ambiguous_event, ambiguous_resolution);
    let ambiguous_session = store
        .session(&nas_session_key(
            "nas-ambiguous-mac",
            "session-ambiguous-mac",
        ))
        .unwrap();
    assert_eq!(
        ambiguous_session.resolved_rate,
        Some(ResolvedSessionRate {
            source: SessionRateSource::Fallback,
            profile: fallback_rate,
        })
    );
    assert_eq!(
        ambiguous_session.pending_reasons,
        vec![PendingSessionReason::AmbiguousMacMatch]
    );
    assert!(ambiguous_session.resolved_shaped_device.is_none());
}

#[test]
fn unmatched_identity_uses_the_configured_default_parent_and_rate_profile() {
    let fallback_rate = SessionRateProfile::new(4.0, 2.0, 40.0, 12.0).unwrap();
    let resolution = DynamicCircuitResolution::from_shaped_devices_match_with_fallback_parent(
        ShapedDevicesMacMatch::NoMatch,
        Some(fallback_rate),
        Some(parent_attachment()),
    );

    assert_eq!(
        resolution.mapping,
        DynamicCircuitMapping::ReadyWithParent(parent_attachment())
    );
    assert_eq!(
        resolution.rate_sources.fallback_profile,
        Some(fallback_rate)
    );
    assert!(resolution.matched_shaped_device.is_none());
}

#[test]
fn fallback_identity_generation_builds_dynamic_shaped_device() {
    let fallback_rate = SessionRateProfile::new(4.0, 2.0, 40.0, 12.0).unwrap();
    let mut event = minimal_session_event(
        AcctStatusType::Start,
        "nas-default-identity",
        "session-default-identity",
    );
    event.user_name = Some("subscriber@example.net".to_string());
    event.calling_station_id = Some("AA-BB-CC-DD-EE-FF".to_string());
    event.framed_ip_address = Some(Ipv4Addr::new(203, 0, 113, 44));
    let key = nas_session_key("nas-default-identity", "session-default-identity");
    let circuit_id = "radius:nas-id:6e61732d64656661756c742d6964656e74697479:username:73756273637269626572406578616d706c652e6e6574";
    let resolution = DynamicCircuitResolution {
        mapping: ready_mapping(),
        rate_sources: SessionRateSources {
            shaped_device_profile: None,
            fallback_profile: Some(fallback_rate),
        },
        matched_shaped_device: None,
    };
    let mut store = AccountingSessionStore::new();

    store.apply_event_with_dynamic_circuit_resolution(event, resolution);

    let session = store.session(&key).unwrap();
    assert_eq!(session.pending_reasons, Vec::new());
    assert_eq!(
        session.resolved_rate,
        Some(ResolvedSessionRate {
            source: SessionRateSource::Fallback,
            profile: fallback_rate,
        })
    );
    let resolved_device = session.resolved_shaped_device.as_ref().unwrap();
    assert_eq!(resolved_device.circuit_id, circuit_id);
    assert_eq!(resolved_device.device_id, circuit_id);
    assert_eq!(resolved_device.circuit_name, "subscriber@example.net");
    assert_eq!(resolved_device.device_name, "AA-BB-CC-DD-EE-FF");
    assert_eq!(resolved_device.mac, "AA-BB-CC-DD-EE-FF");
    assert_eq!(resolved_device.parent_node, "Parent Node");
    assert_resolved_hashes_refreshed(resolved_device);
    assert_eq!(
        resolved_device.parent_node_id.as_deref(),
        Some("parent-node-id")
    );
    assert_eq!(
        resolved_device.anchor_node_id.as_deref(),
        Some("anchor-node-id")
    );
    assert_eq!(
        resolved_device.ipv4,
        vec![(Ipv4Addr::new(203, 0, 113, 44), 32)]
    );
    assert_eq!(resolved_device.download_min_mbps, 4.0);
    assert_eq!(resolved_device.upload_min_mbps, 2.0);
    assert_eq!(resolved_device.download_max_mbps, 40.0);
    assert_eq!(resolved_device.upload_max_mbps, 12.0);
}

#[test]
fn fallback_identity_generation_uses_display_name_fallbacks_and_ipv6_prefixes() {
    let fallback_rate = SessionRateProfile::new(4.0, 2.0, 40.0, 12.0).unwrap();
    let mut store = AccountingSessionStore::new();

    let mut calling_event = minimal_session_event(
        AcctStatusType::Start,
        "nas-fallback-calling",
        "session-fallback-calling",
    );
    calling_event.calling_station_id = Some("AA-BB-CC-DD-EE-FF".to_string());
    calling_event.user_name = None;
    calling_event.framed_ipv6_prefixes = vec![Ipv6Prefix {
        address: "2001:db8:300::".parse().unwrap(),
        prefix_len: 56,
    }];
    let calling_key = nas_session_key("nas-fallback-calling", "session-fallback-calling");
    store.apply_event_with_dynamic_circuit_resolution(
        calling_event,
        fallback_resolution(fallback_rate),
    );

    let calling_device = store
        .session(&calling_key)
        .unwrap()
        .resolved_shaped_device
        .as_ref()
        .unwrap();
    assert_eq!(calling_device.circuit_name, "AA-BB-CC-DD-EE-FF");
    assert_eq!(calling_device.device_name, "AA-BB-CC-DD-EE-FF");
    assert_eq!(calling_device.mac, "AA-BB-CC-DD-EE-FF");
    assert!(calling_device.ipv4.is_empty());
    assert_eq!(
        calling_device.ipv6,
        vec![("2001:db8:300::".parse().unwrap(), 56)]
    );

    let mut acct_event = minimal_session_event(
        AcctStatusType::Start,
        "nas-fallback-acct",
        "session-fallback-acct",
    );
    acct_event.user_name = None;
    acct_event.framed_ip_address = Some(Ipv4Addr::new(203, 0, 113, 46));
    let acct_key = nas_session_key("nas-fallback-acct", "session-fallback-acct");
    store.apply_event_with_dynamic_circuit_resolution(
        acct_event,
        fallback_resolution(fallback_rate),
    );

    let acct_session = store.session(&acct_key).unwrap();
    assert_eq!(
        acct_session.pending_reasons,
        vec![
            PendingSessionReason::MissingCircuitIdentity,
            PendingSessionReason::MissingDeviceIdentity,
        ]
    );
    assert!(acct_session.resolved_shaped_device.is_none());

    let empty_event = AccountingEvent::default();
    assert_eq!(
        default_circuit_name(&empty_event, "generated-circuit"),
        "generated-circuit"
    );
    assert_eq!(
        default_device_name(&empty_event, "generated-device"),
        "generated-device"
    );
}

#[test]
fn fallback_identity_generation_uses_nas_ip_key_variants() {
    let fallback_rate = SessionRateProfile::new(4.0, 2.0, 40.0, 12.0).unwrap();
    let mut store = AccountingSessionStore::new();

    let ipv4_key = AccountingSessionKey::NasSession {
        nas: NasIdentity::Ipv4(Ipv4Addr::new(192, 0, 2, 9)),
        acct_session_id: "s-ip4".to_string(),
    };
    store.apply_event_with_dynamic_circuit_resolution(
        AccountingEvent {
            status_type: Some(AcctStatusType::Start),
            acct_session_id: Some("s-ip4".to_string()),
            nas_ip_address: Some(Ipv4Addr::new(192, 0, 2, 9)),
            user_name: Some("ipv4-subscriber".to_string()),
            framed_ip_address: Some(Ipv4Addr::new(203, 0, 113, 47)),
            ..AccountingEvent::default()
        },
        fallback_resolution(fallback_rate),
    );
    let ipv4_device = store
        .session(&ipv4_key)
        .unwrap()
        .resolved_shaped_device
        .as_ref()
        .unwrap();
    assert_eq!(
        ipv4_device.circuit_id,
        "radius:nas-ipv4:c0000209:username:697076342d73756273637269626572"
    );
    assert_eq!(ipv4_device.device_id, ipv4_device.circuit_id);

    let ipv6_nas = "2001:db8::9".parse().unwrap();
    let ipv6_key = AccountingSessionKey::NasSession {
        nas: NasIdentity::Ipv6(ipv6_nas),
        acct_session_id: "s-ip6".to_string(),
    };
    store.apply_event_with_dynamic_circuit_resolution(
        AccountingEvent {
            status_type: Some(AcctStatusType::Start),
            acct_session_id: Some("s-ip6".to_string()),
            nas_ipv6_address: Some(ipv6_nas),
            user_name: Some("ipv6-subscriber".to_string()),
            framed_ip_address: Some(Ipv4Addr::new(203, 0, 113, 48)),
            ..AccountingEvent::default()
        },
        fallback_resolution(fallback_rate),
    );
    let ipv6_device = store
        .session(&ipv6_key)
        .unwrap()
        .resolved_shaped_device
        .as_ref()
        .unwrap();
    assert_eq!(
        ipv6_device.circuit_id,
        "radius:nas-ipv6:20010db8000000000000000000000009:username:697076362d73756273637269626572"
    );
    assert_eq!(ipv6_device.device_id, ipv6_device.circuit_id);
}

#[test]
fn dynamic_circuit_resolution_updates_when_pending_session_becomes_shapeable() {
    let mut store = AccountingSessionStore::new();
    let key = nas_session_key("nas-late-command", "session-late-command");
    let circuit_id = subscriber_circuit_id("nas-late-command");
    let mut start_without_ip = minimal_session_event(
        AcctStatusType::Start,
        "nas-late-command",
        "session-late-command",
    );
    start_without_ip.user_name = Some("subscriber".to_string());
    start_without_ip.mikrotik_rate_limits = vec![rate_limit()];

    store.apply_event_with_mapping(start_without_ip, ready_mapping());
    let pending_session = store.session(&key).unwrap();
    assert_eq!(
        pending_session.pending_reasons,
        vec![PendingSessionReason::MissingIpAddress]
    );
    assert!(pending_session.resolved_shaped_device.is_none());

    store.apply_event_with_mapping(
        complete_event(
            AcctStatusType::InterimUpdate,
            "nas-late-command",
            "session-late-command",
            Ipv4Addr::new(198, 51, 100, 73),
        ),
        ready_mapping(),
    );

    let resolved_device = store
        .session(&key)
        .unwrap()
        .resolved_shaped_device
        .as_ref()
        .unwrap();
    assert_eq!(resolved_device.circuit_id, circuit_id);
    assert_eq!(
        resolved_device.ipv4,
        vec![(Ipv4Addr::new(198, 51, 100, 73), 32)]
    );
}

#[test]
fn command_sink_receives_deferred_upsert_and_removal_intents() {
    let mut store = AccountingSessionStore::new();
    let mut sink = RecordingCommandSink::default();
    let key = nas_session_key("nas-command", "session-command");
    let mut start_without_ip =
        minimal_session_event(AcctStatusType::Start, "nas-command", "session-command");
    start_without_ip.user_name = Some("subscriber".to_string());
    start_without_ip.mikrotik_rate_limits = vec![rate_limit()];

    store.apply_event_with_mapping_and_commands(start_without_ip, ready_mapping(), &mut sink);
    assert!(sink.intents.is_empty());

    store.apply_event_with_mapping_and_commands(
        complete_event(
            AcctStatusType::InterimUpdate,
            "nas-command",
            "session-command",
            Ipv4Addr::new(198, 51, 100, 74),
        ),
        ready_mapping(),
        &mut sink,
    );

    assert_eq!(sink.intents.len(), 1);
    let DynamicCircuitIntent::CreateDynamicCircuit(create) = &sink.intents[0] else {
        panic!("expected create intent, got {:?}", sink.intents[0]);
    };
    let circuit_id = subscriber_circuit_id("nas-command");
    assert_eq!(create.circuit_id, circuit_id);
    assert_eq!(create.session_key, key);
    assert_eq!(create.shaped_device.circuit_id, circuit_id);
    assert_eq!(
        create.shaped_device.ipv4,
        vec![(Ipv4Addr::new(198, 51, 100, 74), 32)]
    );
    assert_eq!(create.shaped_device.parent_node, "Parent Node");

    let mut missing_ip_update = minimal_session_event(
        AcctStatusType::InterimUpdate,
        "nas-command",
        "session-command",
    );
    missing_ip_update.calling_station_id = Some("AA-BB-CC-DD-EE-FF".to_string());
    store.apply_event_with_dynamic_circuit_resolution_and_commands(
        missing_ip_update,
        DynamicCircuitResolution {
            mapping: DynamicCircuitMapping::NoMacMatch,
            rate_sources: SessionRateSources::default(),
            matched_shaped_device: None,
        },
        &mut sink,
    );

    assert_eq!(sink.intents.len(), 2);
    let DynamicCircuitIntent::RemoveDynamicCircuit(removal) = &sink.intents[1] else {
        panic!("expected removal intent, got {:?}", sink.intents[1]);
    };
    assert_eq!(removal.circuit_id, circuit_id);
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
fn command_sink_preserves_deferred_lifecycle_intent_boundary() {
    let mut store = AccountingSessionStore::new();
    let mut sink = RecordingCommandSink::default();
    let key = nas_session_key("nas-command-lifecycle", "session-command-lifecycle");
    let circuit_id = subscriber_circuit_id("nas-command-lifecycle");

    store.apply_event_with_mapping_and_commands(
        complete_event(
            AcctStatusType::Start,
            "nas-command-lifecycle",
            "session-command-lifecycle",
            Ipv4Addr::new(198, 51, 100, 75),
        ),
        ready_mapping(),
        &mut sink,
    );
    store.apply_event_with_mapping_and_commands(
        complete_event(
            AcctStatusType::InterimUpdate,
            "nas-command-lifecycle",
            "session-command-lifecycle",
            Ipv4Addr::new(198, 51, 100, 76),
        ),
        ready_mapping(),
        &mut sink,
    );
    store.apply_event_with_mapping_and_commands(
        minimal_session_event(
            AcctStatusType::Stop,
            "nas-command-lifecycle",
            "session-command-lifecycle",
        ),
        ready_mapping(),
        &mut sink,
    );

    assert_eq!(sink.intents.len(), 3);
    let DynamicCircuitIntent::CreateDynamicCircuit(create) = &sink.intents[0] else {
        panic!("expected create intent, got {:?}", sink.intents[0]);
    };
    assert_eq!(create.circuit_id, circuit_id);
    assert_eq!(create.session_key, key);

    let DynamicCircuitIntent::UpdateDynamicCircuit(update) = &sink.intents[1] else {
        panic!("expected update intent, got {:?}", sink.intents[1]);
    };
    assert_eq!(update.circuit_id, circuit_id);
    assert_eq!(update.session_key, key);
    assert_eq!(
        update.event.framed_ip_address,
        Some(Ipv4Addr::new(198, 51, 100, 76))
    );

    let DynamicCircuitIntent::RemoveDynamicCircuit(stop) = &sink.intents[2] else {
        panic!("expected stop removal intent, got {:?}", sink.intents[2]);
    };
    assert_eq!(stop.circuit_id, circuit_id);
    assert_eq!(stop.session_key, key);
    assert_eq!(stop.reason, DynamicCircuitRemovalReason::Stop);

    store.apply_event_with_mapping_and_commands(
        minimal_session_event(
            AcctStatusType::Stop,
            "nas-command-lifecycle",
            "session-command-lifecycle",
        ),
        ready_mapping(),
        &mut sink,
    );
    assert_eq!(sink.intents.len(), 3);
}

#[test]
fn activation_counters_track_dynamic_intents_and_expiry_separately() {
    let mut store = AccountingSessionStore::new();
    let mut sink = RecordingCommandSink::default();
    let key = nas_session_key("nas-counter", "session-counter");

    store.apply_event_with_mapping_and_commands(
        complete_event(
            AcctStatusType::Start,
            "nas-counter",
            "session-counter",
            Ipv4Addr::new(198, 51, 100, 83),
        ),
        ready_mapping(),
        &mut sink,
    );
    assert_eq!(
        store.activation_counters(),
        RadiusActivationCounters {
            create: 1,
            update: 0,
            remove: 0,
            expiry: 0,
        }
    );

    store.apply_event_with_mapping_and_commands(
        complete_event(
            AcctStatusType::InterimUpdate,
            "nas-counter",
            "session-counter",
            Ipv4Addr::new(198, 51, 100, 84),
        ),
        ready_mapping(),
        &mut sink,
    );
    assert_eq!(store.activation_counters().update, 1);

    store.apply_event_with_mapping_and_commands(
        minimal_session_event(AcctStatusType::Stop, "nas-counter", "session-counter"),
        ready_mapping(),
        &mut sink,
    );
    assert_eq!(store.activation_counters().remove, 1);

    store.apply_event_with_mapping_and_commands(
        complete_event(
            AcctStatusType::Start,
            "nas-counter",
            "session-counter",
            Ipv4Addr::new(198, 51, 100, 85),
        ),
        ready_mapping(),
        &mut sink,
    );
    assert!(
        store
            .expire_session_with_commands(&key, &mut sink)
            .is_some()
    );
    assert_eq!(
        store.activation_counters(),
        RadiusActivationCounters {
            create: 2,
            update: 1,
            remove: 2,
            expiry: 1,
        }
    );
}

#[test]
fn activation_diagnostics_distinguish_session_outcomes() {
    let mut store = AccountingSessionStore::new();
    let mut sink = RecordingCommandSink::default();
    let active_key = nas_session_key("nas-diagnostic-active", "session-active");
    let stopped_key = nas_session_key("nas-diagnostic-stopped", "session-stopped");
    let stale_key = nas_session_key("nas-diagnostic-stale", "session-stale");
    let expired_key = nas_session_key("nas-diagnostic-expired", "session-expired");

    store.apply_event_with_mapping(
        complete_event(
            AcctStatusType::Start,
            "nas-diagnostic-active",
            "session-active",
            Ipv4Addr::new(198, 51, 100, 86),
        ),
        ready_mapping(),
    );
    store.apply_event(AccountingEvent {
        status_type: Some(AcctStatusType::Start),
        nas_identifier: Some("nas-diagnostic-pending".to_string()),
        calling_station_id: Some("aa-bb-cc-dd-ee-ff".to_string()),
        ..AccountingEvent::default()
    });
    store.apply_event_with_mapping(
        complete_event(
            AcctStatusType::Start,
            "nas-diagnostic-stopped",
            "session-stopped",
            Ipv4Addr::new(198, 51, 100, 87),
        ),
        ready_mapping(),
    );
    store.apply_event(minimal_session_event(
        AcctStatusType::Stop,
        "nas-diagnostic-stopped",
        "session-stopped",
    ));
    store.apply_event_with_mapping(
        complete_event(
            AcctStatusType::Start,
            "nas-diagnostic-stale",
            "session-stale",
            Ipv4Addr::new(198, 51, 100, 88),
        ),
        ready_mapping(),
    );
    store.apply_event(reset_event(
        AcctStatusType::AccountingOff,
        "nas-diagnostic-stale",
    ));
    store.apply_event_with_mapping_and_commands(
        complete_event(
            AcctStatusType::Start,
            "nas-diagnostic-expired",
            "session-expired",
            Ipv4Addr::new(198, 51, 100, 89),
        ),
        ready_mapping(),
        &mut sink,
    );
    let expired_session = store
        .expire_session_with_commands(&expired_key, &mut sink)
        .expect("expiry should return the removed session");
    let expired_diagnostic =
        RadiusActivationDiagnostic::from_expired_session(&expired_key, &expired_session);

    let diagnostics = store.activation_diagnostics();
    assert_eq!(
        diagnostic_state(&diagnostics, &active_key),
        Some(RadiusActivationDiagnosticState::Active)
    );
    assert_eq!(
        diagnostic_state(&diagnostics, &stopped_key),
        Some(RadiusActivationDiagnosticState::Stopped)
    );
    assert_eq!(
        diagnostic_state(&diagnostics, &stale_key),
        Some(RadiusActivationDiagnosticState::Stale(
            NasResetStatus::AccountingOff
        ))
    );
    assert_eq!(
        expired_diagnostic.state,
        RadiusActivationDiagnosticState::Expired
    );
    assert_eq!(expired_diagnostic.session_key, expired_key);
    assert_eq!(
        expired_diagnostic.acct_session_id.as_deref(),
        Some("session-expired")
    );

    let pending = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.state == RadiusActivationDiagnosticState::Pending)
        .expect("pending diagnostic should be present");
    assert!(
        pending
            .pending_reasons
            .contains(&PendingSessionReason::MissingSessionId)
    );
    assert!(
        pending
            .pending_reasons
            .contains(&PendingSessionReason::MissingIpAddress)
    );
    assert!(
        pending
            .pending_reasons
            .contains(&PendingSessionReason::MissingRate)
    );
    assert!(
        pending
            .pending_reasons
            .contains(&PendingSessionReason::MissingParent)
    );
}

#[test]
fn pending_diagnostics_include_mac_match_reasons() {
    let mut store = AccountingSessionStore::new();
    let no_match_key = nas_session_key("nas-no-mac", "session-no-mac");
    let ambiguous_key = nas_session_key("nas-ambiguous-mac", "session-ambiguous-mac");

    store.apply_event_with_mapping(
        complete_event(
            AcctStatusType::Start,
            "nas-no-mac",
            "session-no-mac",
            Ipv4Addr::new(198, 51, 100, 90),
        ),
        DynamicCircuitMapping::NoMacMatch,
    );
    store.apply_event_with_mapping(
        complete_event(
            AcctStatusType::Start,
            "nas-ambiguous-mac",
            "session-ambiguous-mac",
            Ipv4Addr::new(198, 51, 100, 91),
        ),
        DynamicCircuitMapping::AmbiguousMacMatch,
    );

    let diagnostics = store.activation_diagnostics();
    let no_match = diagnostic_for(&diagnostics, &no_match_key).unwrap();
    let ambiguous = diagnostic_for(&diagnostics, &ambiguous_key).unwrap();

    assert_eq!(no_match.state, RadiusActivationDiagnosticState::Pending);
    assert!(
        no_match
            .pending_reasons
            .contains(&PendingSessionReason::NoMacMatch)
    );
    assert_eq!(ambiguous.state, RadiusActivationDiagnosticState::Pending);
    assert!(
        ambiguous
            .pending_reasons
            .contains(&PendingSessionReason::AmbiguousMacMatch)
    );
}

#[test]
fn command_sink_emits_expiry_rekey_and_stale_expiry_removals() {
    let mut expiry_store = AccountingSessionStore::new();
    let mut expiry_sink = RecordingCommandSink::default();
    let expiry_key = nas_session_key("nas-expiry-command", "session-expiry-command");
    let expiry_circuit_id = subscriber_circuit_id("nas-expiry-command");
    expiry_store.apply_event_with_mapping_and_commands(
        complete_event(
            AcctStatusType::Start,
            "nas-expiry-command",
            "session-expiry-command",
            Ipv4Addr::new(198, 51, 100, 77),
        ),
        ready_mapping(),
        &mut expiry_sink,
    );
    assert!(
        expiry_store
            .expire_session_with_commands(&expiry_key, &mut expiry_sink)
            .is_some()
    );
    assert_eq!(expiry_sink.intents.len(), 2);
    let DynamicCircuitIntent::RemoveDynamicCircuit(expiry) = &expiry_sink.intents[1] else {
        panic!(
            "expected expiry removal intent, got {:?}",
            expiry_sink.intents[1]
        );
    };
    assert_eq!(expiry.circuit_id, expiry_circuit_id);
    assert_eq!(expiry.session_key, expiry_key);
    assert_eq!(expiry.reason, DynamicCircuitRemovalReason::Expired);

    let mut rekey_store = AccountingSessionStore::new();
    let mut rekey_sink = RecordingCommandSink::default();
    let rekey_key = nas_session_key("nas-rekey-command", "session-rekey-command");
    let rekey_circuit_id = subscriber_circuit_id("nas-rekey-command");
    rekey_store.apply_event_with_mapping_and_commands(
        complete_event(
            AcctStatusType::Start,
            "nas-rekey-command",
            "session-rekey-command",
            Ipv4Addr::new(198, 51, 100, 78),
        ),
        ready_mapping(),
        &mut rekey_sink,
    );
    let alternate_nas_ip = Ipv4Addr::new(192, 0, 2, 80);
    let alternate_key = AccountingSessionKey::NasSession {
        nas: NasIdentity::Ipv4(alternate_nas_ip),
        acct_session_id: "session-rekey-command".to_string(),
    };
    let alternate_circuit_id = stable_subscriber_circuit_id(
        &alternate_key,
        &AccountingEvent {
            user_name: Some("subscriber".to_string()),
            ..AccountingEvent::default()
        },
    )
    .unwrap();
    let mut alternate_start = complete_event(
        AcctStatusType::Start,
        "ignored-by-test",
        "session-rekey-command",
        Ipv4Addr::new(198, 51, 100, 79),
    );
    alternate_start.nas_identifier = None;
    alternate_start.nas_ip_address = Some(alternate_nas_ip);
    rekey_store.apply_event_with_mapping_and_commands(
        alternate_start,
        ready_mapping(),
        &mut rekey_sink,
    );
    let mut bridge = complete_event(
        AcctStatusType::InterimUpdate,
        "nas-rekey-command",
        "session-rekey-command",
        Ipv4Addr::new(198, 51, 100, 80),
    );
    bridge.nas_ip_address = Some(alternate_nas_ip);
    rekey_store.apply_event_with_mapping_and_commands(bridge, ready_mapping(), &mut rekey_sink);

    assert_eq!(rekey_sink.intents.len(), 4);
    let DynamicCircuitIntent::UpdateDynamicCircuit(update) = &rekey_sink.intents[2] else {
        panic!(
            "expected rekey update intent, got {:?}",
            rekey_sink.intents[2]
        );
    };
    assert_eq!(update.circuit_id, rekey_circuit_id);
    assert_eq!(update.session_key, rekey_key);
    let DynamicCircuitIntent::RemoveDynamicCircuit(rekeyed) = &rekey_sink.intents[3] else {
        panic!(
            "expected rekey removal intent, got {:?}",
            rekey_sink.intents[3]
        );
    };
    assert_eq!(rekeyed.circuit_id, alternate_circuit_id);
    assert_eq!(rekeyed.session_key, rekey_key);
    assert_eq!(rekeyed.reason, DynamicCircuitRemovalReason::Rekeyed);

    let mut reset_store = AccountingSessionStore::new();
    let mut reset_sink = RecordingCommandSink::default();
    let reset_key = nas_session_key("nas-reset-command", "session-reset-command");
    let reset_circuit_id = subscriber_circuit_id("nas-reset-command");
    let unrelated_key = nas_session_key("nas-reset-other", "session-reset-other");
    let unrelated_circuit_id = subscriber_circuit_id("nas-reset-other");
    reset_store.apply_event_with_mapping_and_commands(
        complete_event(
            AcctStatusType::Start,
            "nas-reset-command",
            "session-reset-command",
            Ipv4Addr::new(198, 51, 100, 81),
        ),
        ready_mapping(),
        &mut reset_sink,
    );
    reset_store.apply_event_with_mapping_and_commands(
        minimal_session_event(
            AcctStatusType::Start,
            "nas-reset-command",
            "session-reset-pending",
        ),
        ready_mapping(),
        &mut reset_sink,
    );
    reset_store.apply_event_with_mapping_and_commands(
        complete_event(
            AcctStatusType::Start,
            "nas-reset-other",
            "session-reset-other",
            Ipv4Addr::new(198, 51, 100, 82),
        ),
        ready_mapping(),
        &mut reset_sink,
    );
    reset_store.apply_event_with_mapping_and_commands(
        reset_event(AcctStatusType::AccountingOff, "nas-reset-command"),
        ready_mapping(),
        &mut reset_sink,
    );

    assert_eq!(reset_sink.intents.len(), 2);
    assert_eq!(
        reset_store
            .session(&reset_key)
            .unwrap()
            .active_dynamic_circuit_ids,
        vec![reset_circuit_id.to_string()]
    );
    assert!(
        reset_store
            .expire_session_with_commands(&reset_key, &mut reset_sink)
            .is_some()
    );
    assert_eq!(reset_sink.intents.len(), 3);
    let DynamicCircuitIntent::RemoveDynamicCircuit(reset) = &reset_sink.intents[2] else {
        panic!(
            "expected stale expiry removal intent, got {:?}",
            reset_sink.intents[2]
        );
    };
    assert_eq!(reset.circuit_id, reset_circuit_id);
    assert_eq!(reset.session_key, reset_key);
    assert_eq!(
        reset.reason,
        DynamicCircuitRemovalReason::NasReset(NasResetStatus::AccountingOff)
    );
    assert_eq!(
        reset_store
            .session(&unrelated_key)
            .unwrap()
            .active_dynamic_circuit_ids,
        vec![unrelated_circuit_id]
    );
}

#[test]
fn command_sink_uses_stable_fallback_identity_payload_when_parent_metadata_exists() {
    let mut store = AccountingSessionStore::new();
    let mut sink = RecordingCommandSink::default();
    let fallback_rate = SessionRateProfile::new(4.0, 2.0, 40.0, 12.0).unwrap();
    let mut event = minimal_session_event(
        AcctStatusType::Start,
        "nas-fallback-command",
        "session-fallback-command",
    );
    event.user_name = Some("subscriber@example.net".to_string());
    event.calling_station_id = Some("AA-BB-CC-DD-EE-FF".to_string());
    event.framed_ip_address = Some(Ipv4Addr::new(203, 0, 113, 49));
    let key = nas_session_key("nas-fallback-command", "session-fallback-command");

    store.apply_event_with_dynamic_circuit_resolution_and_commands(
        event,
        fallback_resolution(fallback_rate),
        &mut sink,
    );

    assert_eq!(sink.intents.len(), 1);
    let DynamicCircuitIntent::CreateDynamicCircuit(create) = &sink.intents[0] else {
        panic!("expected fallback create intent, got {:?}", sink.intents[0]);
    };
    assert_eq!(create.session_key, key);
    assert_eq!(
        create.shaped_device.circuit_id,
        "radius:nas-id:6e61732d66616c6c6261636b2d636f6d6d616e64:username:73756273637269626572406578616d706c652e6e6574"
    );
    assert_eq!(create.shaped_device.circuit_name, "subscriber@example.net");
    assert_eq!(create.shaped_device.device_name, "AA-BB-CC-DD-EE-FF");
    assert_eq!(create.shaped_device.parent_node, "Parent Node");
    assert_eq!(create.shaped_device.download_min_mbps, 4.0);
    assert_eq!(create.shaped_device.upload_min_mbps, 2.0);
    assert_eq!(create.shaped_device.download_max_mbps, 40.0);
    assert_eq!(create.shaped_device.upload_max_mbps, 12.0);
    let first_circuit_id = create.circuit_id.clone();

    let mut reconnect = minimal_session_event(
        AcctStatusType::Start,
        "nas-fallback-command",
        "session-fallback-reconnect",
    );
    reconnect.user_name = Some("subscriber@example.net".to_string());
    reconnect.calling_station_id = Some("AA:BB:CC:DD:EE:FF".to_string());
    reconnect.framed_ip_address = Some(Ipv4Addr::new(203, 0, 113, 50));
    store.apply_event_with_dynamic_circuit_resolution_and_commands(
        reconnect,
        fallback_resolution(fallback_rate),
        &mut sink,
    );

    let DynamicCircuitIntent::CreateDynamicCircuit(reconnected) = &sink.intents[1] else {
        panic!(
            "expected reconnect fallback create intent, got {:?}",
            sink.intents[1]
        );
    };
    assert_eq!(reconnected.circuit_id, first_circuit_id);

    let mut other_nas = minimal_session_event(
        AcctStatusType::Start,
        "nas-fallback-command-other",
        "session-fallback-other-nas",
    );
    other_nas.user_name = Some("subscriber@example.net".to_string());
    other_nas.framed_ip_address = Some(Ipv4Addr::new(203, 0, 113, 51));
    store.apply_event_with_dynamic_circuit_resolution_and_commands(
        other_nas,
        fallback_resolution(fallback_rate),
        &mut sink,
    );

    let DynamicCircuitIntent::CreateDynamicCircuit(other_nas) = &sink.intents[2] else {
        panic!("expected other NAS fallback create intent");
    };
    assert_ne!(other_nas.circuit_id, first_circuit_id);
}

#[test]
fn calling_station_identity_is_stable_across_sessions_and_scoped_to_the_nas() {
    let fallback_rate = SessionRateProfile::new(4.0, 2.0, 40.0, 12.0).unwrap();
    let mut store = AccountingSessionStore::new();
    let mut sink = RecordingCommandSink::default();

    for (nas, session, calling_station_id, address) in [
        (
            "nas-calling",
            "calling-session-one",
            "AA-BB-CC-DD-EE-FF",
            51,
        ),
        ("nas-calling", "calling-session-two", "aabb.ccdd.eeff", 52),
        (
            "nas-calling-other",
            "calling-session-three",
            "AA:BB:CC:DD:EE:FF",
            53,
        ),
    ] {
        let mut event = minimal_session_event(AcctStatusType::Start, nas, session);
        event.user_name = None;
        event.calling_station_id = Some(calling_station_id.to_string());
        event.framed_ip_address = Some(Ipv4Addr::new(203, 0, 113, address));
        store.apply_event_with_dynamic_circuit_resolution_and_commands(
            event,
            fallback_resolution(fallback_rate),
            &mut sink,
        );
    }

    let DynamicCircuitIntent::CreateDynamicCircuit(first) = &sink.intents[0] else {
        panic!("expected first Calling-Station-Id create intent");
    };
    let DynamicCircuitIntent::CreateDynamicCircuit(reconnected) = &sink.intents[1] else {
        panic!("expected reconnect Calling-Station-Id create intent");
    };
    let DynamicCircuitIntent::CreateDynamicCircuit(other_nas) = &sink.intents[2] else {
        panic!("expected other NAS Calling-Station-Id create intent");
    };
    assert_eq!(first.circuit_id, reconnected.circuit_id);
    assert_ne!(first.circuit_id, other_nas.circuit_id);
}

#[test]
fn username_identity_preserves_radius_whitespace() {
    let key = nas_session_key("nas-trimmed-username", "trimmed-username-session");
    let canonical = AccountingEvent {
        user_name: Some("subscriber@example.net".to_string()),
        ..AccountingEvent::default()
    };
    let padded = AccountingEvent {
        user_name: Some(" subscriber@example.net ".to_string()),
        ..AccountingEvent::default()
    };

    assert_ne!(
        stable_subscriber_circuit_id(&key, &canonical),
        stable_subscriber_circuit_id(&key, &padded)
    );
}

#[test]
fn username_in_mac_field_creates_the_shaped_devices_dynamic_circuit() {
    let device = shaped_device("username-circuit", "username-device", "pppoe-known");
    let matcher = ShapedDevicesMacMatcher::from_devices(&[device]);
    let mut store = AccountingSessionStore::new();
    let mut sink = RecordingCommandSink::default();
    let mut event = complete_event(
        AcctStatusType::Start,
        "nas-username",
        "session-username",
        Ipv4Addr::new(198, 51, 100, 91),
    );
    event.user_name = Some("pppoe-known".to_string());
    event.calling_station_id = None;
    event.mikrotik_rate_limits.clear();

    store.apply_event_with_shaped_devices_matcher_and_commands(
        event,
        &matcher,
        ShapedDevicesMatchOptions {
            match_by_username: true,
            match_by_mac: false,
            fallback_profile: None,
            fallback_parent: None,
        },
        &mut sink,
    );

    assert_eq!(sink.intents.len(), 1);
    let DynamicCircuitIntent::CreateDynamicCircuit(create) = &sink.intents[0] else {
        panic!("username-only match should create a dynamic circuit");
    };
    assert_eq!(create.circuit_id, "username-circuit");
    assert_eq!(create.shaped_device.device_id, "username-device");
}

#[test]
fn duplicate_username_rows_remain_pending_with_an_identity_diagnostic() {
    let first = shaped_device("first-circuit", "first-device", "duplicate-user");
    let second = shaped_device("second-circuit", "second-device", "duplicate-user");
    let matcher = ShapedDevicesMacMatcher::from_devices(&[first, second]);
    let mut store = AccountingSessionStore::new();
    let mut sink = RecordingCommandSink::default();
    let mut event = complete_event(
        AcctStatusType::Start,
        "nas-duplicate-username",
        "session-duplicate-username",
        Ipv4Addr::new(198, 51, 100, 92),
    );
    event.user_name = Some("duplicate-user".to_string());
    event.calling_station_id = None;
    event.mikrotik_rate_limits.clear();

    store.apply_event_with_shaped_devices_matcher_and_commands(
        event,
        &matcher,
        ShapedDevicesMatchOptions {
            match_by_username: true,
            match_by_mac: false,
            fallback_profile: Some(SessionRateProfile::new(4.0, 2.0, 40.0, 12.0).unwrap()),
            fallback_parent: Some(parent_attachment()),
        },
        &mut sink,
    );

    assert!(sink.intents.is_empty());
    let session = store
        .session(&nas_session_key(
            "nas-duplicate-username",
            "session-duplicate-username",
        ))
        .expect("duplicate username session should be retained for diagnostics");
    assert_eq!(
        session.pending_reasons,
        vec![PendingSessionReason::AmbiguousIdentityMatch]
    );
}

#[test]
fn session_lookup_indexes_track_promotion_stop_reset_and_expiry() {
    let mut store = AccountingSessionStore::new();
    let mut pending = AccountingEvent {
        status_type: Some(AcctStatusType::Start),
        acct_session_id: Some("session-index".to_string()),
        user_name: Some("index-user".to_string()),
        calling_station_id: Some("AA-BB-CC-DD-EE-FF".to_string()),
        nas_port_id: Some("port-index".to_string()),
        ..AccountingEvent::default()
    };
    let pending_update = store.apply_event_with_mapping(pending.clone(), ready_mapping());
    let AccountingSessionUpdate::SessionUpdated {
        key: pending_key, ..
    } = pending_update
    else {
        panic!("expected pending session update, got {pending_update:?}");
    };
    assert_pending_index_contains(
        &store,
        SessionLookupIndexKey::AcctSessionId("session-index".to_string()),
        &pending_key,
    );
    assert_pending_index_contains(
        &store,
        SessionLookupIndexKey::UserName("index-user".to_string()),
        &pending_key,
    );

    pending.status_type = Some(AcctStatusType::InterimUpdate);
    pending.nas_identifier = Some("nas-index".to_string());
    pending.framed_ip_address = Some(Ipv4Addr::new(203, 0, 113, 50));
    pending.mikrotik_rate_limits = vec![rate_limit()];
    let promoted_key = nas_session_key("nas-index", "session-index");
    store.apply_event_with_mapping(pending, ready_mapping());

    assert_no_index_references(&store, &pending_key);
    assert_nas_session_index_contains(&store, "session-index", &promoted_key);
    assert_fallback_index_contains(
        &store,
        SessionLookupIndexKey::AcctSessionId("session-index".to_string()),
        &promoted_key,
    );
    assert_fallback_index_contains(
        &store,
        SessionLookupIndexKey::UserName("index-user".to_string()),
        &promoted_key,
    );

    store.apply_event_with_mapping(
        minimal_session_event(AcctStatusType::Stop, "nas-index", "session-index"),
        ready_mapping(),
    );
    assert_nas_session_index_contains(&store, "session-index", &promoted_key);
    assert_fallback_index_contains(
        &store,
        SessionLookupIndexKey::AcctSessionId("session-index".to_string()),
        &promoted_key,
    );
    assert_fallback_index_not_contains(
        &store,
        SessionLookupIndexKey::CallingStationId("AA-BB-CC-DD-EE-FF".to_string()),
        &promoted_key,
    );
    assert_fallback_index_not_contains(
        &store,
        SessionLookupIndexKey::UserName("index-user".to_string()),
        &promoted_key,
    );

    let mut reset_store = AccountingSessionStore::new();
    let reset_key = nas_session_key("nas-index-reset", "session-index-reset");
    reset_store.apply_event_with_mapping(
        complete_event(
            AcctStatusType::Start,
            "nas-index-reset",
            "session-index-reset",
            Ipv4Addr::new(203, 0, 113, 51),
        ),
        ready_mapping(),
    );
    reset_store.apply_event(reset_event(
        AcctStatusType::AccountingOff,
        "nas-index-reset",
    ));
    assert_nas_session_index_contains(&reset_store, "session-index-reset", &reset_key);
    assert_fallback_index_contains(
        &reset_store,
        SessionLookupIndexKey::AcctSessionId("session-index-reset".to_string()),
        &reset_key,
    );
    let mut sink = RecordingCommandSink::default();
    reset_store.expire_session_with_commands(&reset_key, &mut sink);
    assert_no_index_references(&reset_store, &reset_key);

    store.expire_session_with_commands(&promoted_key, &mut sink);
    assert_no_index_references(&store, &promoted_key);
}

#[test]
fn lookup_indexes_do_not_return_nas_only_candidate_sets() {
    let mut store = AccountingSessionStore::new();
    let pending_key = AccountingSessionKey::Pending {
        fingerprint: PendingSessionFingerprint {
            nas: Some(NasIdentity::Identifier("shared-nas".to_string())),
            acct_session_id: None,
            user_name: Some("pending-user".to_string()),
            calling_station_id: None,
            nas_port_id: None,
            nas_port: None,
        },
    };
    insert_retained_session(
        &mut store,
        pending_key.clone(),
        AccountingSession {
            state: AccountingSessionState::Active,
            latest_event: AccountingEvent {
                status_type: Some(AcctStatusType::Start),
                nas_identifier: Some("shared-nas".to_string()),
                user_name: Some("pending-user".to_string()),
                ..AccountingEvent::default()
            },
            known_nas_identities: vec![NasIdentity::Identifier("shared-nas".to_string())],
            resolved_rate: None,
            resolved_shaped_device: None,
            active_dynamic_circuit_ids: Vec::new(),
            diagnostic_circuit_ids: Vec::new(),
            pending_reasons: Vec::new(),
        },
    );
    let fallback_key = nas_session_key("shared-nas", "fallback-session");
    let (_, fallback_session) = retained_session_with_known_ip(
        "shared-nas",
        "fallback-session",
        Ipv4Addr::new(192, 0, 2, 50),
    );
    insert_retained_session(&mut store, fallback_key.clone(), fallback_session);

    let same_nas_only = AccountingEvent {
        nas_identifier: Some("shared-nas".to_string()),
        ..AccountingEvent::default()
    };
    let identities = NasIdentitySet::from_event(&same_nas_only);
    assert!(
        store
            .lookup_candidates(&store.pending_keys_by_lookup, &same_nas_only, &identities)
            .is_empty(),
        "same-NAS-only pending lookup should not return every session for the NAS"
    );
    assert!(
        store
            .lookup_candidates(&store.fallback_keys_by_lookup, &same_nas_only, &identities)
            .is_empty(),
        "same-NAS-only fallback lookup should not return every session for the NAS"
    );

    let selective_pending = AccountingEvent {
        nas_identifier: Some("shared-nas".to_string()),
        user_name: Some("pending-user".to_string()),
        ..AccountingEvent::default()
    };
    let identities = NasIdentitySet::from_event(&selective_pending);
    assert!(
        store
            .lookup_candidates(
                &store.pending_keys_by_lookup,
                &selective_pending,
                &identities
            )
            .contains(&pending_key),
        "selective pending lookup keys should still find matching sessions"
    );

    let selective_fallback = AccountingEvent {
        nas_identifier: Some("shared-nas".to_string()),
        acct_session_id: Some("fallback-session".to_string()),
        ..AccountingEvent::default()
    };
    let identities = NasIdentitySet::from_event(&selective_fallback);
    assert!(
        store
            .lookup_candidates(
                &store.fallback_keys_by_lookup,
                &selective_fallback,
                &identities
            )
            .contains(&fallback_key),
        "selective fallback lookup keys should still find matching sessions"
    );
}

#[derive(Default)]
struct RecordingCommandSink {
    intents: Vec<DynamicCircuitIntent>,
}

impl DynamicCircuitCommandSink for RecordingCommandSink {
    fn emit(&mut self, intent: DynamicCircuitIntent) {
        self.intents.push(intent);
    }
}

fn diagnostic_for<'a>(
    diagnostics: &'a [RadiusActivationDiagnostic],
    key: &AccountingSessionKey,
) -> Option<&'a RadiusActivationDiagnostic> {
    diagnostics
        .iter()
        .find(|diagnostic| diagnostic.session_key == *key)
}

fn diagnostic_state(
    diagnostics: &[RadiusActivationDiagnostic],
    key: &AccountingSessionKey,
) -> Option<RadiusActivationDiagnosticState> {
    diagnostic_for(diagnostics, key).map(|diagnostic| diagnostic.state)
}

fn ready_mapping() -> DynamicCircuitMapping {
    DynamicCircuitMapping::ReadyWithParent(parent_attachment())
}

fn fallback_resolution(fallback_rate: SessionRateProfile) -> DynamicCircuitResolution {
    DynamicCircuitResolution {
        mapping: ready_mapping(),
        rate_sources: SessionRateSources {
            shaped_device_profile: None,
            fallback_profile: Some(fallback_rate),
        },
        matched_shaped_device: None,
    }
}

fn parent_attachment() -> DynamicCircuitParent {
    DynamicCircuitParent {
        parent_node: "Parent Node".to_string(),
        parent_node_id: Some("parent-node-id".to_string()),
        anchor_node_id: Some("anchor-node-id".to_string()),
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

fn subscriber_circuit_id(nas_identifier: &str) -> String {
    let key = nas_session_key(nas_identifier, "subscriber-id-fixture");
    let event = AccountingEvent {
        user_name: Some("subscriber".to_string()),
        ..AccountingEvent::default()
    };
    stable_subscriber_circuit_id(&key, &event).expect("keyed subscriber should have a circuit ID")
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
            resolved_rate: None,
            resolved_shaped_device: None,
            active_dynamic_circuit_ids: Vec::new(),
            diagnostic_circuit_ids: Vec::new(),
            pending_reasons: Vec::new(),
        },
    )
}

fn insert_retained_session(
    store: &mut AccountingSessionStore,
    key: AccountingSessionKey,
    session: AccountingSession,
) {
    store.sessions.insert(key.clone(), session);
    store.index_session(&key);
}

fn assert_nas_session_index_contains(
    store: &AccountingSessionStore,
    acct_session_id: &str,
    key: &AccountingSessionKey,
) {
    assert!(
        store
            .nas_session_keys_by_acct_session_id
            .get(acct_session_id)
            .is_some_and(|keys| keys.contains(key)),
        "expected NAS session index for {acct_session_id:?} to contain {key:?}"
    );
}

fn assert_pending_index_contains(
    store: &AccountingSessionStore,
    lookup_key: SessionLookupIndexKey,
    key: &AccountingSessionKey,
) {
    assert_index_contains(&store.pending_keys_by_lookup, lookup_key, key);
}

fn assert_fallback_index_contains(
    store: &AccountingSessionStore,
    lookup_key: SessionLookupIndexKey,
    key: &AccountingSessionKey,
) {
    assert_index_contains(&store.fallback_keys_by_lookup, lookup_key, key);
}

fn assert_fallback_index_not_contains(
    store: &AccountingSessionStore,
    lookup_key: SessionLookupIndexKey,
    key: &AccountingSessionKey,
) {
    assert_index_not_contains(&store.fallback_keys_by_lookup, lookup_key, key);
}

fn assert_index_contains(
    index: &std::collections::HashMap<
        SessionLookupIndexKey,
        std::collections::HashSet<AccountingSessionKey>,
    >,
    lookup_key: SessionLookupIndexKey,
    key: &AccountingSessionKey,
) {
    assert!(
        index
            .get(&lookup_key)
            .is_some_and(|keys| keys.contains(key)),
        "expected lookup index {lookup_key:?} to contain {key:?}"
    );
}

fn assert_index_not_contains(
    index: &std::collections::HashMap<
        SessionLookupIndexKey,
        std::collections::HashSet<AccountingSessionKey>,
    >,
    lookup_key: SessionLookupIndexKey,
    key: &AccountingSessionKey,
) {
    assert!(
        !index
            .get(&lookup_key)
            .is_some_and(|keys| keys.contains(key)),
        "expected lookup index {lookup_key:?} not to contain {key:?}"
    );
}

fn assert_no_index_references(store: &AccountingSessionStore, key: &AccountingSessionKey) {
    assert!(
        !store
            .nas_session_keys_by_acct_session_id
            .values()
            .any(|keys| keys.contains(key)),
        "expected NAS session index not to contain {key:?}"
    );
    assert!(
        !store
            .pending_keys_by_lookup
            .values()
            .any(|keys| keys.contains(key)),
        "expected pending lookup index not to contain {key:?}"
    );
    assert!(
        !store
            .fallback_keys_by_lookup
            .values()
            .any(|keys| keys.contains(key)),
        "expected fallback lookup index not to contain {key:?}"
    );
}

fn shaped_device(circuit_id: &str, device_id: &str, mac: &str) -> ShapedDevice {
    ShapedDevice {
        circuit_id: circuit_id.to_string(),
        circuit_name: format!("Circuit {circuit_id}"),
        device_id: device_id.to_string(),
        device_name: format!("Device {device_id}"),
        parent_node: "Parent Node".to_string(),
        parent_node_id: Some("parent-node-id".to_string()),
        anchor_node_id: Some("anchor-node-id".to_string()),
        mac: mac.to_string(),
        ipv4: vec![(Ipv4Addr::new(198, 51, 100, 200), 32)],
        ipv6: vec![("2001:db8:ffff::1".parse().unwrap(), 128)],
        download_min_mbps: 5.0,
        upload_min_mbps: 2.0,
        download_max_mbps: 50.0,
        upload_max_mbps: 20.0,
        comment: "matched from shaped devices".to_string(),
        sqm_override: Some("cake/none".to_string()),
        circuit_hash: 0,
        device_hash: 0,
        parent_hash: 0,
    }
}

fn assert_resolved_hashes_refreshed(device: &ShapedDevice) {
    let mut expected = device.clone();
    expected.circuit_hash = 0;
    expected.device_hash = 0;
    expected.parent_hash = 0;
    expected.refresh_hashes();
    assert_eq!(device.circuit_hash, expected.circuit_hash);
    assert_eq!(device.device_hash, expected.device_hash);
    assert_eq!(device.parent_hash, expected.parent_hash);
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

fn zero_rate_limit() -> MikrotikRateLimit {
    MikrotikRateLimit {
        original: "0/0".to_string(),
        nas_rx_bps: 0,
        nas_tx_bps: 0,
        upload_bps: 0,
        download_bps: 0,
    }
}
