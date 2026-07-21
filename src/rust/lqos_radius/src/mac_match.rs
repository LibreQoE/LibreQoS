//! RADIUS identity matching against `ShapedDevices.csv` rows.

use crate::AccountingEvent;
use lqos_config::ShapedDevice;
use std::collections::HashMap;

/// Matches RADIUS usernames and `Calling-Station-Id` values to shaped-device rows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShapedDevicesMacMatcher {
    matches_by_mac: HashMap<String, ShapedDevicesMacEntry>,
    matches_by_username: HashMap<String, ShapedDevicesMacEntry>,
}

impl ShapedDevicesMacMatcher {
    /// Builds a RADIUS identity matcher from shaped-device rows.
    ///
    /// Empty or invalid MAC values are ignored for MAC matching. Empty usernames
    /// are ignored for username matching. Duplicate identity values are retained
    /// as ambiguous matches.
    ///
    /// Side effects: none. The supplied rows are inspected and cloned in memory.
    #[must_use]
    pub fn from_devices(devices: &[ShapedDevice]) -> Self {
        let mut matches_by_mac = HashMap::new();
        let mut matches_by_username = HashMap::new();
        for device in devices {
            if let Some(normalized_mac) = normalize_radius_mac(&device.mac) {
                matches_by_mac
                    .entry(normalized_mac)
                    .and_modify(|entry| *entry = ShapedDevicesMacEntry::Ambiguous)
                    .or_insert_with(|| ShapedDevicesMacEntry::Unique(Box::new(device.clone())));
            }
            let username = device.radius_username.trim();
            if !username.is_empty() {
                matches_by_username
                    .entry(username.to_string())
                    .and_modify(|entry| *entry = ShapedDevicesMacEntry::Ambiguous)
                    .or_insert_with(|| ShapedDevicesMacEntry::Unique(Box::new(device.clone())));
            }
        }

        Self {
            matches_by_mac,
            matches_by_username,
        }
    }

    /// Matches one accounting event by `Calling-Station-Id`.
    ///
    /// Side effects: none. The event and in-memory matcher are inspected only.
    #[must_use]
    pub fn match_event(&self, event: &AccountingEvent) -> ShapedDevicesMacMatch {
        self.match_event_with_identities(event, false, true)
    }

    /// Matches one accounting event by configured RADIUS username and MAC identities.
    ///
    /// A unique username match takes precedence; MAC matching is attempted only when no
    /// username row matches. Side effects: none.
    #[must_use]
    pub fn match_event_with_identities(
        &self,
        event: &AccountingEvent,
        match_by_username: bool,
        match_by_mac: bool,
    ) -> ShapedDevicesMacMatch {
        if match_by_username
            && let Some(username) = event
                .user_name
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            && let Some(result) = Self::match_entry(&self.matches_by_username, username)
        {
            return result;
        }
        if !match_by_mac {
            return ShapedDevicesMacMatch::NoMatch;
        }
        let Some(calling_station_id) = event.calling_station_id.as_deref() else {
            return ShapedDevicesMacMatch::NoMatch;
        };
        let Some(normalized_mac) = normalize_radius_mac(calling_station_id) else {
            return ShapedDevicesMacMatch::NoMatch;
        };

        Self::match_entry(&self.matches_by_mac, &normalized_mac)
            .unwrap_or(ShapedDevicesMacMatch::NoMatch)
    }

    fn match_entry(
        entries: &HashMap<String, ShapedDevicesMacEntry>,
        key: &str,
    ) -> Option<ShapedDevicesMacMatch> {
        match entries.get(key) {
            Some(ShapedDevicesMacEntry::Unique(device)) => {
                Some(ShapedDevicesMacMatch::Unique(device.clone()))
            }
            Some(ShapedDevicesMacEntry::Ambiguous) => Some(ShapedDevicesMacMatch::Ambiguous),
            None => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum ShapedDevicesMacEntry {
    Unique(Box<ShapedDevice>),
    Ambiguous,
}

/// Result of matching one RADIUS identity to shaped-device rows.
#[derive(Clone, Debug, PartialEq)]
pub enum ShapedDevicesMacMatch {
    /// Exactly one shaped-device row matched the configured identity.
    Unique(Box<ShapedDevice>),
    /// No shaped-device row matched the configured identity.
    NoMatch,
    /// More than one shaped-device row matched the configured identity.
    Ambiguous,
}

/// Normalizes common RADIUS and `ShapedDevices.csv` MAC address formats.
///
/// Accepted inputs include colon-separated, hyphen-separated, dotted,
/// plain-hex, and mixed-case forms. The returned value is twelve lowercase
/// hexadecimal characters.
#[must_use]
pub fn normalize_radius_mac(raw_mac: &str) -> Option<String> {
    let raw_mac = raw_mac.trim();
    if raw_mac.len() == 12 && raw_mac.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Some(raw_mac.to_ascii_lowercase());
    }

    if raw_mac.contains(':') {
        return normalize_delimited_mac(raw_mac, ':', 6, 2);
    }
    if raw_mac.contains('-') {
        return normalize_delimited_mac(raw_mac, '-', 6, 2);
    }
    if raw_mac.contains('.') {
        return normalize_delimited_mac(raw_mac, '.', 3, 4);
    }

    None
}

fn normalize_delimited_mac(
    raw_mac: &str,
    separator: char,
    expected_groups: usize,
    expected_group_len: usize,
) -> Option<String> {
    let mut normalized_mac = String::with_capacity(12);
    let mut group_count = 0;
    for group in raw_mac.split(separator) {
        group_count += 1;
        if group.len() != expected_group_len {
            return None;
        }
        for ch in group.chars() {
            if !ch.is_ascii_hexdigit() {
                return None;
            }
            normalized_mac.push(ch.to_ascii_lowercase());
        }
    }

    (group_count == expected_groups).then_some(normalized_mac)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_common_mac_formats() {
        for raw_mac in [
            "AA:BB:CC:DD:EE:FF",
            "aa-bb-cc-dd-ee-ff",
            "AABB.CCDD.EEFF",
            "aabbccddeeff",
            " AaBbCcDdEeFf ",
        ] {
            assert_eq!(
                normalize_radius_mac(raw_mac).as_deref(),
                Some("aabbccddeeff")
            );
        }

        assert_eq!(normalize_radius_mac("aa:bb:cc"), None);
        assert_eq!(normalize_radius_mac("aa:bbccddeeff"), None);
        assert_eq!(normalize_radius_mac("aa-bbccddeeff"), None);
        assert_eq!(normalize_radius_mac("aabb.ccddeeff"), None);
        assert_eq!(normalize_radius_mac("aa-bb.cc:ddEeFf"), None);
        assert_eq!(normalize_radius_mac("aa:bb:cc:dd:ee:zz"), None);
        assert_eq!(normalize_radius_mac("aa bb cc dd ee ff"), None);
    }

    #[test]
    fn matcher_ignores_empty_and_invalid_shaped_device_mac_rows() {
        let matcher = ShapedDevicesMacMatcher::from_devices(&[
            shaped_device("empty-mac", ""),
            shaped_device("invalid-mac", "aa:bbccddeeff"),
        ]);
        let mut event = AccountingEvent {
            calling_station_id: Some("aa:bb:cc:dd:ee:ff".to_string()),
            ..AccountingEvent::default()
        };

        assert_eq!(matcher.match_event(&event), ShapedDevicesMacMatch::NoMatch);

        let matcher = ShapedDevicesMacMatcher::from_devices(&[
            shaped_device("valid-mac", "aa-bb-cc-dd-ee-ff"),
            shaped_device("ignored-invalid-mac", "aa:bbccddeeff"),
        ]);
        let ShapedDevicesMacMatch::Unique(device) = matcher.match_event(&event) else {
            panic!("valid shaped-device MAC should match uniquely");
        };
        assert_eq!(device.circuit_id, "valid-mac");

        event.calling_station_id = Some("aa:bbccddeeff".to_string());
        assert_eq!(matcher.match_event(&event), ShapedDevicesMacMatch::NoMatch);
    }

    #[test]
    fn matcher_marks_duplicate_normalized_macs_ambiguous() {
        let matcher = ShapedDevicesMacMatcher::from_devices(&[
            shaped_device("first-circuit", "aa-bb-cc-dd-ee-ff"),
            shaped_device("second-circuit", "AABB.CCDD.EEFF"),
        ]);
        let event = AccountingEvent {
            calling_station_id: Some("AA:BB:CC:DD:EE:FF".to_string()),
            ..AccountingEvent::default()
        };

        assert_eq!(
            matcher.match_event(&event),
            ShapedDevicesMacMatch::Ambiguous
        );
    }

    #[test]
    fn username_match_precedes_mac_and_accepts_a_row_without_a_mac() {
        let mut username_device = shaped_device("username-circuit", "");
        username_device.radius_username = "pppoe-known".to_string();
        let mut mac_device = shaped_device("mac-circuit", "aa:bb:cc:dd:ee:ff");
        mac_device.radius_username = "another-user".to_string();
        let matcher = ShapedDevicesMacMatcher::from_devices(&[username_device, mac_device]);
        let event = AccountingEvent {
            user_name: Some(" pppoe-known ".to_string()),
            calling_station_id: Some("aa:bb:cc:dd:ee:ff".to_string()),
            ..AccountingEvent::default()
        };

        let ShapedDevicesMacMatch::Unique(device) =
            matcher.match_event_with_identities(&event, true, true)
        else {
            panic!("username should match uniquely before Calling-Station-Id");
        };
        assert_eq!(device.circuit_id, "username-circuit");
    }

    fn shaped_device(circuit_id: &str, mac: &str) -> ShapedDevice {
        ShapedDevice {
            circuit_id: circuit_id.to_string(),
            mac: mac.to_string(),
            ..ShapedDevice::default()
        }
    }
}
