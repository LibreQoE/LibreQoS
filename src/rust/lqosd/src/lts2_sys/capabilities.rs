use crate::lts2_sys::license_grant::{self, LicenseGrant};
use crate::lts2_sys::lts2_client::get_license_status;
use crate::lts2_sys::shared_types::LtsStatus;
use lqos_bus::LtsCapabilitiesSummary;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use tokio::sync::Notify;
use uuid::Uuid;

#[derive(Default)]
struct RuntimeLicenseState {
    control_service_reachable: bool,
    signup_bootstrap_active: bool,
    suppressed_license_key: Option<String>,
}

static RUNTIME_LICENSE_STATE: Lazy<Mutex<RuntimeLicenseState>> =
    Lazy::new(|| Mutex::new(RuntimeLicenseState::default()));
static CONTROL_CHANNEL_NOTIFY: Lazy<Notify> = Lazy::new(Notify::new);

pub fn set_control_service_reachable(reachable: bool) {
    RUNTIME_LICENSE_STATE.lock().control_service_reachable = reachable;
    if reachable {
        CONTROL_CHANNEL_NOTIFY.notify_waiters();
    }
}

pub fn set_signup_bootstrap_active(active: bool) {
    RUNTIME_LICENSE_STATE.lock().signup_bootstrap_active = active;
    if active {
        CONTROL_CHANNEL_NOTIFY.notify_waiters();
    }
}

pub fn suppress_bootstrap_for_license_key(license_key: &str) {
    let normalized = normalize_non_empty(Some(license_key));
    RUNTIME_LICENSE_STATE.lock().suppressed_license_key = normalized;
}

pub fn clear_bootstrap_suppression() {
    RUNTIME_LICENSE_STATE.lock().suppressed_license_key = None;
    CONTROL_CHANNEL_NOTIFY.notify_waiters();
}

pub fn wake_control_channel() {
    CONTROL_CHANNEL_NOTIFY.notify_waiters();
}

pub async fn wait_for_control_channel_retry(delay: std::time::Duration) {
    tokio::select! {
        _ = tokio::time::sleep(delay) => {}
        _ = CONTROL_CHANNEL_NOTIFY.notified() => {}
    }
}

pub fn current_capabilities() -> LtsCapabilitiesSummary {
    let Ok(config) = lqos_config::load_config() else {
        return LtsCapabilitiesSummary {
            license_state: LtsStatus::Invalid as i32,
            license_state_label: LtsStatus::Invalid.label().to_string(),
            authority_label: "Unlicensed".to_string(),
            control_service_reachable: false,
            bootstrap_intent: false,
            bootstrap_suppressed: false,
            cached_grant_available: false,
            can_open_control_channel: false,
            can_view_insight_ui: false,
            can_use_api_link: false,
            can_use_support_tickets: false,
            can_use_chatbot: false,
            can_receive_remote_commands: false,
            can_collect_long_term_stats: false,
            can_submit_long_term_stats: false,
            mapped_circuit_limit: Some(1000),
        };
    };

    current_capabilities_for_config(config.as_ref())
}

pub fn can_submit_long_term_stats() -> bool {
    current_capabilities().can_submit_long_term_stats
}

pub fn can_open_control_channel() -> bool {
    current_capabilities().can_open_control_channel
}

pub fn control_service_reachable() -> bool {
    RUNTIME_LICENSE_STATE.lock().control_service_reachable
}

fn current_capabilities_for_config(config: &lqos_config::Config) -> LtsCapabilitiesSummary {
    let runtime = RUNTIME_LICENSE_STATE.lock();
    let configured_license_key = normalize_non_empty(config.long_term_stats.license_key.as_deref());
    let valid_bootstrap_key =
        parse_license_uuid(config.long_term_stats.license_key.as_deref()).is_some();
    let bootstrap_intent = configured_license_key.is_some() || runtime.signup_bootstrap_active;
    let bootstrap_suppressed = configured_license_key.is_some()
        && configured_license_key == runtime.suppressed_license_key;
    let control_service_reachable = runtime.control_service_reachable;
    drop(runtime);

    let cached_grant = license_grant::current_valid_grant();
    let cached_grant_available = cached_grant.is_some();
    let live_status = LtsStatus::from_i32(get_license_status().license_type);
    let live_entitled = control_service_reachable && is_entitled_status(live_status);
    let effective_status = if live_entitled {
        live_status
    } else if let Some(grant) = cached_grant.as_ref() {
        LtsStatus::from_i32(grant.license_state)
    } else if bootstrap_intent {
        LtsStatus::NotChecked
    } else {
        LtsStatus::Invalid
    };

    let authority_label = if live_entitled {
        "Live license session"
    } else if cached_grant_available {
        "Cached signed grant"
    } else if bootstrap_intent {
        "Bootstrap pending"
    } else {
        "Unlicensed"
    };

    let can_open_control_channel =
        control_service_reachable || (valid_bootstrap_key && !bootstrap_suppressed);
    let mapped_circuit_limit = mapped_circuit_limit(effective_status, cached_grant.as_ref());
    let can_collect_or_submit =
        config.long_term_stats.gather_stats && supports_long_term_stats(effective_status);

    LtsCapabilitiesSummary {
        license_state: effective_status as i32,
        license_state_label: effective_status.label().to_string(),
        authority_label: authority_label.to_string(),
        control_service_reachable,
        bootstrap_intent,
        bootstrap_suppressed,
        cached_grant_available,
        can_open_control_channel,
        can_view_insight_ui: can_view_insight_ui(effective_status),
        can_use_api_link: can_use_api_link(effective_status),
        can_use_support_tickets: can_use_support_tickets(effective_status),
        can_use_chatbot: can_use_chatbot(effective_status),
        can_receive_remote_commands: can_receive_remote_commands(effective_status),
        can_collect_long_term_stats: can_collect_or_submit,
        can_submit_long_term_stats: can_collect_or_submit,
        mapped_circuit_limit,
    }
}

fn mapped_circuit_limit(status: LtsStatus, cached_grant: Option<&LicenseGrant>) -> Option<u64> {
    if !lifts_mapped_circuit_cap(status) {
        return Some(1000);
    }

    cached_grant.and_then(|grant| grant.max_circuits)
}

fn normalize_non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn parse_license_uuid(value: Option<&str>) -> Option<Uuid> {
    let value = normalize_non_empty(value)?;
    Uuid::parse_str(&value.replace('-', "")).ok()
}

fn is_entitled_status(status: LtsStatus) -> bool {
    !matches!(status, LtsStatus::Invalid | LtsStatus::NotChecked)
}

fn lifts_mapped_circuit_cap(status: LtsStatus) -> bool {
    matches!(
        status,
        LtsStatus::AlwaysFree
            | LtsStatus::FreeTrial
            | LtsStatus::SelfHosted
            | LtsStatus::ApiOnly
            | LtsStatus::Full
    )
}

fn supports_long_term_stats(status: LtsStatus) -> bool {
    matches!(
        status,
        LtsStatus::AlwaysFree | LtsStatus::FreeTrial | LtsStatus::SelfHosted | LtsStatus::Full
    )
}

fn can_view_insight_ui(status: LtsStatus) -> bool {
    matches!(
        status,
        LtsStatus::AlwaysFree | LtsStatus::FreeTrial | LtsStatus::SelfHosted | LtsStatus::Full
    )
}

fn can_use_api_link(status: LtsStatus) -> bool {
    matches!(status, LtsStatus::ApiOnly | LtsStatus::Full)
}

fn can_use_support_tickets(status: LtsStatus) -> bool {
    matches!(
        status,
        LtsStatus::AlwaysFree
            | LtsStatus::FreeTrial
            | LtsStatus::SelfHosted
            | LtsStatus::ApiOnly
            | LtsStatus::Full
    )
}

fn can_use_chatbot(status: LtsStatus) -> bool {
    can_use_support_tickets(status)
}

fn can_receive_remote_commands(status: LtsStatus) -> bool {
    matches!(
        status,
        LtsStatus::AlwaysFree | LtsStatus::FreeTrial | LtsStatus::SelfHosted | LtsStatus::Full
    )
}
