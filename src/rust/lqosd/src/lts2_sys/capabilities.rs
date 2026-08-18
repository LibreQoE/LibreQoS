use crate::lts2_sys::license_grant::{self, LicenseGrant};
use crate::lts2_sys::lts2_client::get_license_status;
use crate::lts2_sys::shared_types::LtsStatus;
use lqos_bus::{DEFAULT_MAPPED_CIRCUIT_LIMIT, LtsCapabilitiesSummary};
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LicenseAuthority {
    Live,
    Cached,
    BootstrapPending,
    Unlicensed,
}

struct EffectiveLicense<'a> {
    status: LtsStatus,
    authority: LicenseAuthority,
    cached_grant: Option<&'a LicenseGrant>,
}

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
            can_use_api_link: mapped_circuit_count() <= DEFAULT_MAPPED_CIRCUIT_LIMIT,
            can_use_support_tickets: false,
            can_use_chatbot: false,
            can_receive_remote_commands: false,
            can_collect_long_term_stats: false,
            can_submit_long_term_stats: false,
            mapped_circuit_limit: Some(DEFAULT_MAPPED_CIRCUIT_LIMIT),
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
    let configured_license_uuid = parse_license_uuid(config.long_term_stats.license_key.as_deref());
    let valid_bootstrap_key = configured_license_uuid.is_some();
    let bootstrap_intent = configured_license_key.is_some() || runtime.signup_bootstrap_active;
    let bootstrap_suppressed = configured_license_key.is_some()
        && configured_license_key == runtime.suppressed_license_key;
    let control_service_reachable = runtime.control_service_reachable;
    drop(runtime);

    let cached_grant = license_grant::current_valid_grant();
    let cached_grant_available = cached_grant.is_some();
    let live_status = LtsStatus::from_i32(get_license_status().license_type);
    let effective = effective_license(
        control_service_reachable,
        live_status,
        cached_grant.as_ref(),
        configured_license_uuid,
        bootstrap_intent,
    );
    let authority_label = match effective.authority {
        LicenseAuthority::Live => "Live license session",
        LicenseAuthority::Cached => "Cached signed grant",
        LicenseAuthority::BootstrapPending => "Bootstrap pending",
        LicenseAuthority::Unlicensed => "Unlicensed",
    };

    let can_open_control_channel =
        control_service_reachable || (valid_bootstrap_key && !bootstrap_suppressed);
    let mapped_circuit_limit = mapped_circuit_limit(effective.status, effective.cached_grant);
    let mapped_circuit_count = mapped_circuit_count();
    let can_collect_or_submit =
        config.long_term_stats.gather_stats && supports_long_term_stats(effective.status);

    LtsCapabilitiesSummary {
        license_state: effective.status as i32,
        license_state_label: effective.status.label().to_string(),
        authority_label: authority_label.to_string(),
        control_service_reachable,
        bootstrap_intent,
        bootstrap_suppressed,
        cached_grant_available,
        can_open_control_channel,
        can_view_insight_ui: can_view_insight_ui(effective.status),
        can_use_api_link: can_use_api_link(mapped_circuit_count, mapped_circuit_limit),
        can_use_support_tickets: can_use_support_tickets(effective.status),
        can_use_chatbot: can_use_chatbot(effective.status),
        can_receive_remote_commands: can_receive_remote_commands(effective.status),
        can_collect_long_term_stats: can_collect_or_submit,
        can_submit_long_term_stats: can_collect_or_submit,
        mapped_circuit_limit,
    }
}

fn mapped_circuit_limit(status: LtsStatus, cached_grant: Option<&LicenseGrant>) -> Option<u64> {
    if !lifts_mapped_circuit_cap(status) {
        return Some(DEFAULT_MAPPED_CIRCUIT_LIMIT);
    }

    cached_grant
        .and_then(|grant| grant.max_circuits)
        .map(|limit| limit.max(DEFAULT_MAPPED_CIRCUIT_LIMIT))
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

fn effective_license<'a>(
    control_service_reachable: bool,
    live_status: LtsStatus,
    cached_grant: Option<&'a LicenseGrant>,
    configured_license_uuid: Option<Uuid>,
    bootstrap_intent: bool,
) -> EffectiveLicense<'a> {
    if control_service_reachable && is_entitled_status(live_status) {
        let matching_grant = cached_grant.filter(|grant| {
            LtsStatus::from_i32(grant.license_state) == live_status
                && grant.license_uuid == configured_license_uuid
        });
        EffectiveLicense {
            status: live_status,
            authority: LicenseAuthority::Live,
            cached_grant: matching_grant,
        }
    } else if let Some(grant) =
        cached_grant.filter(|grant| is_entitled_status(LtsStatus::from_i32(grant.license_state)))
    {
        EffectiveLicense {
            status: LtsStatus::from_i32(grant.license_state),
            authority: LicenseAuthority::Cached,
            cached_grant: Some(grant),
        }
    } else if bootstrap_intent {
        EffectiveLicense {
            status: LtsStatus::NotChecked,
            authority: LicenseAuthority::BootstrapPending,
            cached_grant: None,
        }
    } else {
        EffectiveLicense {
            status: LtsStatus::Invalid,
            authority: LicenseAuthority::Unlicensed,
            cached_grant: None,
        }
    }
}

fn lifts_mapped_circuit_cap(status: LtsStatus) -> bool {
    matches!(
        status,
        LtsStatus::AlwaysFree
            | LtsStatus::FreeTrial
            | LtsStatus::SelfHosted
            | LtsStatus::ApiOnly
            | LtsStatus::Full
            | LtsStatus::ForeverFreeApi
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

fn mapped_circuit_count() -> u64 {
    lqos_network_devices::mapped_circuit_count() as u64
}

fn can_use_api_link(mapped_circuit_count: u64, mapped_circuit_limit: Option<u64>) -> bool {
    mapped_circuit_limit.is_none_or(|limit| mapped_circuit_count <= limit)
}

fn can_use_support_tickets(status: LtsStatus) -> bool {
    matches!(
        status,
        LtsStatus::AlwaysFree
            | LtsStatus::FreeTrial
            | LtsStatus::SelfHosted
            | LtsStatus::ApiOnly
            | LtsStatus::ForeverFreeApi
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

#[cfg(test)]
mod tests {
    use super::{
        LicenseAuthority, can_use_api_link, can_use_support_tickets, can_view_insight_ui,
        effective_license, lifts_mapped_circuit_cap, mapped_circuit_limit,
        supports_long_term_stats,
    };
    use crate::lts2_sys::license_grant::LicenseGrant;
    use crate::lts2_sys::shared_types::LtsStatus;
    use lqos_bus::DEFAULT_MAPPED_CIRCUIT_LIMIT;

    #[test]
    fn free_api_access_includes_the_thousandth_mapped_circuit() {
        let limit = Some(DEFAULT_MAPPED_CIRCUIT_LIMIT);
        assert!(can_use_api_link(DEFAULT_MAPPED_CIRCUIT_LIMIT - 1, limit));
        assert!(can_use_api_link(DEFAULT_MAPPED_CIRCUIT_LIMIT, limit));
        assert!(!can_use_api_link(DEFAULT_MAPPED_CIRCUIT_LIMIT + 1, limit));
    }

    #[test]
    fn api_access_honors_entitled_grant_limits() {
        assert!(can_use_api_link(10_000, None));
        assert!(can_use_api_link(2_000, Some(2_000)));
        assert!(!can_use_api_link(2_001, Some(2_000)));
    }

    #[test]
    fn forever_free_api_is_api_only() {
        assert!(lifts_mapped_circuit_cap(LtsStatus::ForeverFreeApi));
        assert!(!supports_long_term_stats(LtsStatus::ForeverFreeApi));
        assert!(!can_view_insight_ui(LtsStatus::ForeverFreeApi));
        assert!(can_use_support_tickets(LtsStatus::ForeverFreeApi));
        let grant = LicenseGrant {
            license_state: LtsStatus::ForeverFreeApi as i32,
            trial_expiration: 0,
            grant_expires: i64::MAX,
            issued_at: 0,
            license_uuid: None,
            node_id: None,
            max_circuits: Some(2_000),
            lqosd_public_key: Vec::new(),
        };
        assert_eq!(
            mapped_circuit_limit(LtsStatus::ForeverFreeApi, Some(&grant)),
            Some(2_000)
        );
    }

    #[test]
    fn live_entitlement_precedes_cached_grant() {
        let grant = test_grant(LtsStatus::ForeverFreeApi, Some(1_500));
        let effective = effective_license(true, LtsStatus::Full, Some(&grant), None, true);
        assert_eq!(effective.status, LtsStatus::Full);
        assert_eq!(effective.authority, LicenseAuthority::Live);
        assert!(effective.cached_grant.is_none());
    }

    #[test]
    fn cached_entitlement_is_used_when_live_session_is_unavailable() {
        let grant = test_grant(LtsStatus::ForeverFreeApi, Some(1_500));
        let effective = effective_license(false, LtsStatus::Invalid, Some(&grant), None, true);
        assert_eq!(effective.status, LtsStatus::ForeverFreeApi);
        assert_eq!(effective.authority, LicenseAuthority::Cached);
        assert_eq!(
            effective.cached_grant.and_then(|grant| grant.max_circuits),
            Some(1_500)
        );
    }

    #[test]
    fn invalid_cached_state_does_not_authorize_access() {
        let grant = test_grant(LtsStatus::Invalid, Some(2_000));
        let effective = effective_license(false, LtsStatus::Invalid, Some(&grant), None, false);
        assert_eq!(effective.status, LtsStatus::Invalid);
        assert_eq!(effective.authority, LicenseAuthority::Unlicensed);
        assert!(effective.cached_grant.is_none());
    }

    #[test]
    fn signed_grant_limit_never_reduces_the_free_allowance() {
        let grant = test_grant(LtsStatus::ForeverFreeApi, Some(500));
        assert_eq!(
            mapped_circuit_limit(LtsStatus::ForeverFreeApi, Some(&grant)),
            Some(DEFAULT_MAPPED_CIRCUIT_LIMIT)
        );
    }

    #[test]
    fn live_entitlement_honors_a_matching_signed_limit() {
        let grant = test_grant(LtsStatus::Full, Some(1_500));
        let effective = effective_license(true, LtsStatus::Full, Some(&grant), None, true);
        assert_eq!(effective.authority, LicenseAuthority::Live);
        assert_eq!(
            mapped_circuit_limit(effective.status, effective.cached_grant),
            Some(1_500)
        );
    }

    fn test_grant(status: LtsStatus, max_circuits: Option<u64>) -> LicenseGrant {
        LicenseGrant {
            license_state: status as i32,
            trial_expiration: 0,
            grant_expires: i64::MAX,
            issued_at: 0,
            license_uuid: None,
            node_id: None,
            max_circuits,
            lqosd_public_key: Vec::new(),
        }
    }
}
