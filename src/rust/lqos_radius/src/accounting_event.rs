//! Typed RADIUS Accounting-Request event extraction.

use crate::attribute_type::{ACCT_STATUS_TYPE, VENDOR_SPECIFIC};
use crate::packet::split_radius_tlv;
use crate::{AccountingRequest, RadiusAttribute, VerifiedAccountingRequest};
use std::net::{Ipv4Addr, Ipv6Addr};

const USER_NAME: u8 = 1;
const NAS_IP_ADDRESS: u8 = 4;
const NAS_PORT: u8 = 5;
const FRAMED_IP_ADDRESS: u8 = 8;
const FRAMED_IP_NETMASK: u8 = 9;
const FRAMED_ROUTE: u8 = 22;
const CLASS: u8 = 25;
const CALLED_STATION_ID: u8 = 30;
const CALLING_STATION_ID: u8 = 31;
const NAS_IDENTIFIER: u8 = 32;
const ACCT_SESSION_ID: u8 = 44;
const EVENT_TIMESTAMP: u8 = 55;
const NAS_PORT_ID: u8 = 87;
const NAS_IPV6_ADDRESS: u8 = 95;
const FRAMED_IPV6_PREFIX: u8 = 97;
const DELEGATED_IPV6_PREFIX: u8 = 123;
const FRAMED_IPV6_ADDRESS: u8 = 168;

const MIKROTIK_VENDOR_ID: u32 = 14988;
const MIKROTIK_RATE_LIMIT: u8 = 8;

/// Common RADIUS accounting attributes extracted into typed fields.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AccountingEvent {
    /// Acct-Status-Type when present.
    pub status_type: Option<AcctStatusType>,
    /// Acct-Session-Id when present.
    pub acct_session_id: Option<String>,
    /// NAS-IP-Address when present.
    pub nas_ip_address: Option<Ipv4Addr>,
    /// NAS-IPv6-Address when present.
    pub nas_ipv6_address: Option<Ipv6Addr>,
    /// NAS-Identifier when present.
    pub nas_identifier: Option<String>,
    /// NAS-Port when present.
    pub nas_port: Option<u32>,
    /// NAS-Port-Id when present.
    pub nas_port_id: Option<String>,
    /// User-Name when present.
    pub user_name: Option<String>,
    /// Calling-Station-Id when present.
    pub calling_station_id: Option<String>,
    /// Called-Station-Id when present.
    pub called_station_id: Option<String>,
    /// Class attributes when present. RADIUS permits more than one Class value.
    pub class: Vec<Vec<u8>>,
    /// Event-Timestamp as Unix seconds when present.
    pub event_timestamp: Option<u32>,
    /// Framed-IP-Address when present.
    pub framed_ip_address: Option<Ipv4Addr>,
    /// Framed-IP-Netmask when present.
    pub framed_ip_netmask: Option<Ipv4Addr>,
    /// Framed-Route values when present.
    pub framed_routes: Vec<String>,
    /// Framed-IPv6-Prefix values when present.
    pub framed_ipv6_prefixes: Vec<Ipv6Prefix>,
    /// Delegated-IPv6-Prefix values when present.
    pub delegated_ipv6_prefixes: Vec<Ipv6Prefix>,
    /// Framed-IPv6-Address when present.
    pub framed_ipv6_address: Option<Ipv6Addr>,
    /// Standard attributes this extractor does not expose as typed fields or could not decode.
    pub unknown_standard_attributes: Vec<RadiusAttribute>,
    /// Vendor-Specific attributes this extractor does not understand or cannot decode.
    pub unknown_vendor_attributes: Vec<UnknownVendorAttribute>,
    /// Decoded MikroTik-Rate-Limit attributes when present.
    pub mikrotik_rate_limits: Vec<MikrotikRateLimit>,
}

impl AccountingEvent {
    /// Extracts a typed accounting event from a parsed Accounting-Request.
    ///
    /// Side effects: none. The packet is inspected in memory only.
    #[must_use]
    pub fn from_request(request: &AccountingRequest) -> Self {
        Self::from_request_with_options(request, AccountingEventOptions::default())
    }

    /// Extracts a typed accounting event from a parsed Accounting-Request.
    ///
    /// Side effects: none. The packet is inspected in memory only.
    #[must_use]
    pub fn from_request_with_options(
        request: &AccountingRequest,
        options: AccountingEventOptions,
    ) -> Self {
        let mut event = Self::default();

        for attribute in request.packet().attributes() {
            event.apply_attribute(attribute, options);
        }

        event
    }

    /// Extracts a typed accounting event from a verified Accounting-Request.
    ///
    /// Side effects: none. The packet is inspected in memory only.
    #[must_use]
    pub fn from_verified(request: &VerifiedAccountingRequest) -> Self {
        Self::from_verified_with_options(request, AccountingEventOptions::default())
    }

    /// Extracts a typed accounting event from a verified Accounting-Request.
    ///
    /// Side effects: none. The packet is inspected in memory only.
    #[must_use]
    pub fn from_verified_with_options(
        request: &VerifiedAccountingRequest,
        options: AccountingEventOptions,
    ) -> Self {
        Self::from_request_with_options(request.request(), options)
    }

    fn apply_attribute(&mut self, attribute: &RadiusAttribute, options: AccountingEventOptions) {
        match attribute.kind() {
            USER_NAME => set_once(&mut self.user_name, Some(text(attribute.value()))),
            NAS_IP_ADDRESS => set_optional(
                &mut self.unknown_standard_attributes,
                &mut self.nas_ip_address,
                attribute,
                ipv4,
            ),
            NAS_PORT => set_optional(
                &mut self.unknown_standard_attributes,
                &mut self.nas_port,
                attribute,
                u32_value,
            ),
            FRAMED_IP_ADDRESS => set_optional(
                &mut self.unknown_standard_attributes,
                &mut self.framed_ip_address,
                attribute,
                ipv4,
            ),
            FRAMED_IP_NETMASK => set_optional(
                &mut self.unknown_standard_attributes,
                &mut self.framed_ip_netmask,
                attribute,
                ipv4,
            ),
            FRAMED_ROUTE => self.framed_routes.push(text(attribute.value())),
            CLASS => self.class.push(attribute.value().to_vec()),
            VENDOR_SPECIFIC => self.apply_vendor_specific(attribute.value(), options),
            CALLED_STATION_ID => {
                set_once(&mut self.called_station_id, Some(text(attribute.value())))
            }
            CALLING_STATION_ID => {
                set_once(&mut self.calling_station_id, Some(text(attribute.value())));
            }
            NAS_IDENTIFIER => set_once(&mut self.nas_identifier, Some(text(attribute.value()))),
            ACCT_STATUS_TYPE => set_optional(
                &mut self.unknown_standard_attributes,
                &mut self.status_type,
                attribute,
                |value| u32_value(value).map(AcctStatusType::from),
            ),
            ACCT_SESSION_ID => set_once(&mut self.acct_session_id, Some(text(attribute.value()))),
            EVENT_TIMESTAMP => set_optional(
                &mut self.unknown_standard_attributes,
                &mut self.event_timestamp,
                attribute,
                u32_value,
            ),
            NAS_PORT_ID => set_once(&mut self.nas_port_id, Some(text(attribute.value()))),
            NAS_IPV6_ADDRESS => set_optional(
                &mut self.unknown_standard_attributes,
                &mut self.nas_ipv6_address,
                attribute,
                ipv6,
            ),
            FRAMED_IPV6_PREFIX => push_optional(
                &mut self.unknown_standard_attributes,
                &mut self.framed_ipv6_prefixes,
                attribute,
                ipv6_prefix,
            ),
            DELEGATED_IPV6_PREFIX => push_optional(
                &mut self.unknown_standard_attributes,
                &mut self.delegated_ipv6_prefixes,
                attribute,
                ipv6_prefix,
            ),
            FRAMED_IPV6_ADDRESS => set_optional(
                &mut self.unknown_standard_attributes,
                &mut self.framed_ipv6_address,
                attribute,
                ipv6,
            ),
            _ => self.unknown_standard_attributes.push(attribute.clone()),
        }
    }

    fn apply_vendor_specific(&mut self, value: &[u8], options: AccountingEventOptions) {
        let Some((vendor_id, mut remaining)) = vendor_id_and_payload(value) else {
            self.unknown_vendor_attributes
                .push(UnknownVendorAttribute::malformed(value));
            return;
        };

        if remaining.is_empty() {
            self.unknown_vendor_attributes
                .push(UnknownVendorAttribute::vendor_only(vendor_id));
            return;
        }

        if vendor_id != MIKROTIK_VENDOR_ID {
            self.unknown_vendor_attributes
                .push(UnknownVendorAttribute::raw_vendor_payload(
                    vendor_id, remaining,
                ));
            return;
        }

        while !remaining.is_empty() {
            let Ok((vendor_type, vendor_value, next)) = split_radius_tlv(remaining) else {
                self.unknown_vendor_attributes
                    .push(UnknownVendorAttribute::vendor_data(vendor_id, remaining));
                return;
            };

            if vendor_type == MIKROTIK_RATE_LIMIT
                && let Some(rate_limit) = MikrotikRateLimit::parse(vendor_value, options)
            {
                self.mikrotik_rate_limits.push(rate_limit);
                remaining = next;
                continue;
            }

            self.unknown_vendor_attributes
                .push(UnknownVendorAttribute::subattribute(
                    vendor_id,
                    vendor_type,
                    vendor_value,
                ));

            remaining = next;
        }
    }
}

impl From<&AccountingRequest> for AccountingEvent {
    fn from(request: &AccountingRequest) -> Self {
        Self::from_request(request)
    }
}

impl From<&VerifiedAccountingRequest> for AccountingEvent {
    fn from(request: &VerifiedAccountingRequest) -> Self {
        Self::from_verified(request)
    }
}

/// Options that control typed accounting event extraction.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AccountingEventOptions {
    /// Mapping between MikroTik NAS RX/TX rates and LibreQoS upload/download rates.
    pub mikrotik_rate_limit_direction: MikrotikRateLimitDirection,
}

/// Acct-Status-Type values understood by LibreQoS.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AcctStatusType {
    /// Start, value 1.
    Start,
    /// Stop, value 2.
    Stop,
    /// Interim-Update, value 3.
    InterimUpdate,
    /// Accounting-On, value 7.
    AccountingOn,
    /// Accounting-Off, value 8.
    AccountingOff,
    /// A status value not enumerated by this crate.
    Unknown(u32),
}

impl AcctStatusType {
    /// Returns the RADIUS integer value for this Acct-Status-Type.
    #[must_use]
    pub const fn as_u32(self) -> u32 {
        match self {
            Self::Start => 1,
            Self::Stop => 2,
            Self::InterimUpdate => 3,
            Self::AccountingOn => 7,
            Self::AccountingOff => 8,
            Self::Unknown(value) => value,
        }
    }
}

impl From<u32> for AcctStatusType {
    fn from(value: u32) -> Self {
        match value {
            1 => Self::Start,
            2 => Self::Stop,
            3 => Self::InterimUpdate,
            7 => Self::AccountingOn,
            8 => Self::AccountingOff,
            other => Self::Unknown(other),
        }
    }
}

impl From<AcctStatusType> for u32 {
    fn from(status_type: AcctStatusType) -> Self {
        status_type.as_u32()
    }
}

/// IPv6 prefix decoded from RADIUS ipv6prefix attributes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ipv6Prefix {
    /// The IPv6 network address bytes supplied by the NAS.
    pub address: Ipv6Addr,
    /// Prefix length in bits.
    pub prefix_len: u8,
}

/// A Vendor-Specific attribute this extractor did not decode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnknownVendorAttribute {
    /// Vendor ID when the Vendor-Specific attribute had enough bytes to include it.
    pub vendor_id: Option<u32>,
    /// Vendor type when the vendor payload had a complete subattribute header.
    pub vendor_type: Option<u8>,
    /// Vendor attribute value, or the remaining malformed bytes when no value could be split out.
    pub value: Vec<u8>,
}

impl UnknownVendorAttribute {
    fn malformed(value: &[u8]) -> Self {
        Self {
            vendor_id: None,
            vendor_type: None,
            value: value.to_vec(),
        }
    }

    fn vendor_only(vendor_id: u32) -> Self {
        Self {
            vendor_id: Some(vendor_id),
            vendor_type: None,
            value: Vec::new(),
        }
    }

    fn vendor_data(vendor_id: u32, value: &[u8]) -> Self {
        Self {
            vendor_id: Some(vendor_id),
            vendor_type: value.first().copied(),
            value: value.to_vec(),
        }
    }

    fn raw_vendor_payload(vendor_id: u32, value: &[u8]) -> Self {
        Self {
            vendor_id: Some(vendor_id),
            vendor_type: None,
            value: value.to_vec(),
        }
    }

    fn subattribute(vendor_id: u32, vendor_type: u8, value: &[u8]) -> Self {
        Self {
            vendor_id: Some(vendor_id),
            vendor_type: Some(vendor_type),
            value: value.to_vec(),
        }
    }
}

/// Mapping between MikroTik NAS directions and LibreQoS directions.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MikrotikRateLimitDirection {
    /// Map NAS RX to LibreQoS upload and NAS TX to LibreQoS download.
    #[default]
    Normal,
    /// Map NAS RX to LibreQoS download and NAS TX to LibreQoS upload.
    Swapped,
}

impl MikrotikRateLimitDirection {
    const fn map_rates(self, nas_rx_bps: u64, nas_tx_bps: u64) -> (u64, u64) {
        match self {
            Self::Normal => (nas_rx_bps, nas_tx_bps),
            Self::Swapped => (nas_tx_bps, nas_rx_bps),
        }
    }
}

/// MikroTik-Rate-Limit values decoded from a Vendor-Specific attribute.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MikrotikRateLimit {
    /// Original MikroTik-Rate-Limit string.
    pub original: String,
    /// NAS RX rate in bits per second.
    pub nas_rx_bps: u64,
    /// NAS TX rate in bits per second.
    pub nas_tx_bps: u64,
    /// LibreQoS upload rate in bits per second after direction mapping.
    pub upload_bps: u64,
    /// LibreQoS download rate in bits per second after direction mapping.
    pub download_bps: u64,
}

impl MikrotikRateLimit {
    fn parse(value: &[u8], options: AccountingEventOptions) -> Option<Self> {
        let original = text(value);
        let rate_pair = original.split_whitespace().next()?;
        let (nas_rx_bps, nas_tx_bps) = parse_mikrotik_rate_pair(rate_pair)?;
        let (upload_bps, download_bps) = options
            .mikrotik_rate_limit_direction
            .map_rates(nas_rx_bps, nas_tx_bps);

        Some(Self {
            original,
            nas_rx_bps,
            nas_tx_bps,
            upload_bps,
            download_bps,
        })
    }
}

fn set_once<T>(slot: &mut Option<T>, value: Option<T>) {
    if slot.is_none() {
        *slot = value;
    }
}

fn set_optional<T>(
    unknown_standard_attributes: &mut Vec<RadiusAttribute>,
    slot: &mut Option<T>,
    attribute: &RadiusAttribute,
    decode: impl FnOnce(&[u8]) -> Option<T>,
) {
    let decoded = decode(attribute.value());
    if decoded.is_none() {
        unknown_standard_attributes.push(attribute.clone());
    }
    set_once(slot, decoded);
}

fn push_optional<T>(
    unknown_standard_attributes: &mut Vec<RadiusAttribute>,
    items: &mut Vec<T>,
    attribute: &RadiusAttribute,
    decode: impl FnOnce(&[u8]) -> Option<T>,
) {
    let Some(decoded) = decode(attribute.value()) else {
        unknown_standard_attributes.push(attribute.clone());
        return;
    };
    items.push(decoded);
}

fn text(value: &[u8]) -> String {
    String::from_utf8_lossy(value).into_owned()
}

fn ipv4(value: &[u8]) -> Option<Ipv4Addr> {
    let bytes: [u8; 4] = value.try_into().ok()?;
    Some(Ipv4Addr::from(bytes))
}

fn ipv6(value: &[u8]) -> Option<Ipv6Addr> {
    let bytes: [u8; 16] = value.try_into().ok()?;
    Some(Ipv6Addr::from(bytes))
}

fn ipv6_prefix(value: &[u8]) -> Option<Ipv6Prefix> {
    if !(2..=18).contains(&value.len()) {
        return None;
    }
    if value[0] != 0 {
        return None;
    }
    let prefix_len = value[1];
    if prefix_len > 128 {
        return None;
    }
    let expected_prefix_bytes = usize::from(prefix_len).div_ceil(8);
    let prefix_bytes = &value[2..];
    if prefix_bytes.len() < expected_prefix_bytes {
        return None;
    }
    if prefix_bytes[expected_prefix_bytes..]
        .iter()
        .any(|byte| *byte != 0)
    {
        return None;
    }
    let mut bytes = [0_u8; 16];
    bytes[..expected_prefix_bytes].copy_from_slice(&prefix_bytes[..expected_prefix_bytes]);
    if let Some(mask) = final_prefix_byte_mask(prefix_len) {
        if bytes[expected_prefix_bytes - 1] & !mask != 0 {
            return None;
        }
        bytes[expected_prefix_bytes - 1] &= mask;
    }
    Some(Ipv6Prefix {
        address: Ipv6Addr::from(bytes),
        prefix_len,
    })
}

fn final_prefix_byte_mask(prefix_len: u8) -> Option<u8> {
    let remainder = prefix_len % 8;
    if remainder == 0 {
        None
    } else {
        Some(u8::MAX << (8 - remainder))
    }
}

fn u32_value(value: &[u8]) -> Option<u32> {
    let bytes: [u8; 4] = value.try_into().ok()?;
    Some(u32::from_be_bytes(bytes))
}

fn vendor_id_and_payload(value: &[u8]) -> Option<(u32, &[u8])> {
    let vendor_id_bytes: [u8; 4] = value.get(..4)?.try_into().ok()?;
    Some((u32::from_be_bytes(vendor_id_bytes), &value[4..]))
}

fn parse_mikrotik_rate_pair(value: &str) -> Option<(u64, u64)> {
    let (nas_rx, nas_tx) = match value.split_once('/') {
        Some(pair) => pair,
        None => (value, value),
    };
    Some((parse_mikrotik_rate(nas_rx)?, parse_mikrotik_rate(nas_tx)?))
}

fn parse_mikrotik_rate(value: &str) -> Option<u64> {
    let trimmed = value.trim();
    let digit_count = trimmed
        .as_bytes()
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if digit_count == 0 {
        return None;
    }

    let number = trimmed[..digit_count].parse::<u64>().ok()?;
    let multiplier = match trimmed[digit_count..].to_ascii_lowercase().as_str() {
        "" | "bps" => 1,
        "k" | "kb" | "kbit" | "kbps" => 1_000,
        "m" | "mb" | "mbit" | "mbps" => 1_000_000,
        "g" | "gb" | "gbit" | "gbps" => 1_000_000_000,
        _ => return None,
    };

    number.checked_mul(multiplier)
}

#[cfg(test)]
mod tests;
