//! In-memory RADIUS accounting session tracking.

use crate::mac_match::ShapedDevicesMacMatch;
use crate::{
    AccountingEvent, AcctStatusType, DynamicCircuitCommandSink, DynamicCircuitIntent,
    DynamicCircuitRemoval, DynamicCircuitRemovalReason, DynamicCircuitUpsert, MikrotikRateLimit,
};
use lqos_config::{ShapedDevice, validate_rate_profile_mbps};
use std::collections::{HashMap, HashSet};
use std::net::{Ipv4Addr, Ipv6Addr};

pub use lqos_config::RateProfileValidationError as SessionRateProfileError;

/// Identity and fallback settings for ShapedDevices-backed RADIUS resolution.
#[derive(Clone, Debug)]
pub struct ShapedDevicesMatchOptions {
    /// Whether `User-Name` matches the optional ShapedDevices RADIUS username.
    pub match_by_username: bool,
    /// Whether `Calling-Station-Id` matches the ShapedDevices MAC field.
    pub match_by_mac: bool,
    /// Rate profile for an unmatched identity when packet rates are absent.
    pub fallback_profile: Option<SessionRateProfile>,
    /// Parent attachment for an unmatched identity.
    pub fallback_parent: Option<DynamicCircuitParent>,
}

/// In-memory session store for accepted RADIUS accounting events.
#[derive(Debug, Default)]
pub struct AccountingSessionStore {
    sessions: HashMap<AccountingSessionKey, AccountingSession>,
    nas_session_keys_by_acct_session_id: HashMap<String, HashSet<AccountingSessionKey>>,
    pending_keys_by_lookup: HashMap<SessionLookupIndexKey, HashSet<AccountingSessionKey>>,
    fallback_keys_by_lookup: HashMap<SessionLookupIndexKey, HashSet<AccountingSessionKey>>,
    activation_counters: RadiusActivationCounters,
}

impl AccountingSessionStore {
    /// Creates an empty session store.
    ///
    /// Side effects: none. Session state is kept in memory only.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            nas_session_keys_by_acct_session_id: HashMap::new(),
            pending_keys_by_lookup: HashMap::new(),
            fallback_keys_by_lookup: HashMap::new(),
            activation_counters: RadiusActivationCounters::default(),
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

    /// Returns dynamic-circuit activation counters tracked by this session store.
    #[must_use]
    pub const fn activation_counters(&self) -> RadiusActivationCounters {
        self.activation_counters
    }

    /// Returns diagnostics for all retained accounting sessions.
    ///
    /// Side effects: none. The returned snapshot is built from in-memory state.
    #[must_use]
    pub fn activation_diagnostics(&self) -> Vec<RadiusActivationDiagnostic> {
        self.sessions
            .iter()
            .map(|(key, session)| RadiusActivationDiagnostic::from_retained_session(key, session))
            .collect()
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
        self.apply_event_with_mapping_and_rate_sources(
            event,
            mapping,
            SessionRateSources::default(),
        )
    }

    /// Applies one accounting event with explicit mapping and rate-source state.
    ///
    /// Resolution priority is decoded packet rate, then any supplied
    /// ShapedDevices profile, then any supplied fallback profile.
    ///
    /// Side effects: mutates only this in-memory store. It does not write files,
    /// emit dynamic-circuit commands, or touch shaping state.
    pub fn apply_event_with_mapping_and_rate_sources(
        &mut self,
        event: AccountingEvent,
        mapping: DynamicCircuitMapping,
        rate_sources: SessionRateSources,
    ) -> AccountingSessionUpdate {
        self.apply_event_with_dynamic_circuit_resolution(
            event,
            DynamicCircuitResolution {
                mapping,
                rate_sources,
                matched_shaped_device: None,
            },
        )
    }

    /// Applies one accounting event with resolved dynamic-circuit metadata.
    ///
    /// Resolution priority is decoded packet rate, then any supplied
    /// ShapedDevices profile, then any supplied fallback profile.
    ///
    /// Side effects: mutates only this in-memory store. It does not write files,
    /// emit dynamic-circuit commands, or touch shaping state.
    pub fn apply_event_with_dynamic_circuit_resolution(
        &mut self,
        event: AccountingEvent,
        resolution: DynamicCircuitResolution,
    ) -> AccountingSessionUpdate {
        self.apply_event_with_resolution_resolver(event, |_| resolution)
    }

    /// Applies one accounting event using a shaped-devices MAC matcher.
    ///
    /// The matcher is evaluated after active-session sparse fields are merged,
    /// allowing an update packet to use a `Calling-Station-Id` carried from an
    /// earlier packet in the same session.
    ///
    /// Side effects: mutates only this in-memory store. It does not write files,
    /// emit dynamic-circuit commands, or touch shaping state.
    pub fn apply_event_with_shaped_devices_mac_matcher(
        &mut self,
        event: AccountingEvent,
        matcher: &crate::ShapedDevicesMacMatcher,
        fallback_profile: Option<SessionRateProfile>,
    ) -> AccountingSessionUpdate {
        self.apply_event_with_resolution_resolver(event, |latest_event| {
            DynamicCircuitResolution::from_shaped_devices_mac_match(
                matcher.match_event(latest_event),
                fallback_profile,
            )
        })
    }

    /// Applies one accounting event using configured ShapedDevices username and MAC matching.
    ///
    /// A unique username match is preferred over a MAC match. When neither identity matches,
    /// `fallback_parent` permits a default dynamic circuit to be created from the configured
    /// fallback speed profile.
    ///
    /// Side effects: mutates only this in-memory store.
    pub fn apply_event_with_shaped_devices_matcher(
        &mut self,
        event: AccountingEvent,
        matcher: &crate::ShapedDevicesMacMatcher,
        options: ShapedDevicesMatchOptions,
    ) -> AccountingSessionUpdate {
        self.apply_event_with_resolution_resolver(event, |latest_event| {
            DynamicCircuitResolution::from_shaped_devices_identity_match(
                matcher.match_event_with_identities(
                    latest_event,
                    options.match_by_username,
                    options.match_by_mac,
                ),
                options.fallback_profile,
                options.fallback_parent,
            )
        })
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
        self.apply_event_with_dynamic_circuit_resolution_and_commands(
            event,
            DynamicCircuitResolution {
                mapping,
                rate_sources: SessionRateSources::default(),
                matched_shaped_device: None,
            },
            sink,
        )
    }

    /// Applies one accounting event with resolved metadata and emits dynamic-circuit intents.
    ///
    /// Side effects: mutates only this in-memory store and invokes `sink` for
    /// emitted intents. This method does not write files, talk to `lqosd`, or
    /// touch shaping state.
    pub fn apply_event_with_dynamic_circuit_resolution_and_commands(
        &mut self,
        event: AccountingEvent,
        resolution: DynamicCircuitResolution,
        sink: &mut impl DynamicCircuitCommandSink,
    ) -> AccountingSessionUpdate {
        let update = self.apply_event_with_dynamic_circuit_resolution(event, resolution);
        self.emit_dynamic_circuit_intents(&update, sink);
        update
    }

    /// Applies one accounting event with a MAC matcher and emits dynamic-circuit intents.
    ///
    /// Side effects: mutates only this in-memory store and invokes `sink` for
    /// emitted intents. This method does not write files, talk to `lqosd`, or
    /// touch shaping state.
    pub fn apply_event_with_shaped_devices_mac_matcher_and_commands(
        &mut self,
        event: AccountingEvent,
        matcher: &crate::ShapedDevicesMacMatcher,
        fallback_profile: Option<SessionRateProfile>,
        sink: &mut impl DynamicCircuitCommandSink,
    ) -> AccountingSessionUpdate {
        let update =
            self.apply_event_with_shaped_devices_mac_matcher(event, matcher, fallback_profile);
        self.emit_dynamic_circuit_intents(&update, sink);
        update
    }

    /// Applies one accounting event using configured ShapedDevices identity matching and emits intents.
    ///
    /// Side effects: mutates this store and invokes `sink` for emitted dynamic-circuit intents.
    pub fn apply_event_with_shaped_devices_matcher_and_commands(
        &mut self,
        event: AccountingEvent,
        matcher: &crate::ShapedDevicesMacMatcher,
        options: ShapedDevicesMatchOptions,
        sink: &mut impl DynamicCircuitCommandSink,
    ) -> AccountingSessionUpdate {
        let update = self.apply_event_with_shaped_devices_matcher(event, matcher, options);
        self.emit_dynamic_circuit_intents(&update, sink);
        update
    }

    fn apply_event_with_resolution_resolver(
        &mut self,
        event: AccountingEvent,
        resolve_metadata: impl FnOnce(&AccountingEvent) -> DynamicCircuitResolution,
    ) -> AccountingSessionUpdate {
        let Some(status_type) = event.status_type else {
            return AccountingSessionUpdate::Ignored {
                reason: AccountingSessionIgnoreReason::MissingStatusType,
            };
        };

        match status_type {
            AcctStatusType::Start | AcctStatusType::InterimUpdate => {
                self.upsert_active_session(event, resolve_metadata)
            }
            AcctStatusType::Stop => self.stop_session(event, resolve_metadata),
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
        let mut session = self.remove_session(key)?;
        self.activation_counters.expiry += 1;
        let removal_reason = match session.state {
            AccountingSessionState::Stale(reset) => DynamicCircuitRemovalReason::NasReset(reset),
            AccountingSessionState::Active | AccountingSessionState::Stopped => {
                DynamicCircuitRemovalReason::Expired
            }
        };
        emit_active_dynamic_circuit_removals(
            key,
            &mut session,
            removal_reason,
            &mut self.activation_counters,
            sink,
        );
        Some(session)
    }

    fn emit_dynamic_circuit_intents(
        &mut self,
        update: &AccountingSessionUpdate,
        sink: &mut impl DynamicCircuitCommandSink,
    ) {
        match update {
            AccountingSessionUpdate::SessionUpdated { key, state } => {
                self.emit_session_dynamic_circuit_intents(key, *state, sink);
            }
            AccountingSessionUpdate::NasSessionsMarkedStale { .. } => {}
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
            AccountingSessionState::Active => self.emit_active_session_intents(key, sink),
            AccountingSessionState::Stopped => {
                let Some(session) = self.sessions.get_mut(key) else {
                    return;
                };
                emit_active_dynamic_circuit_removals(
                    key,
                    session,
                    DynamicCircuitRemovalReason::Stop,
                    &mut self.activation_counters,
                    sink,
                );
            }
            AccountingSessionState::Stale(_) => {}
        }
    }

    fn emit_active_session_intents(
        &mut self,
        key: &AccountingSessionKey,
        sink: &mut impl DynamicCircuitCommandSink,
    ) {
        let Some(session) = self.sessions.get_mut(key) else {
            return;
        };
        let Some(shaped_device) = session.resolved_shaped_device.clone() else {
            emit_active_dynamic_circuit_removals(
                key,
                session,
                DynamicCircuitRemovalReason::NoLongerShapeable,
                &mut self.activation_counters,
                sink,
            );
            return;
        };
        let circuit_id = shaped_device.circuit_id.clone();
        if circuit_id.trim().is_empty() {
            emit_active_dynamic_circuit_removals(
                key,
                session,
                DynamicCircuitRemovalReason::NoLongerShapeable,
                &mut self.activation_counters,
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
            shaped_device,
        };
        match (already_emitted, session.latest_event.status_type) {
            (true, Some(AcctStatusType::InterimUpdate)) => {
                self.activation_counters.update += 1;
                sink.emit(DynamicCircuitIntent::UpdateDynamicCircuit(upsert));
            }
            (_, Some(AcctStatusType::Start | AcctStatusType::InterimUpdate)) => {
                self.activation_counters.create += 1;
                sink.emit(DynamicCircuitIntent::CreateDynamicCircuit(upsert));
            }
            _ => {}
        }
        push_active_dynamic_circuit_id(&mut session.active_dynamic_circuit_ids, circuit_id.clone());
        push_unique_text(&mut session.diagnostic_circuit_ids, &circuit_id);
        emit_rekeyed_dynamic_circuit_removals(
            key,
            session,
            &circuit_id,
            &mut self.activation_counters,
            sink,
        );
    }

    fn upsert_active_session(
        &mut self,
        event: AccountingEvent,
        resolve_metadata: impl FnOnce(&AccountingEvent) -> DynamicCircuitResolution,
    ) -> AccountingSessionUpdate {
        self.apply_session_state(event, resolve_metadata, AccountingSessionState::Active)
    }

    fn stop_session(
        &mut self,
        event: AccountingEvent,
        resolve_metadata: impl FnOnce(&AccountingEvent) -> DynamicCircuitResolution,
    ) -> AccountingSessionUpdate {
        self.apply_session_state(event, resolve_metadata, AccountingSessionState::Stopped)
    }

    fn apply_session_state(
        &mut self,
        event: AccountingEvent,
        resolve_metadata: impl FnOnce(&AccountingEvent) -> DynamicCircuitResolution,
        state: AccountingSessionState,
    ) -> AccountingSessionUpdate {
        let key_resolution = self.resolve_session_key(&event);
        let promoted_sessions = key_resolution
            .previous_keys
            .iter()
            .filter_map(|previous_key| self.remove_session(previous_key))
            .collect::<Vec<_>>();
        let key = key_resolution.key;

        if self.sessions.contains_key(&key) {
            self.remove_session_indexes(&key);
            let session = self
                .sessions
                .get_mut(&key)
                .expect("session key existed before index removal");
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
                merge_diagnostic_circuit_ids(
                    &mut session.diagnostic_circuit_ids,
                    promoted_session.diagnostic_circuit_ids,
                );
                if state == AccountingSessionState::Active {
                    previous_event =
                        merge_sparse_active_event(&previous_event, promoted_session.latest_event);
                }
            }
            session.state = state;
            session.latest_event = latest_session_event(Some(&previous_event), event, state);
            let metadata = if state == AccountingSessionState::Active {
                session_dynamic_metadata(
                    &key,
                    &session.latest_event,
                    resolve_metadata(&session.latest_event),
                )
            } else {
                SessionDynamicMetadata::inactive()
            };
            session.resolved_rate = metadata.resolved_rate;
            session.resolved_shaped_device = metadata.resolved_shaped_device;
            if let Some(device) = &session.resolved_shaped_device {
                push_unique_text(&mut session.diagnostic_circuit_ids, &device.circuit_id);
            }
            session.pending_reasons = metadata.pending_reasons;
            merge_known_nas_identities(&mut session.known_nas_identities, &session.latest_event);
            self.index_session(&key);
        } else {
            let mut known_nas_identities = Vec::new();
            let mut active_dynamic_circuit_ids = Vec::new();
            let mut diagnostic_circuit_ids = Vec::new();
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
                merge_diagnostic_circuit_ids(
                    &mut diagnostic_circuit_ids,
                    promoted_session.diagnostic_circuit_ids,
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
            let metadata = if state == AccountingSessionState::Active {
                session_dynamic_metadata(&key, &latest_event, resolve_metadata(&latest_event))
            } else {
                SessionDynamicMetadata::inactive()
            };
            if let Some(device) = &metadata.resolved_shaped_device {
                push_unique_text(&mut diagnostic_circuit_ids, &device.circuit_id);
            }
            self.sessions.insert(
                key.clone(),
                AccountingSession {
                    state,
                    latest_event,
                    known_nas_identities,
                    resolved_rate: metadata.resolved_rate,
                    resolved_shaped_device: metadata.resolved_shaped_device,
                    active_dynamic_circuit_ids,
                    diagnostic_circuit_ids,
                    pending_reasons: metadata.pending_reasons,
                },
            );
            self.index_session(&key);
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
        let mut newly_stale_session_keys = HashSet::new();
        let mut stale_session_keys = HashSet::new();
        for (key, session) in &mut self.sessions {
            if !session_matches_identities(key, session, &reset_identities) {
                continue;
            }
            if !matches!(session.state, AccountingSessionState::Stale(_)) {
                newly_stale_session_keys.insert(key.clone());
            }
            stale_session_keys.insert(key.clone());
            session.state = state;
            session.resolved_rate = None;
            session.resolved_shaped_device = None;
            session.pending_reasons.clear();
            marked_count += 1;
        }

        AccountingSessionUpdate::NasSessionsMarkedStale {
            nas,
            reset,
            marked_count,
            newly_stale_session_keys,
            stale_session_keys,
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
        let candidate_keys = self
            .nas_session_keys_by_acct_session_id
            .get(acct_session_id)?;
        for key in candidate_keys {
            if excluded_key.is_some_and(|excluded_key| excluded_key == key) {
                continue;
            }
            let Some(session) = self.sessions.get(key) else {
                continue;
            };
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
        for key in self.lookup_candidates(&self.pending_keys_by_lookup, event, identities) {
            let Some(session) = self.sessions.get(&key) else {
                continue;
            };
            let AccountingSessionKey::Pending { fingerprint } = &key else {
                continue;
            };
            let nas_context_matches =
                !identities.is_empty() && session_matches_identities(&key, session, identities);
            if fingerprint.matches_with_nas_context(&event_fingerprint, nas_context_matches) {
                if found.is_some() {
                    return None;
                }
                found = Some(key);
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
        for key in self.lookup_candidates(&self.fallback_keys_by_lookup, event, identities) {
            if matches!(key, AccountingSessionKey::Pending { .. }) {
                continue;
            }
            let Some(session) = self.sessions.get(&key) else {
                continue;
            };
            if !identities.is_empty() && !session_matches_identities(&key, session, identities) {
                continue;
            }
            let session_fingerprint = session_fingerprint(&key, session);
            if !session_fingerprint
                .matches_with_nas_context(&event_fingerprint, !identities.is_empty())
            {
                continue;
            }
            if found.is_some() {
                return None;
            }
            found = Some(key);
        }
        found
    }

    fn lookup_candidates(
        &self,
        index: &HashMap<SessionLookupIndexKey, HashSet<AccountingSessionKey>>,
        event: &AccountingEvent,
        identities: &NasIdentitySet,
    ) -> HashSet<AccountingSessionKey> {
        let mut candidates = HashSet::new();
        for lookup_key in event_lookup_index_keys(event, identities) {
            let Some(keys) = index.get(&lookup_key) else {
                continue;
            };
            candidates.extend(keys.iter().cloned());
        }
        candidates
    }

    fn remove_session(&mut self, key: &AccountingSessionKey) -> Option<AccountingSession> {
        self.remove_session_indexes(key);
        self.sessions.remove(key)
    }

    fn index_session(&mut self, key: &AccountingSessionKey) {
        let Some(session) = self.sessions.get(key) else {
            return;
        };
        let index_entries = session_index_entries(key, session);

        if let Some(acct_session_id) = index_entries.nas_session_id {
            self.nas_session_keys_by_acct_session_id
                .entry(acct_session_id)
                .or_default()
                .insert(key.clone());
        }
        for lookup_key in index_entries.pending_lookup_keys {
            self.pending_keys_by_lookup
                .entry(lookup_key)
                .or_default()
                .insert(key.clone());
        }
        for lookup_key in index_entries.fallback_lookup_keys {
            self.fallback_keys_by_lookup
                .entry(lookup_key)
                .or_default()
                .insert(key.clone());
        }
    }

    fn remove_session_indexes(&mut self, key: &AccountingSessionKey) {
        let Some(session) = self.sessions.get(key) else {
            return;
        };
        let index_entries = session_index_entries(key, session);

        if let Some(acct_session_id) = index_entries.nas_session_id {
            remove_indexed_session_key(
                &mut self.nas_session_keys_by_acct_session_id,
                &acct_session_id,
                key,
            );
        }
        for lookup_key in index_entries.pending_lookup_keys {
            remove_indexed_session_key(&mut self.pending_keys_by_lookup, &lookup_key, key);
        }
        for lookup_key in index_entries.fallback_lookup_keys {
            remove_indexed_session_key(&mut self.fallback_keys_by_lookup, &lookup_key, key);
        }
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
#[derive(Clone, Debug, PartialEq)]
pub struct AccountingSession {
    /// Current lifecycle state for the session.
    pub state: AccountingSessionState,
    /// Latest decoded event data retained for this session.
    pub latest_event: AccountingEvent,
    /// NAS identities observed across retained events for this session.
    pub known_nas_identities: Vec<NasIdentity>,
    /// Rate profile resolved for this session, when one is available.
    pub resolved_rate: Option<ResolvedSessionRate>,
    /// Resolved in-memory shaped-device payload for this RADIUS session.
    pub resolved_shaped_device: Option<ShapedDevice>,
    /// Dynamic-circuit IDs this session has emitted and not yet removed.
    pub active_dynamic_circuit_ids: Vec<String>,
    /// Dynamic-circuit IDs retained for final stopped or expired diagnostics.
    pub diagnostic_circuit_ids: Vec<String>,
    /// Reasons this session cannot currently become a dynamic circuit.
    pub pending_reasons: Vec<PendingSessionReason>,
}

/// Dynamic-circuit activation counters tracked separately from packet accept/reject counts.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RadiusActivationCounters {
    /// Create intents emitted for shapeable Start or first Interim-Update events.
    pub create: u64,
    /// Update intents emitted for already-active Interim-Update events.
    pub update: u64,
    /// Remove intents emitted for Stop, stale, expiry, rekey, or no-longer-shapeable events.
    pub remove: u64,
    /// Retained sessions expired by TTL or stale grace cleanup.
    pub expiry: u64,
}

/// Listener packet counters tracked separately from dynamic-circuit activation counters.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RadiusPacketCounters {
    /// Verified packets accepted by the listener.
    pub accepted: u64,
    /// Packets rejected before accounting-session processing.
    pub rejected: u64,
}

/// Operator-facing activation state for one RADIUS accounting session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RadiusActivationDiagnosticState {
    /// The retained session is active and currently has a resolved dynamic-circuit payload.
    Active,
    /// The retained session is active but lacks required data to create a circuit.
    Pending,
    /// The retained session has received Acct-Status-Type Stop.
    Stopped,
    /// The retained session was marked stale by Accounting-On or Accounting-Off.
    Stale(NasResetStatus),
    /// The retained session was removed by TTL or stale-grace expiry.
    Expired,
    /// A resolved dynamic-circuit request failed after it was emitted to the caller.
    ApplyFailed,
}

/// Diagnostic snapshot for one RADIUS dynamic-circuit activation outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RadiusActivationDiagnostic {
    /// Stable session key used by the in-memory store.
    pub session_key: AccountingSessionKey,
    /// Best-known Acct-Session-Id for troubleshooting.
    pub acct_session_id: Option<String>,
    /// Best-known NAS identity for troubleshooting.
    pub nas: Option<NasIdentity>,
    /// Dynamic-circuit IDs associated with this session.
    pub circuit_ids: Vec<String>,
    /// Current activation outcome.
    pub state: RadiusActivationDiagnosticState,
    /// Reasons a pending session cannot be shaped yet.
    pub pending_reasons: Vec<PendingSessionReason>,
    /// Sanitized apply-failure detail when `state` is `ApplyFailed`.
    pub apply_error: Option<String>,
}

impl RadiusActivationDiagnostic {
    /// Builds a diagnostic snapshot for a retained session.
    #[must_use]
    pub fn from_retained_session(key: &AccountingSessionKey, session: &AccountingSession) -> Self {
        Self::from_session_with_state(key, session, retained_activation_state(session), None)
    }

    /// Builds a diagnostic snapshot for a session removed by expiry cleanup.
    #[must_use]
    pub fn from_expired_session(key: &AccountingSessionKey, session: &AccountingSession) -> Self {
        Self::from_session_with_state(key, session, RadiusActivationDiagnosticState::Expired, None)
    }

    /// Builds a diagnostic snapshot for a dynamic-circuit apply failure.
    #[must_use]
    pub fn apply_failed(
        session_key: AccountingSessionKey,
        circuit_id: String,
        apply_error: String,
    ) -> Self {
        Self {
            acct_session_id: session_key.acct_session_id().map(ToString::to_string),
            nas: session_key.nas().cloned(),
            session_key,
            circuit_ids: vec![circuit_id],
            state: RadiusActivationDiagnosticState::ApplyFailed,
            pending_reasons: Vec::new(),
            apply_error: Some(apply_error),
        }
    }

    fn from_session_with_state(
        key: &AccountingSessionKey,
        session: &AccountingSession,
        state: RadiusActivationDiagnosticState,
        apply_error: Option<String>,
    ) -> Self {
        Self {
            session_key: key.clone(),
            acct_session_id: key
                .acct_session_id()
                .or(session.latest_event.acct_session_id.as_deref())
                .map(ToString::to_string),
            nas: key
                .nas()
                .cloned()
                .or_else(|| NasIdentitySet::from_event(&session.latest_event).primary()),
            circuit_ids: diagnostic_circuit_ids(key, session),
            state,
            pending_reasons: session.pending_reasons.clone(),
            apply_error,
        }
    }
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

    /// Returns the Acct-Session-Id associated with this key, when known.
    #[must_use]
    pub fn acct_session_id(&self) -> Option<&str> {
        match self {
            Self::NasSession {
                acct_session_id, ..
            } => Some(acct_session_id),
            Self::Pending { fingerprint } => fingerprint.acct_session_id.as_deref(),
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

/// Parent attachment metadata for a resolved dynamic circuit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DynamicCircuitParent {
    /// Human-readable parent node name from `network.json`.
    pub parent_node: String,
    /// Stable parent node identifier, when known.
    pub parent_node_id: Option<String>,
    /// Stable anchor node identifier, when known.
    pub anchor_node_id: Option<String>,
}

impl DynamicCircuitParent {
    /// Creates parent metadata from a parent node name.
    #[must_use]
    pub fn new(parent_node: impl Into<String>) -> Self {
        Self {
            parent_node: parent_node.into(),
            parent_node_id: None,
            anchor_node_id: None,
        }
    }

    /// Returns true when the parent metadata can populate a `ShapedDevice` parent.
    #[must_use]
    pub fn has_parent_node(&self) -> bool {
        !self.parent_node.trim().is_empty()
    }
}

/// Current dynamic-circuit mapping state for a session event.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum DynamicCircuitMapping {
    /// Subscriber IP, rate, and identity can map to a dynamic circuit when
    /// `matched_shaped_device` supplies valid parent attachment metadata.
    ///
    /// This variant is used when another resolved object, such as a
    /// `ShapedDevices.csv` MAC match, carries the parent attachment fields. Use
    /// `ReadyWithParent` for fallback identities that do not have a matched
    /// shaped-device row.
    MatchedShapedDevice,
    /// Subscriber IP, rate, identity, and explicit parent metadata can map to a dynamic circuit.
    ReadyWithParent(DynamicCircuitParent),
    /// No parent node mapping is known yet.
    #[default]
    MissingParent,
    /// More than one parent or subscriber mapping matches the event.
    Ambiguous,
    /// MAC matching is enabled, but no shaped-device row matches the event.
    NoMacMatch,
    /// MAC matching is enabled, and more than one shaped-device row matches the event.
    AmbiguousMacMatch,
    /// Configured ShapedDevices identity matching found no matching row.
    NoIdentityMatch,
    /// Configured ShapedDevices identity matching found duplicate rows.
    AmbiguousIdentityMatch,
}

/// Dynamic-circuit metadata resolved for one accounting event.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DynamicCircuitResolution {
    /// Parent or ShapedDevices mapping state for the event.
    pub mapping: DynamicCircuitMapping,
    /// Rate profiles available outside the accounting packet.
    pub rate_sources: SessionRateSources,
    /// Shaped-device row supplying circuit metadata, when MAC matching found one.
    pub matched_shaped_device: Option<ShapedDevice>,
}

impl DynamicCircuitResolution {
    /// Builds a resolution from a shaped-devices MAC match result.
    ///
    /// Unique ShapedDevices matches use the matched row as the authoritative
    /// account-plan source. The fallback profile is kept only for no-match and
    /// ambiguous-match pending diagnostics, and does not mask invalid rates in
    /// a matched ShapedDevices row. No-match and ambiguous-match results stay
    /// pending with MAC-specific reasons; callers with another parent source
    /// should construct a `ReadyWithParent` resolution directly.
    ///
    /// Side effects: none. The match result is converted in memory only.
    #[must_use]
    pub fn from_shaped_devices_mac_match(
        mac_match: ShapedDevicesMacMatch,
        fallback_profile: Option<SessionRateProfile>,
    ) -> Self {
        Self::from_shaped_devices_match_with_fallback_parent(mac_match, fallback_profile, None)
    }

    /// Builds a resolution from a ShapedDevices identity match with an optional fallback parent.
    ///
    /// Side effects: none.
    #[must_use]
    pub fn from_shaped_devices_match_with_fallback_parent(
        mac_match: ShapedDevicesMacMatch,
        fallback_profile: Option<SessionRateProfile>,
        fallback_parent: Option<DynamicCircuitParent>,
    ) -> Self {
        match mac_match {
            ShapedDevicesMacMatch::Unique(device) => {
                let shaped_device_profile = session_rate_profile_from_shaped_device(&device);
                Self {
                    mapping: DynamicCircuitMapping::MatchedShapedDevice,
                    rate_sources: SessionRateSources {
                        shaped_device_profile,
                        fallback_profile: None,
                    },
                    matched_shaped_device: Some(*device),
                }
            }
            ShapedDevicesMacMatch::NoMatch => Self {
                mapping: fallback_parent
                    .map(DynamicCircuitMapping::ReadyWithParent)
                    .unwrap_or(DynamicCircuitMapping::NoMacMatch),
                rate_sources: SessionRateSources {
                    shaped_device_profile: None,
                    fallback_profile,
                },
                matched_shaped_device: None,
            },
            ShapedDevicesMacMatch::Ambiguous => Self {
                mapping: DynamicCircuitMapping::AmbiguousMacMatch,
                rate_sources: SessionRateSources {
                    shaped_device_profile: None,
                    fallback_profile,
                },
                matched_shaped_device: None,
            },
        }
    }

    /// Builds a resolution from username and/or MAC identity matching.
    ///
    /// Side effects: none.
    #[must_use]
    pub fn from_shaped_devices_identity_match(
        identity_match: ShapedDevicesMacMatch,
        fallback_profile: Option<SessionRateProfile>,
        fallback_parent: Option<DynamicCircuitParent>,
    ) -> Self {
        let mut resolution = Self::from_shaped_devices_match_with_fallback_parent(
            identity_match,
            fallback_profile,
            fallback_parent,
        );
        resolution.mapping = match resolution.mapping {
            DynamicCircuitMapping::NoMacMatch => DynamicCircuitMapping::NoIdentityMatch,
            DynamicCircuitMapping::AmbiguousMacMatch => {
                DynamicCircuitMapping::AmbiguousIdentityMatch
            }
            mapping => mapping,
        };
        resolution
    }
}

/// Complete speed profile resolved for a RADIUS accounting session.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SessionRateProfile {
    download_min_mbps: f32,
    upload_min_mbps: f32,
    download_max_mbps: f32,
    upload_max_mbps: f32,
}

impl SessionRateProfile {
    /// Creates a validated session rate profile.
    ///
    /// Side effects: none.
    pub fn new(
        download_min_mbps: f32,
        upload_min_mbps: f32,
        download_max_mbps: f32,
        upload_max_mbps: f32,
    ) -> Result<Self, SessionRateProfileError> {
        validate_rate_profile_mbps(
            download_min_mbps,
            upload_min_mbps,
            download_max_mbps,
            upload_max_mbps,
        )?;

        Ok(Self {
            download_min_mbps,
            upload_min_mbps,
            download_max_mbps,
            upload_max_mbps,
        })
    }

    /// Returns the minimum download rate in Mbps.
    #[must_use]
    pub const fn download_min_mbps(&self) -> f32 {
        self.download_min_mbps
    }

    /// Returns the minimum upload rate in Mbps.
    #[must_use]
    pub const fn upload_min_mbps(&self) -> f32 {
        self.upload_min_mbps
    }

    /// Returns the maximum download rate in Mbps.
    #[must_use]
    pub const fn download_max_mbps(&self) -> f32 {
        self.download_max_mbps
    }

    /// Returns the maximum upload rate in Mbps.
    #[must_use]
    pub const fn upload_max_mbps(&self) -> f32 {
        self.upload_max_mbps
    }

    fn from_packet_rate(rate_limit: &MikrotikRateLimit) -> Option<Self> {
        let upload_mbps = bits_per_second_to_mbps(rate_limit.upload_bps);
        let download_mbps = bits_per_second_to_mbps(rate_limit.download_bps);

        Self::new(download_mbps, upload_mbps, download_mbps, upload_mbps).ok()
    }
}

/// Candidate non-packet rate profiles for resolving a RADIUS accounting session.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SessionRateSources {
    /// Rate profile supplied by a `ShapedDevices.csv` MAC match.
    ///
    /// Used only when no usable rate was decoded from the accounting packet.
    pub shaped_device_profile: Option<SessionRateProfile>,
    /// Configured fallback profile.
    ///
    /// Used only when no decoded packet rate is usable and no unique ShapedDevices MAC match
    /// supplied the session metadata.
    pub fallback_profile: Option<SessionRateProfile>,
}

/// Rate source selected for a RADIUS accounting session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionRateSource {
    /// Rate decoded from the accounting packet.
    Packet,
    /// Rate supplied by matched `ShapedDevices.csv` metadata.
    ShapedDevice,
    /// Configured fallback profile used when no usable decoded packet or
    /// unique ShapedDevices MAC-match rate is available.
    Fallback,
}

/// Resolved rate profile and its source.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedSessionRate {
    /// Source that supplied the rate profile.
    pub source: SessionRateSource,
    /// Complete min/max speed profile.
    pub profile: SessionRateProfile,
}

/// Reason a retained session cannot currently become a dynamic circuit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PendingSessionReason {
    /// Acct-Session-Id is missing.
    MissingSessionId,
    /// No NAS identity attribute is present.
    MissingNasIdentity,
    /// No stable dynamic-circuit identity is available.
    MissingCircuitIdentity,
    /// No stable dynamic-device identity is available.
    MissingDeviceIdentity,
    /// No subscriber IPv4, IPv6, or prefix address is present.
    MissingIpAddress,
    /// No usable rate source is available.
    MissingRate,
    /// No parent node mapping is available.
    MissingParent,
    /// The event maps ambiguously to more than one parent or subscriber target.
    AmbiguousMapping,
    /// MAC matching is enabled, but no shaped-device MAC matched the event.
    NoMacMatch,
    /// MAC matching is enabled, and more than one shaped-device MAC matched the event.
    AmbiguousMacMatch,
    /// Configured ShapedDevices identity matching found no matching row.
    NoIdentityMatch,
    /// Configured ShapedDevices identity matching found duplicate rows.
    AmbiguousIdentityMatch,
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
        /// Session keys that transitioned from active or stopped to stale.
        newly_stale_session_keys: HashSet<AccountingSessionKey>,
        /// Session keys whose retained stale state was updated by this reset.
        stale_session_keys: HashSet<AccountingSessionKey>,
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

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum SessionLookupIndexKey {
    AcctSessionId(String),
    UserName(String),
    CallingStationId(String),
    NasPortId(String),
    NasPort(u32),
}

struct SessionIndexEntries {
    nas_session_id: Option<String>,
    pending_lookup_keys: Vec<SessionLookupIndexKey>,
    fallback_lookup_keys: Vec<SessionLookupIndexKey>,
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

fn session_index_entries(
    key: &AccountingSessionKey,
    session: &AccountingSession,
) -> SessionIndexEntries {
    let nas_session_id = match key {
        AccountingSessionKey::NasSession {
            acct_session_id, ..
        } => Some(acct_session_id.clone()),
        AccountingSessionKey::Pending { .. } => None,
    };
    let lookup_keys = session_lookup_index_keys(key, session);
    let (pending_lookup_keys, fallback_lookup_keys) = match key {
        AccountingSessionKey::Pending { .. } => (lookup_keys, Vec::new()),
        AccountingSessionKey::NasSession { .. } => (Vec::new(), lookup_keys),
    };

    SessionIndexEntries {
        nas_session_id,
        pending_lookup_keys,
        fallback_lookup_keys,
    }
}

fn session_lookup_index_keys(
    key: &AccountingSessionKey,
    session: &AccountingSession,
) -> Vec<SessionLookupIndexKey> {
    let mut lookup_keys = Vec::new();
    let fingerprint = session_fingerprint(key, session);
    push_fingerprint_lookup_keys(&mut lookup_keys, &fingerprint);
    lookup_keys
}

fn event_lookup_index_keys(
    event: &AccountingEvent,
    identities: &NasIdentitySet,
) -> Vec<SessionLookupIndexKey> {
    let mut lookup_keys = Vec::new();
    let fingerprint = PendingSessionFingerprint::from_event(event, identities.primary());
    push_fingerprint_lookup_keys(&mut lookup_keys, &fingerprint);
    lookup_keys
}

fn push_fingerprint_lookup_keys(
    lookup_keys: &mut Vec<SessionLookupIndexKey>,
    fingerprint: &PendingSessionFingerprint,
) {
    if let Some(acct_session_id) = &fingerprint.acct_session_id {
        push_unique_lookup_key(
            lookup_keys,
            SessionLookupIndexKey::AcctSessionId(acct_session_id.clone()),
        );
    }
    if let Some(user_name) = &fingerprint.user_name {
        push_unique_lookup_key(
            lookup_keys,
            SessionLookupIndexKey::UserName(user_name.clone()),
        );
    }
    if let Some(calling_station_id) = &fingerprint.calling_station_id {
        push_unique_lookup_key(
            lookup_keys,
            SessionLookupIndexKey::CallingStationId(calling_station_id.clone()),
        );
    }
    if let Some(nas_port_id) = &fingerprint.nas_port_id {
        push_unique_lookup_key(
            lookup_keys,
            SessionLookupIndexKey::NasPortId(nas_port_id.clone()),
        );
    }
    if let Some(nas_port) = fingerprint.nas_port {
        push_unique_lookup_key(lookup_keys, SessionLookupIndexKey::NasPort(nas_port));
    }
}

fn push_unique_lookup_key(
    lookup_keys: &mut Vec<SessionLookupIndexKey>,
    lookup_key: SessionLookupIndexKey,
) {
    if !lookup_keys.contains(&lookup_key) {
        lookup_keys.push(lookup_key);
    }
}

fn remove_indexed_session_key<K>(
    index: &mut HashMap<K, HashSet<AccountingSessionKey>>,
    index_key: &K,
    session_key: &AccountingSessionKey,
) where
    K: Eq + std::hash::Hash,
{
    let remove_empty_entry = if let Some(keys) = index.get_mut(index_key) {
        keys.remove(session_key);
        keys.is_empty()
    } else {
        false
    };
    if remove_empty_entry {
        index.remove(index_key);
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

fn retained_activation_state(session: &AccountingSession) -> RadiusActivationDiagnosticState {
    match session.state {
        AccountingSessionState::Active if session.pending_reasons.is_empty() => {
            RadiusActivationDiagnosticState::Active
        }
        AccountingSessionState::Active => RadiusActivationDiagnosticState::Pending,
        AccountingSessionState::Stopped => RadiusActivationDiagnosticState::Stopped,
        AccountingSessionState::Stale(reset) => RadiusActivationDiagnosticState::Stale(reset),
    }
}

fn diagnostic_circuit_ids(key: &AccountingSessionKey, session: &AccountingSession) -> Vec<String> {
    let mut circuit_ids = Vec::new();
    if let Some(device) = &session.resolved_shaped_device {
        push_unique_text(&mut circuit_ids, &device.circuit_id);
    }
    for circuit_id in &session.active_dynamic_circuit_ids {
        push_unique_text(&mut circuit_ids, circuit_id);
    }
    for circuit_id in &session.diagnostic_circuit_ids {
        push_unique_text(&mut circuit_ids, circuit_id);
    }
    if circuit_ids.is_empty()
        && let Some(circuit_id) = key.dynamic_circuit_id()
    {
        push_unique_text(&mut circuit_ids, &circuit_id);
    }
    circuit_ids
}

fn push_unique_text(values: &mut Vec<String>, value: &str) {
    if value.trim().is_empty() || values.iter().any(|existing| existing == value) {
        return;
    }
    values.push(value.to_string());
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

fn resolved_session_rate(
    event: &AccountingEvent,
    rate_sources: SessionRateSources,
) -> Option<ResolvedSessionRate> {
    if let Some(profile) = event
        .mikrotik_rate_limits
        .iter()
        .find_map(SessionRateProfile::from_packet_rate)
    {
        return Some(ResolvedSessionRate {
            source: SessionRateSource::Packet,
            profile,
        });
    }

    if let Some(profile) = rate_sources.shaped_device_profile {
        return Some(ResolvedSessionRate {
            source: SessionRateSource::ShapedDevice,
            profile,
        });
    }

    rate_sources
        .fallback_profile
        .map(|profile| ResolvedSessionRate {
            source: SessionRateSource::Fallback,
            profile,
        })
}

fn session_rate_profile_from_shaped_device(device: &ShapedDevice) -> Option<SessionRateProfile> {
    SessionRateProfile::new(
        device.download_min_mbps,
        device.upload_min_mbps,
        device.download_max_mbps,
        device.upload_max_mbps,
    )
    .ok()
}

struct SessionDynamicMetadata {
    resolved_rate: Option<ResolvedSessionRate>,
    resolved_shaped_device: Option<ShapedDevice>,
    pending_reasons: Vec<PendingSessionReason>,
}

impl SessionDynamicMetadata {
    fn inactive() -> Self {
        Self {
            resolved_rate: None,
            resolved_shaped_device: None,
            pending_reasons: Vec::new(),
        }
    }
}

fn session_dynamic_metadata(
    key: &AccountingSessionKey,
    event: &AccountingEvent,
    resolution: DynamicCircuitResolution,
) -> SessionDynamicMetadata {
    let subscriber_ips = SubscriberIpMetadata::from_event(event);
    let resolved_rate = resolved_session_rate(event, resolution.rate_sources);
    let parent = resolved_parent_attachment(
        resolution.matched_shaped_device.as_ref(),
        &resolution.mapping,
    );
    let has_circuit_identity =
        circuit_identity_available(key, resolution.matched_shaped_device.as_ref(), event);
    let has_device_identity =
        device_identity_available(key, resolution.matched_shaped_device.as_ref(), event);
    let pending_reasons = pending_reasons(
        event,
        &resolution.mapping,
        resolved_rate,
        &subscriber_ips,
        parent.as_ref(),
        has_circuit_identity,
        has_device_identity,
    );
    let resolved_shaped_device = pending_reasons
        .is_empty()
        .then(|| {
            resolved_shaped_device(
                key,
                event,
                resolution.matched_shaped_device.as_ref(),
                resolved_rate,
                &subscriber_ips,
                parent.as_ref(),
            )
        })
        .flatten();

    SessionDynamicMetadata {
        resolved_rate,
        resolved_shaped_device,
        pending_reasons,
    }
}

struct SubscriberIpMetadata {
    ipv4: Vec<(Ipv4Addr, u32)>,
    ipv6: Vec<(Ipv6Addr, u32)>,
}

impl SubscriberIpMetadata {
    fn from_event(event: &AccountingEvent) -> Self {
        Self {
            ipv4: subscriber_ipv4_cidrs(event),
            ipv6: subscriber_ipv6_cidrs(event),
        }
    }

    fn is_empty(&self) -> bool {
        self.ipv4.is_empty() && self.ipv6.is_empty()
    }
}

fn resolved_shaped_device(
    key: &AccountingSessionKey,
    event: &AccountingEvent,
    matched_shaped_device: Option<&ShapedDevice>,
    resolved_rate: Option<ResolvedSessionRate>,
    subscriber_ips: &SubscriberIpMetadata,
    parent: Option<&DynamicCircuitParent>,
) -> Option<ShapedDevice> {
    let resolved_rate = resolved_rate?;
    let parent = parent?;
    let mut device = if let Some(matched_shaped_device) = matched_shaped_device {
        matched_shaped_device.clone()
    } else {
        default_resolved_shaped_device(key, event, parent)?
    };
    apply_parent_attachment(&mut device, parent);
    device.ipv4 = subscriber_ips.ipv4.clone();
    device.ipv6 = subscriber_ips.ipv6.clone();
    apply_session_rate_profile(&mut device, resolved_rate.profile);
    device.refresh_hashes();
    Some(device)
}

fn resolved_parent_attachment(
    matched_shaped_device: Option<&ShapedDevice>,
    mapping: &DynamicCircuitMapping,
) -> Option<DynamicCircuitParent> {
    matched_shaped_device
        .and_then(parent_from_shaped_device)
        .or_else(|| parent_from_mapping(mapping))
}

fn parent_from_shaped_device(device: &ShapedDevice) -> Option<DynamicCircuitParent> {
    // lqos_bakery dynamic-circuit validation still requires parent_node.
    let parent = DynamicCircuitParent {
        parent_node: device.parent_node.clone(),
        parent_node_id: device.parent_node_id.clone(),
        anchor_node_id: device.anchor_node_id.clone(),
    };
    parent.has_parent_node().then_some(parent)
}

fn parent_from_mapping(mapping: &DynamicCircuitMapping) -> Option<DynamicCircuitParent> {
    let DynamicCircuitMapping::ReadyWithParent(parent) = mapping else {
        return None;
    };
    parent.has_parent_node().then(|| parent.clone())
}

fn circuit_identity_available(
    key: &AccountingSessionKey,
    matched_shaped_device: Option<&ShapedDevice>,
    event: &AccountingEvent,
) -> bool {
    if let Some(device) = matched_shaped_device {
        return !device.circuit_id.trim().is_empty();
    }

    stable_subscriber_circuit_id(key, event).is_some()
}

fn device_identity_available(
    key: &AccountingSessionKey,
    matched_shaped_device: Option<&ShapedDevice>,
    event: &AccountingEvent,
) -> bool {
    if let Some(device) = matched_shaped_device {
        return !device.device_id.trim().is_empty();
    }

    stable_subscriber_circuit_id(key, event).is_some()
}

fn default_resolved_shaped_device(
    key: &AccountingSessionKey,
    event: &AccountingEvent,
    parent: &DynamicCircuitParent,
) -> Option<ShapedDevice> {
    let circuit_id = stable_subscriber_circuit_id(key, event)?;
    let device_id = circuit_id.clone();
    Some(ShapedDevice {
        circuit_name: default_circuit_name(event, &circuit_id),
        device_name: default_device_name(event, &device_id),
        circuit_id,
        device_id,
        parent_node: parent.parent_node.clone(),
        parent_node_id: parent.parent_node_id.clone(),
        anchor_node_id: parent.anchor_node_id.clone(),
        mac: non_empty_str(&event.calling_station_id)
            .unwrap_or_default()
            .to_string(),
        comment: String::new(),
        sqm_override: None,
        circuit_hash: 0,
        device_hash: 0,
        parent_hash: 0,
        ..ShapedDevice::default()
    })
}

/// Returns the stable customer circuit ID for an unmatched dynamic RADIUS session.
///
/// This deliberately does not use the RADIUS accounting session ID: a subscriber
/// reconnects with a new session ID, but must retain the same circuit identity for
/// Insight and exported historical data. The NAS identity scopes the subscriber
/// identity so identical usernames on separate BNGs cannot collide. Username is
/// preferred because it is the configured subscriber identity for PPPoE and
/// DHCP-RADIUS; otherwise a normalized Calling-Station-Id is used.
fn stable_subscriber_circuit_id(
    key: &AccountingSessionKey,
    event: &AccountingEvent,
) -> Option<String> {
    let nas = key.nas()?.circuit_id_component();
    if let Some(username) = event
        .user_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(format!(
            "radius:{nas}:username:{}",
            hex_component(username.as_bytes())
        ));
    }

    let calling_station_id = non_empty_str(&event.calling_station_id)?;
    let identity = crate::mac_match::normalize_radius_mac(calling_station_id)
        .unwrap_or_else(|| calling_station_id.to_string());
    Some(format!(
        "radius:{nas}:calling-station:{}",
        hex_component(identity.as_bytes())
    ))
}

fn apply_parent_attachment(device: &mut ShapedDevice, parent: &DynamicCircuitParent) {
    device.parent_node = parent.parent_node.clone();
    device.parent_node_id = parent.parent_node_id.clone();
    device.anchor_node_id = parent.anchor_node_id.clone();
}

fn default_circuit_name(event: &AccountingEvent, fallback: &str) -> String {
    non_empty_str(&event.user_name)
        .or_else(|| non_empty_str(&event.calling_station_id))
        .or_else(|| non_empty_str(&event.acct_session_id))
        .unwrap_or(fallback)
        .to_string()
}

fn default_device_name(event: &AccountingEvent, fallback: &str) -> String {
    non_empty_str(&event.calling_station_id)
        .or_else(|| non_empty_str(&event.user_name))
        .or_else(|| non_empty_str(&event.acct_session_id))
        .unwrap_or(fallback)
        .to_string()
}

fn apply_session_rate_profile(device: &mut ShapedDevice, profile: SessionRateProfile) {
    device.download_min_mbps = profile.download_min_mbps();
    device.upload_min_mbps = profile.upload_min_mbps();
    device.download_max_mbps = profile.download_max_mbps();
    device.upload_max_mbps = profile.upload_max_mbps();
}

fn bits_per_second_to_mbps(bits_per_second: u64) -> f32 {
    bits_per_second as f32 / 1_000_000.0
}

fn pending_reasons(
    event: &AccountingEvent,
    mapping: &DynamicCircuitMapping,
    resolved_rate: Option<ResolvedSessionRate>,
    subscriber_ips: &SubscriberIpMetadata,
    parent: Option<&DynamicCircuitParent>,
    has_circuit_identity: bool,
    has_device_identity: bool,
) -> Vec<PendingSessionReason> {
    let mut reasons = Vec::new();

    let missing_session_id = event.acct_session_id.as_ref().is_none_or(String::is_empty);
    if missing_session_id {
        reasons.push(PendingSessionReason::MissingSessionId);
    }
    let missing_nas_identity = NasIdentitySet::from_event(event).is_empty();
    if missing_nas_identity {
        reasons.push(PendingSessionReason::MissingNasIdentity);
    }
    if !has_circuit_identity && !missing_session_id && !missing_nas_identity {
        reasons.push(PendingSessionReason::MissingCircuitIdentity);
    }
    if !has_device_identity && !missing_session_id && !missing_nas_identity {
        reasons.push(PendingSessionReason::MissingDeviceIdentity);
    }
    if subscriber_ips.is_empty() {
        reasons.push(PendingSessionReason::MissingIpAddress);
    }
    if resolved_rate.is_none() {
        reasons.push(PendingSessionReason::MissingRate);
    }
    if parent.is_none()
        && matches!(
            mapping,
            DynamicCircuitMapping::MatchedShapedDevice
                | DynamicCircuitMapping::ReadyWithParent(_)
                | DynamicCircuitMapping::MissingParent
        )
    {
        reasons.push(PendingSessionReason::MissingParent);
    }
    match mapping {
        DynamicCircuitMapping::MatchedShapedDevice
        | DynamicCircuitMapping::ReadyWithParent(_)
        | DynamicCircuitMapping::MissingParent => {}
        DynamicCircuitMapping::Ambiguous => reasons.push(PendingSessionReason::AmbiguousMapping),
        DynamicCircuitMapping::NoMacMatch => reasons.push(PendingSessionReason::NoMacMatch),
        DynamicCircuitMapping::AmbiguousMacMatch => {
            reasons.push(PendingSessionReason::AmbiguousMacMatch);
        }
        DynamicCircuitMapping::NoIdentityMatch => {
            reasons.push(PendingSessionReason::NoIdentityMatch);
        }
        DynamicCircuitMapping::AmbiguousIdentityMatch => {
            reasons.push(PendingSessionReason::AmbiguousIdentityMatch);
        }
    }

    reasons
}

fn subscriber_ipv4_cidrs(event: &AccountingEvent) -> Vec<(Ipv4Addr, u32)> {
    let mut ipv4_cidrs = Vec::new();
    if let Some(framed_ip_address) = event.framed_ip_address {
        let prefix_len = event
            .framed_ip_netmask
            .and_then(ipv4_netmask_prefix_len)
            .filter(|prefix_len| *prefix_len > 0)
            .unwrap_or(32);
        if let Some(cidr) = subscriber_ipv4_cidr(framed_ip_address, prefix_len) {
            ipv4_cidrs.push(cidr);
        }
    }
    for framed_route in &event.framed_routes {
        if let Some(route_destination) = framed_route_destination(framed_route) {
            ipv4_cidrs.push(route_destination);
        }
    }
    ipv4_cidrs
}

fn subscriber_ipv6_cidrs(event: &AccountingEvent) -> Vec<(Ipv6Addr, u32)> {
    let mut ipv6_cidrs = Vec::new();
    if let Some(framed_ipv6_address) = event.framed_ipv6_address
        && let Some(cidr) = subscriber_ipv6_cidr(framed_ipv6_address, 128)
    {
        ipv6_cidrs.push(cidr);
    }
    ipv6_cidrs.extend(
        event.framed_ipv6_prefixes.iter().filter_map(|prefix| {
            subscriber_ipv6_cidr(prefix.address, u32::from(prefix.prefix_len))
        }),
    );
    ipv6_cidrs.extend(
        event.delegated_ipv6_prefixes.iter().filter_map(|prefix| {
            subscriber_ipv6_cidr(prefix.address, u32::from(prefix.prefix_len))
        }),
    );
    ipv6_cidrs
}

fn subscriber_ipv6_cidr(address: Ipv6Addr, prefix_len: u32) -> Option<(Ipv6Addr, u32)> {
    if prefix_len == 0 || prefix_len > 128 || address.is_unspecified() || address.is_multicast() {
        return None;
    }

    Some((address, prefix_len))
}

fn framed_route_destination(framed_route: &str) -> Option<(Ipv4Addr, u32)> {
    parse_ipv4_cidr(framed_route.split_whitespace().next()?)
}

fn parse_ipv4_cidr(raw_cidr: &str) -> Option<(Ipv4Addr, u32)> {
    let Some((address, prefix_len)) = raw_cidr.split_once('/') else {
        return subscriber_ipv4_cidr(raw_cidr.parse().ok()?, 32);
    };
    let address = address.parse().ok()?;
    let prefix_len = prefix_len.parse().ok()?;
    subscriber_ipv4_cidr(address, prefix_len)
}

fn subscriber_ipv4_cidr(address: Ipv4Addr, prefix_len: u32) -> Option<(Ipv4Addr, u32)> {
    if prefix_len == 0
        || prefix_len > 32
        || address.is_unspecified()
        || address.is_multicast()
        || address.octets() == [255, 255, 255, 255]
    {
        return None;
    }

    Some((address, prefix_len))
}

fn ipv4_netmask_prefix_len(netmask: Ipv4Addr) -> Option<u32> {
    let netmask_bits = u32::from(netmask);
    let prefix_len = netmask_bits.count_ones();
    let expected_netmask = if prefix_len == 0 {
        0
    } else {
        u32::MAX << (32 - prefix_len)
    };
    (netmask_bits == expected_netmask).then_some(prefix_len)
}

fn non_empty_text(value: &Option<String>) -> Option<String> {
    value.as_ref().filter(|value| !value.is_empty()).cloned()
}

fn non_empty_str(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|value| !value.trim().is_empty())
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
        push_active_dynamic_circuit_id(active_ids, id);
    }
}

fn merge_diagnostic_circuit_ids(diagnostic_ids: &mut Vec<String>, ids: Vec<String>) {
    for id in ids {
        push_unique_text(diagnostic_ids, &id);
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
    counters: &mut RadiusActivationCounters,
    sink: &mut impl DynamicCircuitCommandSink,
) {
    for circuit_id in std::mem::take(&mut session.active_dynamic_circuit_ids) {
        counters.remove += 1;
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
    current_circuit_id: &str,
    counters: &mut RadiusActivationCounters,
    sink: &mut impl DynamicCircuitCommandSink,
) {
    let active_ids = std::mem::take(&mut session.active_dynamic_circuit_ids);
    for circuit_id in active_ids {
        if circuit_id == current_circuit_id {
            push_active_dynamic_circuit_id(&mut session.active_dynamic_circuit_ids, circuit_id);
            continue;
        }
        counters.remove += 1;
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
