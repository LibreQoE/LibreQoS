//! In-memory RADIUS accounting session tracking.

use crate::{AccountingEvent, AcctStatusType};
use crate::{
    DynamicCircuitCommandSink, DynamicCircuitIntent, DynamicCircuitRemoval,
    DynamicCircuitRemovalReason, DynamicCircuitUpsert,
};
use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr};

/// In-memory session store for accepted RADIUS accounting events.
#[derive(Debug, Default)]
pub struct AccountingSessionStore {
    sessions: HashMap<AccountingSessionKey, AccountingSession>,
}

impl AccountingSessionStore {
    /// Creates an empty session store.
    ///
    /// Side effects: none. Session state is kept in memory only.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    /// Returns the number of retained sessions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Returns true when the store has no retained sessions.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Iterates over retained sessions and their keys.
    pub fn sessions(&self) -> impl Iterator<Item = (&AccountingSessionKey, &AccountingSession)> {
        self.sessions.iter()
    }

    /// Returns one retained session by key.
    #[must_use]
    pub fn session(&self, key: &AccountingSessionKey) -> Option<&AccountingSession> {
        self.sessions.get(key)
    }

    /// Applies one accounting event using the default missing-parent mapping.
    ///
    /// Side effects: mutates only this in-memory store. It does not write files,
    /// emit dynamic-circuit commands, or touch shaping state.
    pub fn apply_event(&mut self, event: AccountingEvent) -> AccountingSessionUpdate {
        self.apply_event_with_mapping(event, DynamicCircuitMapping::default())
    }

    /// Applies one accounting event with explicit dynamic-circuit mapping state.
    ///
    /// Side effects: mutates only this in-memory store. It does not write files,
    /// emit dynamic-circuit commands, or touch shaping state.
    pub fn apply_event_with_mapping(
        &mut self,
        event: AccountingEvent,
        mapping: DynamicCircuitMapping,
    ) -> AccountingSessionUpdate {
        let Some(status_type) = event.status_type else {
            return AccountingSessionUpdate::Ignored {
                reason: AccountingSessionIgnoreReason::MissingStatusType,
            };
        };

        match status_type {
            AcctStatusType::Start | AcctStatusType::InterimUpdate => {
                self.upsert_active_session(event, mapping)
            }
            AcctStatusType::Stop => self.stop_session(event, mapping),
            AcctStatusType::AccountingOn => {
                self.mark_nas_sessions_stale(event, NasResetStatus::AccountingOn)
            }
            AcctStatusType::AccountingOff => {
                self.mark_nas_sessions_stale(event, NasResetStatus::AccountingOff)
            }
            AcctStatusType::Unknown(value) => AccountingSessionUpdate::Ignored {
                reason: AccountingSessionIgnoreReason::UnsupportedStatusType(value),
            },
        }
    }

    /// Applies one accounting event and emits dynamic-circuit intents for shapeable sessions.
    ///
    /// Side effects: mutates only this in-memory store and invokes `sink` for
    /// emitted intents. This method does not write files, talk to `lqosd`, or
    /// touch shaping state.
    pub fn apply_event_with_mapping_and_commands(
        &mut self,
        event: AccountingEvent,
        mapping: DynamicCircuitMapping,
        sink: &mut impl DynamicCircuitCommandSink,
    ) -> AccountingSessionUpdate {
        let reset_identities = match event.status_type {
            Some(AcctStatusType::AccountingOn | AcctStatusType::AccountingOff) => {
                Some(NasIdentitySet::from_event(&event))
            }
            _ => None,
        };
        let update = self.apply_event_with_mapping(event, mapping);
        self.emit_dynamic_circuit_intents(&update, reset_identities.as_ref(), sink);
        update
    }

    /// Removes one retained session and emits a dynamic-circuit removal when possible.
    ///
    /// Side effects: mutates only this in-memory store and invokes `sink` for
    /// dynamic circuits previously emitted by this session. This method does not
    /// write files, talk to `lqosd`, or touch shaping state.
    pub fn expire_session_with_commands(
        &mut self,
        key: &AccountingSessionKey,
        sink: &mut impl DynamicCircuitCommandSink,
    ) -> Option<AccountingSession> {
        let mut session = self.sessions.remove(key)?;
        emit_active_dynamic_circuit_removals(
            key,
            &mut session,
            DynamicCircuitRemovalReason::Expired,
            sink,
        );
        Some(session)
    }

    fn emit_dynamic_circuit_intents(
        &mut self,
        update: &AccountingSessionUpdate,
        reset_identities: Option<&NasIdentitySet>,
        sink: &mut impl DynamicCircuitCommandSink,
    ) {
        match update {
            AccountingSessionUpdate::SessionUpdated { key, state } => {
                self.emit_session_dynamic_circuit_intents(key, *state, sink);
            }
            AccountingSessionUpdate::NasSessionsMarkedStale { reset, .. } => {
                let Some(reset_identities) = reset_identities else {
                    return;
                };
                self.emit_nas_reset_dynamic_circuit_removals(reset_identities, *reset, sink);
            }
            AccountingSessionUpdate::Ignored { .. } => {}
        }
    }

    fn emit_session_dynamic_circuit_intents(
        &mut self,
        key: &AccountingSessionKey,
        state: AccountingSessionState,
        sink: &mut impl DynamicCircuitCommandSink,
    ) {
        match state {
            AccountingSessionState::Active => {
                let Some(circuit_id) = key.dynamic_circuit_id() else {
                    return;
                };
                let Some(session) = self.sessions.get_mut(key) else {
                    return;
                };
                if !session.pending_reasons.is_empty() {
                    emit_active_dynamic_circuit_removals(
                        key,
                        session,
                        DynamicCircuitRemovalReason::NoLongerShapeable,
                        sink,
                    );
                    return;
                }

                let already_emitted = session
                    .active_dynamic_circuit_ids
                    .iter()
                    .any(|active_id| active_id == &circuit_id);
                let upsert = DynamicCircuitUpsert {
                    circuit_id: circuit_id.clone(),
                    session_key: key.clone(),
                    event: session.latest_event.clone(),
                };
                match (already_emitted, session.latest_event.status_type) {
                    (true, Some(AcctStatusType::InterimUpdate)) => {
                        sink.emit(DynamicCircuitIntent::UpdateDynamicCircuit(upsert));
                    }
                    (_, Some(AcctStatusType::Start | AcctStatusType::InterimUpdate)) => {
                        sink.emit(DynamicCircuitIntent::CreateDynamicCircuit(upsert));
                    }
                    _ => {}
                }
                push_active_dynamic_circuit_id(&mut session.active_dynamic_circuit_ids, circuit_id);
                emit_rekeyed_dynamic_circuit_removals(key, session, sink);
            }
            AccountingSessionState::Stopped => {
                let Some(session) = self.sessions.get_mut(key) else {
                    return;
                };
                emit_active_dynamic_circuit_removals(
                    key,
                    session,
                    DynamicCircuitRemovalReason::Stop,
                    sink,
                );
            }
            AccountingSessionState::Stale(_) => {}
        }
    }

    fn emit_nas_reset_dynamic_circuit_removals(
        &mut self,
        reset_identities: &NasIdentitySet,
        reset: NasResetStatus,
        sink: &mut impl DynamicCircuitCommandSink,
    ) {
        for (key, session) in &mut self.sessions {
            if !session_matches_identities(key, session, reset_identities) {
                continue;
            }
            emit_active_dynamic_circuit_removals(
                key,
                session,
                DynamicCircuitRemovalReason::NasReset(reset),
                sink,
            );
        }
    }

    fn upsert_active_session(
        &mut self,
        event: AccountingEvent,
        mapping: DynamicCircuitMapping,
    ) -> AccountingSessionUpdate {
        self.apply_session_state(event, mapping, AccountingSessionState::Active)
    }

    fn stop_session(
        &mut self,
        event: AccountingEvent,
        mapping: DynamicCircuitMapping,
    ) -> AccountingSessionUpdate {
        self.apply_session_state(event, mapping, AccountingSessionState::Stopped)
    }

    fn apply_session_state(
        &mut self,
        event: AccountingEvent,
        mapping: DynamicCircuitMapping,
        state: AccountingSessionState,
    ) -> AccountingSessionUpdate {
        let key_resolution = self.resolve_session_key(&event);
        let promoted_sessions = key_resolution
            .previous_keys
            .iter()
            .filter_map(|previous_key| self.sessions.remove(previous_key))
            .collect::<Vec<_>>();
        let key = key_resolution.key;

        if let Some(session) = self.sessions.get_mut(&key) {
            let mut previous_event = session.latest_event.clone();
            for promoted_session in promoted_sessions {
                merge_known_nas_identity_values(
                    &mut session.known_nas_identities,
                    promoted_session.known_nas_identities,
                );
                merge_active_dynamic_circuit_ids(
                    &mut session.active_dynamic_circuit_ids,
                    promoted_session.active_dynamic_circuit_ids,
                );
                if state == AccountingSessionState::Active {
                    previous_event =
                        merge_sparse_active_event(&previous_event, promoted_session.latest_event);
                }
            }
            session.state = state;
            session.latest_event = latest_session_event(Some(&previous_event), event, state);
            session.pending_reasons = pending_reasons(&session.latest_event, mapping);
            merge_known_nas_identities(&mut session.known_nas_identities, &session.latest_event);
        } else {
            let mut known_nas_identities = Vec::new();
            let mut active_dynamic_circuit_ids = Vec::new();
            let mut previous_event = None;
            for promoted_session in promoted_sessions {
                merge_known_nas_identity_values(
                    &mut known_nas_identities,
                    promoted_session.known_nas_identities,
                );
                merge_active_dynamic_circuit_ids(
                    &mut active_dynamic_circuit_ids,
                    promoted_session.active_dynamic_circuit_ids,
                );
                previous_event = Some(match previous_event {
                    Some(previous_event) if state == AccountingSessionState::Active => {
                        merge_sparse_active_event(&previous_event, promoted_session.latest_event)
                    }
                    Some(previous_event) => previous_event,
                    None => promoted_session.latest_event,
                });
            }
            let latest_event = latest_session_event(previous_event.as_ref(), event, state);
            merge_known_nas_identities(&mut known_nas_identities, &latest_event);
            let pending_reasons = pending_reasons(&latest_event, mapping);
            self.sessions.insert(
                key.clone(),
                AccountingSession {
                    state,
                    latest_event,
                    known_nas_identities,
                    active_dynamic_circuit_ids,
                    pending_reasons,
                },
            );
        }

        AccountingSessionUpdate::SessionUpdated { key, state }
    }

    fn mark_nas_sessions_stale(
        &mut self,
        event: AccountingEvent,
        reset: NasResetStatus,
    ) -> AccountingSessionUpdate {
        let reset_identities = NasIdentitySet::from_event(&event);
        let Some(nas) = reset_identities.primary() else {
            return AccountingSessionUpdate::Ignored {
                reason: AccountingSessionIgnoreReason::MissingNasIdentityForReset(reset),
            };
        };

        let state = AccountingSessionState::Stale(reset);
        let mut marked_count = 0;
        for (key, session) in &mut self.sessions {
            if !session_matches_identities(key, session, &reset_identities) {
                continue;
            }
            session.state = state;
            marked_count += 1;
        }

        AccountingSessionUpdate::NasSessionsMarkedStale {
            nas,
            reset,
            marked_count,
        }
    }

    fn resolve_session_key(&self, event: &AccountingEvent) -> SessionKeyResolution {
        let identities = NasIdentitySet::from_event(event);
        let nas = identities.primary();
        let acct_session_id = event
            .acct_session_id
            .as_ref()
            .filter(|acct_session_id| !acct_session_id.is_empty());

        if let Some(acct_session_id) = acct_session_id
            && let Some(nas) = nas.clone()
        {
            let candidate = AccountingSessionKey::NasSession {
                nas,
                acct_session_id: acct_session_id.clone(),
            };
            if self.sessions.contains_key(&candidate) {
                let mut previous_keys = Vec::new();
                if let Some(previous_key) =
                    self.alternate_session_key(acct_session_id, &identities, &candidate)
                {
                    previous_keys.push(previous_key);
                }
                if let Some(previous_key) = self.existing_pending_key(event, &identities) {
                    push_unique_key(&mut previous_keys, previous_key);
                }
                if !previous_keys.is_empty() {
                    return SessionKeyResolution::promoted(candidate, previous_keys);
                }
                return SessionKeyResolution::new(candidate);
            }
            if let Some(existing_key) = self.existing_session_key(acct_session_id, &identities) {
                if let Some(previous_key) = self.existing_pending_key(event, &identities) {
                    return SessionKeyResolution::promoted(existing_key, vec![previous_key]);
                }
                return SessionKeyResolution::new(existing_key);
            }
            if let Some(previous_key) = self.existing_pending_key(event, &identities) {
                return SessionKeyResolution::promoted(candidate, vec![previous_key]);
            }

            return SessionKeyResolution::new(candidate);
        }

        SessionKeyResolution::new(AccountingSessionKey::Pending {
            fingerprint: PendingSessionFingerprint::from_event(event, nas),
        })
        .or_existing(self.existing_fallback_session_key(event, &identities))
    }

    fn existing_session_key(
        &self,
        acct_session_id: &str,
        identities: &NasIdentitySet,
    ) -> Option<AccountingSessionKey> {
        self.matching_session_key(acct_session_id, identities, None)
    }

    fn alternate_session_key(
        &self,
        acct_session_id: &str,
        identities: &NasIdentitySet,
        candidate: &AccountingSessionKey,
    ) -> Option<AccountingSessionKey> {
        self.matching_session_key(acct_session_id, identities, Some(candidate))
    }

    fn matching_session_key(
        &self,
        acct_session_id: &str,
        identities: &NasIdentitySet,
        excluded_key: Option<&AccountingSessionKey>,
    ) -> Option<AccountingSessionKey> {
        let mut found = None;
        for (key, session) in &self.sessions {
            if excluded_key.is_some_and(|excluded_key| excluded_key == key) {
                continue;
            }
            let AccountingSessionKey::NasSession {
                acct_session_id: existing_session_id,
                ..
            } = key
            else {
                continue;
            };
            if existing_session_id != acct_session_id {
                continue;
            }
            if !session_matches_identities(key, session, identities) {
                continue;
            }
            if found.is_some() {
                return None;
            }
            found = Some(key.clone());
        }
        found
    }

    fn existing_pending_key(
        &self,
        event: &AccountingEvent,
        identities: &NasIdentitySet,
    ) -> Option<AccountingSessionKey> {
        let event_fingerprint = PendingSessionFingerprint::from_event(event, identities.primary());
        let mut found = None;
        for (key, session) in &self.sessions {
            let AccountingSessionKey::Pending { fingerprint } = key else {
                continue;
            };
            let nas_context_matches =
                !identities.is_empty() && session_matches_identities(key, session, identities);
            if fingerprint.matches_with_nas_context(&event_fingerprint, nas_context_matches) {
                if found.is_some() {
                    return None;
                }
                found = Some(key.clone());
            }
        }
        found
    }

    fn existing_fallback_session_key(
        &self,
        event: &AccountingEvent,
        identities: &NasIdentitySet,
    ) -> Option<AccountingSessionKey> {
        let event_fingerprint = PendingSessionFingerprint::from_event(event, identities.primary());
        let mut found = None;
        for (key, session) in &self.sessions {
            if matches!(key, AccountingSessionKey::Pending { .. }) {
                continue;
            }
            if !identities.is_empty() && !session_matches_identities(key, session, identities) {
                continue;
            }
            let session_fingerprint = session_fingerprint(key, session);
            if !session_fingerprint
                .matches_with_nas_context(&event_fingerprint, !identities.is_empty())
            {
                continue;
            }
            if found.is_some() {
                return None;
            }
            found = Some(key.clone());
        }
        found
    }
}

struct SessionKeyResolution {
    key: AccountingSessionKey,
    previous_keys: Vec<AccountingSessionKey>,
}

impl SessionKeyResolution {
    fn new(key: AccountingSessionKey) -> Self {
        Self {
            key,
            previous_keys: Vec::new(),
        }
    }

    fn promoted(key: AccountingSessionKey, previous_keys: Vec<AccountingSessionKey>) -> Self {
        Self { key, previous_keys }
    }

    fn or_existing(self, existing_key: Option<AccountingSessionKey>) -> Self {
        if let Some(key) = existing_key {
            Self::new(key)
        } else {
            self
        }
    }
}

/// One retained accounting session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountingSession {
    /// Current lifecycle state for the session.
    pub state: AccountingSessionState,
    /// Latest decoded event data retained for this session.
    pub latest_event: AccountingEvent,
    /// NAS identities observed across retained events for this session.
    pub known_nas_identities: Vec<NasIdentity>,
    /// Dynamic-circuit IDs this session has emitted and not yet removed.
    pub active_dynamic_circuit_ids: Vec<String>,
    /// Reasons this session cannot currently become a dynamic circuit.
    pub pending_reasons: Vec<PendingSessionReason>,
}

/// Stable key used for retained accounting sessions.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum AccountingSessionKey {
    /// Deterministic key when both NAS identity and Acct-Session-Id are known.
    NasSession {
        /// NAS identity selected for the key.
        nas: NasIdentity,
        /// Acct-Session-Id supplied by the NAS.
        acct_session_id: String,
    },
    /// Fingerprinted key for events that cannot be keyed by Acct-Session-Id.
    Pending {
        /// Best-effort identity fingerprint for coalescing pending events.
        fingerprint: PendingSessionFingerprint,
    },
}

impl AccountingSessionKey {
    /// Returns the NAS identity associated with this key, when known.
    #[must_use]
    pub fn nas(&self) -> Option<&NasIdentity> {
        match self {
            Self::NasSession { nas, .. } => Some(nas),
            Self::Pending { fingerprint } => fingerprint.nas.as_ref(),
        }
    }

    /// Returns the deterministic dynamic-circuit id for keyed sessions.
    #[must_use]
    pub fn dynamic_circuit_id(&self) -> Option<String> {
        let Self::NasSession {
            nas,
            acct_session_id,
        } = self
        else {
            return None;
        };

        Some(format!(
            "radius:{}:session:{}",
            nas.circuit_id_component(),
            hex_component(acct_session_id.as_bytes())
        ))
    }
}

/// Best-effort identity for sessions missing Acct-Session-Id.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PendingSessionFingerprint {
    /// NAS identity when present.
    pub nas: Option<NasIdentity>,
    /// Acct-Session-Id when present but not enough to form a deterministic key.
    pub acct_session_id: Option<String>,
    /// User-Name when present.
    pub user_name: Option<String>,
    /// Calling-Station-Id when present.
    pub calling_station_id: Option<String>,
    /// NAS-Port-Id when present.
    pub nas_port_id: Option<String>,
    /// NAS-Port when present.
    pub nas_port: Option<u32>,
}

impl PendingSessionFingerprint {
    fn from_event(event: &AccountingEvent, nas: Option<NasIdentity>) -> Self {
        Self {
            nas,
            acct_session_id: non_empty_text(&event.acct_session_id),
            user_name: non_empty_text(&event.user_name),
            calling_station_id: non_empty_text(&event.calling_station_id),
            nas_port_id: non_empty_text(&event.nas_port_id),
            nas_port: event.nas_port,
        }
    }

    fn matches_with_nas_context(&self, other: &Self, nas_context_matches: bool) -> bool {
        let nas_matches = if nas_context_matches {
            true
        } else {
            let Some(nas_matches) = optional_match(&self.nas, &other.nas) else {
                return false;
            };
            nas_matches
        };
        let Some(session_id_matches) =
            optional_match(&self.acct_session_id, &other.acct_session_id)
        else {
            return false;
        };
        if session_id_matches && nas_matches {
            return true;
        }
        let Some(user_name_matches) = optional_match(&self.user_name, &other.user_name) else {
            return false;
        };
        let Some(calling_station_matches) =
            optional_match(&self.calling_station_id, &other.calling_station_id)
        else {
            return false;
        };
        let Some(nas_port_id_matches) = optional_match(&self.nas_port_id, &other.nas_port_id)
        else {
            return false;
        };
        let Some(nas_port_matches) = optional_match(&self.nas_port, &other.nas_port) else {
            return false;
        };

        let non_session_matches = [
            user_name_matches,
            calling_station_matches,
            nas_port_id_matches,
            nas_port_matches,
        ]
        .into_iter()
        .filter(|matched| *matched)
        .count();
        ((session_id_matches || nas_matches) && non_session_matches > 0) || non_session_matches >= 2
    }
}

/// NAS identity used to group accounting sessions.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum NasIdentity {
    /// NAS-Identifier text.
    Identifier(String),
    /// NAS-IP-Address value.
    Ipv4(Ipv4Addr),
    /// NAS-IPv6-Address value.
    Ipv6(Ipv6Addr),
}

impl NasIdentity {
    fn circuit_id_component(&self) -> String {
        match self {
            Self::Identifier(identifier) => {
                format!("nas-id:{}", hex_component(identifier.as_bytes()))
            }
            Self::Ipv4(address) => format!("nas-ipv4:{}", hex_component(&address.octets())),
            Self::Ipv6(address) => format!("nas-ipv6:{}", hex_component(&address.octets())),
        }
    }
}

/// Lifecycle state retained for an accounting session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountingSessionState {
    /// The latest lifecycle event is Start or Interim-Update.
    Active,
    /// A Stop event has been observed.
    Stopped,
    /// A NAS reset event marked the session stale.
    Stale(NasResetStatus),
}

/// NAS reset status that can stale existing sessions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NasResetStatus {
    /// Acct-Status-Type Accounting-On.
    AccountingOn,
    /// Acct-Status-Type Accounting-Off.
    AccountingOff,
}

/// Current dynamic-circuit mapping state for a session event.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DynamicCircuitMapping {
    /// Subscriber IP, rate, parent, and identity can map to a dynamic circuit.
    Ready,
    /// No parent node mapping is known yet.
    #[default]
    MissingParent,
    /// More than one parent or subscriber mapping matches the event.
    Ambiguous,
}

/// Reason a retained session cannot currently become a dynamic circuit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PendingSessionReason {
    /// Acct-Session-Id is missing.
    MissingSessionId,
    /// No NAS identity attribute is present.
    MissingNasIdentity,
    /// No subscriber IPv4, IPv6, or prefix address is present.
    MissingIpAddress,
    /// No decoded rate-limit attribute is present.
    MissingRate,
    /// No parent node mapping is available.
    MissingParent,
    /// The event maps ambiguously to more than one parent or subscriber target.
    AmbiguousMapping,
}

/// Result of applying one accounting event to the session store.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AccountingSessionUpdate {
    /// A single session was created, refreshed, or stopped.
    SessionUpdated {
        /// Session key that was updated.
        key: AccountingSessionKey,
        /// Lifecycle state after the update.
        state: AccountingSessionState,
    },
    /// A NAS reset event marked matching sessions stale.
    NasSessionsMarkedStale {
        /// NAS identity used to match sessions.
        nas: NasIdentity,
        /// Reset status that caused the stale mark.
        reset: NasResetStatus,
        /// Number of sessions marked stale.
        marked_count: usize,
    },
    /// The event did not carry an actionable accounting status.
    Ignored {
        /// Reason the event was ignored.
        reason: AccountingSessionIgnoreReason,
    },
}

/// Reason an accounting event did not update session state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountingSessionIgnoreReason {
    /// Acct-Status-Type is missing.
    MissingStatusType,
    /// Acct-Status-Type is not handled by this store.
    UnsupportedStatusType(u32),
    /// Accounting-On or Accounting-Off had no NAS identity to match sessions.
    MissingNasIdentityForReset(NasResetStatus),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct NasIdentitySet {
    identifier: Option<String>,
    ipv4: Option<Ipv4Addr>,
    ipv6: Option<Ipv6Addr>,
}

impl NasIdentitySet {
    fn from_event(event: &AccountingEvent) -> Self {
        Self {
            identifier: event
                .nas_identifier
                .as_ref()
                .filter(|identifier| !identifier.is_empty())
                .cloned(),
            ipv4: event.nas_ip_address,
            ipv6: event.nas_ipv6_address,
        }
    }

    fn primary(&self) -> Option<NasIdentity> {
        if let Some(identifier) = &self.identifier {
            return Some(NasIdentity::Identifier(identifier.clone()));
        }
        if let Some(ipv4) = self.ipv4 {
            return Some(NasIdentity::Ipv4(ipv4));
        }
        self.ipv6.map(NasIdentity::Ipv6)
    }

    fn is_empty(&self) -> bool {
        self.identifier.is_none() && self.ipv4.is_none() && self.ipv6.is_none()
    }

    fn contains(&self, identity: &NasIdentity) -> bool {
        match identity {
            NasIdentity::Identifier(identifier) => self.identifier.as_ref() == Some(identifier),
            NasIdentity::Ipv4(ipv4) => self.ipv4 == Some(*ipv4),
            NasIdentity::Ipv6(ipv6) => self.ipv6 == Some(*ipv6),
        }
    }

    fn identities(&self) -> Vec<NasIdentity> {
        let mut identities = Vec::new();
        if let Some(identifier) = &self.identifier {
            identities.push(NasIdentity::Identifier(identifier.clone()));
        }
        if let Some(ipv4) = self.ipv4 {
            identities.push(NasIdentity::Ipv4(ipv4));
        }
        if let Some(ipv6) = self.ipv6 {
            identities.push(NasIdentity::Ipv6(ipv6));
        }
        identities
    }

    fn overlaps(&self, other: &Self) -> bool {
        self.identifier
            .as_ref()
            .zip(other.identifier.as_ref())
            .is_some_and(|(left, right)| left == right)
            || self
                .ipv4
                .zip(other.ipv4)
                .is_some_and(|(left, right)| left == right)
            || self
                .ipv6
                .zip(other.ipv6)
                .is_some_and(|(left, right)| left == right)
    }
}

fn session_matches_identities(
    key: &AccountingSessionKey,
    session: &AccountingSession,
    identities: &NasIdentitySet,
) -> bool {
    if let Some(nas) = key.nas()
        && identities.contains(nas)
    {
        return true;
    }
    if session
        .known_nas_identities
        .iter()
        .any(|identity| identities.contains(identity))
    {
        return true;
    }

    identities.overlaps(&NasIdentitySet::from_event(&session.latest_event))
}

fn latest_session_event(
    previous_event: Option<&AccountingEvent>,
    event: AccountingEvent,
    state: AccountingSessionState,
) -> AccountingEvent {
    if state != AccountingSessionState::Active {
        return event;
    }
    let Some(previous_event) = previous_event else {
        return event;
    };
    merge_sparse_active_event(previous_event, event)
}

fn merge_sparse_active_event(
    previous_event: &AccountingEvent,
    mut event: AccountingEvent,
) -> AccountingEvent {
    carry_text(&mut event.acct_session_id, &previous_event.acct_session_id);
    carry_text(&mut event.nas_identifier, &previous_event.nas_identifier);
    carry_text(&mut event.nas_port_id, &previous_event.nas_port_id);
    carry_text(&mut event.user_name, &previous_event.user_name);
    carry_text(
        &mut event.calling_station_id,
        &previous_event.calling_station_id,
    );
    carry_text(
        &mut event.called_station_id,
        &previous_event.called_station_id,
    );
    carry_optional(&mut event.nas_ip_address, previous_event.nas_ip_address);
    carry_optional(&mut event.nas_ipv6_address, previous_event.nas_ipv6_address);
    carry_optional(&mut event.nas_port, previous_event.nas_port);
    carry_optional(
        &mut event.framed_ip_address,
        previous_event.framed_ip_address,
    );
    carry_optional(
        &mut event.framed_ip_netmask,
        previous_event.framed_ip_netmask,
    );
    carry_optional(
        &mut event.framed_ipv6_address,
        previous_event.framed_ipv6_address,
    );
    if event.framed_routes.is_empty() {
        event.framed_routes = previous_event.framed_routes.clone();
    }
    if event.class.is_empty() {
        event.class = previous_event.class.clone();
    }
    if event.framed_ipv6_prefixes.is_empty() {
        event.framed_ipv6_prefixes = previous_event.framed_ipv6_prefixes.clone();
    }
    if event.delegated_ipv6_prefixes.is_empty() {
        event.delegated_ipv6_prefixes = previous_event.delegated_ipv6_prefixes.clone();
    }
    if event.mikrotik_rate_limits.is_empty() {
        event.mikrotik_rate_limits = previous_event.mikrotik_rate_limits.clone();
    }
    event
}

fn pending_reasons(
    event: &AccountingEvent,
    mapping: DynamicCircuitMapping,
) -> Vec<PendingSessionReason> {
    let mut reasons = Vec::new();

    if event.acct_session_id.as_ref().is_none_or(String::is_empty) {
        reasons.push(PendingSessionReason::MissingSessionId);
    }
    if NasIdentitySet::from_event(event).is_empty() {
        reasons.push(PendingSessionReason::MissingNasIdentity);
    }
    if !has_subscriber_ip(event) {
        reasons.push(PendingSessionReason::MissingIpAddress);
    }
    if event.mikrotik_rate_limits.is_empty() {
        reasons.push(PendingSessionReason::MissingRate);
    }
    match mapping {
        DynamicCircuitMapping::Ready => {}
        DynamicCircuitMapping::MissingParent => reasons.push(PendingSessionReason::MissingParent),
        DynamicCircuitMapping::Ambiguous => reasons.push(PendingSessionReason::AmbiguousMapping),
    }

    reasons
}

fn has_subscriber_ip(event: &AccountingEvent) -> bool {
    event.framed_ip_address.is_some()
        || event.framed_ipv6_address.is_some()
        || !event.framed_routes.is_empty()
        || !event.framed_ipv6_prefixes.is_empty()
        || !event.delegated_ipv6_prefixes.is_empty()
}

fn non_empty_text(value: &Option<String>) -> Option<String> {
    value.as_ref().filter(|value| !value.is_empty()).cloned()
}

fn carry_text(target: &mut Option<String>, previous: &Option<String>) {
    if target.is_none() {
        *target = previous.clone();
    }
}

fn carry_optional<T: Copy>(target: &mut Option<T>, previous: Option<T>) {
    if target.is_none() {
        *target = previous;
    }
}

fn optional_match<T: Eq>(left: &Option<T>, right: &Option<T>) -> Option<bool> {
    match (left, right) {
        (Some(left), Some(right)) if left == right => Some(true),
        (Some(_), Some(_)) => None,
        _ => Some(false),
    }
}

fn session_fingerprint(
    key: &AccountingSessionKey,
    session: &AccountingSession,
) -> PendingSessionFingerprint {
    let mut fingerprint = PendingSessionFingerprint::from_event(&session.latest_event, None);
    if let AccountingSessionKey::NasSession {
        acct_session_id, ..
    } = key
    {
        fingerprint.acct_session_id = Some(acct_session_id.clone());
    }
    fingerprint
}

fn merge_known_nas_identities(
    known_nas_identities: &mut Vec<NasIdentity>,
    event: &AccountingEvent,
) {
    merge_known_nas_identity_values(
        known_nas_identities,
        NasIdentitySet::from_event(event).identities(),
    );
}

fn merge_known_nas_identity_values(
    known_nas_identities: &mut Vec<NasIdentity>,
    identities: Vec<NasIdentity>,
) {
    for identity in identities {
        if !known_nas_identities.contains(&identity) {
            known_nas_identities.push(identity);
        }
    }
}

fn merge_active_dynamic_circuit_ids(active_ids: &mut Vec<String>, ids: Vec<String>) {
    for id in ids {
        if !active_ids.contains(&id) {
            active_ids.push(id);
        }
    }
}

fn push_active_dynamic_circuit_id(active_ids: &mut Vec<String>, id: String) {
    if !active_ids.contains(&id) {
        active_ids.push(id);
    }
}

fn emit_active_dynamic_circuit_removals(
    key: &AccountingSessionKey,
    session: &mut AccountingSession,
    reason: DynamicCircuitRemovalReason,
    sink: &mut impl DynamicCircuitCommandSink,
) {
    for circuit_id in std::mem::take(&mut session.active_dynamic_circuit_ids) {
        sink.emit(DynamicCircuitIntent::RemoveDynamicCircuit(
            DynamicCircuitRemoval {
                circuit_id,
                session_key: key.clone(),
                reason,
            },
        ));
    }
}

fn emit_rekeyed_dynamic_circuit_removals(
    key: &AccountingSessionKey,
    session: &mut AccountingSession,
    sink: &mut impl DynamicCircuitCommandSink,
) {
    let Some(current_circuit_id) = key.dynamic_circuit_id() else {
        return;
    };
    let active_ids = std::mem::take(&mut session.active_dynamic_circuit_ids);
    for circuit_id in active_ids {
        if circuit_id == current_circuit_id {
            push_active_dynamic_circuit_id(&mut session.active_dynamic_circuit_ids, circuit_id);
            continue;
        }
        sink.emit(DynamicCircuitIntent::RemoveDynamicCircuit(
            DynamicCircuitRemoval {
                circuit_id,
                session_key: key.clone(),
                reason: DynamicCircuitRemovalReason::Rekeyed,
            },
        ));
    }
}

fn push_unique_key(keys: &mut Vec<AccountingSessionKey>, key: AccountingSessionKey) {
    if !keys.contains(&key) {
        keys.push(key);
    }
}

fn hex_component(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    encoded
}

#[cfg(test)]
mod tests;
