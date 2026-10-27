//! Moves version checking out of the web system and into its
//! own module/thread/actor. This removes any delay when the
//! web system is running without Internet access.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;
use std::thread;

const VERSION_STRING: &str = include_str!("../../../VERSION_STRING");
const FORCE_UPDATE_ENV: &str = "LQOS_FORCE_UPDATE_AVAILABLE";

#[derive(Serialize, Debug)]
struct VersionCheckRequest {
    current_git_hash: String,
    version_string: String,
    node_id: String,
}

#[derive(Deserialize, Debug, Default)]
pub struct VersionCheckResponse {
    update_available: bool,
}

static NEW_VERSION_AVAILABLE: AtomicBool = AtomicBool::new(false);

fn parse_force_update_override(raw: &str) -> Option<bool> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if matches!(
        trimmed.to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    ) {
        return Some(true);
    }

    if matches!(
        trimmed.to_ascii_lowercase().as_str(),
        "0" | "false" | "no" | "off"
    ) {
        return Some(false);
    }

    None
}

fn force_update_available() -> Option<bool> {
    let raw = std::env::var(FORCE_UPDATE_ENV).ok()?;
    let parsed = parse_force_update_override(&raw);
    if parsed.is_none() {
        tracing::warn!(
            "{} has invalid value {:?}; expected one of: 1/0 true/false yes/no on/off",
            FORCE_UPDATE_ENV,
            raw
        );
    }
    parsed
}

/// Initializes the version checking system.
pub fn start_version_check() -> Result<()> {
    thread::Builder::new()
        .name("version_check".to_string())
        .spawn(|| {
            loop {
                let Ok(cfg) = lqos_config::load_config() else {
                    continue;
                };

                let current_hash = env!("GIT_HASH");
                let request = VersionCheckRequest {
                    current_git_hash: current_hash.to_string(),
                    version_string: VERSION_STRING.trim().to_string(),
                    node_id: cfg.node_id.to_string(),
                };

                let update_available = check_version(request);
                match update_available {
                    Err(e) => {
                        tracing::error!("Failed to check for version update: {}", e);
                        thread::sleep(std::time::Duration::from_secs(60));
                        continue;
                    }
                    Ok(update_available) => {
                        NEW_VERSION_AVAILABLE
                            .store(update_available, std::sync::atomic::Ordering::Relaxed);
                    }
                }

                // Sleep for 12 hours
                thread::sleep(std::time::Duration::from_secs(12 * 60 * 60));
            }
        })
        .expect("Failed to start version check thread");

    Ok(())
}

/// Returns true if a new version is available.
///
/// the sidebar UI-hygiene card: a NON-RELEASE build (dev/build-stamp version) and any build
/// with `LIBREQOS_DISABLE_UPDATE_CHECK` set never advertise an update — the
/// check is a release-channel feature.
pub fn new_version_available() -> bool {
    if update_check_suppressed() {
        return false;
    }
    if let Some(forced) = force_update_available() {
        return forced;
    }
    NEW_VERSION_AVAILABLE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Env knob that hides the update indicator entirely.
const DISABLE_UPDATE_CHECK_ENV: &str = "LIBREQOS_DISABLE_UPDATE_CHECK";

/// True when `version` is not a release identity: a `-`/`+` suffix, a
/// build-date stamp (a fourth dot component), or an explicit `dev` marker.
fn is_non_release_version(version: &str) -> bool {
    let v = version.trim();
    v.contains('-')
        || v.contains('+')
        || v.split('.').count() > 3
        || v.to_ascii_lowercase().contains("dev")
}

/// True when the update check must be hidden (the disable env knob or a
/// non-release build).
fn update_check_suppressed() -> bool {
    let disabled = std::env::var(DISABLE_UPDATE_CHECK_ENV)
        .map(|raw| !raw.trim().is_empty())
        .unwrap_or(false);
    disabled || is_non_release_version(VERSION_STRING)
}

fn check_version(request: VersionCheckRequest) -> Result<bool> {
    let response: VersionCheckResponse =
        ureq::post("https://insight.libreqos.com/shaper_api/version_check")
            .send_json(serde_json::to_value(&request)?)?
            .into_json()?;
    Ok(response.update_available)
}

#[cfg(test)]
mod tests {
    use super::{is_non_release_version, parse_force_update_override};

    #[test]
    fn release_versions_are_checked() {
        assert!(!is_non_release_version("3.0.7"));
        assert!(!is_non_release_version(" 3.0.7 "));
    }

    #[test]
    fn dev_stamps_suppress_the_update_check() {
        assert!(is_non_release_version("3.0.7-beta"));
        assert!(is_non_release_version("3.0.7+dev"));
        assert!(is_non_release_version("3.0.7.202609201036"));
        assert!(is_non_release_version("3.0.7-dev"));
    }

    #[test]
    fn parses_truthy_force_update_values() {
        assert_eq!(parse_force_update_override("1"), Some(true));
        assert_eq!(parse_force_update_override("true"), Some(true));
        assert_eq!(parse_force_update_override("YES"), Some(true));
        assert_eq!(parse_force_update_override(" on "), Some(true));
    }

    #[test]
    fn parses_falsey_force_update_values() {
        assert_eq!(parse_force_update_override("0"), Some(false));
        assert_eq!(parse_force_update_override("false"), Some(false));
        assert_eq!(parse_force_update_override("NO"), Some(false));
        assert_eq!(parse_force_update_override(" off "), Some(false));
    }

    #[test]
    fn rejects_invalid_force_update_values() {
        assert_eq!(parse_force_update_override(""), None);
        assert_eq!(parse_force_update_override("maybe"), None);
    }
}
