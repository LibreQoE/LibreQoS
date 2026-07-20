//! lqosd integration for the RADIUS accounting listener.

use lqos_bus::{BusReply, BusRequest, BusResponse};
use lqos_config::{
    Config, ConfigShapedDevices, RadiusAccountingClient, RadiusAccountingConfig,
    RadiusFallbackSpeedProfile,
};
use lqos_radius::{
    AccountingEvent, AccountingListenerOutcome, AccountingSessionKey, AccountingSessionState,
    AccountingSessionStore, AccountingSessionUpdate, DynamicCircuitCommandSink,
    DynamicCircuitIntent, DynamicCircuitMapping, DynamicCircuitParent, DynamicCircuitRemoval,
    DynamicCircuitResolution, DynamicCircuitUpsert, ListenerConfig, RadiusActivationDiagnostic,
    RadiusListener, RadiusPacketCounters, SessionRateProfile, SessionRateProfileError,
    SessionRateSources, ShapedDevicesMacMatcher, TrustedClientSource, TrustedRadiusClient,
    start_listener,
};
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{Sleep, sleep_until};
use tracing::{Level, debug, error, info, trace, warn};

const DYNAMIC_CIRCUIT_APPLICATION_TIMEOUT: Duration = Duration::from_secs(35);
const DYNAMIC_CIRCUIT_APPLICATION_QUEUE_CAPACITY: usize = 1024;
const RADIUS_RECENT_EXPIRED_DIAGNOSTIC_LIMIT: usize = 1024;
const RADIUS_APPLY_FAILED_DIAGNOSTIC_LIMIT: usize = 1024;
const DYNAMIC_CIRCUIT_BUS_FAILURE_DETAIL_LIMIT: usize = 240;

/// lqosd bus channel used to submit daemon-local requests without opening a Unix socket.
pub(crate) type DynamicCircuitBusSender = mpsc::Sender<(oneshot::Sender<BusReply>, BusRequest)>;

/// Starts the RADIUS accounting listener when it is enabled in configuration.
///
/// Side effects: reads configured shared-secret files, conditionally loads
/// `ShapedDevices.csv` for MAC matching during startup, binds the configured
/// UDP socket when enabled, and spawns a Tokio task to receive accounting
/// packets. When both dynamic-circuit safety gates are enabled, the spawned task
/// submits dynamic-circuit bus requests after sending Accounting-Response
/// packets; it does not write dynamic-circuit files directly or touch TC/XDP in
/// the UDP response path.
pub(crate) async fn start_configured_radius_accounting(
    config: Option<RadiusAccountingConfig>,
    config_snapshot: &Config,
    dynamic_circuit_bus_tx: DynamicCircuitBusSender,
) -> Result<Option<JoinHandle<()>>, RadiusAccountingStartupError> {
    let Some(runtime_config) = runtime_config_from_config(config, config_snapshot).await? else {
        return Ok(None);
    };

    let listener = start_listener(ListenerConfig {
        listen_addr: runtime_config.listen_addr,
    })
    .await
    .map_err(RadiusAccountingStartupError::Listener)?;
    let local_addr = listener
        .local_addr()
        .map_err(RadiusAccountingStartupError::Listener)?;
    info!(
        "RADIUS accounting listener started on {local_addr} with {} trusted client(s)",
        runtime_config.clients.len()
    );

    Ok(Some(tokio::spawn(run_radius_accounting_listener(
        listener,
        runtime_config,
        dynamic_circuit_bus_tx,
    ))))
}

async fn runtime_config_from_config(
    config: Option<RadiusAccountingConfig>,
    config_snapshot: &Config,
) -> Result<Option<RadiusAccountingRuntimeConfig>, RadiusAccountingStartupError> {
    let Some(config) = config else {
        return Ok(None);
    };
    if !config.enabled {
        return Ok(None);
    }

    let listen_addr = config
        .listen
        .ok_or(RadiusAccountingStartupError::MissingListen)?;
    let dynamic_circuit_application_enabled = config.dynamic_circuit_application.enabled;
    let apply_dynamic_circuits = dynamic_circuit_application_enabled
        && config_snapshot
            .dynamic_circuits
            .as_ref()
            .is_some_and(|dynamic_circuits| dynamic_circuits.enabled);
    let fallback_rate_profile = if apply_dynamic_circuits {
        session_rate_profile_from_config(config.fallback_speed_profile.as_ref())?
    } else {
        None
    };
    let fallback_parent = if apply_dynamic_circuits {
        fallback_parent_from_config(&config.dynamic_circuit_application)
    } else {
        None
    };
    let mac_matcher = if apply_dynamic_circuits
        && config
            .dynamic_circuit_application
            .match_shaped_devices_by_mac
    {
        Some(load_mac_matcher(config_snapshot)?)
    } else {
        None
    };
    let mut clients = Vec::with_capacity(config.clients.len());
    for (index, client) in config.clients.iter().enumerate() {
        clients.push(load_trusted_client(index, client).await?);
    }
    if clients.is_empty() {
        return Err(RadiusAccountingStartupError::NoClients);
    }

    Ok(Some(RadiusAccountingRuntimeConfig {
        listen_addr,
        clients,
        default_ttl: Duration::from_secs(config.default_ttl_seconds),
        stale_grace: Duration::from_secs(config.stale_grace_seconds),
        fallback_rate_profile,
        fallback_parent,
        mac_matcher,
        apply_dynamic_circuits,
    }))
}

fn load_mac_matcher(
    config_snapshot: &Config,
) -> Result<ShapedDevicesMacMatcher, RadiusAccountingStartupError> {
    let shaped_devices_path = ConfigShapedDevices::path_for_config(config_snapshot)
        .to_string_lossy()
        .into_owned();
    let shaped_devices =
        ConfigShapedDevices::load_for_config(config_snapshot).map_err(|source| {
            RadiusAccountingStartupError::ShapedDevicesLoad {
                path: shaped_devices_path,
                detail: source.to_string(),
            }
        })?;
    Ok(ShapedDevicesMacMatcher::from_devices(
        &shaped_devices.devices,
    ))
}

fn session_rate_profile_from_config(
    fallback_speed_profile: Option<&RadiusFallbackSpeedProfile>,
) -> Result<Option<SessionRateProfile>, RadiusAccountingStartupError> {
    let Some(profile) = fallback_speed_profile else {
        return Ok(None);
    };

    SessionRateProfile::new(
        profile.download_min_mbps,
        profile.upload_min_mbps,
        profile.download_max_mbps,
        profile.upload_max_mbps,
    )
    .map(Some)
    .map_err(RadiusAccountingStartupError::InvalidFallbackSpeedProfile)
}

fn fallback_parent_from_config(
    application_config: &lqos_config::RadiusDynamicCircuitApplicationConfig,
) -> Option<DynamicCircuitParent> {
    let parent_node = trimmed_config_text(application_config.fallback_parent_node.as_deref())?;
    Some(DynamicCircuitParent {
        parent_node,
        parent_node_id: trimmed_config_text(application_config.fallback_parent_node_id.as_deref()),
        anchor_node_id: trimmed_config_text(application_config.fallback_anchor_node_id.as_deref()),
    })
}

fn trimmed_config_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

async fn load_trusted_client(
    index: usize,
    client: &RadiusAccountingClient,
) -> Result<TrustedRadiusClient, RadiusAccountingStartupError> {
    let label = client_label(index, &client.name);
    let mut shared_secret =
        tokio::fs::read(client.secret_file.as_str())
            .await
            .map_err(|source| RadiusAccountingStartupError::SecretFileRead {
                label: label.clone(),
                source,
            })?;
    trim_line_endings(&mut shared_secret);
    if shared_secret.is_empty() {
        return Err(RadiusAccountingStartupError::EmptySecretFile { label });
    }

    TrustedRadiusClient::new(trusted_sources(index, client)?, shared_secret).map_err(|source| {
        RadiusAccountingStartupError::TrustedClient {
            label: client_label(index, &client.name),
            source,
        }
    })
}

fn trusted_sources(
    index: usize,
    client: &RadiusAccountingClient,
) -> Result<Vec<TrustedClientSource>, RadiusAccountingStartupError> {
    client
        .source
        .iter()
        .map(|source| {
            let network = source.network();
            TrustedClientSource::network(network.network_address(), network.netmask()).map_err(
                |source| RadiusAccountingStartupError::TrustedSource {
                    label: client_label(index, &client.name),
                    source,
                },
            )
        })
        .collect()
}

async fn run_radius_accounting_listener(
    listener: RadiusListener,
    runtime_config: RadiusAccountingRuntimeConfig,
    dynamic_circuit_bus_tx: DynamicCircuitBusSender,
) {
    let mut sessions = RadiusAccountingSessions::new_with_fallback_and_mac_matcher(
        runtime_config.default_ttl,
        runtime_config.stale_grace,
        runtime_config.fallback_rate_profile,
        runtime_config.fallback_parent,
        runtime_config.mac_matcher,
    );
    let mut applying_sink = dynamic_circuit_application_sink(
        runtime_config.apply_dynamic_circuits,
        dynamic_circuit_bus_tx,
    );
    let mut expiry_timer = RadiusExpiryTimer::new(&sessions, radius_accounting_now());

    loop {
        tokio::select! {
            outcome = listener.receive_next_verified(&runtime_config.clients) => {
                let now = radius_accounting_now();
                expire_due_before_packet(
                    &mut sessions,
                    &mut expiry_timer,
                    now,
                    &mut applying_sink,
                );
                match outcome {
                    Ok(outcome) => {
                        handle_listener_outcome_with_application_sink(
                            outcome,
                            &mut sessions,
                            &mut expiry_timer,
                            now,
                            &mut applying_sink,
                        );
                    }
                    Err(err) if listener_error_is_recoverable(&err) => {
                        warn!("RADIUS accounting listener packet handling failed: {err}");
                    }
                    Err(err) => {
                        error!("RADIUS accounting listener stopped: {err}");
                        return;
                    }
                }
            }
            _ = expiry_timer.sleep_mut() => {
                let now = radius_accounting_now();
                expire_due_after_timer_wake(
                    &mut sessions,
                    &mut expiry_timer,
                    now,
                    &mut applying_sink,
                );
            }
        }
    }
}

struct RadiusExpiryTimer {
    sleep: Pin<Box<Sleep>>,
    wake_at: Instant,
}

impl RadiusExpiryTimer {
    fn new(sessions: &RadiusAccountingSessions, now: Instant) -> Self {
        let wake_at = radius_expiry_wake_at(sessions, now);
        Self {
            sleep: Box::pin(sleep_until(tokio::time::Instant::from_std(wake_at))),
            wake_at,
        }
    }

    fn reset_from_sessions(&mut self, sessions: &RadiusAccountingSessions, now: Instant) {
        self.wake_at = radius_expiry_wake_at(sessions, now);
        self.sleep
            .as_mut()
            .reset(tokio::time::Instant::from_std(self.wake_at));
    }

    fn schedule_after_update(
        &mut self,
        sessions: &RadiusAccountingSessions,
        update: &AccountingSessionUpdate,
        now: Instant,
    ) {
        let changed_deadline = sessions.next_changed_expiry_deadline(update);
        let next_wake_at = changed_deadline
            .map(|deadline| deadline.min(now + sessions.expiry_check_interval()))
            .unwrap_or(self.wake_at);

        if next_wake_at < self.wake_at {
            self.wake_at = next_wake_at;
            self.sleep
                .as_mut()
                .reset(tokio::time::Instant::from_std(self.wake_at));
        }
    }

    fn is_due(&self, now: Instant) -> bool {
        now >= self.wake_at
    }

    fn sleep_mut(&mut self) -> Pin<&mut Sleep> {
        self.sleep.as_mut()
    }
}

fn radius_expiry_wake_at(sessions: &RadiusAccountingSessions, now: Instant) -> Instant {
    let next_cleanup = now + sessions.expiry_check_interval();
    sessions
        .next_expiry_deadline()
        .map_or(next_cleanup, |deadline| deadline.min(next_cleanup))
}

fn radius_accounting_now() -> Instant {
    tokio::time::Instant::now().into_std()
}

fn expire_due_before_packet(
    sessions: &mut RadiusAccountingSessions,
    expiry_timer: &mut RadiusExpiryTimer,
    now: Instant,
    applying_sink: &mut Option<ApplyingDynamicCircuitSink>,
) {
    if expiry_timer.is_due(now) {
        expire_due_and_log_with_application_sink(sessions, now, applying_sink);
        expiry_timer.reset_from_sessions(sessions, now);
    }
}

fn expire_due_after_timer_wake(
    sessions: &mut RadiusAccountingSessions,
    expiry_timer: &mut RadiusExpiryTimer,
    now: Instant,
    applying_sink: &mut Option<ApplyingDynamicCircuitSink>,
) {
    expire_due_and_log_with_application_sink(sessions, now, applying_sink);
    expiry_timer.reset_from_sessions(sessions, now);
}

fn dynamic_circuit_application_sink(
    apply_dynamic_circuits: bool,
    dynamic_circuit_bus_tx: DynamicCircuitBusSender,
) -> Option<ApplyingDynamicCircuitSink> {
    apply_dynamic_circuits.then(|| ApplyingDynamicCircuitSink::new(dynamic_circuit_bus_tx))
}

fn handle_listener_outcome_with_application_sink(
    outcome: AccountingListenerOutcome,
    sessions: &mut RadiusAccountingSessions,
    expiry_timer: &mut RadiusExpiryTimer,
    now: Instant,
    applying_sink: &mut Option<ApplyingDynamicCircuitSink>,
) {
    match outcome {
        AccountingListenerOutcome::Accepted(accepted) => {
            sessions.record_packet_accepted();
            let event = AccountingEvent::from_verified(&accepted.request);
            handle_accounting_event_with_application_sink_and_expiry_schedule(
                event,
                sessions,
                expiry_timer,
                now,
                applying_sink,
                AccountingPacketLogContext {
                    peer: accepted.peer,
                    received_len: accepted.received_len,
                    response_len: accepted.response_len,
                },
            );
        }
        AccountingListenerOutcome::RejectedSource { peer, received_len } => {
            sessions.record_packet_rejected();
            warn!(
                peer = %peer,
                received_len,
                "rejected RADIUS accounting packet from untrusted source"
            );
        }
        AccountingListenerOutcome::RejectedAmbiguousSource { peer, received_len } => {
            sessions.record_packet_rejected();
            warn!(
                peer = %peer,
                received_len,
                "rejected RADIUS accounting packet matching multiple trusted clients"
            );
        }
        AccountingListenerOutcome::RejectedPacket {
            peer,
            received_len,
            source,
        } => {
            sessions.record_packet_rejected();
            warn!(
                peer = %peer,
                received_len,
                error = %source,
                "rejected RADIUS accounting packet"
            );
        }
    }
}

fn handle_accounting_event_with_application_sink_and_expiry_schedule(
    event: AccountingEvent,
    sessions: &mut RadiusAccountingSessions,
    expiry_timer: &mut RadiusExpiryTimer,
    now: Instant,
    applying_sink: &mut Option<ApplyingDynamicCircuitSink>,
    log_context: AccountingPacketLogContext,
) {
    let update = {
        let mut command_sink = selected_dynamic_circuit_sink(applying_sink);
        handle_accounting_event_with_command_sink(
            event,
            sessions,
            &mut command_sink,
            now,
            log_context.peer,
            log_context.received_len,
            log_context.response_len,
        )
    };
    expiry_timer.schedule_after_update(sessions, &update, now);
    trace_activation_diagnostics(sessions, applying_sink.as_ref());
}

#[derive(Clone, Copy)]
struct AccountingPacketLogContext {
    peer: SocketAddr,
    received_len: usize,
    response_len: usize,
}

fn expire_due_with_application_sink(
    sessions: &mut RadiusAccountingSessions,
    now: Instant,
    applying_sink: &mut Option<ApplyingDynamicCircuitSink>,
) -> usize {
    let mut command_sink = selected_dynamic_circuit_sink(applying_sink);
    sessions.expire_due_with_command_sink(now, &mut command_sink)
}

fn expire_due_and_log_with_application_sink(
    sessions: &mut RadiusAccountingSessions,
    now: Instant,
    applying_sink: &mut Option<ApplyingDynamicCircuitSink>,
) {
    let expired = expire_due_with_application_sink(sessions, now, applying_sink);
    if expired > 0 {
        debug!(expired, "expired RADIUS accounting session(s)");
    }
    trace_activation_diagnostics(sessions, applying_sink.as_ref());
}

fn trace_activation_diagnostics(
    sessions: &RadiusAccountingSessions,
    applying_sink: Option<&ApplyingDynamicCircuitSink>,
) {
    if !tracing::enabled!(Level::TRACE) {
        return;
    }
    let counters = sessions.activation_counters();
    let packet_counters = sessions.packet_counters();
    let diagnostics = applying_sink.map_or_else(
        || sessions.activation_diagnostics(),
        |sink| sink.activation_diagnostics(sessions),
    );
    trace!(
        ?counters,
        ?packet_counters,
        diagnostic_count = diagnostics.len(),
        "retained RADIUS activation diagnostic snapshot"
    );
    for diagnostic in diagnostics {
        trace!(?diagnostic, "retained RADIUS activation diagnostic");
    }
}

fn push_limited<T>(items: &mut VecDeque<T>, item: T, limit: usize) {
    if limit == 0 {
        return;
    }
    items.push_back(item);
    while items.len() > limit {
        items.pop_front();
    }
}

fn next_update_sequence(next_sequence: &mut u64) -> u64 {
    let sequence = *next_sequence;
    *next_sequence = next_sequence.wrapping_add(1);
    sequence
}

enum SelectedDynamicCircuitSink<'a> {
    Applying(&'a mut ApplyingDynamicCircuitSink),
    Deferred(DeferredDynamicCircuitSink),
}

impl DynamicCircuitCommandSink for SelectedDynamicCircuitSink<'_> {
    fn emit(&mut self, intent: DynamicCircuitIntent) {
        match self {
            Self::Applying(sink) => sink.emit(intent),
            Self::Deferred(sink) => sink.emit(intent),
        }
    }
}

fn selected_dynamic_circuit_sink(
    applying_sink: &mut Option<ApplyingDynamicCircuitSink>,
) -> SelectedDynamicCircuitSink<'_> {
    applying_sink.as_mut().map_or_else(
        || SelectedDynamicCircuitSink::Deferred(DeferredDynamicCircuitSink::application_disabled()),
        SelectedDynamicCircuitSink::Applying,
    )
}

#[cfg(test)]
fn handle_accounting_event(
    event: AccountingEvent,
    sessions: &mut RadiusAccountingSessions,
    now: Instant,
    peer: SocketAddr,
    received_len: usize,
    response_len: usize,
) {
    let mut command_sink = DeferredDynamicCircuitSink::application_disabled();
    handle_accounting_event_with_command_sink(
        event,
        sessions,
        &mut command_sink,
        now,
        peer,
        received_len,
        response_len,
    );
}

fn handle_accounting_event_with_command_sink(
    event: AccountingEvent,
    sessions: &mut RadiusAccountingSessions,
    command_sink: &mut impl DynamicCircuitCommandSink,
    now: Instant,
    peer: SocketAddr,
    received_len: usize,
    response_len: usize,
) -> AccountingSessionUpdate {
    let status = event.status_type;
    let update = sessions.apply_event_with_command_sink(event, now, command_sink);
    debug!(
        peer = %peer,
        received_len,
        response_len,
        ?status,
        ?update,
        "accepted RADIUS accounting packet"
    );
    update
}

fn listener_error_is_recoverable(err: &lqos_radius::ListenerError) -> bool {
    matches!(err, lqos_radius::ListenerError::Send { .. })
}

struct RadiusAccountingSessions {
    store: AccountingSessionStore,
    updated_at: HashMap<AccountingSessionKey, Instant>,
    update_sequence_by_key: HashMap<AccountingSessionKey, u64>,
    next_update_sequence: u64,
    activation_diagnostics_by_key: HashMap<AccountingSessionKey, RadiusActivationDiagnostic>,
    recent_expired_activation_diagnostics: VecDeque<RadiusActivationDiagnostic>,
    packet_counters: RadiusPacketCounters,
    default_ttl: Duration,
    stale_grace: Duration,
    fallback_rate_profile: Option<SessionRateProfile>,
    fallback_parent: Option<DynamicCircuitParent>,
    mac_matcher: Option<ShapedDevicesMacMatcher>,
}

impl RadiusAccountingSessions {
    #[cfg(test)]
    fn new(default_ttl: Duration, stale_grace: Duration) -> Self {
        Self::new_with_fallback_and_mac_matcher(default_ttl, stale_grace, None, None, None)
    }

    fn new_with_fallback_and_mac_matcher(
        default_ttl: Duration,
        stale_grace: Duration,
        fallback_rate_profile: Option<SessionRateProfile>,
        fallback_parent: Option<DynamicCircuitParent>,
        mac_matcher: Option<ShapedDevicesMacMatcher>,
    ) -> Self {
        Self {
            store: AccountingSessionStore::new(),
            updated_at: HashMap::new(),
            update_sequence_by_key: HashMap::new(),
            next_update_sequence: 0,
            activation_diagnostics_by_key: HashMap::new(),
            recent_expired_activation_diagnostics: VecDeque::new(),
            packet_counters: RadiusPacketCounters::default(),
            default_ttl,
            stale_grace,
            fallback_rate_profile,
            fallback_parent,
            mac_matcher,
        }
    }

    #[cfg(test)]
    fn apply_event(&mut self, event: AccountingEvent, now: Instant) -> AccountingSessionUpdate {
        let mut command_sink = DeferredDynamicCircuitSink::application_disabled();
        self.apply_event_with_command_sink(event, now, &mut command_sink)
    }

    fn apply_event_with_command_sink(
        &mut self,
        event: AccountingEvent,
        now: Instant,
        command_sink: &mut impl DynamicCircuitCommandSink,
    ) -> AccountingSessionUpdate {
        let update = if let Some(mac_matcher) = &self.mac_matcher {
            self.store
                .apply_event_with_shaped_devices_mac_matcher_and_commands(
                    event,
                    mac_matcher,
                    self.fallback_rate_profile,
                    command_sink,
                )
        } else {
            let mapping = self
                .fallback_parent
                .clone()
                .map(DynamicCircuitMapping::ReadyWithParent)
                .unwrap_or(DynamicCircuitMapping::MissingParent);
            self.store
                .apply_event_with_dynamic_circuit_resolution_and_commands(
                    event,
                    DynamicCircuitResolution {
                        mapping,
                        rate_sources: SessionRateSources {
                            shaped_device_profile: None,
                            fallback_profile: self.fallback_rate_profile,
                        },
                        matched_shaped_device: None,
                    },
                    command_sink,
                )
        };
        self.record_update(&update, now);
        if self.updated_at.len() > self.store.len() {
            self.prune_removed_sessions();
            self.prune_retained_activation_diagnostics();
        }
        self.record_activation_diagnostics_for_update(&update);
        update
    }

    fn activation_counters(&self) -> lqos_radius::RadiusActivationCounters {
        self.store.activation_counters()
    }

    fn packet_counters(&self) -> RadiusPacketCounters {
        self.packet_counters
    }

    fn record_packet_accepted(&mut self) {
        self.packet_counters.accepted += 1;
    }

    fn record_packet_rejected(&mut self) {
        self.packet_counters.rejected += 1;
    }

    fn activation_diagnostics(&self) -> Vec<RadiusActivationDiagnostic> {
        self.activation_diagnostics_by_key
            .values()
            .chain(self.recent_expired_activation_diagnostics.iter())
            .cloned()
            .collect()
    }

    fn record_activation_diagnostics_for_update(&mut self, update: &AccountingSessionUpdate) {
        match update {
            AccountingSessionUpdate::SessionUpdated { key, .. } => {
                self.record_retained_activation_diagnostic(key);
            }
            AccountingSessionUpdate::NasSessionsMarkedStale {
                stale_session_keys, ..
            } => {
                for key in stale_session_keys {
                    self.record_retained_activation_diagnostic(key);
                }
            }
            AccountingSessionUpdate::Ignored { .. } => {}
        }
    }

    fn record_retained_activation_diagnostic(&mut self, key: &AccountingSessionKey) {
        let Some(session) = self.store.session(key) else {
            self.activation_diagnostics_by_key.remove(key);
            return;
        };
        let diagnostic = RadiusActivationDiagnostic::from_retained_session(key, session);
        self.remove_recent_expired_activation_diagnostic(key);
        self.activation_diagnostics_by_key
            .insert(key.clone(), diagnostic);
    }

    fn remove_recent_expired_activation_diagnostic(&mut self, key: &AccountingSessionKey) {
        if self.recent_expired_activation_diagnostics.is_empty() {
            return;
        }
        self.recent_expired_activation_diagnostics
            .retain(|diagnostic| &diagnostic.session_key != key);
    }

    fn prune_retained_activation_diagnostics(&mut self) {
        let store = &self.store;
        self.activation_diagnostics_by_key
            .retain(|key, _| store.session(key).is_some());
    }

    fn record_update(&mut self, update: &AccountingSessionUpdate, now: Instant) {
        match update {
            AccountingSessionUpdate::SessionUpdated { key, .. } => {
                self.record_session_update_time(key, now);
            }
            AccountingSessionUpdate::NasSessionsMarkedStale {
                newly_stale_session_keys,
                ..
            } => {
                for key in newly_stale_session_keys {
                    self.updated_at.insert(key.clone(), now);
                    if !self.update_sequence_by_key.contains_key(key) {
                        let sequence = next_update_sequence(&mut self.next_update_sequence);
                        self.update_sequence_by_key.insert(key.clone(), sequence);
                    }
                }
            }
            AccountingSessionUpdate::Ignored { .. } => {}
        }
    }

    fn prune_removed_sessions(&mut self) {
        let store = &self.store;
        self.updated_at
            .retain(|key, _| store.session(key).is_some());
        self.update_sequence_by_key
            .retain(|key, _| store.session(key).is_some());
    }

    fn record_session_update_time(&mut self, key: &AccountingSessionKey, now: Instant) {
        self.updated_at.insert(key.clone(), now);
        self.update_sequence_by_key.insert(
            key.clone(),
            next_update_sequence(&mut self.next_update_sequence),
        );
    }

    #[cfg(test)]
    fn expire_due(&mut self, now: Instant) -> usize {
        let mut command_sink = DeferredDynamicCircuitSink::application_disabled();
        self.expire_due_with_command_sink(now, &mut command_sink)
    }

    fn expire_due_with_command_sink(
        &mut self,
        now: Instant,
        command_sink: &mut impl DynamicCircuitCommandSink,
    ) -> usize {
        let mut expired_keys = self
            .store
            .sessions()
            .filter_map(|(key, session)| {
                let updated_at = *self.updated_at.get(key)?;
                let ttl = self.expiry_duration_for(session.state);
                let deadline = updated_at + ttl;
                let update_sequence = self.update_sequence_by_key.get(key).copied().unwrap_or(0);
                (now >= deadline).then(|| (deadline, update_sequence, key.clone()))
            })
            .collect::<Vec<_>>();
        expired_keys.sort_by_key(|(deadline, update_sequence, _)| (*deadline, *update_sequence));

        let expired_count = expired_keys.len();
        for (_, _, key) in expired_keys {
            self.updated_at.remove(&key);
            self.update_sequence_by_key.remove(&key);
            if let Some(expired_session) =
                self.store.expire_session_with_commands(&key, command_sink)
            {
                self.activation_diagnostics_by_key.remove(&key);
                push_limited(
                    &mut self.recent_expired_activation_diagnostics,
                    RadiusActivationDiagnostic::from_expired_session(&key, &expired_session),
                    RADIUS_RECENT_EXPIRED_DIAGNOSTIC_LIMIT,
                );
            }
        }
        expired_count
    }

    fn next_expiry_deadline(&self) -> Option<Instant> {
        self.store
            .sessions()
            .filter_map(|(key, session)| {
                self.updated_at
                    .get(key)
                    .map(|updated_at| *updated_at + self.expiry_duration_for(session.state))
            })
            .min()
    }

    fn next_changed_expiry_deadline(&self, update: &AccountingSessionUpdate) -> Option<Instant> {
        match update {
            AccountingSessionUpdate::SessionUpdated { key, .. } => self.expiry_deadline_for(key),
            AccountingSessionUpdate::NasSessionsMarkedStale {
                newly_stale_session_keys,
                ..
            } => newly_stale_session_keys
                .iter()
                .filter_map(|key| self.expiry_deadline_for(key))
                .min(),
            AccountingSessionUpdate::Ignored { .. } => None,
        }
    }

    fn expiry_deadline_for(&self, key: &AccountingSessionKey) -> Option<Instant> {
        let session = self.store.session(key)?;
        let updated_at = self.updated_at.get(key)?;
        Some(*updated_at + self.expiry_duration_for(session.state))
    }

    fn expiry_duration_for(&self, state: AccountingSessionState) -> Duration {
        match state {
            AccountingSessionState::Stale(_) => self.stale_grace,
            AccountingSessionState::Active | AccountingSessionState::Stopped => self.default_ttl,
        }
    }

    fn expiry_check_interval(&self) -> Duration {
        cleanup_interval(self.default_ttl, self.stale_grace)
    }
}

fn cleanup_interval(default_ttl: Duration, stale_grace: Duration) -> Duration {
    Duration::from_secs(default_ttl.min(stale_grace).as_secs().clamp(1, 60))
}

struct DeferredDynamicCircuitSink {
    reason: &'static str,
}

impl DeferredDynamicCircuitSink {
    const fn application_disabled() -> Self {
        Self {
            reason: "daemon-side application is disabled",
        }
    }
}

impl DynamicCircuitCommandSink for DeferredDynamicCircuitSink {
    fn emit(&mut self, intent: DynamicCircuitIntent) {
        let intent_name = dynamic_circuit_intent_name(&intent);
        debug!(
            circuit_id = intent.circuit_id(),
            intent = intent_name,
            reason = self.reason,
            "RADIUS dynamic-circuit intent deferred"
        );
    }
}

struct ApplyingDynamicCircuitSink {
    queue_tx: mpsc::Sender<QueuedDynamicCircuitIntent>,
    application_state: Arc<Mutex<DynamicCircuitApplicationState>>,
    next_sequence: u64,
}

impl ApplyingDynamicCircuitSink {
    fn new(bus_tx: DynamicCircuitBusSender) -> Self {
        Self::new_with_capacity(bus_tx, DYNAMIC_CIRCUIT_APPLICATION_QUEUE_CAPACITY)
    }

    fn new_with_capacity(bus_tx: DynamicCircuitBusSender, capacity: usize) -> Self {
        let (queue_tx, queue_rx) = mpsc::channel(capacity);
        let application_state = Arc::new(Mutex::new(DynamicCircuitApplicationState::default()));
        tokio::spawn(run_dynamic_circuit_application_worker(
            bus_tx,
            application_state.clone(),
            queue_rx,
        ));
        Self {
            queue_tx,
            application_state,
            next_sequence: 0,
        }
    }

    #[cfg(test)]
    fn new_with_queue_for_test(
        queue_tx: mpsc::Sender<QueuedDynamicCircuitIntent>,
        application_state: Arc<Mutex<DynamicCircuitApplicationState>>,
        next_sequence: u64,
    ) -> Self {
        Self {
            queue_tx,
            application_state,
            next_sequence,
        }
    }

    fn next_sequence(&mut self) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        sequence
    }

    fn activation_diagnostics(
        &self,
        sessions: &RadiusAccountingSessions,
    ) -> Vec<RadiusActivationDiagnostic> {
        let mut diagnostics = sessions.activation_diagnostics();
        let application_diagnostics = self.application_state.lock().application_diagnostics();
        let failed_owners = application_diagnostics
            .iter()
            .flat_map(diagnostic_owner_keys)
            .collect::<HashSet<_>>();
        suppress_failed_owner_diagnostics(&mut diagnostics, &failed_owners);
        diagnostics.extend(application_diagnostics);
        diagnostics
    }
}

fn suppress_failed_owner_diagnostics(
    diagnostics: &mut Vec<RadiusActivationDiagnostic>,
    failed_owners: &HashSet<DynamicCircuitOwnerKey>,
) {
    diagnostics.retain_mut(|diagnostic| {
        let had_circuit_ids = !diagnostic.circuit_ids.is_empty();
        let session_key = diagnostic.session_key.clone();
        diagnostic.circuit_ids.retain(|circuit_id| {
            !failed_owners.contains(&(circuit_id.clone(), session_key.clone()))
        });
        !had_circuit_ids || !diagnostic.circuit_ids.is_empty()
    });
}

fn diagnostic_owner_keys(diagnostic: &RadiusActivationDiagnostic) -> Vec<DynamicCircuitOwnerKey> {
    diagnostic
        .circuit_ids
        .iter()
        .map(|circuit_id| (circuit_id.clone(), diagnostic.session_key.clone()))
        .collect()
}

impl DynamicCircuitCommandSink for ApplyingDynamicCircuitSink {
    fn emit(&mut self, intent: DynamicCircuitIntent) {
        let context = DynamicCircuitApplicationContext::from_intent(&intent);
        let is_upsert = matches!(
            intent,
            DynamicCircuitIntent::CreateDynamicCircuit(_)
                | DynamicCircuitIntent::UpdateDynamicCircuit(_)
        );
        let is_removal = matches!(intent, DynamicCircuitIntent::RemoveDynamicCircuit(_));
        let sequence = self.next_sequence();
        if is_upsert {
            self.application_state
                .lock()
                .track_pending_upsert(&context, sequence);
        }
        if is_removal {
            self.application_state
                .lock()
                .track_deferred_removal_for_pending_upserts(&context, sequence);
        }
        let queued_intent = QueuedDynamicCircuitIntent { sequence, intent };

        if let Err(queue_failure) = try_queue_dynamic_circuit_intent(&self.queue_tx, queued_intent)
        {
            let queue_failure = *queue_failure;
            let error = log_dynamic_circuit_queue_unavailable(&context, queue_failure.reason);
            let defer_removal_until_worker_drains =
                is_removal && queue_failure.reason == DynamicCircuitQueueUnavailableReason::Full;
            let mut application_state = self.application_state.lock();
            application_state.record_application_failure(&context, error.to_string());
            if is_upsert {
                application_state.clear_pending_upsert(&context, sequence);
            }
            if defer_removal_until_worker_drains {
                application_state.record_pending_removal(&context, queue_failure.queued_intent);
            } else if is_removal {
                log_dynamic_circuit_removal_not_queued(&context);
            }
        }
    }
}

struct QueuedDynamicCircuitIntent {
    sequence: u64,
    intent: DynamicCircuitIntent,
}

struct DynamicCircuitQueueFailure {
    reason: DynamicCircuitQueueUnavailableReason,
    queued_intent: QueuedDynamicCircuitIntent,
}

fn try_queue_dynamic_circuit_intent(
    queue_tx: &mpsc::Sender<QueuedDynamicCircuitIntent>,
    queued_intent: QueuedDynamicCircuitIntent,
) -> Result<(), Box<DynamicCircuitQueueFailure>> {
    match queue_tx.try_send(queued_intent) {
        Ok(()) => Ok(()),
        Err(mpsc::error::TrySendError::Full(queued_intent)) => {
            Err(Box::new(DynamicCircuitQueueFailure {
                reason: DynamicCircuitQueueUnavailableReason::Full,
                queued_intent,
            }))
        }
        Err(mpsc::error::TrySendError::Closed(queued_intent)) => {
            Err(Box::new(DynamicCircuitQueueFailure {
                reason: DynamicCircuitQueueUnavailableReason::Closed,
                queued_intent,
            }))
        }
    }
}

#[cfg(test)]
fn queue_dynamic_circuit_intent(
    queue_tx: &mpsc::Sender<QueuedDynamicCircuitIntent>,
    context: &DynamicCircuitApplicationContext,
    queued_intent: QueuedDynamicCircuitIntent,
) -> Result<(), DynamicCircuitApplicationError> {
    match try_queue_dynamic_circuit_intent(queue_tx, queued_intent) {
        Ok(()) => Ok(()),
        Err(queue_failure) => Err(log_dynamic_circuit_queue_unavailable(
            context,
            queue_failure.reason,
        )),
    }
}

fn log_dynamic_circuit_queue_unavailable(
    context: &DynamicCircuitApplicationContext,
    reason: DynamicCircuitQueueUnavailableReason,
) -> DynamicCircuitApplicationError {
    log_dynamic_circuit_application_result(
        context,
        Err(DynamicCircuitApplicationError::QueueUnavailable(reason)),
    );
    DynamicCircuitApplicationError::QueueUnavailable(reason)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DynamicCircuitQueueUnavailableReason {
    Full,
    Closed,
}

impl fmt::Display for DynamicCircuitQueueUnavailableReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Full => "queue full",
            Self::Closed => "queue closed",
        })
    }
}

async fn run_dynamic_circuit_application_worker(
    bus_tx: DynamicCircuitBusSender,
    application_state: Arc<Mutex<DynamicCircuitApplicationState>>,
    mut queue_rx: mpsc::Receiver<QueuedDynamicCircuitIntent>,
) {
    while let Some(queued_intent) = queue_rx.recv().await {
        process_dynamic_circuit_intent_and_pending_removals(
            bus_tx.clone(),
            &application_state,
            queued_intent,
        )
        .await;
    }
}

async fn process_dynamic_circuit_intent_and_pending_removals(
    bus_tx: DynamicCircuitBusSender,
    application_state: &Arc<Mutex<DynamicCircuitApplicationState>>,
    queued_intent: QueuedDynamicCircuitIntent,
) {
    process_dynamic_circuit_intent(bus_tx.clone(), application_state, queued_intent).await;
    drain_pending_dynamic_circuit_removals(bus_tx, application_state).await;
}

async fn drain_pending_dynamic_circuit_removals(
    bus_tx: DynamicCircuitBusSender,
    application_state: &Arc<Mutex<DynamicCircuitApplicationState>>,
) {
    loop {
        let queued_intent = {
            let mut application_state = application_state.lock();
            application_state.take_next_pending_removal()
        };
        let Some(queued_intent) = queued_intent else {
            break;
        };
        process_dynamic_circuit_intent(bus_tx.clone(), application_state, queued_intent).await;
    }
}

async fn process_dynamic_circuit_intent(
    bus_tx: DynamicCircuitBusSender,
    application_state: &Arc<Mutex<DynamicCircuitApplicationState>>,
    queued_intent: QueuedDynamicCircuitIntent,
) {
    let QueuedDynamicCircuitIntent { sequence, intent } = queued_intent;
    let context = DynamicCircuitApplicationContext::from_intent(&intent);
    match intent {
        DynamicCircuitIntent::CreateDynamicCircuit(upsert)
        | DynamicCircuitIntent::UpdateDynamicCircuit(upsert) => {
            process_dynamic_circuit_upsert(bus_tx, application_state, context, sequence, upsert)
                .await;
        }
        DynamicCircuitIntent::RemoveDynamicCircuit(removal) => {
            process_dynamic_circuit_removal(bus_tx, application_state, context, sequence, removal)
                .await;
        }
    }
}

async fn process_dynamic_circuit_upsert(
    bus_tx: DynamicCircuitBusSender,
    application_state: &Arc<Mutex<DynamicCircuitApplicationState>>,
    context: DynamicCircuitApplicationContext,
    sequence: u64,
    upsert: DynamicCircuitUpsert,
) {
    if application_state
        .lock()
        .upsert_was_removed_before_application(&context, sequence)
    {
        log_dynamic_circuit_stale_upsert_dropped(&context);
        application_state
            .lock()
            .clear_pending_upsert(&context, sequence);
        return;
    }

    let shared_upsert_may_apply = {
        let mut application_state = application_state.lock();
        let may_apply = application_state.shared_upsert_may_apply(&context);
        if !may_apply {
            application_state.record_retained_upsert(&context, sequence, upsert.clone());
        }
        may_apply
    };

    if !shared_upsert_may_apply {
        log_dynamic_circuit_shared_upsert_retained(&context);
        application_state
            .lock()
            .clear_pending_upsert(&context, sequence);
        return;
    }

    let request = BusRequest::CreateDynamicCircuit {
        shaped_device: Box::new(upsert.shaped_device),
    };
    apply_dynamic_circuit_request_and_record(
        bus_tx,
        application_state,
        &context,
        sequence,
        request,
    )
    .await;
    application_state
        .lock()
        .clear_pending_upsert(&context, sequence);
}

async fn process_dynamic_circuit_removal(
    bus_tx: DynamicCircuitBusSender,
    application_state: &Arc<Mutex<DynamicCircuitApplicationState>>,
    context: DynamicCircuitApplicationContext,
    sequence: u64,
    removal: DynamicCircuitRemoval,
) {
    let owner_context = {
        let mut state = application_state.lock();
        let Some(owner_context) = state.removal_owner_context(&context, removal.reason) else {
            state.record_application_success(&context);
            log_dynamic_circuit_removal_skipped(&context);
            return;
        };
        let removal_is_stale = state.removal_is_stale(&owner_context, sequence);
        if removal_is_stale {
            log_dynamic_circuit_removal_skipped(&owner_context);
            return;
        }
        owner_context
    };
    let action = application_state.lock().removal_action(&owner_context);
    match action {
        DynamicCircuitRemovalAction::ReleaseOnly => {
            application_state.lock().release_owner(&owner_context);
            log_dynamic_circuit_removal_skipped(&owner_context);
        }
        DynamicCircuitRemovalAction::Remove => {
            apply_dynamic_circuit_removal_request(
                bus_tx,
                application_state,
                &owner_context,
                removal.circuit_id,
                None,
            )
            .await;
        }
        DynamicCircuitRemovalAction::Promote(upsert) => {
            let retained_upsert = *upsert;
            let upsert = retained_upsert.upsert;
            let promoted_context = DynamicCircuitApplicationContext::from_upsert("update", &upsert);
            let request = BusRequest::CreateDynamicCircuit {
                shaped_device: Box::new(upsert.shaped_device),
            };
            let result = apply_dynamic_circuit_request(bus_tx.clone(), request).await;
            if dynamic_circuit_upsert_result_records_success(&result) {
                let mut application_state = application_state.lock();
                application_state.release_owner(&owner_context);
                application_state
                    .record_applied_upsert(&promoted_context, retained_upsert.first_sequence);
                application_state.record_application_success(&promoted_context);
                log_dynamic_circuit_retained_upsert_promoted(&owner_context, &promoted_context);
                log_dynamic_circuit_application_result(&promoted_context, result);
                return;
            }
            if let Err(err) = &result {
                application_state
                    .lock()
                    .record_application_failure(&promoted_context, err.to_string());
            }
            log_dynamic_circuit_application_result(&promoted_context, result);

            apply_dynamic_circuit_removal_request(
                bus_tx,
                application_state,
                &owner_context,
                removal.circuit_id,
                Some(&promoted_context),
            )
            .await;
        }
    }
}

#[derive(Clone)]
struct DynamicCircuitApplicationContext {
    intent_name: &'static str,
    circuit_id: String,
    session_key: AccountingSessionKey,
}

type DynamicCircuitOwnerKey = (String, AccountingSessionKey);

impl DynamicCircuitApplicationContext {
    fn from_intent(intent: &DynamicCircuitIntent) -> Self {
        let intent_name = dynamic_circuit_intent_name(intent);
        match intent {
            DynamicCircuitIntent::CreateDynamicCircuit(upsert) => {
                Self::from_upsert(intent_name, upsert)
            }
            DynamicCircuitIntent::UpdateDynamicCircuit(upsert) => {
                Self::from_upsert(intent_name, upsert)
            }
            DynamicCircuitIntent::RemoveDynamicCircuit(removal) => Self {
                intent_name,
                circuit_id: removal.circuit_id.clone(),
                session_key: removal.session_key.clone(),
            },
        }
    }

    fn from_upsert(intent_name: &'static str, upsert: &DynamicCircuitUpsert) -> Self {
        Self {
            intent_name,
            circuit_id: upsert.circuit_id.clone(),
            session_key: upsert.session_key.clone(),
        }
    }

    fn owner_key(&self) -> DynamicCircuitOwnerKey {
        (self.circuit_id.clone(), self.session_key.clone())
    }
}

#[derive(Default)]
struct DynamicCircuitApplicationState {
    current_owners_by_circuit: HashMap<String, HashSet<AccountingSessionKey>>,
    authoritative_session_by_circuit: HashMap<String, AccountingSessionKey>,
    pending_upsert_sequences_by_owner: HashMap<DynamicCircuitOwnerKey, HashSet<u64>>,
    deferred_removal_sequence_by_owner: HashMap<DynamicCircuitOwnerKey, u64>,
    pending_removals_by_owner: HashMap<DynamicCircuitOwnerKey, QueuedDynamicCircuitIntent>,
    pending_removal_order: VecDeque<DynamicCircuitOwnerKey>,
    latest_applied_upsert_sequence_by_owner: HashMap<DynamicCircuitOwnerKey, u64>,
    retained_upserts_by_owner: HashMap<DynamicCircuitOwnerKey, RetainedDynamicCircuitUpsert>,
    retained_owners_by_circuit: HashMap<String, HashSet<AccountingSessionKey>>,
    application_diagnostics_by_owner: HashMap<DynamicCircuitOwnerKey, RadiusActivationDiagnostic>,
    application_diagnostic_order: VecDeque<DynamicCircuitOwnerKey>,
}

#[derive(Clone)]
struct RetainedDynamicCircuitUpsert {
    first_sequence: u64,
    upsert: DynamicCircuitUpsert,
}

enum DynamicCircuitRemovalAction {
    ReleaseOnly,
    Remove,
    Promote(Box<RetainedDynamicCircuitUpsert>),
}

impl DynamicCircuitApplicationState {
    fn record_application_success(&mut self, context: &DynamicCircuitApplicationContext) {
        let owner_key = context.owner_key();
        self.application_diagnostics_by_owner.remove(&owner_key);
        self.application_diagnostic_order
            .retain(|retained_owner| retained_owner != &owner_key);
    }

    fn record_application_failure(
        &mut self,
        context: &DynamicCircuitApplicationContext,
        error: String,
    ) {
        let owner_key = context.owner_key();
        self.application_diagnostic_order
            .retain(|existing_owner| existing_owner != &owner_key);
        push_limited(
            &mut self.application_diagnostic_order,
            owner_key.clone(),
            RADIUS_APPLY_FAILED_DIAGNOSTIC_LIMIT,
        );
        self.application_diagnostics_by_owner.insert(
            owner_key,
            RadiusActivationDiagnostic::apply_failed(
                context.session_key.clone(),
                context.circuit_id.clone(),
                error,
            ),
        );
        self.prune_application_diagnostics_to_order();
    }

    fn application_diagnostics(&self) -> Vec<RadiusActivationDiagnostic> {
        self.application_diagnostic_order
            .iter()
            .filter_map(|owner_key| self.application_diagnostics_by_owner.get(owner_key))
            .cloned()
            .collect()
    }

    fn prune_application_diagnostics_to_order(&mut self) {
        let retained_owners = self
            .application_diagnostic_order
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        self.application_diagnostics_by_owner
            .retain(|owner_key, _| retained_owners.contains(owner_key));
    }

    fn shared_upsert_may_apply(&self, context: &DynamicCircuitApplicationContext) -> bool {
        let Some(session_keys) = self.current_owners_by_circuit.get(&context.circuit_id) else {
            return true;
        };
        if !session_keys
            .iter()
            .any(|session_key| session_key != &context.session_key)
        {
            return true;
        }

        self.authoritative_session_by_circuit
            .get(&context.circuit_id)
            .is_none_or(|session_key| session_key == &context.session_key)
    }

    fn record_applied_upsert(&mut self, context: &DynamicCircuitApplicationContext, sequence: u64) {
        self.record_upsert_owner(context);
        self.latest_applied_upsert_sequence_by_owner
            .insert(context.owner_key(), sequence);
    }

    #[cfg(test)]
    fn record_upsert(&mut self, context: &DynamicCircuitApplicationContext) {
        self.record_applied_upsert(context, 0);
    }

    fn record_retained_upsert(
        &mut self,
        context: &DynamicCircuitApplicationContext,
        sequence: u64,
        upsert: DynamicCircuitUpsert,
    ) {
        let owner_key = context.owner_key();
        if let Some(removal_sequence) = self
            .deferred_removal_sequence_by_owner
            .get(&owner_key)
            .copied()
        {
            if sequence <= removal_sequence {
                return;
            }
            if !self.has_pending_upsert_at_or_before(&owner_key, removal_sequence) {
                self.deferred_removal_sequence_by_owner.remove(&owner_key);
            }
        }

        self.record_upsert_owner(context);
        self.retained_upserts_by_owner
            .entry(owner_key.clone())
            .and_modify(|retained| retained.upsert = upsert.clone())
            .or_insert(RetainedDynamicCircuitUpsert {
                first_sequence: sequence,
                upsert,
            });
        self.retained_owners_by_circuit
            .entry(owner_key.0)
            .or_default()
            .insert(owner_key.1);
    }

    fn record_upsert_owner(&mut self, context: &DynamicCircuitApplicationContext) {
        let circuit_id = context.circuit_id.clone();
        let session_key = context.session_key.clone();
        self.remove_retained_upsert(&context.owner_key());
        self.current_owners_by_circuit
            .entry(circuit_id.clone())
            .or_default()
            .insert(session_key.clone());
        self.authoritative_session_by_circuit
            .entry(circuit_id.clone())
            .or_insert_with(|| session_key.clone());
    }

    fn track_pending_upsert(&mut self, context: &DynamicCircuitApplicationContext, sequence: u64) {
        self.pending_upsert_sequences_by_owner
            .entry(context.owner_key())
            .or_default()
            .insert(sequence);
    }

    fn upsert_was_removed_before_application(
        &self,
        context: &DynamicCircuitApplicationContext,
        sequence: u64,
    ) -> bool {
        self.deferred_removal_sequence_by_owner
            .get(&context.owner_key())
            .is_some_and(|removal_sequence| sequence <= *removal_sequence)
    }

    fn clear_pending_upsert(&mut self, context: &DynamicCircuitApplicationContext, sequence: u64) {
        let owner_key = context.owner_key();
        if let Some(pending_sequences) = self.pending_upsert_sequences_by_owner.get_mut(&owner_key)
        {
            pending_sequences.remove(&sequence);
            if pending_sequences.is_empty() {
                self.pending_upsert_sequences_by_owner.remove(&owner_key);
            }
        }
        self.prune_deferred_removal(&owner_key);
    }

    fn prune_deferred_removal(&mut self, owner_key: &DynamicCircuitOwnerKey) {
        let Some(removal_sequence) = self
            .deferred_removal_sequence_by_owner
            .get(owner_key)
            .copied()
        else {
            return;
        };
        if !self.has_pending_upsert_at_or_before(owner_key, removal_sequence) {
            self.deferred_removal_sequence_by_owner.remove(owner_key);
        }
    }

    fn has_pending_upsert_at_or_before(
        &self,
        owner_key: &DynamicCircuitOwnerKey,
        removal_sequence: u64,
    ) -> bool {
        self.pending_upsert_sequences_by_owner
            .get(owner_key)
            .is_some_and(|pending_sequences| {
                pending_sequences
                    .iter()
                    .any(|pending_sequence| *pending_sequence <= removal_sequence)
            })
    }

    fn track_deferred_removal_for_pending_upserts(
        &mut self,
        context: &DynamicCircuitApplicationContext,
        sequence: u64,
    ) {
        let owner_key = context.owner_key();
        if self.has_pending_upsert_at_or_before(&owner_key, sequence) {
            self.deferred_removal_sequence_by_owner
                .insert(owner_key, sequence);
        }
    }

    fn record_pending_removal(
        &mut self,
        context: &DynamicCircuitApplicationContext,
        queued_intent: QueuedDynamicCircuitIntent,
    ) {
        let owner_key = context.owner_key();
        if self.pending_removals_by_owner.contains_key(&owner_key) {
            self.pending_removal_order
                .retain(|pending_owner| pending_owner != &owner_key);
        }
        self.pending_removals_by_owner
            .insert(owner_key.clone(), queued_intent);
        self.pending_removal_order.push_back(owner_key);
    }

    fn take_next_pending_removal(&mut self) -> Option<QueuedDynamicCircuitIntent> {
        while let Some(owner_key) = self.pending_removal_order.pop_front() {
            if let Some(queued_intent) = self.pending_removals_by_owner.remove(&owner_key) {
                return Some(queued_intent);
            }
        }
        None
    }

    fn removal_is_stale(
        &self,
        context: &DynamicCircuitApplicationContext,
        removal_sequence: u64,
    ) -> bool {
        self.latest_applied_upsert_sequence_by_owner
            .get(&context.owner_key())
            .is_some_and(|upsert_sequence| *upsert_sequence > removal_sequence)
    }

    fn release_owner(&mut self, context: &DynamicCircuitApplicationContext) {
        let owner_was_authoritative = self
            .authoritative_session_by_circuit
            .get(&context.circuit_id)
            .is_some_and(|session_key| session_key == &context.session_key);
        let owner_key = context.owner_key();
        self.record_application_success(context);
        self.remove_retained_upsert(&owner_key);
        self.pending_upsert_sequences_by_owner.remove(&owner_key);
        self.deferred_removal_sequence_by_owner.remove(&owner_key);
        self.pending_removals_by_owner.remove(&owner_key);
        self.latest_applied_upsert_sequence_by_owner
            .remove(&owner_key);

        let remaining_owners = if let Some(session_keys) =
            self.current_owners_by_circuit.get_mut(&context.circuit_id)
        {
            session_keys.remove(&context.session_key);
            session_keys.len()
        } else {
            0
        };

        if owner_was_authoritative {
            self.authoritative_session_by_circuit
                .remove(&context.circuit_id);
        }
        if remaining_owners == 0 {
            self.current_owners_by_circuit.remove(&context.circuit_id);
            self.authoritative_session_by_circuit
                .remove(&context.circuit_id);
        }
    }

    fn removal_action(
        &self,
        context: &DynamicCircuitApplicationContext,
    ) -> DynamicCircuitRemovalAction {
        let owner_is_authoritative = self
            .authoritative_session_by_circuit
            .get(&context.circuit_id)
            .is_some_and(|session_key| session_key == &context.session_key);
        if !owner_is_authoritative {
            return DynamicCircuitRemovalAction::ReleaseOnly;
        }

        self.promotable_retained_upsert(context)
            .map(|upsert| DynamicCircuitRemovalAction::Promote(Box::new(upsert)))
            .unwrap_or(DynamicCircuitRemovalAction::Remove)
    }

    fn removal_owner_context(
        &self,
        context: &DynamicCircuitApplicationContext,
        reason: lqos_radius::DynamicCircuitRemovalReason,
    ) -> Option<DynamicCircuitApplicationContext> {
        if self
            .current_owners_by_circuit
            .get(&context.circuit_id)
            .is_some_and(|session_keys| session_keys.contains(&context.session_key))
        {
            return Some(context.clone());
        }

        if reason != lqos_radius::DynamicCircuitRemovalReason::Rekeyed {
            return None;
        }

        let session_keys = self.current_owners_by_circuit.get(&context.circuit_id)?;
        if session_keys.len() != 1 {
            return None;
        }
        let owner = session_keys.iter().next()?;
        if self
            .authoritative_session_by_circuit
            .get(&context.circuit_id)
            != Some(owner)
        {
            return None;
        }

        Some(DynamicCircuitApplicationContext {
            intent_name: context.intent_name,
            circuit_id: context.circuit_id.clone(),
            session_key: owner.clone(),
        })
    }

    fn promotable_retained_upsert(
        &self,
        context: &DynamicCircuitApplicationContext,
    ) -> Option<RetainedDynamicCircuitUpsert> {
        let retained_owners = self.retained_owners_by_circuit.get(&context.circuit_id)?;
        retained_owners
            .iter()
            .filter(|session_key| *session_key != &context.session_key)
            .filter_map(|session_key| {
                self.retained_upserts_by_owner
                    .get(&(context.circuit_id.clone(), session_key.clone()))
            })
            .min_by_key(|retained| retained.first_sequence)
            .cloned()
    }

    fn remove_retained_upsert(&mut self, owner_key: &DynamicCircuitOwnerKey) {
        self.retained_upserts_by_owner.remove(owner_key);
        if let Some(retained_owners) = self.retained_owners_by_circuit.get_mut(&owner_key.0) {
            retained_owners.remove(&owner_key.1);
            if retained_owners.is_empty() {
                self.retained_owners_by_circuit.remove(&owner_key.0);
            }
        }
    }
}

async fn apply_dynamic_circuit_request_and_record(
    bus_tx: DynamicCircuitBusSender,
    application_state: &Arc<Mutex<DynamicCircuitApplicationState>>,
    context: &DynamicCircuitApplicationContext,
    sequence: u64,
    request: BusRequest,
) {
    let result = apply_dynamic_circuit_request(bus_tx, request).await;
    if dynamic_circuit_upsert_result_records_success(&result) {
        application_state
            .lock()
            .record_applied_upsert(context, sequence);
    }
    record_dynamic_circuit_application_result(application_state, context, &result);
    log_dynamic_circuit_application_result(context, result);
}

async fn apply_dynamic_circuit_removal_request(
    bus_tx: DynamicCircuitBusSender,
    application_state: &Arc<Mutex<DynamicCircuitApplicationState>>,
    context: &DynamicCircuitApplicationContext,
    circuit_id: String,
    extra_release_context: Option<&DynamicCircuitApplicationContext>,
) {
    let request = BusRequest::RemoveDynamicCircuit { circuit_id };
    let result = apply_dynamic_circuit_request(bus_tx, request).await;
    if dynamic_circuit_removal_result_releases_owner(&result) {
        let mut application_state = application_state.lock();
        application_state.release_owner(context);
        if let Some(extra_release_context) = extra_release_context {
            application_state.release_owner(extra_release_context);
        }
    }
    record_dynamic_circuit_application_result(application_state, context, &result);
    log_dynamic_circuit_application_result(context, result);
}

fn record_dynamic_circuit_application_result(
    application_state: &Arc<Mutex<DynamicCircuitApplicationState>>,
    context: &DynamicCircuitApplicationContext,
    result: &Result<(), DynamicCircuitApplicationError>,
) {
    let mut application_state = application_state.lock();
    match result {
        Ok(()) => application_state.record_application_success(context),
        Err(err) => application_state.record_application_failure(context, err.to_string()),
    }
}

fn dynamic_circuit_upsert_result_records_success(
    result: &Result<(), DynamicCircuitApplicationError>,
) -> bool {
    // Once lqosd accepts an upsert into the internal bus, a missing reply is ambiguous.
    // Tracking ownership keeps later shared-circuit upserts from overwriting it.
    result.is_ok()
        || matches!(
            result,
            Err(DynamicCircuitApplicationError::ReplyDropped
                | DynamicCircuitApplicationError::ReplyTimeout)
        )
}

fn dynamic_circuit_removal_result_releases_owner(
    result: &Result<(), DynamicCircuitApplicationError>,
) -> bool {
    result.is_ok()
}

async fn apply_dynamic_circuit_request(
    bus_tx: DynamicCircuitBusSender,
    request: BusRequest,
) -> Result<(), DynamicCircuitApplicationError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    tokio::time::timeout(
        DYNAMIC_CIRCUIT_APPLICATION_TIMEOUT,
        bus_tx.send((reply_tx, request)),
    )
    .await
    .map_err(|_| DynamicCircuitApplicationError::BusSendTimeout)?
    .map_err(|_| DynamicCircuitApplicationError::BusClosed)?;
    let reply = tokio::time::timeout(DYNAMIC_CIRCUIT_APPLICATION_TIMEOUT, reply_rx)
        .await
        .map_err(|_| DynamicCircuitApplicationError::ReplyTimeout)?
        .map_err(|_| DynamicCircuitApplicationError::ReplyDropped)?;
    dynamic_circuit_application_result(reply)
}

fn dynamic_circuit_application_result(
    reply: BusReply,
) -> Result<(), DynamicCircuitApplicationError> {
    let mut responses = reply.responses;
    match responses.len() {
        0 => Err(DynamicCircuitApplicationError::MissingResponse),
        1 => match responses.remove(0) {
            BusResponse::Ack => Ok(()),
            BusResponse::Fail(message) => {
                Err(DynamicCircuitApplicationError::RequestFailed(message))
            }
            response => Err(DynamicCircuitApplicationError::UnexpectedResponse(format!(
                "{response:?}"
            ))),
        },
        count => Err(DynamicCircuitApplicationError::UnexpectedResponseCount(
            count,
        )),
    }
}

fn log_dynamic_circuit_application_result(
    context: &DynamicCircuitApplicationContext,
    result: Result<(), DynamicCircuitApplicationError>,
) {
    match result {
        Ok(()) => {
            debug!(
                circuit_id = %context.circuit_id,
                session = ?context.session_key,
                intent = context.intent_name,
                "applied RADIUS dynamic-circuit request"
            );
        }
        Err(err) => {
            warn!(
                circuit_id = %context.circuit_id,
                session = ?context.session_key,
                intent = context.intent_name,
                error = %err,
                "failed to apply RADIUS dynamic-circuit request"
            );
        }
    }
}

fn log_dynamic_circuit_shared_upsert_retained(context: &DynamicCircuitApplicationContext) {
    warn!(
        circuit_id = %context.circuit_id,
        session = ?context.session_key,
        intent = context.intent_name,
        "skipped RADIUS dynamic-circuit upsert because another session already owns the circuit id"
    );
}

fn log_dynamic_circuit_stale_upsert_dropped(context: &DynamicCircuitApplicationContext) {
    debug!(
        circuit_id = %context.circuit_id,
        session = ?context.session_key,
        intent = context.intent_name,
        "skipped RADIUS dynamic-circuit upsert because a later removal was already deferred"
    );
}

fn log_dynamic_circuit_retained_upsert_promoted(
    removed_context: &DynamicCircuitApplicationContext,
    promoted_context: &DynamicCircuitApplicationContext,
) {
    debug!(
        circuit_id = %removed_context.circuit_id,
        removed_session = ?removed_context.session_key,
        promoted_session = ?promoted_context.session_key,
        "promoted retained RADIUS dynamic-circuit owner after authoritative owner removal"
    );
}

fn log_dynamic_circuit_removal_not_queued(context: &DynamicCircuitApplicationContext) {
    debug!(
        circuit_id = %context.circuit_id,
        session = ?context.session_key,
        intent = context.intent_name,
        "kept RADIUS dynamic-circuit owner after removal queue failure"
    );
}

fn log_dynamic_circuit_removal_skipped(context: &DynamicCircuitApplicationContext) {
    debug!(
        circuit_id = %context.circuit_id,
        session = ?context.session_key,
        intent = context.intent_name,
        "skipped RADIUS dynamic-circuit removal because this session did not own an applied circuit"
    );
}

#[derive(Debug, Error, PartialEq, Eq)]
enum DynamicCircuitApplicationError {
    #[error("dynamic circuit application queue unavailable: {0}")]
    QueueUnavailable(DynamicCircuitQueueUnavailableReason),
    #[error("timed out sending dynamic circuit bus request")]
    BusSendTimeout,
    #[error("dynamic circuit bus channel is closed")]
    BusClosed,
    #[error("timed out waiting for dynamic circuit bus reply")]
    ReplyTimeout,
    #[error("dynamic circuit bus reply sender was dropped")]
    ReplyDropped,
    #[error("dynamic circuit bus returned no response")]
    MissingResponse,
    #[error("dynamic circuit bus returned {0} responses for one request")]
    UnexpectedResponseCount(usize),
    #[error(
        "dynamic circuit request failed: {}",
        sanitize_dynamic_circuit_bus_failure_detail(.0)
    )]
    RequestFailed(String),
    #[error(
        "dynamic circuit request returned unexpected response: {}",
        sanitize_dynamic_circuit_bus_failure_detail(.0)
    )]
    UnexpectedResponse(String),
}

fn sanitize_dynamic_circuit_bus_failure_detail(detail: &str) -> String {
    let mut sanitized = String::new();
    let mut sanitized_bytes = 0;
    let mut redact_next_value = false;
    let mut tokens = detail.split_whitespace().peekable();
    while let Some(token) = tokens.next() {
        let token_is_sensitive = dynamic_circuit_bus_failure_token_is_sensitive(token)
            || dynamic_circuit_bus_failure_token_starts_sensitive_phrase(
                token,
                tokens.peek().copied(),
            );
        let redact_token = redact_next_value || token_is_sensitive;
        let sanitized_token = if redact_token { "[redacted]" } else { token };
        if !push_bounded_dynamic_circuit_bus_failure_token(
            &mut sanitized,
            &mut sanitized_bytes,
            sanitized_token,
        ) {
            sanitized.push_str("...");
            return sanitized;
        }
        if token_is_sensitive {
            redact_next_value = !dynamic_circuit_bus_failure_token_has_inline_secret_value(token);
        } else if redact_next_value && !dynamic_circuit_bus_failure_token_is_secret_connector(token)
        {
            redact_next_value = false;
        }
    }
    if sanitized.is_empty() {
        "no additional detail".to_string()
    } else {
        sanitized
    }
}

fn dynamic_circuit_bus_failure_token_is_sensitive(token: &str) -> bool {
    let compact = compact_dynamic_circuit_bus_failure_token(token);
    compact.contains("secret")
        || compact.contains("password")
        || compact.contains("passwd")
        || compact.contains("token")
        || compact.contains("apikey")
        || compact.contains("privatekey")
        || compact.contains("accesskey")
        || compact.contains("credential")
        || compact.contains("authorization")
        || compact.contains("bearer")
        || dynamic_circuit_bus_failure_token_is_bare_key_assignment_label(token)
}

fn dynamic_circuit_bus_failure_token_starts_sensitive_phrase(
    token: &str,
    next_token: Option<&str>,
) -> bool {
    let token = compact_dynamic_circuit_bus_failure_token(token);
    let Some(next_token) = next_token else {
        return false;
    };
    if token == "key"
        && (dynamic_circuit_bus_failure_token_is_secret_connector(next_token)
            || dynamic_circuit_bus_failure_token_has_inline_secret_value(next_token))
    {
        return true;
    }
    matches!(token.as_str(), "api" | "private" | "access")
        && dynamic_circuit_bus_failure_token_is_key_label(next_token)
}

fn dynamic_circuit_bus_failure_token_is_secret_connector(token: &str) -> bool {
    let token = trimmed_dynamic_circuit_bus_failure_token(token);
    let connector =
        token.trim_matches(|character: char| matches!(character, ':' | '=' | '>' | '-'));
    matches!(connector, "" | "is" | "was" | "value" | "key")
}

fn dynamic_circuit_bus_failure_token_is_key_label(token: &str) -> bool {
    let token = trimmed_dynamic_circuit_bus_failure_token(token);
    if token == "key" {
        return true;
    }
    token
        .strip_prefix("key")
        .is_some_and(|suffix| suffix.starts_with([':', '=', '-']))
}

fn dynamic_circuit_bus_failure_token_is_bare_key_assignment_label(token: &str) -> bool {
    let token = trimmed_dynamic_circuit_bus_failure_token(token);
    token
        .strip_prefix("key")
        .is_some_and(|suffix| suffix.starts_with([':', '=']))
}

fn dynamic_circuit_bus_failure_token_has_inline_secret_value(token: &str) -> bool {
    let token = token.trim_matches(|character: char| {
        matches!(character, '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}')
    });
    let Some(separator_index) = token.find([':', '=']) else {
        return false;
    };
    token[separator_index + 1..]
        .chars()
        .any(|character| !matches!(character, ':' | '=' | '>' | '-'))
}

fn trimmed_dynamic_circuit_bus_failure_token(token: &str) -> String {
    token
        .trim_matches(|character: char| {
            matches!(character, '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}')
        })
        .chars()
        .take(16)
        .collect::<String>()
        .to_ascii_lowercase()
}

fn compact_dynamic_circuit_bus_failure_token(token: &str) -> String {
    token
        .chars()
        .take(DYNAMIC_CIRCUIT_BUS_FAILURE_DETAIL_LIMIT)
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_lowercase())
        .collect()
}

fn push_bounded_dynamic_circuit_bus_failure_token(
    sanitized: &mut String,
    sanitized_bytes: &mut usize,
    token: &str,
) -> bool {
    if !sanitized.is_empty() {
        if *sanitized_bytes >= DYNAMIC_CIRCUIT_BUS_FAILURE_DETAIL_LIMIT {
            return false;
        }
        sanitized.push(' ');
        *sanitized_bytes += 1;
    }
    for character in token.chars() {
        let character_bytes = character.len_utf8();
        if *sanitized_bytes + character_bytes > DYNAMIC_CIRCUIT_BUS_FAILURE_DETAIL_LIMIT {
            return false;
        }
        sanitized.push(character);
        *sanitized_bytes += character_bytes;
    }
    true
}

fn dynamic_circuit_intent_name(intent: &DynamicCircuitIntent) -> &'static str {
    match intent {
        DynamicCircuitIntent::CreateDynamicCircuit(_) => "create",
        DynamicCircuitIntent::UpdateDynamicCircuit(_) => "update",
        DynamicCircuitIntent::RemoveDynamicCircuit(_) => "remove",
    }
}

fn trim_line_endings(value: &mut Vec<u8>) {
    while value
        .last()
        .is_some_and(|byte| matches!(byte, b'\n' | b'\r'))
    {
        value.pop();
    }
}

fn client_label(index: usize, name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        format!("radius_accounting.clients[{index}]")
    } else {
        format!("radius_accounting.clients[{index}] ('{trimmed}')")
    }
}

struct RadiusAccountingRuntimeConfig {
    listen_addr: SocketAddr,
    clients: Vec<TrustedRadiusClient>,
    default_ttl: Duration,
    stale_grace: Duration,
    fallback_rate_profile: Option<SessionRateProfile>,
    fallback_parent: Option<DynamicCircuitParent>,
    mac_matcher: Option<ShapedDevicesMacMatcher>,
    apply_dynamic_circuits: bool,
}

/// Startup errors for lqosd RADIUS accounting integration.
#[derive(Debug, Error)]
pub(crate) enum RadiusAccountingStartupError {
    /// Enabled configuration is missing a listen address.
    #[error("radius_accounting.listen must be configured when enabled")]
    MissingListen,
    /// Enabled configuration is missing trusted clients.
    #[error("radius_accounting.clients must include at least one client when enabled")]
    NoClients,
    /// Configured fallback speed profile failed runtime validation.
    #[error("radius_accounting.fallback_speed_profile is invalid: {0}")]
    InvalidFallbackSpeedProfile(#[source] SessionRateProfileError),
    /// `ShapedDevices.csv` could not be loaded for MAC matching.
    #[error(
        "radius_accounting.dynamic_circuit_application.match_shaped_devices_by_mac failed to load {path}: {detail}"
    )]
    ShapedDevicesLoad {
        /// Resolved `ShapedDevices.csv` path.
        path: String,
        /// Underlying load error text.
        detail: String,
    },
    /// Shared-secret file read failed.
    #[error("{label}: failed to read configured RADIUS shared-secret file: {source}")]
    SecretFileRead {
        /// Client label from configuration.
        label: String,
        /// File read error.
        #[source]
        source: io::Error,
    },
    /// Shared-secret file had no bytes after line-ending cleanup.
    #[error("{label}: configured RADIUS shared-secret file is empty")]
    EmptySecretFile {
        /// Client label from configuration.
        label: String,
    },
    /// Configured source could not become a runtime source matcher.
    #[error("{label}: invalid RADIUS client source: {source}")]
    TrustedSource {
        /// Client label from configuration.
        label: String,
        /// Runtime source conversion error.
        #[source]
        source: lqos_radius::TrustedClientSourceError,
    },
    /// Runtime trusted-client construction failed.
    #[error("{label}: invalid RADIUS trusted client: {source}")]
    TrustedClient {
        /// Client label from configuration.
        label: String,
        /// Runtime client conversion error.
        #[source]
        source: lqos_radius::TrustedRadiusClientError,
    },
    /// UDP listener startup or local-address lookup failed.
    #[error("RADIUS accounting listener startup failed: {0}")]
    Listener(#[source] lqos_radius::ListenerError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use ip_network::IpNetwork;
    use lqos_config::{
        DynamicCircuitsConfig, RadiusClientSource, RadiusDynamicCircuitApplicationConfig,
        RadiusSharedSecretSource, ShapedDevice,
    };
    use lqos_radius::{
        AcctStatusType, DynamicCircuitRemoval, DynamicCircuitRemovalReason,
        MessageAuthenticatorPolicy, MikrotikRateLimit, NasIdentity, PendingSessionReason,
        RadiusActivationDiagnosticState, ReceivedVerifiedAccountingPacket, ShapedDevicesMacMatch,
        verify_accounting_request,
    };
    use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[tokio::test]
    async fn absent_or_disabled_config_does_not_build_runtime_listener() -> anyhow::Result<()> {
        let config_snapshot = Config::default();
        assert!(
            runtime_config_from_config(None, &config_snapshot)
                .await?
                .is_none()
        );
        assert!(
            runtime_config_from_config(Some(RadiusAccountingConfig::default()), &config_snapshot)
                .await?
                .is_none()
        );

        Ok(())
    }

    #[tokio::test]
    async fn disabled_start_returns_no_listener_handle() -> anyhow::Result<()> {
        let config_snapshot = Config::default();
        assert!(
            start_configured_radius_accounting(None, &config_snapshot, test_bus_sender())
                .await?
                .is_none()
        );
        assert!(
            start_configured_radius_accounting(
                Some(RadiusAccountingConfig::default()),
                &config_snapshot,
                test_bus_sender(),
            )
            .await?
            .is_none()
        );

        Ok(())
    }

    #[tokio::test]
    async fn enabled_start_binds_listener_and_abort_finishes() -> anyhow::Result<()> {
        let secret_path = unique_secret_path("bind-secret")?;
        std::fs::write(&secret_path, b"radius-secret")?;
        let mut config = enabled_config(&secret_path);
        config.listen = Some(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)));
        let config_snapshot = Config::default();

        let handle =
            start_configured_radius_accounting(Some(config), &config_snapshot, test_bus_sender())
                .await;
        let _ = std::fs::remove_file(&secret_path);
        let Some(handle) = handle? else {
            anyhow::bail!("enabled config should start a listener task");
        };

        handle.abort();
        let join_result = tokio::time::timeout(Duration::from_secs(1), handle).await?;
        let Err(join_error) = join_result else {
            anyhow::bail!("aborted listener task should not complete normally");
        };
        assert!(join_error.is_cancelled());

        Ok(())
    }

    #[tokio::test]
    async fn enabled_config_loads_secret_file_without_exposing_secret() -> anyhow::Result<()> {
        let secret_path = unique_secret_path("configured-secret")?;
        std::fs::write(&secret_path, b" shared secret \r\n")?;

        let mut config = enabled_config(&secret_path);
        config.default_ttl_seconds = 321;
        config.stale_grace_seconds = 45;
        let config_snapshot = Config::default();
        let runtime_config = runtime_config_from_config(Some(config), &config_snapshot).await;
        let _ = std::fs::remove_file(&secret_path);
        let Some(runtime_config) = runtime_config? else {
            anyhow::bail!("enabled config should produce runtime config");
        };

        assert_eq!(runtime_config.listen_addr, test_listen_addr());
        assert_eq!(runtime_config.default_ttl, Duration::from_secs(321));
        assert_eq!(runtime_config.stale_grace, Duration::from_secs(45));
        assert_eq!(runtime_config.clients.len(), 1);
        assert_eq!(
            runtime_config.clients[0].shared_secret(),
            b" shared secret "
        );
        assert!(!format!("{:?}", runtime_config.clients[0]).contains("shared secret"));

        Ok(())
    }

    #[tokio::test]
    async fn fallback_speed_profile_is_ignored_when_dynamic_application_disabled()
    -> anyhow::Result<()> {
        let secret_path = unique_secret_path("disabled-fallback-speed-profile")?;
        std::fs::write(&secret_path, b"radius-secret")?;

        let mut config = enabled_config(&secret_path);
        config.fallback_speed_profile = Some(RadiusFallbackSpeedProfile {
            download_min_mbps: f32::NAN,
            upload_min_mbps: 3.0,
            download_max_mbps: 25.0,
            upload_max_mbps: 10.0,
        });
        let config_snapshot = Config::default();
        let runtime_config = runtime_config_from_config(Some(config), &config_snapshot).await;
        let _ = std::fs::remove_file(&secret_path);
        let Some(runtime_config) = runtime_config? else {
            anyhow::bail!("enabled config should produce runtime config");
        };

        assert!(runtime_config.fallback_rate_profile.is_none());

        Ok(())
    }

    #[tokio::test]
    async fn mac_match_setting_is_ignored_when_dynamic_application_disabled() -> anyhow::Result<()>
    {
        let secret_path = unique_secret_path("disabled-mac-match-secret")?;
        let lqos_directory = unique_secret_path("disabled-mac-match-lqos-directory")?;
        std::fs::write(&secret_path, b"radius-secret")?;

        let mut config = enabled_config(&secret_path);
        config
            .dynamic_circuit_application
            .match_shaped_devices_by_mac = true;
        let config_snapshot = Config {
            dynamic_circuits: Some(DynamicCircuitsConfig {
                enabled: true,
                ..DynamicCircuitsConfig::default()
            }),
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            ..Config::default()
        };

        let runtime_config = runtime_config_from_config(Some(config), &config_snapshot).await;
        let _ = std::fs::remove_file(&secret_path);
        let Some(runtime_config) = runtime_config? else {
            anyhow::bail!("enabled config should produce runtime config");
        };

        assert!(runtime_config.mac_matcher.is_none());

        Ok(())
    }

    #[tokio::test]
    async fn fallback_speed_profile_builds_runtime_config_when_dynamic_application_enabled()
    -> anyhow::Result<()> {
        let secret_path = unique_secret_path("fallback-speed-profile")?;
        std::fs::write(&secret_path, b"radius-secret")?;

        let mut config = enabled_config(&secret_path);
        config.dynamic_circuit_application.enabled = true;
        config.dynamic_circuit_application.fallback_parent_node = Some("Core PPPoE".to_string());
        config.dynamic_circuit_application.fallback_parent_node_id = Some("core-pppoe".to_string());
        config.dynamic_circuit_application.fallback_anchor_node_id =
            Some("radius-anchor".to_string());
        config.fallback_speed_profile = Some(RadiusFallbackSpeedProfile {
            download_min_mbps: 5.0,
            upload_min_mbps: 3.0,
            download_max_mbps: 25.0,
            upload_max_mbps: 10.0,
        });
        let config_snapshot = Config {
            dynamic_circuits: Some(DynamicCircuitsConfig {
                enabled: true,
                ..DynamicCircuitsConfig::default()
            }),
            ..Config::default()
        };
        let runtime_config = runtime_config_from_config(Some(config), &config_snapshot).await;
        let _ = std::fs::remove_file(&secret_path);
        let Some(runtime_config) = runtime_config? else {
            anyhow::bail!("enabled config should produce runtime config");
        };

        assert_eq!(
            runtime_config.fallback_rate_profile,
            Some(SessionRateProfile::new(5.0, 3.0, 25.0, 10.0)?)
        );
        assert_eq!(
            runtime_config.fallback_parent,
            Some(DynamicCircuitParent {
                parent_node: "Core PPPoE".to_string(),
                parent_node_id: Some("core-pppoe".to_string()),
                anchor_node_id: Some("radius-anchor".to_string()),
            })
        );

        Ok(())
    }

    #[tokio::test]
    async fn invalid_fallback_speed_profile_fails_before_secrets() -> anyhow::Result<()> {
        let secret_path = unique_secret_path("invalid-fallback-speed-profile")?;
        let mut config = enabled_config(&secret_path);
        config.dynamic_circuit_application.enabled = true;
        config.fallback_speed_profile = Some(RadiusFallbackSpeedProfile {
            download_min_mbps: f32::NAN,
            upload_min_mbps: 3.0,
            download_max_mbps: 25.0,
            upload_max_mbps: 10.0,
        });

        let config_snapshot = Config {
            dynamic_circuits: Some(DynamicCircuitsConfig {
                enabled: true,
                ..DynamicCircuitsConfig::default()
            }),
            ..Config::default()
        };
        let error = match runtime_config_from_config(Some(config), &config_snapshot).await {
            Ok(_) => anyhow::bail!("invalid fallback speed profile should fail startup"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            RadiusAccountingStartupError::InvalidFallbackSpeedProfile(_)
        ));

        Ok(())
    }

    #[tokio::test]
    async fn secret_file_errors_identify_client_without_secret_bytes() -> anyhow::Result<()> {
        let secret_path = unique_secret_path("do-not-log-this-secret")?;
        let mut config = enabled_config(&secret_path);
        config.clients[0].name = "core-nas".to_string();

        let config_snapshot = Config::default();
        let error = match runtime_config_from_config(Some(config), &config_snapshot).await {
            Ok(_) => anyhow::bail!("missing secret file should fail startup"),
            Err(error) => error,
        };
        let message = error.to_string();

        assert!(message.contains("core-nas"));
        assert!(!message.contains("do-not-log-this-secret"));

        Ok(())
    }

    #[tokio::test]
    async fn enabled_dynamic_application_builds_runtime_config() -> anyhow::Result<()> {
        let secret_path = unique_secret_path("application-secret")?;
        std::fs::write(&secret_path, b"radius-secret")?;

        let mut config = enabled_config(&secret_path);
        config.dynamic_circuit_application.enabled = true;
        let config_snapshot = Config::default();
        let runtime_config = runtime_config_from_config(Some(config), &config_snapshot).await;
        let _ = std::fs::remove_file(&secret_path);
        let Some(runtime_config) = runtime_config? else {
            anyhow::bail!("enabled config should produce runtime config");
        };

        assert_eq!(runtime_config.listen_addr, test_listen_addr());
        assert!(!runtime_config.apply_dynamic_circuits);

        Ok(())
    }

    #[tokio::test]
    async fn dynamic_application_requires_both_safety_gates_to_apply() -> anyhow::Result<()> {
        let secret_path = unique_secret_path("application-safety-gates")?;
        std::fs::write(&secret_path, b"radius-secret")?;

        let mut config = enabled_config(&secret_path);
        config.dynamic_circuit_application.enabled = true;
        let missing_dynamic_circuits = runtime_config_from_config(
            Some(config.clone()),
            &Config {
                dynamic_circuits: None,
                ..Config::default()
            },
        )
        .await?;
        let disabled_dynamic_circuits = runtime_config_from_config(
            Some(config.clone()),
            &Config {
                dynamic_circuits: Some(DynamicCircuitsConfig::default()),
                ..Config::default()
            },
        )
        .await?;
        let enabled_dynamic_circuits = runtime_config_from_config(
            Some(config),
            &Config {
                dynamic_circuits: Some(DynamicCircuitsConfig {
                    enabled: true,
                    ..DynamicCircuitsConfig::default()
                }),
                ..Config::default()
            },
        )
        .await?;
        let _ = std::fs::remove_file(&secret_path);

        assert!(
            !missing_dynamic_circuits
                .expect("runtime config should build")
                .apply_dynamic_circuits
        );
        assert!(
            !disabled_dynamic_circuits
                .expect("runtime config should build")
                .apply_dynamic_circuits
        );
        assert!(
            enabled_dynamic_circuits
                .expect("runtime config should build")
                .apply_dynamic_circuits
        );

        Ok(())
    }

    #[tokio::test]
    async fn safety_gates_control_event_bus_application() -> anyhow::Result<()> {
        let secret_path = unique_secret_path("application-gate-bus-use")?;
        std::fs::write(&secret_path, b"radius-secret")?;

        let mut config = enabled_config(&secret_path);
        config.dynamic_circuit_application.enabled = true;
        config.dynamic_circuit_application.fallback_parent_node = Some("Core PPPoE".to_string());
        config.fallback_speed_profile = Some(RadiusFallbackSpeedProfile {
            download_min_mbps: 5.0,
            upload_min_mbps: 3.0,
            download_max_mbps: 25.0,
            upload_max_mbps: 10.0,
        });
        let disabled_runtime = runtime_config_from_config(
            Some(config.clone()),
            &Config {
                dynamic_circuits: Some(DynamicCircuitsConfig::default()),
                ..Config::default()
            },
        )
        .await?;
        let enabled_runtime = runtime_config_from_config(
            Some(config),
            &Config {
                dynamic_circuits: Some(DynamicCircuitsConfig {
                    enabled: true,
                    ..DynamicCircuitsConfig::default()
                }),
                ..Config::default()
            },
        )
        .await?;
        let _ = std::fs::remove_file(&secret_path);

        let disabled_runtime = disabled_runtime.expect("runtime config should build");
        let (disabled_bus_tx, mut disabled_bus_rx) = mpsc::channel(4);
        let mut disabled_sink = dynamic_circuit_application_sink(
            disabled_runtime.apply_dynamic_circuits,
            disabled_bus_tx,
        );
        assert!(disabled_sink.is_none());
        let mut disabled_sessions = sessions_from_runtime_config(disabled_runtime);

        let mut disabled_command_sink = selected_dynamic_circuit_sink(&mut disabled_sink);
        handle_accounting_event_with_command_sink(
            complete_event(AcctStatusType::Start),
            &mut disabled_sessions,
            &mut disabled_command_sink,
            Instant::now(),
            test_listen_addr(),
            64,
            20,
        );
        tokio::task::yield_now().await;
        assert!(matches!(
            disabled_bus_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected)
        ));
        handle_accounting_event_with_command_sink(
            complete_event(AcctStatusType::Stop),
            &mut disabled_sessions,
            &mut disabled_command_sink,
            Instant::now() + Duration::from_secs(1),
            test_listen_addr(),
            64,
            20,
        );
        tokio::task::yield_now().await;
        assert!(matches!(
            disabled_bus_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected)
        ));

        let enabled_runtime = enabled_runtime.expect("runtime config should build");
        let (enabled_bus_tx, mut enabled_bus_rx) = mpsc::channel(4);
        let mut enabled_sink = dynamic_circuit_application_sink(
            enabled_runtime.apply_dynamic_circuits,
            enabled_bus_tx,
        );
        assert!(enabled_sink.is_some());
        let mut enabled_sessions = sessions_from_runtime_config(enabled_runtime);

        let mut enabled_command_sink = selected_dynamic_circuit_sink(&mut enabled_sink);
        handle_accounting_event_with_command_sink(
            complete_event(AcctStatusType::Start),
            &mut enabled_sessions,
            &mut enabled_command_sink,
            Instant::now(),
            test_listen_addr(),
            64,
            20,
        );
        let (reply_tx, shaped_device) = receive_create_dynamic_circuit(&mut enabled_bus_rx).await?;
        assert_eq!(shaped_device.parent_node, "Core PPPoE");
        ack_bus_reply(reply_tx)?;

        Ok(())
    }

    #[tokio::test]
    async fn mac_match_setting_loads_shaped_devices_matcher() -> anyhow::Result<()> {
        let secret_path = unique_secret_path("mac-match-secret")?;
        let lqos_directory = unique_secret_path("mac-match-lqos-directory")?;
        std::fs::write(&secret_path, b"radius-secret")?;
        std::fs::create_dir(&lqos_directory)?;
        std::fs::write(
            lqos_directory.join("ShapedDevices.csv"),
            "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment,sqm\n\
circuit-runtime,Runtime Circuit,device-runtime,Runtime Device,Parent,ParentId,AnchorId,aa-bb-cc-dd-ee-ff,198.51.100.200,,5,2,50,20,fixture,cake\n",
        )?;

        let mut config = enabled_config(&secret_path);
        config.dynamic_circuit_application.enabled = true;
        config
            .dynamic_circuit_application
            .match_shaped_devices_by_mac = true;
        let config_snapshot = Config {
            dynamic_circuits: Some(DynamicCircuitsConfig {
                enabled: true,
                ..DynamicCircuitsConfig::default()
            }),
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            ..Config::default()
        };

        let dynamic_circuits_path = lqos_directory.join("dynamic_circuits.json");
        assert!(!dynamic_circuits_path.exists());

        let runtime_config = runtime_config_from_config(Some(config), &config_snapshot).await;
        let _ = std::fs::remove_file(&secret_path);
        let Some(runtime_config) = runtime_config? else {
            anyhow::bail!("enabled config should produce runtime config");
        };
        let default_ttl = runtime_config.default_ttl;
        let stale_grace = runtime_config.stale_grace;
        let fallback_rate_profile = runtime_config.fallback_rate_profile;
        let matcher = runtime_config
            .mac_matcher
            .expect("MAC match setting should load a matcher");
        let mut event = complete_event(AcctStatusType::Start);
        event.calling_station_id = Some("AABB.CCDD.EEFF".to_string());

        let ShapedDevicesMacMatch::Unique(device) = matcher.match_event(&event) else {
            anyhow::bail!("runtime matcher should find shaped-device MAC");
        };
        assert_eq!(device.circuit_id, "circuit-runtime");
        assert_eq!(device.device_id, "device-runtime");

        let mut sessions = RadiusAccountingSessions::new_with_fallback_and_mac_matcher(
            default_ttl,
            stale_grace,
            fallback_rate_profile,
            None,
            Some(matcher),
        );
        let mut event = complete_event_for(AcctStatusType::Start, "nas-runtime", "session-runtime");
        event.calling_station_id = Some("aa-bb-cc-dd-ee-ff".to_string());
        sessions.apply_event(event, Instant::now());

        let key = session_key_for("nas-runtime", "session-runtime");
        let session = sessions
            .store
            .session(&key)
            .expect("configured MAC matcher should retain the RADIUS session");
        assert!(session.pending_reasons.is_empty());
        let Some(resolved_device) = session.resolved_shaped_device.as_ref() else {
            anyhow::bail!("configured MAC matcher should resolve an in-memory ShapedDevice");
        };
        assert_eq!(resolved_device.circuit_id, "circuit-runtime");
        assert_eq!(resolved_device.device_id, "device-runtime");
        assert_eq!(resolved_device.parent_node, "Parent");
        assert_eq!(resolved_device.parent_node_id.as_deref(), Some("ParentId"));
        assert_eq!(resolved_device.anchor_node_id.as_deref(), Some("AnchorId"));
        assert_eq!(
            resolved_device.ipv4,
            vec![(Ipv4Addr::new(198, 51, 100, 20), 32)]
        );
        assert_eq!(resolved_device.download_min_mbps, 25.0);
        assert_eq!(resolved_device.upload_min_mbps, 10.0);
        assert_eq!(resolved_device.download_max_mbps, 25.0);
        assert_eq!(resolved_device.upload_max_mbps, 10.0);

        let mut shaped_rate_event = complete_event_for(
            AcctStatusType::Start,
            "nas-runtime",
            "session-runtime-shaped-rate",
        );
        shaped_rate_event.calling_station_id = Some("aa-bb-cc-dd-ee-ff".to_string());
        shaped_rate_event.mikrotik_rate_limits.clear();
        sessions.apply_event(shaped_rate_event, Instant::now());

        let shaped_rate_session = sessions
            .store
            .session(&session_key_for(
                "nas-runtime",
                "session-runtime-shaped-rate",
            ))
            .expect("configured MAC matcher should retain the no-packet-rate session");
        assert!(shaped_rate_session.pending_reasons.is_empty());
        let Some(shaped_rate_device) = shaped_rate_session.resolved_shaped_device.as_ref() else {
            anyhow::bail!("configured MAC matcher should use ShapedDevices.csv rates");
        };
        assert_eq!(shaped_rate_device.download_min_mbps, 5.0);
        assert_eq!(shaped_rate_device.upload_min_mbps, 2.0);
        assert_eq!(shaped_rate_device.download_max_mbps, 50.0);
        assert_eq!(shaped_rate_device.upload_max_mbps, 20.0);
        assert!(!dynamic_circuits_path.exists());

        let _ = std::fs::remove_file(lqos_directory.join("ShapedDevices.csv"));
        let _ = std::fs::remove_dir(&lqos_directory);

        Ok(())
    }

    #[tokio::test]
    async fn duplicate_mac_rows_load_and_leave_sessions_pending() -> anyhow::Result<()> {
        let secret_path = unique_secret_path("duplicate-mac-secret")?;
        let lqos_directory = unique_secret_path("duplicate-mac-lqos-directory")?;
        std::fs::write(&secret_path, b"radius-secret")?;
        std::fs::create_dir(&lqos_directory)?;
        std::fs::write(
            lqos_directory.join("ShapedDevices.csv"),
            "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment,sqm\n\
circuit-a,Runtime Circuit A,device-a,Runtime Device A,Parent,ParentId,AnchorId,aa-bb-cc-dd-ee-ff,198.51.100.200,,5,2,50,20,fixture,cake\n\
circuit-b,Runtime Circuit B,device-b,Runtime Device B,Parent,ParentId,AnchorId,AABB.CCDD.EEFF,198.51.100.201,,5,2,50,20,fixture,cake\n",
        )?;

        let mut config = enabled_config(&secret_path);
        config.dynamic_circuit_application.enabled = true;
        config
            .dynamic_circuit_application
            .match_shaped_devices_by_mac = true;
        let config_snapshot = Config {
            dynamic_circuits: Some(DynamicCircuitsConfig {
                enabled: true,
                ..DynamicCircuitsConfig::default()
            }),
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            ..Config::default()
        };

        let runtime_config = runtime_config_from_config(Some(config), &config_snapshot).await;
        let _ = std::fs::remove_file(&secret_path);
        let Some(runtime_config) = runtime_config? else {
            anyhow::bail!("enabled config should produce runtime config");
        };
        let default_ttl = runtime_config.default_ttl;
        let stale_grace = runtime_config.stale_grace;
        let fallback_rate_profile = runtime_config.fallback_rate_profile;
        let matcher = runtime_config
            .mac_matcher
            .expect("MAC match setting should load a matcher");

        let mut event =
            complete_event_for(AcctStatusType::Start, "nas-duplicate", "session-duplicate");
        event.calling_station_id = Some("AA:BB:CC:DD:EE:FF".to_string());
        assert_eq!(
            matcher.match_event(&event),
            ShapedDevicesMacMatch::Ambiguous
        );

        let mut sessions = RadiusAccountingSessions::new_with_fallback_and_mac_matcher(
            default_ttl,
            stale_grace,
            fallback_rate_profile,
            None,
            Some(matcher),
        );
        sessions.apply_event(event, Instant::now());

        let session = sessions
            .store
            .session(&session_key_for("nas-duplicate", "session-duplicate"))
            .expect("ambiguous MAC match should retain the RADIUS session");
        assert_eq!(
            session.pending_reasons,
            vec![PendingSessionReason::AmbiguousMacMatch]
        );
        assert!(session.resolved_shaped_device.is_none());

        let _ = std::fs::remove_file(lqos_directory.join("ShapedDevices.csv"));
        let _ = std::fs::remove_dir(&lqos_directory);

        Ok(())
    }

    #[tokio::test]
    async fn disabled_top_level_dynamic_circuits_skip_mac_matcher_load() -> anyhow::Result<()> {
        let secret_path = unique_secret_path("mac-match-top-gate-secret")?;
        let lqos_directory = unique_secret_path("mac-match-top-gate-lqos-directory")?;
        std::fs::write(&secret_path, b"radius-secret")?;
        std::fs::create_dir(&lqos_directory)?;

        let mut config = enabled_config(&secret_path);
        config.dynamic_circuit_application.enabled = true;
        config
            .dynamic_circuit_application
            .match_shaped_devices_by_mac = true;
        let config_snapshot = Config {
            dynamic_circuits: Some(DynamicCircuitsConfig::default()),
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            ..Config::default()
        };

        let runtime_config = runtime_config_from_config(Some(config), &config_snapshot).await;
        let _ = std::fs::remove_file(&secret_path);
        let _ = std::fs::remove_dir(&lqos_directory);
        let Some(runtime_config) = runtime_config? else {
            anyhow::bail!("enabled config should produce runtime config")
        };

        assert!(!runtime_config.apply_dynamic_circuits);
        assert!(runtime_config.mac_matcher.is_none());

        Ok(())
    }

    #[tokio::test]
    async fn mac_match_load_error_reports_shaped_devices_path() -> anyhow::Result<()> {
        let secret_path = unique_secret_path("mac-match-missing-secret")?;
        let lqos_directory = unique_secret_path("mac-match-missing-lqos-directory")?;
        std::fs::create_dir(&lqos_directory)?;

        let mut config = enabled_config(&secret_path);
        config.dynamic_circuit_application.enabled = true;
        config
            .dynamic_circuit_application
            .match_shaped_devices_by_mac = true;
        let config_snapshot = Config {
            dynamic_circuits: Some(DynamicCircuitsConfig {
                enabled: true,
                ..DynamicCircuitsConfig::default()
            }),
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            ..Config::default()
        };
        let expected_path = lqos_directory.join("ShapedDevices.csv");

        let error = match runtime_config_from_config(Some(config), &config_snapshot).await {
            Ok(_) => {
                let _ = std::fs::remove_dir(&lqos_directory);
                anyhow::bail!("missing ShapedDevices.csv should fail startup")
            }
            Err(error) => error,
        };
        let _ = std::fs::remove_dir(&lqos_directory);

        assert!(matches!(
            error,
            RadiusAccountingStartupError::ShapedDevicesLoad { .. }
        ));
        assert!(
            error
                .to_string()
                .contains(&expected_path.display().to_string())
        );

        Ok(())
    }

    #[test]
    fn mac_matching_uses_carried_session_mac_after_sparse_merge() -> anyhow::Result<()> {
        let matched_device = shaped_device("circuit-carried-mac", "device-carried-mac");
        let matcher = ShapedDevicesMacMatcher::from_devices(&[matched_device]);
        let mut sessions = RadiusAccountingSessions::new_with_fallback_and_mac_matcher(
            Duration::from_secs(900),
            Duration::from_secs(120),
            None,
            None,
            Some(matcher),
        );
        let key = session_key_for("nas-carried-mac", "session-carried-mac");

        let mut start = complete_event_for(
            AcctStatusType::Start,
            "nas-carried-mac",
            "session-carried-mac",
        );
        start.calling_station_id = Some("aa:bb:cc:dd:ee:ff".to_string());
        start.framed_ip_address = None;
        start.mikrotik_rate_limits.clear();
        sessions.apply_event(start, Instant::now());
        assert_eq!(
            sessions
                .store
                .session(&key)
                .expect("start should retain the RADIUS session")
                .pending_reasons,
            vec![PendingSessionReason::MissingIpAddress]
        );

        let mut interim = complete_event_for(
            AcctStatusType::InterimUpdate,
            "nas-carried-mac",
            "session-carried-mac",
        );
        interim.calling_station_id = None;
        interim.framed_ip_address = Some(Ipv4Addr::new(203, 0, 113, 45));
        sessions.apply_event(interim, Instant::now() + Duration::from_secs(1));

        let session = sessions
            .store
            .session(&key)
            .expect("interim update should retain the RADIUS session");
        assert_eq!(
            session.latest_event.calling_station_id.as_deref(),
            Some("aa:bb:cc:dd:ee:ff")
        );
        assert!(session.pending_reasons.is_empty());
        let Some(resolved_device) = session.resolved_shaped_device.as_ref() else {
            anyhow::bail!("MAC match should resolve a shaped-device definition");
        };
        assert_eq!(resolved_device.circuit_id, "circuit-carried-mac");
        assert_eq!(resolved_device.device_id, "device-carried-mac");
        assert_eq!(resolved_device.parent_node, "Parent Node");
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
            vec![(Ipv4Addr::new(203, 0, 113, 45), 32)]
        );
        assert!(resolved_device.ipv6.is_empty());
        assert_eq!(resolved_device.download_min_mbps, 25.0);
        assert_eq!(resolved_device.upload_min_mbps, 10.0);
        assert_eq!(resolved_device.download_max_mbps, 25.0);
        assert_eq!(resolved_device.upload_max_mbps, 10.0);

        Ok(())
    }

    #[test]
    fn fallback_profile_without_parent_metadata_stays_pending() -> anyhow::Result<()> {
        let fallback_rate = SessionRateProfile::new(4.0, 2.0, 40.0, 12.0)?;
        let mut sessions = RadiusAccountingSessions::new_with_fallback_and_mac_matcher(
            Duration::from_secs(900),
            Duration::from_secs(120),
            Some(fallback_rate),
            None,
            None,
        );
        let mut event = complete_event(AcctStatusType::Start);
        event.mikrotik_rate_limits.clear();
        event.user_name = Some("subscriber@example.net".to_string());
        event.calling_station_id = Some("AA-BB-CC-DD-EE-FF".to_string());
        let now = Instant::now();

        sessions.apply_event(event, now);

        let key = session_key();
        let Some(session) = sessions.store.session(&key) else {
            anyhow::bail!("accepted event should create an in-memory session");
        };
        assert_eq!(
            session.pending_reasons,
            vec![PendingSessionReason::MissingParent]
        );
        assert_eq!(sessions.updated_at.get(&key), Some(&now));
        assert_eq!(
            session.resolved_rate.map(|rate| rate.profile),
            Some(fallback_rate)
        );
        assert!(session.resolved_shaped_device.is_none());

        Ok(())
    }

    #[test]
    fn fallback_parent_metadata_resolves_default_identity() -> anyhow::Result<()> {
        let fallback_rate = SessionRateProfile::new(4.0, 2.0, 40.0, 12.0)?;
        let fallback_parent = DynamicCircuitParent {
            parent_node: "Core PPPoE".to_string(),
            parent_node_id: Some("core-pppoe".to_string()),
            anchor_node_id: Some("radius-anchor".to_string()),
        };
        let mut sessions = RadiusAccountingSessions::new_with_fallback_and_mac_matcher(
            Duration::from_secs(900),
            Duration::from_secs(120),
            Some(fallback_rate),
            Some(fallback_parent),
            None,
        );
        let mut event = complete_event(AcctStatusType::Start);
        event.mikrotik_rate_limits.clear();
        event.user_name = Some("subscriber@example.net".to_string());
        event.calling_station_id = Some("AA-BB-CC-DD-EE-FF".to_string());

        sessions.apply_event(event, Instant::now());

        let Some(session) = sessions.store.session(&session_key()) else {
            anyhow::bail!("accepted event should create an in-memory session");
        };
        assert!(session.pending_reasons.is_empty());
        let Some(resolved_device) = session.resolved_shaped_device.as_ref() else {
            anyhow::bail!("fallback parent should resolve a default shaped device");
        };
        assert_eq!(resolved_device.circuit_name, "subscriber@example.net");
        assert_eq!(resolved_device.device_name, "AA-BB-CC-DD-EE-FF");
        assert_eq!(resolved_device.parent_node, "Core PPPoE");
        assert_eq!(
            resolved_device.parent_node_id.as_deref(),
            Some("core-pppoe")
        );
        assert_eq!(
            resolved_device.anchor_node_id.as_deref(),
            Some("radius-anchor")
        );
        assert_eq!(resolved_device.download_min_mbps, 4.0);
        assert_eq!(resolved_device.upload_min_mbps, 2.0);
        assert_eq!(resolved_device.download_max_mbps, 40.0);
        assert_eq!(resolved_device.upload_max_mbps, 12.0);

        Ok(())
    }

    #[test]
    fn adapter_boundary_emits_deferred_command_intents() -> anyhow::Result<()> {
        let fallback_rate = SessionRateProfile::new(4.0, 2.0, 40.0, 12.0)?;
        let fallback_parent = DynamicCircuitParent {
            parent_node: "Core PPPoE".to_string(),
            parent_node_id: Some("core-pppoe".to_string()),
            anchor_node_id: Some("radius-anchor".to_string()),
        };
        let mut sessions = RadiusAccountingSessions::new_with_fallback_and_mac_matcher(
            Duration::from_secs(900),
            Duration::from_secs(120),
            Some(fallback_rate),
            Some(fallback_parent),
            None,
        );
        let mut sink = RecordingDynamicCircuitSink::default();
        let key = session_key();
        let Some(circuit_id) = key.dynamic_circuit_id() else {
            anyhow::bail!("test session key should have a dynamic circuit id");
        };

        handle_accounting_event_with_command_sink(
            complete_event(AcctStatusType::Start),
            &mut sessions,
            &mut sink,
            Instant::now(),
            test_listen_addr(),
            64,
            20,
        );
        handle_accounting_event_with_command_sink(
            complete_event(AcctStatusType::InterimUpdate),
            &mut sessions,
            &mut sink,
            Instant::now() + Duration::from_secs(1),
            test_listen_addr(),
            64,
            20,
        );
        handle_accounting_event_with_command_sink(
            complete_event(AcctStatusType::Stop),
            &mut sessions,
            &mut sink,
            Instant::now() + Duration::from_secs(2),
            test_listen_addr(),
            64,
            20,
        );

        assert_eq!(sink.intents.len(), 3);
        let DynamicCircuitIntent::CreateDynamicCircuit(create) = &sink.intents[0] else {
            anyhow::bail!("expected create intent, got {:?}", sink.intents[0]);
        };
        assert_eq!(create.circuit_id, circuit_id);
        assert_eq!(create.session_key, key);
        assert_eq!(create.shaped_device.parent_node, "Core PPPoE");
        assert_eq!(create.shaped_device.circuit_name, "subscriber-adapter");

        let DynamicCircuitIntent::UpdateDynamicCircuit(update) = &sink.intents[1] else {
            anyhow::bail!("expected update intent, got {:?}", sink.intents[1]);
        };
        assert_eq!(update.circuit_id, circuit_id);
        assert_eq!(update.session_key, key);

        let DynamicCircuitIntent::RemoveDynamicCircuit(stop) = &sink.intents[2] else {
            anyhow::bail!("expected stop removal intent, got {:?}", sink.intents[2]);
        };
        assert_eq!(stop.circuit_id, circuit_id);
        assert_eq!(stop.session_key, key);
        assert_eq!(stop.reason, DynamicCircuitRemovalReason::Stop);

        Ok(())
    }

    #[tokio::test]
    async fn applying_sink_orders_create_requests_without_blocking_packet_handling()
    -> anyhow::Result<()> {
        let fallback_rate = SessionRateProfile::new(4.0, 2.0, 40.0, 12.0)?;
        let fallback_parent = DynamicCircuitParent::new("Core PPPoE");
        let mut sessions = RadiusAccountingSessions::new_with_fallback_and_mac_matcher(
            Duration::from_secs(900),
            Duration::from_secs(120),
            Some(fallback_rate),
            Some(fallback_parent),
            None,
        );
        let (bus_tx, mut bus_rx) = mpsc::channel(4);
        let mut sink = ApplyingDynamicCircuitSink::new(bus_tx);
        let key = session_key();
        let circuit_id = key
            .dynamic_circuit_id()
            .expect("test session key should have a dynamic circuit id");

        handle_accounting_event_with_command_sink(
            complete_event(AcctStatusType::Start),
            &mut sessions,
            &mut sink,
            Instant::now(),
            test_listen_addr(),
            64,
            20,
        );
        let (start_reply, start_device) = receive_create_dynamic_circuit(&mut bus_rx).await?;
        assert_eq!(start_device.circuit_id, circuit_id);
        assert_eq!(start_device.parent_node, "Core PPPoE");
        assert_eq!(start_device.circuit_name, "subscriber-adapter");

        let mut interim = complete_event(AcctStatusType::InterimUpdate);
        interim.framed_ip_address = Some(Ipv4Addr::new(198, 51, 100, 21));
        handle_accounting_event_with_command_sink(
            interim,
            &mut sessions,
            &mut sink,
            Instant::now() + Duration::from_secs(1),
            test_listen_addr(),
            64,
            20,
        );
        tokio::task::yield_now().await;
        assert!(
            matches!(bus_rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
            "second upsert should wait behind the first bus reply"
        );
        fail_bus_reply(start_reply, "fake create failure")?;

        let (update_reply, update_device) = receive_create_dynamic_circuit(&mut bus_rx).await?;
        assert_eq!(update_device.circuit_id, circuit_id);
        assert_eq!(
            update_device.ipv4,
            vec![(Ipv4Addr::new(198, 51, 100, 21), 32)]
        );
        ack_bus_reply(update_reply)?;

        Ok(())
    }

    #[tokio::test]
    async fn failed_create_records_apply_failed_diagnostic_without_secret() -> anyhow::Result<()> {
        let fallback_rate = SessionRateProfile::new(4.0, 2.0, 40.0, 12.0)?;
        let fallback_parent = DynamicCircuitParent::new("Core PPPoE");
        let mut sessions = RadiusAccountingSessions::new_with_fallback_and_mac_matcher(
            Duration::from_secs(900),
            Duration::from_secs(120),
            Some(fallback_rate),
            Some(fallback_parent),
            None,
        );
        let (bus_tx, mut bus_rx) = mpsc::channel(4);
        let mut sink = ApplyingDynamicCircuitSink::new(bus_tx);
        let key = session_key();
        let circuit_id = key
            .dynamic_circuit_id()
            .expect("test session key should have a dynamic circuit id");

        handle_accounting_event_with_command_sink(
            complete_event(AcctStatusType::Start),
            &mut sessions,
            &mut sink,
            Instant::now(),
            test_listen_addr(),
            64,
            20,
        );
        let (reply_tx, shaped_device) = receive_create_dynamic_circuit(&mut bus_rx).await?;
        assert_eq!(shaped_device.circuit_id, circuit_id);
        fail_bus_reply(
            reply_tx,
            "fake apply failure code=queue-parent-missing shared secret secret-value api key = diagnostic-key",
        )?;

        let diagnostic =
            wait_for_apply_failed_diagnostic(&sink, &sessions, &circuit_id, &key).await?;
        assert_eq!(
            diagnostic.state,
            RadiusActivationDiagnosticState::ApplyFailed
        );
        assert_eq!(diagnostic.session_key, key);
        assert_eq!(diagnostic.circuit_ids, vec![circuit_id.clone()]);
        let apply_error = diagnostic
            .apply_error
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("expected apply_error detail"))?;
        assert!(apply_error.contains("code=queue-parent-missing"));
        assert!(apply_error.contains("[redacted]"));
        assert!(!apply_error.contains("secret-value"));
        assert!(!apply_error.contains("diagnostic-key"));
        let diagnostic_debug = format!("{diagnostic:?}");
        assert!(!diagnostic_debug.contains("secret-value"));
        assert!(!diagnostic_debug.contains("diagnostic-key"));
        let diagnostics = sink.activation_diagnostics(&sessions);
        assert_eq!(
            diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.session_key == key
                    && diagnostic.circuit_ids.iter().any(|id| id == &circuit_id))
                .count(),
            1
        );
        assert!(!diagnostics.iter().any(|diagnostic| {
            diagnostic.session_key == key
                && diagnostic.circuit_ids.iter().any(|id| id == &circuit_id)
                && diagnostic.state == RadiusActivationDiagnosticState::Active
        }));

        handle_accounting_event_with_command_sink(
            complete_event(AcctStatusType::Stop),
            &mut sessions,
            &mut sink,
            Instant::now() + Duration::from_secs(1),
            test_listen_addr(),
            64,
            20,
        );
        tokio::task::yield_now().await;
        assert_bus_request_channel_empty(&mut bus_rx);
        let diagnostics = sink.activation_diagnostics(&sessions);
        assert!(!diagnostics.iter().any(|diagnostic| {
            diagnostic.session_key == key
                && diagnostic.circuit_ids.iter().any(|id| id == &circuit_id)
                && diagnostic.state == RadiusActivationDiagnosticState::ApplyFailed
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.session_key == key
                && diagnostic.circuit_ids.iter().any(|id| id == &circuit_id)
                && diagnostic.state == RadiusActivationDiagnosticState::Stopped
        }));

        Ok(())
    }

    #[tokio::test]
    async fn stop_while_create_is_pending_queues_removal_after_create_reply() -> anyhow::Result<()>
    {
        let fallback_rate = SessionRateProfile::new(4.0, 2.0, 40.0, 12.0)?;
        let fallback_parent = DynamicCircuitParent::new("Core PPPoE");
        let mut sessions = RadiusAccountingSessions::new_with_fallback_and_mac_matcher(
            Duration::from_secs(900),
            Duration::from_secs(120),
            Some(fallback_rate),
            Some(fallback_parent),
            None,
        );
        let (bus_tx, mut bus_rx) = mpsc::channel(4);
        let mut sink = ApplyingDynamicCircuitSink::new(bus_tx);
        let key = session_key();
        let circuit_id = key
            .dynamic_circuit_id()
            .expect("test session key should have a dynamic circuit id");
        let started_at = Instant::now();

        handle_accounting_event_with_command_sink(
            complete_event(AcctStatusType::Start),
            &mut sessions,
            &mut sink,
            started_at,
            test_listen_addr(),
            64,
            20,
        );
        let (start_reply, start_device) = receive_create_dynamic_circuit(&mut bus_rx).await?;
        assert_eq!(start_device.circuit_id, circuit_id);

        handle_accounting_event_with_command_sink(
            complete_event(AcctStatusType::Stop),
            &mut sessions,
            &mut sink,
            started_at + Duration::from_secs(1),
            test_listen_addr(),
            64,
            20,
        );
        tokio::task::yield_now().await;
        assert_bus_request_channel_empty(&mut bus_rx);

        ack_bus_reply(start_reply)?;
        assert_eq!(receive_remove_and_ack(&mut bus_rx).await?, circuit_id);

        Ok(())
    }

    #[tokio::test]
    async fn applying_sink_submits_stop_expiry_and_stale_removals() -> anyhow::Result<()> {
        let fallback_rate = SessionRateProfile::new(4.0, 2.0, 40.0, 12.0)?;
        let fallback_parent = DynamicCircuitParent::new("Core PPPoE");
        let (bus_tx, mut bus_rx) = mpsc::channel(8);
        let mut sink = ApplyingDynamicCircuitSink::new(bus_tx);

        let mut stop_sessions = RadiusAccountingSessions::new_with_fallback_and_mac_matcher(
            Duration::from_secs(900),
            Duration::from_secs(120),
            Some(fallback_rate),
            Some(fallback_parent.clone()),
            None,
        );
        let stop_circuit_id = session_key()
            .dynamic_circuit_id()
            .expect("test session key should have a dynamic circuit id");
        let started_at = Instant::now();
        handle_accounting_event_with_command_sink(
            complete_event(AcctStatusType::Start),
            &mut stop_sessions,
            &mut sink,
            started_at,
            test_listen_addr(),
            64,
            20,
        );
        receive_create_and_ack(&mut bus_rx).await?;

        handle_accounting_event_with_command_sink(
            complete_event(AcctStatusType::Stop),
            &mut stop_sessions,
            &mut sink,
            started_at + Duration::from_secs(1),
            test_listen_addr(),
            64,
            20,
        );
        assert_eq!(receive_remove_and_ack(&mut bus_rx).await?, stop_circuit_id);
        handle_accounting_event_with_command_sink(
            complete_event(AcctStatusType::Stop),
            &mut stop_sessions,
            &mut sink,
            started_at + Duration::from_secs(2),
            test_listen_addr(),
            64,
            20,
        );
        tokio::task::yield_now().await;
        assert_bus_request_channel_empty(&mut bus_rx);

        let mut expiry_sessions = RadiusAccountingSessions::new_with_fallback_and_mac_matcher(
            Duration::from_secs(10),
            Duration::from_secs(2),
            Some(fallback_rate),
            Some(fallback_parent.clone()),
            None,
        );
        let expiry_key = session_key_for("nas-expiry", "session-expiry");
        let expiry_circuit_id = expiry_key
            .dynamic_circuit_id()
            .expect("test session key should have a dynamic circuit id");
        expiry_sessions.apply_event_with_command_sink(
            complete_event_for(AcctStatusType::Start, "nas-expiry", "session-expiry"),
            started_at,
            &mut sink,
        );
        receive_create_and_ack(&mut bus_rx).await?;
        assert_eq!(
            expiry_sessions
                .expire_due_with_command_sink(started_at + Duration::from_secs(9), &mut sink,),
            0
        );
        assert_bus_request_channel_empty(&mut bus_rx);
        assert_eq!(
            expiry_sessions
                .expire_due_with_command_sink(started_at + Duration::from_secs(10), &mut sink,),
            1
        );
        assert_eq!(
            receive_remove_and_ack(&mut bus_rx).await?,
            expiry_circuit_id
        );

        let mut reset_sessions = RadiusAccountingSessions::new_with_fallback_and_mac_matcher(
            Duration::from_secs(10),
            Duration::from_secs(2),
            Some(fallback_rate),
            Some(fallback_parent),
            None,
        );
        let reset_key = session_key_for("nas-reset", "session-reset");
        let reset_circuit_id = reset_key
            .dynamic_circuit_id()
            .expect("test session key should have a dynamic circuit id");
        reset_sessions.apply_event_with_command_sink(
            complete_event_for(AcctStatusType::Start, "nas-reset", "session-reset"),
            started_at,
            &mut sink,
        );
        receive_create_and_ack(&mut bus_rx).await?;
        let reset_at = started_at + Duration::from_secs(1);
        reset_sessions.apply_event_with_command_sink(
            reset_event_for("nas-reset"),
            reset_at,
            &mut sink,
        );
        tokio::task::yield_now().await;
        assert_bus_request_channel_empty(&mut bus_rx);
        assert_eq!(
            reset_sessions
                .expire_due_with_command_sink(reset_at + Duration::from_secs(1), &mut sink),
            0
        );
        assert_bus_request_channel_empty(&mut bus_rx);
        assert_eq!(
            reset_sessions
                .expire_due_with_command_sink(reset_at + Duration::from_secs(2), &mut sink),
            1
        );
        assert_eq!(receive_remove_and_ack(&mut bus_rx).await?, reset_circuit_id);

        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn stale_reset_after_cleanup_wake_removes_at_grace_deadline() -> anyhow::Result<()> {
        let fallback_rate = SessionRateProfile::new(4.0, 2.0, 40.0, 12.0)?;
        let fallback_parent = DynamicCircuitParent::new("Core PPPoE");
        let sessions = RadiusAccountingSessions::new_with_fallback_and_mac_matcher(
            Duration::from_secs(60),
            Duration::from_secs(2),
            Some(fallback_rate),
            Some(fallback_parent),
            None,
        );
        let (bus_tx, mut bus_rx) = mpsc::channel(4);
        let sink = ApplyingDynamicCircuitSink::new(bus_tx);
        let (event_tx, event_rx) = mpsc::channel(4);
        let loop_handle = tokio::spawn(run_test_accounting_event_loop(
            event_rx,
            sessions,
            Some(sink),
        ));
        let session_key = session_key_for("nas-deadline", "session-deadline");
        let circuit_id = session_key
            .dynamic_circuit_id()
            .expect("test session key should have a dynamic circuit id");

        send_test_accounting_event(
            &event_tx,
            complete_event_for(AcctStatusType::Start, "nas-deadline", "session-deadline"),
        )
        .await?;
        receive_create_and_ack(&mut bus_rx).await?;

        tokio::time::advance(Duration::from_secs(2)).await;
        tokio::task::yield_now().await;
        assert_bus_request_channel_empty(&mut bus_rx);

        tokio::time::advance(Duration::from_millis(1)).await;
        let reset_at = radius_accounting_now();
        send_test_accounting_event(&event_tx, reset_event_for("nas-deadline")).await?;
        assert_bus_request_channel_empty(&mut bus_rx);

        tokio::time::advance(Duration::from_millis(1_999)).await;
        tokio::task::yield_now().await;
        assert!(radius_accounting_now() < reset_at + Duration::from_secs(2));
        assert_bus_request_channel_empty(&mut bus_rx);

        tokio::time::advance(Duration::from_millis(1)).await;
        send_test_accounting_event(
            &event_tx,
            complete_event_for(
                AcctStatusType::InterimUpdate,
                "nas-deadline",
                "session-deadline",
            ),
        )
        .await?;
        assert_eq!(receive_remove_and_ack(&mut bus_rx).await?, circuit_id);
        let refreshed_device = receive_create_and_ack(&mut bus_rx).await?;
        assert_eq!(refreshed_device.circuit_id, circuit_id);

        drop(event_tx);
        loop_handle.await?;

        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn stale_reset_timer_removes_without_followup_packet() -> anyhow::Result<()> {
        let fallback_rate = SessionRateProfile::new(4.0, 2.0, 40.0, 12.0)?;
        let fallback_parent = DynamicCircuitParent::new("Core PPPoE");
        let stale_grace = Duration::from_millis(500);
        let sessions = RadiusAccountingSessions::new_with_fallback_and_mac_matcher(
            Duration::from_secs(60),
            stale_grace,
            Some(fallback_rate),
            Some(fallback_parent),
            None,
        );
        let (bus_tx, mut bus_rx) = mpsc::channel(4);
        let sink = ApplyingDynamicCircuitSink::new(bus_tx);
        let (event_tx, event_rx) = mpsc::channel(4);
        let loop_handle = tokio::spawn(run_test_accounting_event_loop(
            event_rx,
            sessions,
            Some(sink),
        ));
        let session_key = session_key_for("nas-timer", "session-timer");
        let circuit_id = session_key
            .dynamic_circuit_id()
            .expect("test session key should have a dynamic circuit id");

        send_test_accounting_event(
            &event_tx,
            complete_event_for(AcctStatusType::Start, "nas-timer", "session-timer"),
        )
        .await?;
        receive_create_and_ack(&mut bus_rx).await?;
        send_test_accounting_event(&event_tx, reset_event_for("nas-timer")).await?;
        assert_bus_request_channel_empty(&mut bus_rx);

        tokio::time::advance(stale_grace).await;
        tokio::task::yield_now().await;
        assert_eq!(receive_remove_and_ack(&mut bus_rx).await?, circuit_id);

        drop(event_tx);
        loop_handle.await?;

        Ok(())
    }

    #[tokio::test]
    async fn refresh_before_expiry_keeps_radius_circuit_active() -> anyhow::Result<()> {
        let fallback_rate = SessionRateProfile::new(4.0, 2.0, 40.0, 12.0)?;
        let fallback_parent = DynamicCircuitParent::new("Core PPPoE");
        let (bus_tx, mut bus_rx) = mpsc::channel(4);
        let mut sink = ApplyingDynamicCircuitSink::new(bus_tx);
        let mut sessions = RadiusAccountingSessions::new_with_fallback_and_mac_matcher(
            Duration::from_secs(10),
            Duration::from_secs(2),
            Some(fallback_rate),
            Some(fallback_parent),
            None,
        );
        let started_at = Instant::now();
        let reset_at = started_at + Duration::from_secs(1);
        let refreshed_at = started_at + Duration::from_secs(2);
        let key = session_key_for("nas-refresh", "session-refresh");
        let circuit_id = key
            .dynamic_circuit_id()
            .expect("test session key should have a dynamic circuit id");

        sessions.apply_event_with_command_sink(
            complete_event_for(AcctStatusType::Start, "nas-refresh", "session-refresh"),
            started_at,
            &mut sink,
        );
        receive_create_and_ack(&mut bus_rx).await?;
        sessions.apply_event_with_command_sink(reset_event_for("nas-refresh"), reset_at, &mut sink);
        tokio::task::yield_now().await;
        assert_bus_request_channel_empty(&mut bus_rx);

        sessions.apply_event_with_command_sink(
            complete_event_for(
                AcctStatusType::InterimUpdate,
                "nas-refresh",
                "session-refresh",
            ),
            refreshed_at,
            &mut sink,
        );
        let (refresh_reply, refreshed_device) = receive_create_dynamic_circuit(&mut bus_rx).await?;
        assert_eq!(refreshed_device.circuit_id, circuit_id);
        ack_bus_reply(refresh_reply)?;
        assert_eq!(
            sessions.expire_due_with_command_sink(reset_at + Duration::from_secs(2), &mut sink),
            0
        );
        assert_bus_request_channel_empty(&mut bus_rx);
        assert_eq!(
            sessions
                .expire_due_with_command_sink(refreshed_at + Duration::from_secs(10), &mut sink),
            1
        );
        assert_eq!(receive_remove_and_ack(&mut bus_rx).await?, circuit_id);

        Ok(())
    }

    #[tokio::test]
    async fn single_owner_mac_matched_stop_removes_shared_circuit_id() -> anyhow::Result<()> {
        let (bus_tx, mut bus_rx) = mpsc::channel(4);
        let mut sink = ApplyingDynamicCircuitSink::new(bus_tx);
        let mut sessions =
            shared_circuit_sessions(Duration::from_secs(900), Duration::from_secs(120));
        let started_at = Instant::now();

        sessions.apply_event_with_command_sink(
            shared_circuit_event(AcctStatusType::Start, "nas-a", "session-a", SHARED_MAC_A),
            started_at,
            &mut sink,
        );
        assert_shared_circuit_create(&mut bus_rx, "device-a").await?;

        sessions.apply_event_with_command_sink(
            complete_event_for(AcctStatusType::Stop, "nas-a", "session-a"),
            started_at + Duration::from_secs(1),
            &mut sink,
        );
        assert_eq!(
            receive_remove_and_ack(&mut bus_rx).await?,
            SHARED_CIRCUIT_ID
        );

        Ok(())
    }

    #[test]
    fn mac_matched_stopped_diagnostic_retains_shaped_devices_circuit_id() -> anyhow::Result<()> {
        let mut sink = RecordingDynamicCircuitSink::default();
        let mut sessions =
            shared_circuit_sessions(Duration::from_secs(900), Duration::from_secs(120));
        let started_at = Instant::now();
        let session_key = session_key_for("nas-stop-diagnostic", "session-stop-diagnostic");

        sessions.apply_event_with_command_sink(
            shared_circuit_event(
                AcctStatusType::Start,
                "nas-stop-diagnostic",
                "session-stop-diagnostic",
                SHARED_MAC_A,
            ),
            started_at,
            &mut sink,
        );
        sessions.apply_event_with_command_sink(
            complete_event_for(
                AcctStatusType::Stop,
                "nas-stop-diagnostic",
                "session-stop-diagnostic",
            ),
            started_at + Duration::from_secs(1),
            &mut sink,
        );

        let diagnostic = sessions
            .activation_diagnostics()
            .into_iter()
            .find(|diagnostic| diagnostic.session_key == session_key)
            .expect("stopped diagnostic should be retained");
        assert_eq!(diagnostic.state, RadiusActivationDiagnosticState::Stopped);
        assert_eq!(diagnostic.circuit_ids, vec![SHARED_CIRCUIT_ID.to_string()]);

        Ok(())
    }

    #[test]
    fn mac_matched_stale_expired_diagnostic_retains_shaped_devices_circuit_id() -> anyhow::Result<()>
    {
        let mut sink = RecordingDynamicCircuitSink::default();
        let mut sessions =
            shared_circuit_sessions(Duration::from_secs(900), Duration::from_secs(2));
        let started_at = Instant::now();
        let reset_at = started_at + Duration::from_secs(1);
        let session_key = session_key_for("nas-expired-diagnostic", "session-expired-diagnostic");

        sessions.apply_event_with_command_sink(
            shared_circuit_event(
                AcctStatusType::Start,
                "nas-expired-diagnostic",
                "session-expired-diagnostic",
                SHARED_MAC_A,
            ),
            started_at,
            &mut sink,
        );
        sessions.apply_event_with_command_sink(
            reset_event_for("nas-expired-diagnostic"),
            reset_at,
            &mut sink,
        );
        assert_eq!(
            sessions.expire_due_with_command_sink(reset_at + Duration::from_secs(2), &mut sink),
            1
        );

        let diagnostic = sessions
            .activation_diagnostics()
            .into_iter()
            .find(|diagnostic| diagnostic.session_key == session_key)
            .expect("expired diagnostic should be retained");
        assert_eq!(diagnostic.state, RadiusActivationDiagnosticState::Expired);
        assert_eq!(diagnostic.circuit_ids, vec![SHARED_CIRCUIT_ID.to_string()]);

        Ok(())
    }

    #[tokio::test]
    async fn stop_removal_promotes_retained_shared_circuit_owner() -> anyhow::Result<()> {
        let (bus_tx, mut bus_rx) = mpsc::channel(4);
        let mut sink = ApplyingDynamicCircuitSink::new(bus_tx);
        let mut sessions =
            shared_circuit_sessions(Duration::from_secs(900), Duration::from_secs(120));
        let started_at = Instant::now();

        sessions.apply_event_with_command_sink(
            shared_circuit_event(AcctStatusType::Start, "nas-a", "session-a", SHARED_MAC_A),
            started_at,
            &mut sink,
        );
        assert_shared_circuit_create(&mut bus_rx, "device-a").await?;
        wait_for_authoritative_owner(&sink, &session_key_for("nas-a", "session-a")).await?;

        sessions.apply_event_with_command_sink(
            shared_circuit_event(AcctStatusType::Start, "nas-b", "session-b", SHARED_MAC_B),
            started_at + Duration::from_secs(1),
            &mut sink,
        );

        sessions.apply_event_with_command_sink(
            complete_event_for(AcctStatusType::Stop, "nas-a", "session-a"),
            started_at + Duration::from_secs(2),
            &mut sink,
        );
        assert_shared_circuit_create(&mut bus_rx, "device-b").await?;
        wait_for_authoritative_owner(&sink, &session_key_for("nas-b", "session-b")).await?;

        let session_c = session_key_for("nas-c", "session-c");
        sessions.apply_event_with_command_sink(
            shared_circuit_event(AcctStatusType::Start, "nas-c", "session-c", SHARED_MAC_C),
            started_at + Duration::from_secs(3),
            &mut sink,
        );
        wait_for_retained_owner(&sink, &session_c).await?;
        assert_bus_request_channel_empty(&mut bus_rx);

        sessions.apply_event_with_command_sink(
            complete_event_for(AcctStatusType::Stop, "nas-a", "session-a"),
            started_at + Duration::from_secs(4),
            &mut sink,
        );
        tokio::task::yield_now().await;
        assert_bus_request_channel_empty(&mut bus_rx);

        Ok(())
    }

    #[tokio::test]
    async fn failed_shared_owner_promotion_falls_back_to_removal() -> anyhow::Result<()> {
        let (bus_tx, mut bus_rx) = mpsc::channel(4);
        let mut sink = ApplyingDynamicCircuitSink::new(bus_tx);
        let mut sessions =
            shared_circuit_sessions(Duration::from_secs(900), Duration::from_secs(120));
        let started_at = Instant::now();
        let owner_session = session_key_for("nas-a", "session-a");
        let retained_session = session_key_for("nas-b", "session-b");

        sessions.apply_event_with_command_sink(
            shared_circuit_event(AcctStatusType::Start, "nas-a", "session-a", SHARED_MAC_A),
            started_at,
            &mut sink,
        );
        assert_shared_circuit_create(&mut bus_rx, "device-a").await?;
        wait_for_authoritative_owner(&sink, &owner_session).await?;

        sessions.apply_event_with_command_sink(
            shared_circuit_event(AcctStatusType::Start, "nas-b", "session-b", SHARED_MAC_B),
            started_at + Duration::from_secs(1),
            &mut sink,
        );
        wait_for_retained_owner(&sink, &retained_session).await?;

        sessions.apply_event_with_command_sink(
            complete_event_for(AcctStatusType::Stop, "nas-a", "session-a"),
            started_at + Duration::from_secs(2),
            &mut sink,
        );
        let (promotion_reply, promotion_device) =
            receive_create_dynamic_circuit(&mut bus_rx).await?;
        assert_eq!(promotion_device.device_id, "device-b");
        fail_bus_reply(promotion_reply, "fake promotion failure")?;
        assert_eq!(
            receive_remove_and_ack(&mut bus_rx).await?,
            SHARED_CIRCUIT_ID
        );
        wait_for_released_owner(&sink, &owner_session).await?;

        let state = sink.application_state.lock();
        assert!(
            !state
                .current_owners_by_circuit
                .contains_key(SHARED_CIRCUIT_ID)
        );
        assert!(
            !state
                .authoritative_session_by_circuit
                .contains_key(SHARED_CIRCUIT_ID)
        );
        assert!(
            !state
                .retained_upserts_by_owner
                .contains_key(&(SHARED_CIRCUIT_ID.to_string(), retained_session.clone()))
        );
        drop(state);
        let diagnostics = sink.activation_diagnostics(&sessions);
        assert!(!diagnostics.iter().any(|diagnostic| {
            diagnostic.session_key == retained_session
                && diagnostic.circuit_ids == vec![SHARED_CIRCUIT_ID.to_string()]
                && diagnostic.state == RadiusActivationDiagnosticState::ApplyFailed
        }));

        Ok(())
    }

    #[tokio::test]
    async fn expiry_removal_promotes_retained_shared_circuit_owner() -> anyhow::Result<()> {
        let (bus_tx, mut bus_rx) = mpsc::channel(4);
        let mut sink = ApplyingDynamicCircuitSink::new(bus_tx);
        let mut sessions = shared_circuit_sessions(Duration::from_secs(10), Duration::from_secs(2));
        let started_at = Instant::now();

        sessions.apply_event_with_command_sink(
            shared_circuit_event(AcctStatusType::Start, "nas-a", "session-a", SHARED_MAC_A),
            started_at,
            &mut sink,
        );
        assert_shared_circuit_create(&mut bus_rx, "device-a").await?;
        wait_for_authoritative_owner(&sink, &session_key_for("nas-a", "session-a")).await?;

        sessions.apply_event_with_command_sink(
            shared_circuit_event(AcctStatusType::Start, "nas-b", "session-b", SHARED_MAC_B),
            started_at + Duration::from_secs(1),
            &mut sink,
        );

        assert_eq!(
            sessions.expire_due_with_command_sink(started_at + Duration::from_secs(10), &mut sink),
            1
        );
        assert_shared_circuit_create(&mut bus_rx, "device-b").await?;
        wait_for_authoritative_owner(&sink, &session_key_for("nas-b", "session-b")).await?;

        let session_c = session_key_for("nas-c", "session-c");
        sessions.apply_event_with_command_sink(
            shared_circuit_event(AcctStatusType::Start, "nas-c", "session-c", SHARED_MAC_C),
            started_at + Duration::from_secs(11),
            &mut sink,
        );
        wait_for_retained_owner(&sink, &session_c).await?;
        assert_bus_request_channel_empty(&mut bus_rx);

        Ok(())
    }

    #[tokio::test]
    async fn stale_expiry_removal_promotes_retained_shared_circuit_owner() -> anyhow::Result<()> {
        let (bus_tx, mut bus_rx) = mpsc::channel(4);
        let mut sink = ApplyingDynamicCircuitSink::new(bus_tx);
        let mut sessions =
            shared_circuit_sessions(Duration::from_secs(900), Duration::from_secs(2));
        let started_at = Instant::now();

        sessions.apply_event_with_command_sink(
            shared_circuit_event(AcctStatusType::Start, "nas-a", "session-a", SHARED_MAC_A),
            started_at,
            &mut sink,
        );
        assert_shared_circuit_create(&mut bus_rx, "device-a").await?;
        wait_for_authoritative_owner(&sink, &session_key_for("nas-a", "session-a")).await?;

        sessions.apply_event_with_command_sink(
            shared_circuit_event(AcctStatusType::Start, "nas-b", "session-b", SHARED_MAC_B),
            started_at + Duration::from_secs(1),
            &mut sink,
        );

        sessions.apply_event_with_command_sink(
            reset_event_for("nas-a"),
            started_at + Duration::from_secs(2),
            &mut sink,
        );
        tokio::task::yield_now().await;
        assert_bus_request_channel_empty(&mut bus_rx);
        assert_eq!(
            sessions.expire_due_with_command_sink(started_at + Duration::from_secs(3), &mut sink),
            0
        );
        assert_eq!(
            sessions.expire_due_with_command_sink(started_at + Duration::from_secs(4), &mut sink),
            1
        );
        assert_shared_circuit_create(&mut bus_rx, "device-b").await?;
        wait_for_authoritative_owner(&sink, &session_key_for("nas-b", "session-b")).await?;

        let session_c = session_key_for("nas-c", "session-c");
        sessions.apply_event_with_command_sink(
            shared_circuit_event(AcctStatusType::Start, "nas-c", "session-c", SHARED_MAC_C),
            started_at + Duration::from_secs(5),
            &mut sink,
        );
        wait_for_retained_owner(&sink, &session_c).await?;
        assert_bus_request_channel_empty(&mut bus_rx);

        Ok(())
    }

    #[test]
    fn dynamic_circuit_application_queue_reports_unavailable_states() {
        let (queue_tx, _queue_rx) = mpsc::channel(1);
        assert_eq!(
            queue_test_application_intent(&queue_tx, 0, "queued-circuit"),
            Ok(())
        );
        assert_eq!(
            queue_test_application_intent(&queue_tx, 1, "overflow-circuit"),
            Err(DynamicCircuitApplicationError::QueueUnavailable(
                DynamicCircuitQueueUnavailableReason::Full
            ))
        );

        let (closed_queue_tx, closed_queue_rx) = mpsc::channel(1);
        drop(closed_queue_rx);
        assert_eq!(
            queue_test_application_intent(&closed_queue_tx, 2, "closed-circuit"),
            Err(DynamicCircuitApplicationError::QueueUnavailable(
                DynamicCircuitQueueUnavailableReason::Closed
            ))
        );
    }

    #[test]
    fn applying_sink_records_queue_failure_diagnostic() {
        let sessions =
            RadiusAccountingSessions::new(Duration::from_secs(900), Duration::from_secs(120));
        let (queue_tx, queue_rx) = mpsc::channel(1);
        drop(queue_rx);
        let application_state = Arc::new(Mutex::new(DynamicCircuitApplicationState::default()));
        let mut sink = ApplyingDynamicCircuitSink::new_with_queue_for_test(
            queue_tx,
            application_state.clone(),
            0,
        );
        let intent = test_application_intent("closed-queue-circuit");
        let context = DynamicCircuitApplicationContext::from_intent(&intent);

        sink.emit(intent);

        let diagnostics = sink.activation_diagnostics(&sessions);
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.session_key == context.session_key
                    && diagnostic.circuit_ids == vec![context.circuit_id.clone()]
            })
            .expect("queue failure should record an apply-failed diagnostic");
        assert_eq!(
            diagnostic.state,
            RadiusActivationDiagnosticState::ApplyFailed
        );
        assert_eq!(diagnostic.session_key, context.session_key);
        assert_eq!(diagnostic.circuit_ids, vec![context.circuit_id]);
        assert!(
            diagnostic
                .apply_error
                .as_deref()
                .is_some_and(|error| error.contains("queue closed"))
        );
        assert!(!format!("{diagnostic:?}").contains("shared secret"));
    }

    #[tokio::test]
    async fn full_queue_removal_defers_and_drops_older_pending_upsert() -> anyhow::Result<()> {
        let (queue_tx, mut queue_rx) = mpsc::channel(1);
        let application_state = Arc::new(Mutex::new(DynamicCircuitApplicationState::default()));
        let (owner_session, owner_intent, owner_context) =
            shared_owner_intent("nas-a", "session-a", "device-a");
        application_state.lock().record_upsert(&owner_context);
        application_state
            .lock()
            .track_pending_upsert(&owner_context, 3);
        assert!(
            queue_tx
                .try_send(test_queued_intent(3, owner_intent.clone()))
                .is_ok()
        );

        let mut sink = ApplyingDynamicCircuitSink::new_with_queue_for_test(
            queue_tx,
            application_state.clone(),
            4,
        );
        sink.emit(shared_removal_intent(owner_session.clone()));

        {
            let state = application_state.lock();
            let owners = state
                .current_owners_by_circuit
                .get(SHARED_CIRCUIT_ID)
                .expect("owner should remain tracked until removal applies");
            assert_eq!(owners.len(), 1);
            assert!(owners.contains(&owner_session));
            assert!(
                state
                    .authoritative_session_by_circuit
                    .get(SHARED_CIRCUIT_ID)
                    == Some(&owner_session)
            );
            assert_eq!(state.deferred_removal_sequence_by_owner.len(), 1);
        }

        let queued_upsert = queue_rx
            .recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("expected older queued upsert"))?;

        let (bus_tx, mut bus_rx) = mpsc::channel(1);
        let _keep_bus_open = bus_tx.clone();
        let queued_intent = process_dynamic_circuit_intent_and_pending_removals(
            bus_tx,
            &application_state,
            queued_upsert,
        );
        let (removed_circuit_id, ()) =
            tokio::join!(receive_remove_and_ack(&mut bus_rx), queued_intent);
        assert_eq!(removed_circuit_id?, SHARED_CIRCUIT_ID);

        {
            let state = application_state.lock();
            assert!(
                !state
                    .current_owners_by_circuit
                    .contains_key(SHARED_CIRCUIT_ID)
            );
            assert!(state.deferred_removal_sequence_by_owner.is_empty());
            assert!(state.pending_upsert_sequences_by_owner.is_empty());
        }

        Ok(())
    }

    #[tokio::test]
    async fn bus_failure_detail_is_sanitized_in_apply_failed_diagnostic() -> anyhow::Result<()> {
        let sessions =
            RadiusAccountingSessions::new(Duration::from_secs(900), Duration::from_secs(120));
        let (bus_tx, mut bus_rx) = mpsc::channel(1);
        let mut sink = ApplyingDynamicCircuitSink::new(bus_tx);
        let intent = test_application_intent("failed-detail-circuit");
        let context = DynamicCircuitApplicationContext::from_intent(&intent);

        sink.emit(intent);
        let (reply_tx, shaped_device) = receive_create_dynamic_circuit(&mut bus_rx).await?;
        assert_eq!(shaped_device.circuit_id, context.circuit_id);
        fail_bus_reply(
            reply_tx,
            "bakery failure code=runtime-node-missing shared secret super-secret-value token = abc123",
        )?;

        let diagnostic = wait_for_apply_failed_diagnostic(
            &sink,
            &sessions,
            &context.circuit_id,
            &context.session_key,
        )
        .await?;
        let apply_error = diagnostic
            .apply_error
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("expected apply_error detail"))?;

        assert!(apply_error.contains("code=runtime-node-missing"));
        assert!(apply_error.contains("[redacted]"));
        assert!(!apply_error.contains("super-secret-value"));
        assert!(!apply_error.contains("abc123"));

        Ok(())
    }

    #[tokio::test]
    async fn queued_removal_drops_older_pending_upsert_before_bus_request() -> anyhow::Result<()> {
        let (queue_tx, mut queue_rx) = mpsc::channel(2);
        let application_state = Arc::new(Mutex::new(DynamicCircuitApplicationState::default()));
        let (owner_session, owner_intent, owner_context) =
            shared_owner_intent("nas-queued", "session-queued", "device-queued");
        application_state.lock().record_upsert(&owner_context);
        application_state
            .lock()
            .track_pending_upsert(&owner_context, 2);
        assert!(
            queue_tx
                .try_send(test_queued_intent(2, owner_intent))
                .is_ok()
        );

        let mut sink = ApplyingDynamicCircuitSink::new_with_queue_for_test(
            queue_tx,
            application_state.clone(),
            3,
        );
        sink.emit(shared_removal_intent(owner_session));

        let Ok(queued_upsert) = queue_rx.try_recv() else {
            anyhow::bail!("expected older queued upsert");
        };
        let Ok(queued_removal) = queue_rx.try_recv() else {
            anyhow::bail!("expected queued removal");
        };
        let (bus_tx, mut bus_rx) = mpsc::channel(1);
        let _keep_bus_open = bus_tx.clone();
        process_dynamic_circuit_intent(bus_tx.clone(), &application_state, queued_upsert).await;
        assert_bus_request_channel_empty(&mut bus_rx);

        let removal = process_dynamic_circuit_intent(bus_tx, &application_state, queued_removal);
        let (removed_circuit_id, ()) = tokio::join!(receive_remove_and_ack(&mut bus_rx), removal);
        assert_eq!(removed_circuit_id?, SHARED_CIRCUIT_ID);
        let state = application_state.lock();
        assert!(
            !state
                .current_owners_by_circuit
                .contains_key(SHARED_CIRCUIT_ID)
        );
        assert!(state.deferred_removal_sequence_by_owner.is_empty());

        Ok(())
    }

    #[test]
    fn pending_removal_replacement_moves_owner_to_latest_fifo_position() -> anyhow::Result<()> {
        let mut state = DynamicCircuitApplicationState::default();
        let (session_a, _intent_a, context_a) =
            shared_owner_intent("nas-a", "session-a", "device-a");
        let (session_b, _intent_b, context_b) =
            shared_owner_intent("nas-b", "session-b", "device-b");

        state.record_pending_removal(
            &context_a,
            test_queued_intent(10, shared_removal_intent(session_a.clone())),
        );
        state.record_pending_removal(
            &context_b,
            test_queued_intent(11, shared_removal_intent(session_b.clone())),
        );
        state.record_pending_removal(
            &context_a,
            test_queued_intent(12, shared_removal_intent(session_a.clone())),
        );

        let Some(queued_b) = state.take_next_pending_removal() else {
            anyhow::bail!("expected second owner removal first");
        };
        assert_eq!(queued_b.sequence, 11);
        assert_eq!(
            DynamicCircuitApplicationContext::from_intent(&queued_b.intent).session_key,
            session_b
        );

        let Some(queued_a) = state.take_next_pending_removal() else {
            anyhow::bail!("expected latest first-owner removal second");
        };
        assert_eq!(queued_a.sequence, 12);
        assert_eq!(
            DynamicCircuitApplicationContext::from_intent(&queued_a.intent).session_key,
            session_a
        );
        assert!(state.take_next_pending_removal().is_none());

        Ok(())
    }

    #[tokio::test]
    async fn failed_queued_removal_keeps_authoritative_owner() -> anyhow::Result<()> {
        let (bus_tx, mut bus_rx) = mpsc::channel(1);
        let application_state = Arc::new(Mutex::new(DynamicCircuitApplicationState::default()));
        let (owner_session, _owner_intent, owner_context) = shared_owner_intent(
            "nas-failed-remove",
            "session-failed-remove",
            "device-failed",
        );
        application_state.lock().record_upsert(&owner_context);

        let application_state_for_removal = application_state.clone();
        let owner_session_for_removal = owner_session.clone();
        let removal_task = tokio::spawn(async move {
            process_dynamic_circuit_intent(
                bus_tx,
                &application_state_for_removal,
                test_queued_intent(2, shared_removal_intent(owner_session_for_removal)),
            )
            .await;
        });

        let (reply_tx, circuit_id) = receive_remove_dynamic_circuit(&mut bus_rx).await?;
        assert_eq!(circuit_id, SHARED_CIRCUIT_ID);
        fail_bus_reply(reply_tx, "fake remove failure")?;
        removal_task.await?;

        let state = application_state.lock();
        let owners = state
            .current_owners_by_circuit
            .get(SHARED_CIRCUIT_ID)
            .expect("authoritative owner should remain tracked after failed removal");
        assert!(owners.contains(&owner_session));
        assert_eq!(
            state
                .authoritative_session_by_circuit
                .get(SHARED_CIRCUIT_ID),
            Some(&owner_session)
        );

        Ok(())
    }

    #[tokio::test]
    async fn stale_removal_after_newer_upsert_is_skipped() -> anyhow::Result<()> {
        let (bus_tx, mut bus_rx) = mpsc::channel(1);
        let _keep_bus_open = bus_tx.clone();
        let application_state = Arc::new(Mutex::new(DynamicCircuitApplicationState::default()));
        let (owner_session, owner_intent, _owner_context) =
            shared_owner_intent("nas-newer", "session-newer", "device-newer");

        let newer_upsert = process_dynamic_circuit_intent(
            bus_tx.clone(),
            &application_state,
            test_queued_intent(5, owner_intent),
        );
        let (upserted, ()) = tokio::join!(receive_create_and_ack(&mut bus_rx), newer_upsert);
        assert_eq!(upserted?.device_id, "device-newer");
        assert_bus_request_channel_empty(&mut bus_rx);

        process_dynamic_circuit_intent(
            bus_tx,
            &application_state,
            test_queued_intent(4, shared_removal_intent(owner_session.clone())),
        )
        .await;
        assert_bus_request_channel_empty(&mut bus_rx);

        let state = application_state.lock();
        let owners = state
            .current_owners_by_circuit
            .get(SHARED_CIRCUIT_ID)
            .expect("newer owner should remain tracked");
        assert!(owners.contains(&owner_session));
        assert_eq!(
            state
                .authoritative_session_by_circuit
                .get(SHARED_CIRCUIT_ID),
            Some(&owner_session)
        );

        Ok(())
    }

    #[tokio::test]
    async fn rekeyed_removal_uses_original_single_owner() -> anyhow::Result<()> {
        let (bus_tx, mut bus_rx) = mpsc::channel(1);
        let application_state = Arc::new(Mutex::new(DynamicCircuitApplicationState::default()));
        let original_session = session_key_for("nas-original", "session-rekeyed");
        let current_session = session_key_for("nas-current", "session-rekeyed");
        let circuit_id = original_session
            .dynamic_circuit_id()
            .expect("test session key should have a dynamic circuit id");
        let upsert = DynamicCircuitIntent::CreateDynamicCircuit(DynamicCircuitUpsert {
            circuit_id: circuit_id.clone(),
            session_key: original_session.clone(),
            event: complete_event_for(AcctStatusType::Start, "nas-original", "session-rekeyed"),
            shaped_device: shaped_device(&circuit_id, "device-rekeyed"),
        });
        let owner_context = DynamicCircuitApplicationContext::from_intent(&upsert);
        application_state.lock().record_upsert(&owner_context);

        let application_state_for_removal = application_state.clone();
        let circuit_id_for_removal = circuit_id.clone();
        let removal_task = tokio::spawn(async move {
            process_dynamic_circuit_intent(
                bus_tx,
                &application_state_for_removal,
                test_queued_intent(
                    3,
                    DynamicCircuitIntent::RemoveDynamicCircuit(DynamicCircuitRemoval {
                        circuit_id: circuit_id_for_removal,
                        session_key: current_session,
                        reason: DynamicCircuitRemovalReason::Rekeyed,
                    }),
                ),
            )
            .await;
        });

        assert_eq!(receive_remove_and_ack(&mut bus_rx).await?, circuit_id);
        removal_task.await?;

        let state = application_state.lock();
        assert!(!state.current_owners_by_circuit.contains_key(&circuit_id));
        assert!(
            !state
                .authoritative_session_by_circuit
                .contains_key(&circuit_id)
        );

        Ok(())
    }

    #[test]
    fn closed_queue_removal_keeps_authoritative_owner_and_deferred_marker() -> anyhow::Result<()> {
        let (queue_tx, queue_rx) = mpsc::channel(1);
        drop(queue_rx);
        let application_state = Arc::new(Mutex::new(DynamicCircuitApplicationState::default()));
        let (owner_session, _owner_intent, owner_context) =
            shared_owner_intent("nas-closed", "session-closed", "device-closed");
        let (other_session, _other_intent, other_context) =
            shared_owner_intent("nas-other", "session-other", "device-other");
        application_state.lock().record_upsert(&owner_context);
        application_state.lock().record_upsert(&other_context);
        application_state
            .lock()
            .track_pending_upsert(&owner_context, 3);

        let mut sink = ApplyingDynamicCircuitSink::new_with_queue_for_test(
            queue_tx,
            application_state.clone(),
            4,
        );
        sink.emit(shared_removal_intent(owner_session.clone()));

        let state = application_state.lock();
        let owners = state
            .current_owners_by_circuit
            .get(SHARED_CIRCUIT_ID)
            .expect("shared owners should remain tracked");
        assert_eq!(owners.len(), 2);
        assert!(owners.contains(&other_session));
        assert!(owners.contains(&owner_session));
        assert!(
            state
                .authoritative_session_by_circuit
                .get(SHARED_CIRCUIT_ID)
                == Some(&owner_session)
        );
        assert_eq!(state.deferred_removal_sequence_by_owner.len(), 1);
        assert_eq!(state.pending_upsert_sequences_by_owner.len(), 1);

        Ok(())
    }

    #[test]
    fn authoritative_release_preserves_other_shared_owners() {
        let mut state = DynamicCircuitApplicationState::default();
        let (_session_a, _intent_a, context_a) =
            shared_owner_intent("nas-a", "session-a", "device-a");
        let (_session_b, _intent_b, context_b) =
            shared_owner_intent("nas-b", "session-b", "device-b");
        let (_session_c, _intent_c, context_c) =
            shared_owner_intent("nas-c", "session-c", "device-c");

        state.record_upsert(&context_a);
        state.record_upsert(&context_b);
        state.release_owner(&context_a);

        let owners = state
            .current_owners_by_circuit
            .get(SHARED_CIRCUIT_ID)
            .expect("non-authoritative owner should remain tracked");
        assert!(!owners.contains(&context_a.session_key));
        assert!(owners.contains(&context_b.session_key));
        assert!(
            !state
                .authoritative_session_by_circuit
                .contains_key(SHARED_CIRCUIT_ID)
        );
        assert!(state.shared_upsert_may_apply(&context_c));
    }

    #[test]
    fn dynamic_circuit_application_result_classifies_bus_replies() {
        assert_eq!(
            dynamic_circuit_application_result(BusReply {
                responses: vec![BusResponse::Ack],
            }),
            Ok(())
        );
        assert_eq!(
            dynamic_circuit_application_result(BusReply { responses: vec![] }),
            Err(DynamicCircuitApplicationError::MissingResponse)
        );
        assert_eq!(
            dynamic_circuit_application_result(BusReply {
                responses: vec![BusResponse::Fail("fake failure".to_string())],
            }),
            Err(DynamicCircuitApplicationError::RequestFailed(
                "fake failure".to_string()
            ))
        );
        assert_eq!(
            DynamicCircuitApplicationError::RequestFailed("shared secret value".to_string())
                .to_string(),
            "dynamic circuit request failed: shared [redacted] [redacted]"
        );
        assert_eq!(
            DynamicCircuitApplicationError::RequestFailed(
                "bakery failure code=runtime-node-missing parent not found".to_string()
            )
            .to_string(),
            "dynamic circuit request failed: bakery failure code=runtime-node-missing parent not found"
        );
        assert_eq!(
            dynamic_circuit_application_result(BusReply {
                responses: vec![BusResponse::Ack, BusResponse::Ack],
            }),
            Err(DynamicCircuitApplicationError::UnexpectedResponseCount(2))
        );
        assert_eq!(
            dynamic_circuit_application_result(BusReply {
                responses: vec![BusResponse::NotReadyYet],
            }),
            Err(DynamicCircuitApplicationError::UnexpectedResponse(
                "NotReadyYet".to_string()
            ))
        );
        assert_eq!(
            DynamicCircuitApplicationError::UnexpectedResponse("NotReadyYet".to_string())
                .to_string(),
            "dynamic circuit request returned unexpected response: NotReadyYet"
        );
    }

    #[test]
    fn dynamic_circuit_application_error_display_redacts_and_bounds_bus_detail() {
        let secret_error = DynamicCircuitApplicationError::RequestFailed(
            "backend rejected shared secret super-secret-value token -> abc123 password => hunter2 api key = keyvalue private key - privatevalue authorization Bearer bearer-value"
                .to_string(),
        )
        .to_string();

        assert!(secret_error.contains("backend rejected"));
        assert!(secret_error.contains("[redacted]"));
        assert!(!secret_error.contains("super-secret-value"));
        assert!(!secret_error.contains("abc123"));
        assert!(!secret_error.contains("hunter2"));
        assert!(!secret_error.contains("keyvalue"));
        assert!(!secret_error.contains("privatevalue"));
        assert!(!secret_error.contains("bearer-value"));

        let bounded_error = DynamicCircuitApplicationError::RequestFailed(
            "x".repeat(DYNAMIC_CIRCUIT_BUS_FAILURE_DETAIL_LIMIT + 10),
        )
        .to_string();

        assert!(bounded_error.ends_with("..."));
        assert!(
            bounded_error.len()
                <= "dynamic circuit request failed: ".len()
                    + DYNAMIC_CIRCUIT_BUS_FAILURE_DETAIL_LIMIT
                    + "...".len()
        );

        let bounded_non_ascii_error = DynamicCircuitApplicationError::RequestFailed(
            "á".repeat(DYNAMIC_CIRCUIT_BUS_FAILURE_DETAIL_LIMIT),
        )
        .to_string();
        assert!(bounded_non_ascii_error.ends_with("..."));
        assert!(
            bounded_non_ascii_error.len()
                <= "dynamic circuit request failed: ".len()
                    + DYNAMIC_CIRCUIT_BUS_FAILURE_DETAIL_LIMIT
                    + "...".len()
        );
    }

    #[test]
    fn dynamic_circuit_bus_failure_sanitizer_redacts_two_word_key_labels() {
        for (detail, expected) in [
            (
                "backend rejected api key: abc123 parent missing",
                "backend rejected [redacted] [redacted] [redacted] parent missing",
            ),
            (
                "backend rejected private key: abc123 parent missing",
                "backend rejected [redacted] [redacted] [redacted] parent missing",
            ),
            (
                "backend rejected access key: abc123 parent missing",
                "backend rejected [redacted] [redacted] [redacted] parent missing",
            ),
            (
                "backend rejected api key abc123 parent missing",
                "backend rejected [redacted] [redacted] [redacted] parent missing",
            ),
            (
                "backend rejected private key abc123 parent missing",
                "backend rejected [redacted] [redacted] [redacted] parent missing",
            ),
            (
                "backend rejected access key abc123 parent missing",
                "backend rejected [redacted] [redacted] [redacted] parent missing",
            ),
            (
                "backend rejected api key= abc123 parent missing",
                "backend rejected [redacted] [redacted] [redacted] parent missing",
            ),
            (
                "backend rejected api key=abc123 parent missing",
                "backend rejected [redacted] [redacted] parent missing",
            ),
            (
                "backend rejected api-key=abc123 parent missing",
                "backend rejected [redacted] parent missing",
            ),
            (
                "backend rejected private-key=abc123 parent missing",
                "backend rejected [redacted] parent missing",
            ),
            (
                "backend rejected access-key=abc123 parent missing",
                "backend rejected [redacted] parent missing",
            ),
            (
                "backend rejected accesskey=abc123 parent missing",
                "backend rejected [redacted] parent missing",
            ),
            (
                "backend rejected key: abc123 parent missing",
                "backend rejected [redacted] [redacted] parent missing",
            ),
            (
                "backend rejected key= abc123 parent missing",
                "backend rejected [redacted] [redacted] parent missing",
            ),
            (
                "backend rejected key=abc123 parent missing",
                "backend rejected [redacted] parent missing",
            ),
            (
                "backend rejected key = abc123 parent missing",
                "backend rejected [redacted] [redacted] [redacted] parent missing",
            ),
            (
                "backend rejected key =abc123 parent missing",
                "backend rejected [redacted] [redacted] parent missing",
            ),
            (
                "backend rejected key :abc123 parent missing",
                "backend rejected [redacted] [redacted] parent missing",
            ),
        ] {
            let sanitized = sanitize_dynamic_circuit_bus_failure_detail(detail);

            assert_eq!(sanitized, expected);
            assert!(!sanitized.contains("abc123"));
        }
    }

    #[test]
    fn dynamic_circuit_upsert_ownership_records_ambiguous_replies() {
        assert!(dynamic_circuit_upsert_result_records_success(&Ok(())));
        assert!(dynamic_circuit_upsert_result_records_success(&Err(
            DynamicCircuitApplicationError::ReplyDropped
        )));
        assert!(dynamic_circuit_upsert_result_records_success(&Err(
            DynamicCircuitApplicationError::ReplyTimeout
        )));
        assert!(!dynamic_circuit_upsert_result_records_success(&Err(
            DynamicCircuitApplicationError::BusClosed
        )));

        assert!(dynamic_circuit_removal_result_releases_owner(&Ok(())));
        assert!(!dynamic_circuit_removal_result_releases_owner(&Err(
            DynamicCircuitApplicationError::ReplyDropped
        )));
        assert!(!dynamic_circuit_removal_result_releases_owner(&Err(
            DynamicCircuitApplicationError::ReplyTimeout
        )));
        assert!(!dynamic_circuit_removal_result_releases_owner(&Err(
            DynamicCircuitApplicationError::BusClosed
        )));
    }

    #[tokio::test]
    async fn stop_after_dropped_create_reply_still_removes_dynamic_circuit() -> anyhow::Result<()> {
        let fallback_rate = SessionRateProfile::new(4.0, 2.0, 40.0, 12.0)?;
        let fallback_parent = DynamicCircuitParent::new("Core PPPoE");
        let mut sessions = RadiusAccountingSessions::new_with_fallback_and_mac_matcher(
            Duration::from_secs(900),
            Duration::from_secs(120),
            Some(fallback_rate),
            Some(fallback_parent),
            None,
        );
        let (bus_tx, mut bus_rx) = mpsc::channel(4);
        let mut sink = ApplyingDynamicCircuitSink::new(bus_tx);
        let key = session_key();
        let circuit_id = key
            .dynamic_circuit_id()
            .expect("test session key should have a dynamic circuit id");
        let started_at = Instant::now();

        handle_accounting_event_with_command_sink(
            complete_event(AcctStatusType::Start),
            &mut sessions,
            &mut sink,
            started_at,
            test_listen_addr(),
            64,
            20,
        );
        let (start_reply, start_device) = receive_create_dynamic_circuit(&mut bus_rx).await?;
        assert_eq!(start_device.circuit_id, circuit_id);
        drop(start_reply);
        wait_for_authoritative_owner_for_circuit(&sink, &circuit_id, &key).await?;

        handle_accounting_event_with_command_sink(
            complete_event(AcctStatusType::Stop),
            &mut sessions,
            &mut sink,
            started_at + Duration::from_secs(1),
            test_listen_addr(),
            64,
            20,
        );
        assert_eq!(receive_remove_and_ack(&mut bus_rx).await?, circuit_id);

        Ok(())
    }

    #[test]
    fn adapter_expiry_emits_deferred_removal_intent() -> anyhow::Result<()> {
        let fallback_rate = SessionRateProfile::new(4.0, 2.0, 40.0, 12.0)?;
        let fallback_parent = DynamicCircuitParent::new("Core PPPoE");
        let mut sessions = RadiusAccountingSessions::new_with_fallback_and_mac_matcher(
            Duration::from_secs(10),
            Duration::from_secs(2),
            Some(fallback_rate),
            Some(fallback_parent),
            None,
        );
        let mut sink = RecordingDynamicCircuitSink::default();
        let started_at = Instant::now();
        let key = session_key();
        let Some(circuit_id) = key.dynamic_circuit_id() else {
            anyhow::bail!("test session key should have a dynamic circuit id");
        };

        sessions.apply_event_with_command_sink(
            complete_event(AcctStatusType::Start),
            started_at,
            &mut sink,
        );
        assert_eq!(sink.intents.len(), 1);
        assert_eq!(sessions.activation_counters().create, 1);
        assert_eq!(
            sessions.expire_due_with_command_sink(started_at + Duration::from_secs(9), &mut sink),
            0
        );
        assert_eq!(sink.intents.len(), 1);
        assert_eq!(
            sessions.expire_due_with_command_sink(started_at + Duration::from_secs(10), &mut sink),
            1
        );

        assert_eq!(sink.intents.len(), 2);
        let DynamicCircuitIntent::RemoveDynamicCircuit(expiry) = &sink.intents[1] else {
            anyhow::bail!("expected expiry removal intent, got {:?}", sink.intents[1]);
        };
        assert_eq!(expiry.circuit_id, circuit_id);
        assert_eq!(expiry.session_key, key);
        assert_eq!(expiry.reason, DynamicCircuitRemovalReason::Expired);
        assert_eq!(sessions.activation_counters().remove, 1);
        assert_eq!(sessions.activation_counters().expiry, 1);
        let diagnostics = sessions.activation_diagnostics();
        let expired = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.session_key == key)
            .expect("expired diagnostic should be retained");
        assert_eq!(expired.state, RadiusActivationDiagnosticState::Expired);
        assert_eq!(expired.circuit_ids, vec![circuit_id]);

        Ok(())
    }

    #[test]
    fn default_adapter_paths_use_deferred_sink() -> anyhow::Result<()> {
        let fallback_rate = SessionRateProfile::new(4.0, 2.0, 40.0, 12.0)?;
        let fallback_parent = DynamicCircuitParent::new("Core PPPoE");
        let mut sessions = RadiusAccountingSessions::new_with_fallback_and_mac_matcher(
            Duration::from_secs(10),
            Duration::from_secs(2),
            Some(fallback_rate),
            Some(fallback_parent),
            None,
        );
        let started_at = Instant::now();

        handle_accounting_event(
            complete_event(AcctStatusType::Start),
            &mut sessions,
            started_at,
            test_listen_addr(),
            64,
            20,
        );
        assert_eq!(sessions.expire_due(started_at + Duration::from_secs(10)), 1);

        Ok(())
    }

    #[test]
    fn accepted_event_updates_sessions_without_dynamic_application() -> anyhow::Result<()> {
        let mut sessions =
            RadiusAccountingSessions::new(Duration::from_secs(900), Duration::from_secs(120));
        let now = Instant::now();

        handle_accounting_event(
            complete_event(AcctStatusType::Start),
            &mut sessions,
            now,
            test_listen_addr(),
            64,
            20,
        );

        let key = session_key();
        let Some(session) = sessions.store.session(&key) else {
            anyhow::bail!("accepted event should create an in-memory session");
        };
        assert_eq!(
            session.pending_reasons,
            vec![PendingSessionReason::MissingParent]
        );
        assert_eq!(sessions.updated_at.get(&key), Some(&now));

        Ok(())
    }

    #[tokio::test]
    async fn packet_counters_track_listener_outcomes_separately_from_activation_counters()
    -> anyhow::Result<()> {
        const SHARED_SECRET: &[u8] = b"radius-secret";
        const ACCOUNTING_START_REQUEST: [u8; 26] = [
            4, 7, 0, 26, 234, 109, 208, 193, 96, 89, 23, 174, 213, 177, 203, 9, 123, 217, 127, 22,
            40, 6, 0, 0, 0, 1,
        ];
        let mut sessions =
            RadiusAccountingSessions::new(Duration::from_secs(900), Duration::from_secs(120));
        let mut expiry_timer = RadiusExpiryTimer::new(&sessions, Instant::now());
        let mut applying_sink = None;
        let accepted_request = verify_accounting_request(
            &ACCOUNTING_START_REQUEST,
            SHARED_SECRET,
            MessageAuthenticatorPolicy::Optional,
        )?;

        handle_listener_outcome_with_application_sink(
            AccountingListenerOutcome::Accepted(ReceivedVerifiedAccountingPacket {
                peer: test_listen_addr(),
                received_len: ACCOUNTING_START_REQUEST.len(),
                response_len: 20,
                request: accepted_request,
            }),
            &mut sessions,
            &mut expiry_timer,
            Instant::now(),
            &mut applying_sink,
        );
        handle_listener_outcome_with_application_sink(
            AccountingListenerOutcome::RejectedSource {
                peer: test_listen_addr(),
                received_len: 20,
            },
            &mut sessions,
            &mut expiry_timer,
            Instant::now(),
            &mut applying_sink,
        );

        assert_eq!(
            sessions.packet_counters(),
            RadiusPacketCounters {
                accepted: 1,
                rejected: 1,
            }
        );
        assert_eq!(sessions.activation_counters(), Default::default());

        Ok(())
    }

    #[test]
    fn session_expiry_uses_default_ttl_and_stale_grace() -> anyhow::Result<()> {
        let mut sessions =
            RadiusAccountingSessions::new(Duration::from_secs(10), Duration::from_secs(2));
        let started_at = Instant::now();

        sessions.apply_event(complete_event(AcctStatusType::Start), started_at);
        assert_eq!(sessions.expire_due(started_at + Duration::from_secs(9)), 0);
        assert_eq!(sessions.expire_due(started_at + Duration::from_secs(10)), 1);
        assert!(!sessions.updated_at.contains_key(&session_key()));

        let restarted_at = started_at + Duration::from_secs(20);
        let reset_at = restarted_at + Duration::from_secs(1);
        sessions.apply_event(complete_event(AcctStatusType::Start), restarted_at);
        sessions.apply_event(reset_event(), reset_at);
        assert_eq!(
            sessions
                .store
                .session(&session_key())
                .map(|session| session.state),
            Some(AccountingSessionState::Stale(
                lqos_radius::NasResetStatus::AccountingOff
            ))
        );
        assert_eq!(sessions.expire_due(reset_at + Duration::from_secs(1)), 0);
        assert_eq!(sessions.expire_due(reset_at + Duration::from_secs(2)), 1);
        assert!(!sessions.updated_at.contains_key(&session_key()));

        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn stale_update_advances_timer_to_changed_deadline() {
        let stale_grace = Duration::from_millis(500);
        let mut sessions = RadiusAccountingSessions::new(Duration::from_secs(60), stale_grace);
        let now = radius_accounting_now();
        let mut expiry_timer = RadiusExpiryTimer::new(&sessions, now);

        let start_update = sessions.apply_event(complete_event(AcctStatusType::Start), now);
        expiry_timer.schedule_after_update(&sessions, &start_update, now);
        assert_eq!(expiry_timer.wake_at, now + Duration::from_secs(1));

        let stale_update = sessions.apply_event(reset_event(), now);
        expiry_timer.schedule_after_update(&sessions, &stale_update, now);
        assert_eq!(expiry_timer.wake_at, now + stale_grace);
    }

    #[test]
    fn promoted_sessions_prune_old_timestamp_keys() {
        let mut sessions =
            RadiusAccountingSessions::new(Duration::from_secs(900), Duration::from_secs(120));
        let started_at = Instant::now();
        let mut pending_event = complete_event(AcctStatusType::Start);
        pending_event.nas_identifier = None;

        sessions.apply_event(pending_event, started_at);
        assert_eq!(sessions.updated_at.len(), 1);

        let promoted_at = started_at + Duration::from_secs(1);
        sessions.apply_event(complete_event(AcctStatusType::InterimUpdate), promoted_at);

        assert_eq!(sessions.updated_at.len(), 1);
        assert_eq!(sessions.updated_at.get(&session_key()), Some(&promoted_at));
    }

    #[test]
    fn nas_reset_refreshes_only_matching_stale_sessions() {
        let mut sessions =
            RadiusAccountingSessions::new(Duration::from_secs(60), Duration::from_secs(2));
        let started_at = Instant::now();
        let nas_a_reset_at = started_at + Duration::from_secs(1);
        let nas_b_started_at = started_at + Duration::from_secs(2);
        let nas_b_reset_at = started_at + Duration::from_secs(3);
        let nas_a_key = session_key_for("nas-a", "session-a");

        sessions.apply_event(
            complete_event_for(AcctStatusType::Start, "nas-a", "session-a"),
            started_at,
        );
        sessions.apply_event(reset_event_for("nas-a"), nas_a_reset_at);
        assert_eq!(sessions.updated_at.get(&nas_a_key), Some(&nas_a_reset_at));

        sessions.apply_event(
            complete_event_for(AcctStatusType::Start, "nas-b", "session-b"),
            nas_b_started_at,
        );
        sessions.apply_event(reset_event_for("nas-b"), nas_b_reset_at);

        assert_eq!(sessions.updated_at.get(&nas_a_key), Some(&nas_a_reset_at));
        assert_eq!(sessions.expire_due(nas_b_reset_at), 1);
        assert!(sessions.store.session(&nas_a_key).is_none());
    }

    #[test]
    fn repeated_nas_reset_does_not_refresh_already_stale_sessions() {
        let mut sessions =
            RadiusAccountingSessions::new(Duration::from_secs(60), Duration::from_secs(2));
        let started_at = Instant::now();
        let first_reset_at = started_at + Duration::from_secs(1);
        let repeated_reset_at = first_reset_at + Duration::from_secs(1);
        let stale_deadline = first_reset_at + Duration::from_secs(2);
        let nas_a_key = session_key_for("nas-a", "session-a");

        sessions.apply_event(
            complete_event_for(AcctStatusType::Start, "nas-a", "session-a"),
            started_at,
        );
        sessions.apply_event(reset_event_for("nas-a"), first_reset_at);
        sessions.apply_event(reset_event_for("nas-a"), repeated_reset_at);

        assert_eq!(sessions.updated_at.get(&nas_a_key), Some(&first_reset_at));
        assert_eq!(sessions.expire_due(stale_deadline), 1);
        assert!(sessions.store.session(&nas_a_key).is_none());
    }

    #[test]
    fn alternating_nas_reset_status_does_not_refresh_already_stale_sessions() {
        let mut sessions =
            RadiusAccountingSessions::new(Duration::from_secs(60), Duration::from_secs(2));
        let started_at = Instant::now();
        let first_reset_at = started_at + Duration::from_secs(1);
        let second_reset_at = first_reset_at + Duration::from_secs(1);
        let stale_deadline = first_reset_at + Duration::from_secs(2);
        let nas_a_key = session_key_for("nas-a", "session-a");

        sessions.apply_event(
            complete_event_for(AcctStatusType::Start, "nas-a", "session-a"),
            started_at,
        );
        sessions.apply_event(reset_event_for("nas-a"), first_reset_at);
        sessions.apply_event(
            reset_event_for_status("nas-a", AcctStatusType::AccountingOn),
            second_reset_at,
        );

        assert_eq!(sessions.updated_at.get(&nas_a_key), Some(&first_reset_at));
        assert_eq!(
            sessions
                .store
                .session(&nas_a_key)
                .map(|session| session.state),
            Some(AccountingSessionState::Stale(
                lqos_radius::NasResetStatus::AccountingOn
            ))
        );
        assert_eq!(sessions.expire_due(stale_deadline), 1);
        assert!(sessions.store.session(&nas_a_key).is_none());
    }

    #[test]
    fn repeated_nas_reset_refreshes_cached_stale_diagnostic_reason() {
        let mut sessions =
            RadiusAccountingSessions::new(Duration::from_secs(60), Duration::from_secs(2));
        let started_at = Instant::now();
        let first_reset_at = started_at + Duration::from_secs(1);
        let second_reset_at = first_reset_at + Duration::from_secs(1);
        let nas_a_key = session_key_for("nas-cached-stale", "session-cached-stale");

        sessions.apply_event(
            complete_event_for(
                AcctStatusType::Start,
                "nas-cached-stale",
                "session-cached-stale",
            ),
            started_at,
        );
        sessions.apply_event(reset_event_for("nas-cached-stale"), first_reset_at);
        sessions.apply_event(
            reset_event_for_status("nas-cached-stale", AcctStatusType::AccountingOn),
            second_reset_at,
        );

        let diagnostic = sessions
            .activation_diagnostics()
            .into_iter()
            .find(|diagnostic| diagnostic.session_key == nas_a_key)
            .expect("stale diagnostic should remain cached");
        assert_eq!(
            diagnostic.state,
            RadiusActivationDiagnosticState::Stale(lqos_radius::NasResetStatus::AccountingOn)
        );
        assert_eq!(sessions.updated_at.get(&nas_a_key), Some(&first_reset_at));
    }

    #[test]
    fn expired_activation_diagnostics_are_bounded() {
        let mut sessions =
            RadiusAccountingSessions::new(Duration::from_secs(1), Duration::from_secs(1));
        let started_at = Instant::now();

        for index in 0..(RADIUS_RECENT_EXPIRED_DIAGNOSTIC_LIMIT + 2) {
            let session_id = format!("session-{index}");
            sessions.apply_event(
                complete_event_for(AcctStatusType::Start, "nas-expiry", &session_id),
                started_at,
            );
        }

        assert_eq!(
            sessions.expire_due(started_at + Duration::from_secs(2)),
            RADIUS_RECENT_EXPIRED_DIAGNOSTIC_LIMIT + 2
        );
        let diagnostics = sessions.activation_diagnostics();
        assert_eq!(diagnostics.len(), RADIUS_RECENT_EXPIRED_DIAGNOSTIC_LIMIT);
        assert_eq!(
            sessions.recent_expired_activation_diagnostics.len(),
            RADIUS_RECENT_EXPIRED_DIAGNOSTIC_LIMIT
        );
        assert!(sessions.update_sequence_by_key.is_empty());
        assert!(sessions.activation_diagnostics_by_key.is_empty());
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| { diagnostic.state == RadiusActivationDiagnosticState::Expired })
        );
        let retained_keys = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.session_key.clone())
            .collect::<HashSet<_>>();
        assert!(!retained_keys.contains(&session_key_for("nas-expiry", "session-0")));
        assert!(!retained_keys.contains(&session_key_for("nas-expiry", "session-1")));
        for index in 2..(RADIUS_RECENT_EXPIRED_DIAGNOSTIC_LIMIT + 2) {
            assert!(
                retained_keys.contains(&session_key_for("nas-expiry", &format!("session-{index}")))
            );
        }
    }

    #[test]
    fn expired_activation_diagnostic_is_removed_on_reactivation() {
        let mut sessions =
            RadiusAccountingSessions::new(Duration::from_secs(1), Duration::from_secs(1));
        let started_at = Instant::now();
        let reactivated_at = started_at + Duration::from_secs(2);
        let key = session_key_for("nas-reactivated", "session-reactivated");
        let other_key = session_key_for("nas-reactivated", "session-other");

        sessions.apply_event(
            complete_event_for(
                AcctStatusType::Start,
                "nas-reactivated",
                "session-reactivated",
            ),
            started_at,
        );
        sessions.apply_event(
            complete_event_for(AcctStatusType::Start, "nas-reactivated", "session-other"),
            started_at,
        );
        assert_eq!(sessions.expire_due(started_at + Duration::from_secs(1)), 2);
        assert_eq!(sessions.activation_diagnostics().len(), 2);

        sessions.apply_event(
            complete_event_for(
                AcctStatusType::Start,
                "nas-reactivated",
                "session-reactivated",
            ),
            reactivated_at,
        );

        let diagnostics = sessions.activation_diagnostics();
        assert_eq!(diagnostics.len(), 2);
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.session_key == key
                && diagnostic.state == RadiusActivationDiagnosticState::Pending
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.session_key == other_key
                && diagnostic.state == RadiusActivationDiagnosticState::Expired
        }));
        assert_eq!(sessions.recent_expired_activation_diagnostics.len(), 1);
        assert_eq!(
            sessions.recent_expired_activation_diagnostics[0].session_key,
            other_key
        );
    }

    #[test]
    fn stale_expired_activation_diagnostics_are_bounded() {
        let mut sessions =
            RadiusAccountingSessions::new(Duration::from_secs(60), Duration::from_secs(1));
        let started_at = Instant::now();
        let reset_at = started_at + Duration::from_secs(1);

        for index in 0..(RADIUS_RECENT_EXPIRED_DIAGNOSTIC_LIMIT + 2) {
            let session_id = format!("session-{index}");
            sessions.apply_event(
                complete_event_for(AcctStatusType::Start, "nas-stale-expiry", &session_id),
                started_at,
            );
        }

        sessions.apply_event(reset_event_for("nas-stale-expiry"), reset_at);
        assert_eq!(
            sessions.expire_due(reset_at + Duration::from_secs(1)),
            RADIUS_RECENT_EXPIRED_DIAGNOSTIC_LIMIT + 2
        );

        let diagnostics = sessions.activation_diagnostics();
        assert_eq!(diagnostics.len(), RADIUS_RECENT_EXPIRED_DIAGNOSTIC_LIMIT);
        assert_eq!(
            sessions.recent_expired_activation_diagnostics.len(),
            RADIUS_RECENT_EXPIRED_DIAGNOSTIC_LIMIT
        );
        assert!(sessions.update_sequence_by_key.is_empty());
        assert!(sessions.activation_diagnostics_by_key.is_empty());
        let retained_keys = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.session_key.clone())
            .collect::<HashSet<_>>();
        assert!(!retained_keys.contains(&session_key_for("nas-stale-expiry", "session-0")));
        assert!(!retained_keys.contains(&session_key_for("nas-stale-expiry", "session-1")));
        for index in 2..(RADIUS_RECENT_EXPIRED_DIAGNOSTIC_LIMIT + 2) {
            assert!(retained_keys.contains(&session_key_for(
                "nas-stale-expiry",
                &format!("session-{index}")
            )));
        }
    }

    #[test]
    fn apply_failed_activation_diagnostics_are_bounded() {
        let mut state = DynamicCircuitApplicationState::default();

        for index in 0..(RADIUS_APPLY_FAILED_DIAGNOSTIC_LIMIT + 2) {
            let session_id = format!("session-{index}");
            let (_, _, context) = shared_owner_intent("nas-apply-failed", &session_id, "device-a");
            state
                .record_application_failure(&context, "dynamic circuit request failed".to_string());
        }

        let diagnostics = state.application_diagnostics();
        assert_eq!(diagnostics.len(), RADIUS_APPLY_FAILED_DIAGNOSTIC_LIMIT);
        assert_eq!(
            state.application_diagnostic_order.len(),
            RADIUS_APPLY_FAILED_DIAGNOSTIC_LIMIT
        );
        assert_eq!(
            state.application_diagnostics_by_owner.len(),
            RADIUS_APPLY_FAILED_DIAGNOSTIC_LIMIT
        );
        assert_eq!(
            diagnostics
                .first()
                .map(|diagnostic| &diagnostic.session_key),
            Some(&session_key_for("nas-apply-failed", "session-2"))
        );
        assert!(diagnostics.iter().all(|diagnostic| {
            diagnostic.state == RadiusActivationDiagnosticState::ApplyFailed
        }));
    }

    #[test]
    fn apply_failed_filter_preserves_other_circuit_ids_for_same_session() {
        let session_key = session_key_for("nas-filter", "session-filter");
        let failed_owner = ("failed-circuit".to_string(), session_key.clone());
        let failed_owners = HashSet::from([failed_owner]);
        let mut diagnostics = vec![RadiusActivationDiagnostic {
            session_key: session_key.clone(),
            acct_session_id: Some("session-filter".to_string()),
            nas: Some(NasIdentity::Identifier("nas-filter".to_string())),
            circuit_ids: vec!["failed-circuit".to_string(), "healthy-circuit".to_string()],
            state: RadiusActivationDiagnosticState::Active,
            pending_reasons: Vec::new(),
            apply_error: None,
        }];

        suppress_failed_owner_diagnostics(&mut diagnostics, &failed_owners);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].session_key, session_key);
        assert_eq!(diagnostics[0].circuit_ids, vec!["healthy-circuit"]);
    }

    #[test]
    fn push_limited_handles_zero_exact_and_over_limit() {
        let mut items = VecDeque::from([1, 2]);
        push_limited(&mut items, 3, 0);
        assert_eq!(items, VecDeque::from([1, 2]));

        push_limited(&mut items, 3, 3);
        assert_eq!(items, VecDeque::from([1, 2, 3]));

        push_limited(&mut items, 4, 3);
        assert_eq!(items, VecDeque::from([2, 3, 4]));
    }

    #[test]
    fn listener_send_errors_are_recoverable() {
        let err = lqos_radius::ListenerError::Send {
            peer: test_listen_addr(),
            source: std::io::Error::from_raw_os_error(nix::libc::ECONNREFUSED),
        };

        assert!(listener_error_is_recoverable(&err));
    }

    #[derive(Default)]
    struct RecordingDynamicCircuitSink {
        intents: Vec<DynamicCircuitIntent>,
    }

    impl DynamicCircuitCommandSink for RecordingDynamicCircuitSink {
        fn emit(&mut self, intent: DynamicCircuitIntent) {
            self.intents.push(intent);
        }
    }

    fn enabled_config(secret_path: &Path) -> RadiusAccountingConfig {
        RadiusAccountingConfig {
            enabled: true,
            listen: Some(test_listen_addr()),
            default_ttl_seconds: 900,
            stale_grace_seconds: 120,
            dynamic_circuit_application: RadiusDynamicCircuitApplicationConfig::default(),
            fallback_speed_profile: None,
            clients: vec![RadiusAccountingClient {
                name: "pppoe-core-1".to_string(),
                source: vec![RadiusClientSource::new(IpNetwork::from(IpAddr::V4(
                    Ipv4Addr::new(127, 0, 0, 1),
                )))],
                secret_file: RadiusSharedSecretSource::from(secret_path.to_string_lossy().as_ref()),
            }],
        }
    }

    fn test_listen_addr() -> SocketAddr {
        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 18130))
    }

    fn test_bus_sender() -> DynamicCircuitBusSender {
        let (bus_tx, _bus_rx) = mpsc::channel(1);
        bus_tx
    }

    fn sessions_from_runtime_config(
        runtime_config: RadiusAccountingRuntimeConfig,
    ) -> RadiusAccountingSessions {
        RadiusAccountingSessions::new_with_fallback_and_mac_matcher(
            runtime_config.default_ttl,
            runtime_config.stale_grace,
            runtime_config.fallback_rate_profile,
            runtime_config.fallback_parent,
            runtime_config.mac_matcher,
        )
    }

    const SHARED_CIRCUIT_ID: &str = "circuit-shared";
    const SHARED_MAC_A: &str = "aa-bb-cc-dd-ee-ff";
    const SHARED_MAC_B: &str = "11-22-33-44-55-66";
    const SHARED_MAC_C: &str = "77-88-99-aa-bb-cc";

    fn shared_circuit_sessions(
        default_ttl: Duration,
        stale_grace: Duration,
    ) -> RadiusAccountingSessions {
        let mut device_a = shaped_device(SHARED_CIRCUIT_ID, "device-a");
        device_a.mac = SHARED_MAC_A.to_string();
        let mut device_b = shaped_device(SHARED_CIRCUIT_ID, "device-b");
        device_b.mac = SHARED_MAC_B.to_string();
        let mut device_c = shaped_device(SHARED_CIRCUIT_ID, "device-c");
        device_c.mac = SHARED_MAC_C.to_string();

        RadiusAccountingSessions::new_with_fallback_and_mac_matcher(
            default_ttl,
            stale_grace,
            None,
            None,
            Some(ShapedDevicesMacMatcher::from_devices(&[
                device_a, device_b, device_c,
            ])),
        )
    }

    fn shared_circuit_event(
        status_type: AcctStatusType,
        nas: &str,
        session_id: &str,
        mac: &str,
    ) -> AccountingEvent {
        let mut event = complete_event_for(status_type, nas, session_id);
        event.calling_station_id = Some(mac.to_string());
        event
    }

    async fn assert_shared_circuit_create(
        bus_rx: &mut TestBusReceiver,
        device_id: &str,
    ) -> anyhow::Result<()> {
        let shaped_device = receive_create_and_ack(bus_rx).await?;
        assert_eq!(shaped_device.circuit_id, SHARED_CIRCUIT_ID);
        assert_eq!(shaped_device.device_id, device_id);
        Ok(())
    }

    fn test_application_intent(circuit_id: &str) -> DynamicCircuitIntent {
        DynamicCircuitIntent::CreateDynamicCircuit(DynamicCircuitUpsert {
            circuit_id: circuit_id.to_string(),
            session_key: session_key(),
            event: complete_event(AcctStatusType::Start),
            shaped_device: shaped_device(circuit_id, "device-test"),
        })
    }

    fn shared_owner_intent(
        nas: &str,
        session_id: &str,
        device_id: &str,
    ) -> (
        AccountingSessionKey,
        DynamicCircuitIntent,
        DynamicCircuitApplicationContext,
    ) {
        let session_key = session_key_for(nas, session_id);
        let intent = DynamicCircuitIntent::CreateDynamicCircuit(DynamicCircuitUpsert {
            circuit_id: SHARED_CIRCUIT_ID.to_string(),
            session_key: session_key.clone(),
            event: complete_event_for(AcctStatusType::Start, nas, session_id),
            shaped_device: shaped_device(SHARED_CIRCUIT_ID, device_id),
        });
        let context = DynamicCircuitApplicationContext::from_intent(&intent);
        (session_key, intent, context)
    }

    fn shared_removal_intent(session_key: AccountingSessionKey) -> DynamicCircuitIntent {
        DynamicCircuitIntent::RemoveDynamicCircuit(DynamicCircuitRemoval {
            circuit_id: SHARED_CIRCUIT_ID.to_string(),
            session_key,
            reason: DynamicCircuitRemovalReason::Stop,
        })
    }

    fn test_queued_intent(
        sequence: u64,
        intent: DynamicCircuitIntent,
    ) -> QueuedDynamicCircuitIntent {
        QueuedDynamicCircuitIntent { sequence, intent }
    }

    fn queue_test_application_intent(
        queue_tx: &mpsc::Sender<QueuedDynamicCircuitIntent>,
        sequence: u64,
        circuit_id: &str,
    ) -> Result<(), DynamicCircuitApplicationError> {
        let intent = test_application_intent(circuit_id);
        let context = DynamicCircuitApplicationContext::from_intent(&intent);
        queue_dynamic_circuit_intent(queue_tx, &context, test_queued_intent(sequence, intent))
    }

    type TestAccountingEvent = (AccountingEvent, oneshot::Sender<()>);

    async fn run_test_accounting_event_loop(
        mut event_rx: mpsc::Receiver<TestAccountingEvent>,
        mut sessions: RadiusAccountingSessions,
        mut applying_sink: Option<ApplyingDynamicCircuitSink>,
    ) {
        let mut expiry_timer = RadiusExpiryTimer::new(&sessions, radius_accounting_now());
        loop {
            tokio::select! {
                maybe_event = event_rx.recv() => {
                    let Some((event, processed_tx)) = maybe_event else {
                        return;
                    };
                    let now = radius_accounting_now();
                    expire_due_before_packet(
                        &mut sessions,
                        &mut expiry_timer,
                        now,
                        &mut applying_sink,
                    );
                    handle_accounting_event_with_application_sink_and_expiry_schedule(
                        event,
                        &mut sessions,
                        &mut expiry_timer,
                        now,
                        &mut applying_sink,
                        AccountingPacketLogContext {
                            peer: test_listen_addr(),
                            received_len: 64,
                            response_len: 20,
                        },
                    );
                    let _ = processed_tx.send(());
                }
                _ = expiry_timer.sleep_mut() => {
                    let now = radius_accounting_now();
                    expire_due_after_timer_wake(
                        &mut sessions,
                        &mut expiry_timer,
                        now,
                        &mut applying_sink,
                    );
                }
            }
        }
    }

    async fn send_test_accounting_event(
        event_tx: &mpsc::Sender<TestAccountingEvent>,
        event: AccountingEvent,
    ) -> anyhow::Result<()> {
        let (processed_tx, processed_rx) = oneshot::channel();
        event_tx
            .send((event, processed_tx))
            .await
            .map_err(|_| anyhow::anyhow!("test accounting loop stopped before receiving event"))?;
        processed_rx
            .await
            .map_err(|_| anyhow::anyhow!("test accounting loop stopped before processing event"))
    }

    type TestBusReceiver = mpsc::Receiver<(oneshot::Sender<BusReply>, BusRequest)>;

    async fn receive_bus_request(
        bus_rx: &mut TestBusReceiver,
    ) -> anyhow::Result<(oneshot::Sender<BusReply>, BusRequest)> {
        match tokio::time::timeout(Duration::from_secs(1), bus_rx.recv()).await {
            Ok(Some(request)) => Ok(request),
            Ok(None) => Err(anyhow::anyhow!(
                "dynamic circuit bus request channel closed"
            )),
            Err(_) => Err(anyhow::anyhow!(
                "timed out waiting for dynamic circuit bus request"
            )),
        }
    }

    async fn receive_create_dynamic_circuit(
        bus_rx: &mut TestBusReceiver,
    ) -> anyhow::Result<(oneshot::Sender<BusReply>, ShapedDevice)> {
        let (reply_tx, request) = receive_bus_request(bus_rx).await?;
        let shaped_device = expect_create_dynamic_circuit_request(request)?;
        Ok((reply_tx, shaped_device))
    }

    async fn receive_create_and_ack(bus_rx: &mut TestBusReceiver) -> anyhow::Result<ShapedDevice> {
        let (reply_tx, shaped_device) = receive_create_dynamic_circuit(bus_rx).await?;
        ack_bus_reply(reply_tx)?;
        Ok(shaped_device)
    }

    async fn receive_remove_dynamic_circuit(
        bus_rx: &mut TestBusReceiver,
    ) -> anyhow::Result<(oneshot::Sender<BusReply>, String)> {
        let (reply_tx, request) = receive_bus_request(bus_rx).await?;
        let circuit_id = expect_remove_dynamic_circuit_request(request)?;
        Ok((reply_tx, circuit_id))
    }

    async fn receive_remove_and_ack(bus_rx: &mut TestBusReceiver) -> anyhow::Result<String> {
        let (reply_tx, circuit_id) = receive_remove_dynamic_circuit(bus_rx).await?;
        ack_bus_reply(reply_tx)?;
        Ok(circuit_id)
    }

    async fn wait_for_authoritative_owner(
        sink: &ApplyingDynamicCircuitSink,
        session_key: &AccountingSessionKey,
    ) -> anyhow::Result<()> {
        wait_for_authoritative_owner_for_circuit(sink, SHARED_CIRCUIT_ID, session_key).await
    }

    async fn wait_for_authoritative_owner_for_circuit(
        sink: &ApplyingDynamicCircuitSink,
        circuit_id: &str,
        session_key: &AccountingSessionKey,
    ) -> anyhow::Result<()> {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if sink
                    .application_state
                    .lock()
                    .authoritative_session_by_circuit
                    .get(circuit_id)
                    == Some(session_key)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for authoritative dynamic-circuit owner"))
    }

    async fn wait_for_retained_owner(
        sink: &ApplyingDynamicCircuitSink,
        session_key: &AccountingSessionKey,
    ) -> anyhow::Result<()> {
        let owner_key = (SHARED_CIRCUIT_ID.to_string(), session_key.clone());
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if sink
                    .application_state
                    .lock()
                    .retained_upserts_by_owner
                    .contains_key(&owner_key)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for retained shared-circuit owner"))
    }

    async fn wait_for_apply_failed_diagnostic(
        sink: &ApplyingDynamicCircuitSink,
        sessions: &RadiusAccountingSessions,
        circuit_id: &str,
        session_key: &AccountingSessionKey,
    ) -> anyhow::Result<RadiusActivationDiagnostic> {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let Some(diagnostic) =
                    sink.activation_diagnostics(sessions)
                        .into_iter()
                        .find(|diagnostic| {
                            diagnostic.session_key == *session_key
                                && diagnostic.circuit_ids.iter().any(|id| id == circuit_id)
                                && diagnostic.state == RadiusActivationDiagnosticState::ApplyFailed
                        })
                {
                    break diagnostic;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for apply-failed diagnostic"))
    }

    async fn wait_for_released_owner(
        sink: &ApplyingDynamicCircuitSink,
        session_key: &AccountingSessionKey,
    ) -> anyhow::Result<()> {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let owner_released = {
                    let state = sink.application_state.lock();
                    state
                        .authoritative_session_by_circuit
                        .get(SHARED_CIRCUIT_ID)
                        != Some(session_key)
                        && state
                            .current_owners_by_circuit
                            .get(SHARED_CIRCUIT_ID)
                            .is_none_or(|owners| !owners.contains(session_key))
                };
                if owner_released {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for released shared-circuit owner"))
    }

    fn assert_bus_request_channel_empty(bus_rx: &mut TestBusReceiver) {
        match bus_rx.try_recv() {
            Err(mpsc::error::TryRecvError::Empty) => {}
            Ok((_reply_tx, request)) => panic!("unexpected bus request: {request:?}"),
            Err(err) => panic!("unexpected bus channel state: {err:?}"),
        }
    }

    fn ack_bus_reply(reply_tx: oneshot::Sender<BusReply>) -> anyhow::Result<()> {
        send_bus_response(reply_tx, BusResponse::Ack)
    }

    fn fail_bus_reply(reply_tx: oneshot::Sender<BusReply>, message: &str) -> anyhow::Result<()> {
        send_bus_response(reply_tx, BusResponse::Fail(message.to_string()))
    }

    fn send_bus_response(
        reply_tx: oneshot::Sender<BusReply>,
        response: BusResponse,
    ) -> anyhow::Result<()> {
        reply_tx
            .send(BusReply {
                responses: vec![response],
            })
            .map_err(|_| anyhow::anyhow!("failed to send fake bus response"))
    }

    fn expect_create_dynamic_circuit_request(request: BusRequest) -> anyhow::Result<ShapedDevice> {
        let BusRequest::CreateDynamicCircuit { shaped_device } = request else {
            anyhow::bail!("expected CreateDynamicCircuit request, got {request:?}");
        };
        Ok(*shaped_device)
    }

    fn expect_remove_dynamic_circuit_request(request: BusRequest) -> anyhow::Result<String> {
        let BusRequest::RemoveDynamicCircuit { circuit_id } = request else {
            anyhow::bail!("expected RemoveDynamicCircuit request, got {request:?}");
        };
        Ok(circuit_id)
    }

    fn session_key() -> AccountingSessionKey {
        session_key_for("nas-adapter", "session-adapter")
    }

    fn session_key_for(nas: &str, session_id: &str) -> AccountingSessionKey {
        AccountingSessionKey::NasSession {
            nas: NasIdentity::Identifier(nas.to_string()),
            acct_session_id: session_id.to_string(),
        }
    }

    fn complete_event(status_type: AcctStatusType) -> AccountingEvent {
        complete_event_for(status_type, "nas-adapter", "session-adapter")
    }

    fn complete_event_for(
        status_type: AcctStatusType,
        nas: &str,
        session_id: &str,
    ) -> AccountingEvent {
        AccountingEvent {
            status_type: Some(status_type),
            acct_session_id: Some(session_id.to_string()),
            nas_identifier: Some(nas.to_string()),
            user_name: Some("subscriber-adapter".to_string()),
            framed_ip_address: Some(Ipv4Addr::new(198, 51, 100, 20)),
            mikrotik_rate_limits: vec![MikrotikRateLimit {
                original: "10M/25M".to_string(),
                nas_rx_bps: 10_000_000,
                nas_tx_bps: 25_000_000,
                upload_bps: 10_000_000,
                download_bps: 25_000_000,
            }],
            ..AccountingEvent::default()
        }
    }

    fn reset_event() -> AccountingEvent {
        reset_event_for("nas-adapter")
    }

    fn reset_event_for(nas: &str) -> AccountingEvent {
        reset_event_for_status(nas, AcctStatusType::AccountingOff)
    }

    fn reset_event_for_status(nas: &str, status_type: AcctStatusType) -> AccountingEvent {
        AccountingEvent {
            status_type: Some(status_type),
            nas_identifier: Some(nas.to_string()),
            ..AccountingEvent::default()
        }
    }

    fn shaped_device(circuit_id: &str, device_id: &str) -> ShapedDevice {
        ShapedDevice {
            circuit_id: circuit_id.to_string(),
            circuit_name: format!("Circuit {circuit_id}"),
            device_id: device_id.to_string(),
            device_name: format!("Device {device_id}"),
            parent_node: "Parent Node".to_string(),
            parent_node_id: Some("parent-node-id".to_string()),
            anchor_node_id: Some("anchor-node-id".to_string()),
            mac: "aa-bb-cc-dd-ee-ff".to_string(),
            ipv4: Vec::new(),
            ipv6: Vec::new(),
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

    fn unique_secret_path(label: &str) -> anyhow::Result<std::path::PathBuf> {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        Ok(std::env::temp_dir().join(format!(
            "lqosd-radius-{label}-{}-{nanos}",
            std::process::id()
        )))
    }
}
