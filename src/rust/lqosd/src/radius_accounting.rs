//! lqosd integration for the RADIUS accounting listener.

use lqos_config::{RadiusAccountingClient, RadiusAccountingConfig};
use lqos_radius::{
    AccountingEvent, AccountingListenerOutcome, AccountingSessionKey, AccountingSessionState,
    AccountingSessionStore, AccountingSessionUpdate, DynamicCircuitCommandSink,
    DynamicCircuitIntent, DynamicCircuitMapping, ListenerConfig, RadiusListener,
    TrustedClientSource, TrustedRadiusClient, start_listener,
};
use std::collections::{HashMap, HashSet};
use std::io;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

/// Starts the RADIUS accounting listener when it is enabled in configuration.
///
/// Side effects: reads configured shared-secret files, binds the configured UDP
/// socket when enabled, and spawns a Tokio task to receive accounting packets.
/// It does not write files, call dynamic-circuit persistence, or touch TC/XDP.
pub(crate) async fn start_configured_radius_accounting(
    config: Option<RadiusAccountingConfig>,
) -> Result<Option<JoinHandle<()>>, RadiusAccountingStartupError> {
    let Some(runtime_config) = runtime_config_from_config(config).await? else {
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
    ))))
}

async fn runtime_config_from_config(
    config: Option<RadiusAccountingConfig>,
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
    }))
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
) {
    let mut sessions =
        RadiusAccountingSessions::new(runtime_config.default_ttl, runtime_config.stale_grace);
    let mut command_sink = DeferredDynamicCircuitSink;
    let mut expiry_interval = tokio::time::interval(cleanup_interval(
        runtime_config.default_ttl,
        runtime_config.stale_grace,
    ));

    loop {
        tokio::select! {
            outcome = listener.receive_next_verified(&runtime_config.clients) => {
                match outcome {
                    Ok(outcome) => handle_listener_outcome(
                        outcome,
                        &mut sessions,
                        &mut command_sink,
                        Instant::now(),
                    ),
                    Err(err) if listener_error_is_recoverable(&err) => {
                        warn!("RADIUS accounting listener packet handling failed: {err}");
                    }
                    Err(err) => {
                        error!("RADIUS accounting listener stopped: {err}");
                        return;
                    }
                }
            }
            _ = expiry_interval.tick() => {
                let expired = sessions.expire_due(Instant::now(), &mut command_sink);
                if expired > 0 {
                    debug!(expired, "expired RADIUS accounting session(s)");
                }
            }
        }
    }
}

fn handle_listener_outcome(
    outcome: AccountingListenerOutcome,
    sessions: &mut RadiusAccountingSessions,
    command_sink: &mut impl DynamicCircuitCommandSink,
    now: Instant,
) {
    match outcome {
        AccountingListenerOutcome::Accepted(accepted) => {
            let event = AccountingEvent::from_verified(&accepted.request);
            handle_accounting_event(
                event,
                sessions,
                command_sink,
                now,
                accepted.peer,
                accepted.received_len,
                accepted.response_len,
            );
        }
        AccountingListenerOutcome::RejectedSource { peer, received_len } => {
            warn!(
                peer = %peer,
                received_len,
                "rejected RADIUS accounting packet from untrusted source"
            );
        }
        AccountingListenerOutcome::RejectedAmbiguousSource { peer, received_len } => {
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
            warn!(
                peer = %peer,
                received_len,
                error = %source,
                "rejected RADIUS accounting packet"
            );
        }
    }
}

fn handle_accounting_event(
    event: AccountingEvent,
    sessions: &mut RadiusAccountingSessions,
    command_sink: &mut impl DynamicCircuitCommandSink,
    now: Instant,
    peer: SocketAddr,
    received_len: usize,
    response_len: usize,
) {
    let update = sessions.apply_event(event, now, command_sink);
    debug!(
        peer = %peer,
        received_len,
        response_len,
        ?update,
        "accepted RADIUS accounting packet"
    );
}

fn listener_error_is_recoverable(err: &lqos_radius::ListenerError) -> bool {
    matches!(err, lqos_radius::ListenerError::Send { .. })
}

struct RadiusAccountingSessions {
    store: AccountingSessionStore,
    updated_at: HashMap<AccountingSessionKey, Instant>,
    default_ttl: Duration,
    stale_grace: Duration,
}

impl RadiusAccountingSessions {
    fn new(default_ttl: Duration, stale_grace: Duration) -> Self {
        Self {
            store: AccountingSessionStore::new(),
            updated_at: HashMap::new(),
            default_ttl,
            stale_grace,
        }
    }

    fn apply_event(
        &mut self,
        event: AccountingEvent,
        now: Instant,
        command_sink: &mut impl DynamicCircuitCommandSink,
    ) -> AccountingSessionUpdate {
        let previous_stale_keys = previous_stale_keys(&self.store, &event);
        let update = self.store.apply_event_with_mapping_and_commands(
            event,
            DynamicCircuitMapping::MissingParent,
            command_sink,
        );
        self.record_update(&update, previous_stale_keys.as_ref(), now);
        if self.updated_at.len() > self.store.len() {
            self.prune_removed_sessions();
        }
        update
    }

    fn record_update(
        &mut self,
        update: &AccountingSessionUpdate,
        previous_stale_keys: Option<&HashSet<AccountingSessionKey>>,
        now: Instant,
    ) {
        match update {
            AccountingSessionUpdate::SessionUpdated { key, .. } => {
                self.updated_at.insert(key.clone(), now);
            }
            AccountingSessionUpdate::NasSessionsMarkedStale { reset, .. } => {
                let Some(previous_stale_keys) = previous_stale_keys else {
                    return;
                };
                for (key, session) in self.store.sessions() {
                    if !previous_stale_keys.contains(key)
                        && session.state == AccountingSessionState::Stale(*reset)
                    {
                        self.updated_at.insert(key.clone(), now);
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
    }

    fn expire_due(
        &mut self,
        now: Instant,
        command_sink: &mut impl DynamicCircuitCommandSink,
    ) -> usize {
        let expired_keys = self
            .store
            .sessions()
            .filter_map(|(key, session)| {
                let updated_at = *self.updated_at.get(key)?;
                let ttl = match session.state {
                    AccountingSessionState::Stale(_) => self.stale_grace,
                    AccountingSessionState::Active | AccountingSessionState::Stopped => {
                        self.default_ttl
                    }
                };
                (now.duration_since(updated_at) >= ttl).then(|| key.clone())
            })
            .collect::<Vec<_>>();

        let expired_count = expired_keys.len();
        for key in expired_keys {
            self.updated_at.remove(&key);
            self.store.expire_session_with_commands(&key, command_sink);
        }
        expired_count
    }
}

fn previous_stale_keys(
    store: &AccountingSessionStore,
    event: &AccountingEvent,
) -> Option<HashSet<AccountingSessionKey>> {
    if !matches!(
        event.status_type,
        Some(
            lqos_radius::AcctStatusType::AccountingOn | lqos_radius::AcctStatusType::AccountingOff
        )
    ) {
        return None;
    }

    Some(
        store
            .sessions()
            .filter(|(_, session)| matches!(session.state, AccountingSessionState::Stale(_)))
            .map(|(key, _)| key.clone())
            .collect(),
    )
}

fn cleanup_interval(default_ttl: Duration, stale_grace: Duration) -> Duration {
    Duration::from_secs(default_ttl.min(stale_grace).as_secs().clamp(1, 60))
}

struct DeferredDynamicCircuitSink;

impl DynamicCircuitCommandSink for DeferredDynamicCircuitSink {
    fn emit(&mut self, intent: DynamicCircuitIntent) {
        debug!(
            circuit_id = intent.circuit_id(),
            "RADIUS dynamic-circuit intent deferred until daemon-side application is enabled"
        );
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
    use lqos_config::{RadiusClientSource, RadiusSharedSecretSource};
    use lqos_radius::{AcctStatusType, MikrotikRateLimit, NasIdentity, PendingSessionReason};
    use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[tokio::test]
    async fn absent_or_disabled_config_does_not_build_runtime_listener() -> anyhow::Result<()> {
        assert!(runtime_config_from_config(None).await?.is_none());
        assert!(
            runtime_config_from_config(Some(RadiusAccountingConfig::default()))
                .await?
                .is_none()
        );

        Ok(())
    }

    #[tokio::test]
    async fn disabled_start_returns_no_listener_handle() -> anyhow::Result<()> {
        assert!(start_configured_radius_accounting(None).await?.is_none());
        assert!(
            start_configured_radius_accounting(Some(RadiusAccountingConfig::default()))
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

        let handle = start_configured_radius_accounting(Some(config)).await;
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
        let runtime_config = runtime_config_from_config(Some(config)).await;
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
    async fn secret_file_errors_identify_client_without_secret_bytes() -> anyhow::Result<()> {
        let secret_path = unique_secret_path("do-not-log-this-secret")?;
        let mut config = enabled_config(&secret_path);
        config.clients[0].name = "core-nas".to_string();

        let error = match runtime_config_from_config(Some(config)).await {
            Ok(_) => anyhow::bail!("missing secret file should fail startup"),
            Err(error) => error,
        };
        let message = error.to_string();

        assert!(message.contains("core-nas"));
        assert!(!message.contains("do-not-log-this-secret"));

        Ok(())
    }

    #[test]
    fn accepted_event_updates_sessions_without_dynamic_application() -> anyhow::Result<()> {
        let mut sessions =
            RadiusAccountingSessions::new(Duration::from_secs(900), Duration::from_secs(120));
        let mut sink = RecordingDynamicCircuitSink::default();
        let now = Instant::now();

        handle_accounting_event(
            complete_event(AcctStatusType::Start),
            &mut sessions,
            &mut sink,
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
        assert!(sink.intents.is_empty());

        Ok(())
    }

    #[test]
    fn session_expiry_uses_default_ttl_and_stale_grace() -> anyhow::Result<()> {
        let mut sessions =
            RadiusAccountingSessions::new(Duration::from_secs(10), Duration::from_secs(2));
        let mut sink = RecordingDynamicCircuitSink::default();
        let started_at = Instant::now();

        sessions.apply_event(complete_event(AcctStatusType::Start), started_at, &mut sink);
        assert_eq!(
            sessions.expire_due(started_at + Duration::from_secs(9), &mut sink),
            0
        );
        assert_eq!(
            sessions.expire_due(started_at + Duration::from_secs(10), &mut sink),
            1
        );
        assert!(!sessions.updated_at.contains_key(&session_key()));

        let restarted_at = started_at + Duration::from_secs(20);
        let reset_at = restarted_at + Duration::from_secs(1);
        sessions.apply_event(
            complete_event(AcctStatusType::Start),
            restarted_at,
            &mut sink,
        );
        sessions.apply_event(reset_event(), reset_at, &mut sink);
        assert_eq!(
            sessions
                .store
                .session(&session_key())
                .map(|session| session.state),
            Some(AccountingSessionState::Stale(
                lqos_radius::NasResetStatus::AccountingOff
            ))
        );
        assert_eq!(
            sessions.expire_due(reset_at + Duration::from_secs(1), &mut sink),
            0
        );
        assert_eq!(
            sessions.expire_due(reset_at + Duration::from_secs(2), &mut sink),
            1
        );
        assert!(!sessions.updated_at.contains_key(&session_key()));
        assert!(sink.intents.is_empty());

        Ok(())
    }

    #[test]
    fn promoted_sessions_prune_old_timestamp_keys() {
        let mut sessions =
            RadiusAccountingSessions::new(Duration::from_secs(900), Duration::from_secs(120));
        let mut sink = RecordingDynamicCircuitSink::default();
        let started_at = Instant::now();
        let mut pending_event = complete_event(AcctStatusType::Start);
        pending_event.nas_identifier = None;

        sessions.apply_event(pending_event, started_at, &mut sink);
        assert_eq!(sessions.updated_at.len(), 1);

        let promoted_at = started_at + Duration::from_secs(1);
        sessions.apply_event(
            complete_event(AcctStatusType::InterimUpdate),
            promoted_at,
            &mut sink,
        );

        assert_eq!(sessions.updated_at.len(), 1);
        assert_eq!(sessions.updated_at.get(&session_key()), Some(&promoted_at));
    }

    #[test]
    fn nas_reset_refreshes_only_matching_stale_sessions() {
        let mut sessions =
            RadiusAccountingSessions::new(Duration::from_secs(60), Duration::from_secs(2));
        let mut sink = RecordingDynamicCircuitSink::default();
        let started_at = Instant::now();
        let nas_a_reset_at = started_at + Duration::from_secs(1);
        let nas_b_started_at = started_at + Duration::from_secs(2);
        let nas_b_reset_at = started_at + Duration::from_secs(3);
        let nas_a_key = session_key_for("nas-a", "session-a");

        sessions.apply_event(
            complete_event_for(AcctStatusType::Start, "nas-a", "session-a"),
            started_at,
            &mut sink,
        );
        sessions.apply_event(reset_event_for("nas-a"), nas_a_reset_at, &mut sink);
        assert_eq!(sessions.updated_at.get(&nas_a_key), Some(&nas_a_reset_at));

        sessions.apply_event(
            complete_event_for(AcctStatusType::Start, "nas-b", "session-b"),
            nas_b_started_at,
            &mut sink,
        );
        sessions.apply_event(reset_event_for("nas-b"), nas_b_reset_at, &mut sink);

        assert_eq!(sessions.updated_at.get(&nas_a_key), Some(&nas_a_reset_at));
        assert_eq!(sessions.expire_due(nas_b_reset_at, &mut sink), 1);
        assert!(sessions.store.session(&nas_a_key).is_none());
    }

    #[test]
    fn repeated_nas_reset_does_not_refresh_already_stale_sessions() {
        let mut sessions =
            RadiusAccountingSessions::new(Duration::from_secs(60), Duration::from_secs(2));
        let mut sink = RecordingDynamicCircuitSink::default();
        let started_at = Instant::now();
        let first_reset_at = started_at + Duration::from_secs(1);
        let repeated_reset_at = first_reset_at + Duration::from_secs(1);
        let stale_deadline = first_reset_at + Duration::from_secs(2);
        let nas_a_key = session_key_for("nas-a", "session-a");

        sessions.apply_event(
            complete_event_for(AcctStatusType::Start, "nas-a", "session-a"),
            started_at,
            &mut sink,
        );
        sessions.apply_event(reset_event_for("nas-a"), first_reset_at, &mut sink);
        sessions.apply_event(reset_event_for("nas-a"), repeated_reset_at, &mut sink);

        assert_eq!(sessions.updated_at.get(&nas_a_key), Some(&first_reset_at));
        assert_eq!(sessions.expire_due(stale_deadline, &mut sink), 1);
        assert!(sessions.store.session(&nas_a_key).is_none());
    }

    #[test]
    fn alternating_nas_reset_status_does_not_refresh_already_stale_sessions() {
        let mut sessions =
            RadiusAccountingSessions::new(Duration::from_secs(60), Duration::from_secs(2));
        let mut sink = RecordingDynamicCircuitSink::default();
        let started_at = Instant::now();
        let first_reset_at = started_at + Duration::from_secs(1);
        let second_reset_at = first_reset_at + Duration::from_secs(1);
        let stale_deadline = first_reset_at + Duration::from_secs(2);
        let nas_a_key = session_key_for("nas-a", "session-a");

        sessions.apply_event(
            complete_event_for(AcctStatusType::Start, "nas-a", "session-a"),
            started_at,
            &mut sink,
        );
        sessions.apply_event(reset_event_for("nas-a"), first_reset_at, &mut sink);
        sessions.apply_event(
            reset_event_for_status("nas-a", AcctStatusType::AccountingOn),
            second_reset_at,
            &mut sink,
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
        assert_eq!(sessions.expire_due(stale_deadline, &mut sink), 1);
        assert!(sessions.store.session(&nas_a_key).is_none());
    }

    #[test]
    fn listener_send_errors_are_recoverable() {
        let err = lqos_radius::ListenerError::Send {
            peer: test_listen_addr(),
            source: std::io::Error::from_raw_os_error(nix::libc::ECONNREFUSED),
        };

        assert!(listener_error_is_recoverable(&err));
    }

    fn enabled_config(secret_path: &Path) -> RadiusAccountingConfig {
        RadiusAccountingConfig {
            enabled: true,
            listen: Some(test_listen_addr()),
            default_ttl_seconds: 900,
            stale_grace_seconds: 120,
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

    #[derive(Default)]
    struct RecordingDynamicCircuitSink {
        intents: Vec<DynamicCircuitIntent>,
    }

    impl DynamicCircuitCommandSink for RecordingDynamicCircuitSink {
        fn emit(&mut self, intent: DynamicCircuitIntent) {
            self.intents.push(intent);
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
