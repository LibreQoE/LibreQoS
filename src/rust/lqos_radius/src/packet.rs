//! RADIUS packet framing, Accounting-Request verification, and response building.

use hmac::{Hmac, Mac};
use md5::{Digest, Md5};
use subtle::ConstantTimeEq;
use thiserror::Error;

type HmacMd5 = Hmac<Md5>;

pub(crate) const RADIUS_HEADER_LEN: usize = 20;
pub(crate) const RADIUS_MAX_PACKET_LEN: usize = 4096;
// RFC 2866 section 3 caps RADIUS Accounting packet Length at 4095 bytes.
pub(crate) const RADIUS_MAX_ACCOUNTING_PACKET_LEN: usize = 4095;
pub(crate) const RADIUS_AUTHENTICATOR_LEN: usize = 16;
pub(crate) const RADIUS_ATTRIBUTE_HEADER_LEN: usize = 2;
const RADIUS_MIN_ATTRIBUTE_LEN: usize = 2;
const ACCOUNTING_REQUEST_CODE: u8 = 4;
const ACCOUNTING_RESPONSE_CODE: u8 = 5;
const PROXY_STATE_TYPE: u8 = 33;
pub(crate) const MESSAGE_AUTHENTICATOR_TYPE: u8 = 80;
pub(crate) const MESSAGE_AUTHENTICATOR_VALUE_LEN: usize = 16;
#[cfg(test)]
pub(crate) const MESSAGE_AUTHENTICATOR_ATTRIBUTE_LEN: usize =
    RADIUS_ATTRIBUTE_HEADER_LEN + MESSAGE_AUTHENTICATOR_VALUE_LEN;

/// RADIUS packet code values understood by the packet parser.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RadiusCode {
    /// Access-Request packet code.
    AccessRequest,
    /// Access-Accept packet code.
    AccessAccept,
    /// Access-Reject packet code.
    AccessReject,
    /// Accounting-Request packet code.
    AccountingRequest,
    /// Accounting-Response packet code.
    AccountingResponse,
    /// Any RADIUS packet code not enumerated by this crate.
    Other(u8),
}

impl RadiusCode {
    /// Returns the wire value for this packet code.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::AccessRequest => 1,
            Self::AccessAccept => 2,
            Self::AccessReject => 3,
            Self::AccountingRequest => ACCOUNTING_REQUEST_CODE,
            Self::AccountingResponse => ACCOUNTING_RESPONSE_CODE,
            Self::Other(code) => code,
        }
    }
}

impl From<u8> for RadiusCode {
    fn from(code: u8) -> Self {
        match code {
            1 => Self::AccessRequest,
            2 => Self::AccessAccept,
            3 => Self::AccessReject,
            ACCOUNTING_REQUEST_CODE => Self::AccountingRequest,
            ACCOUNTING_RESPONSE_CODE => Self::AccountingResponse,
            other => Self::Other(other),
        }
    }
}

impl From<RadiusCode> for u8 {
    fn from(code: RadiusCode) -> Self {
        code.as_u8()
    }
}

/// One decoded RADIUS attribute.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RadiusAttribute {
    kind: u8,
    value: Vec<u8>,
}

impl RadiusAttribute {
    /// Returns the raw RADIUS attribute type value.
    #[must_use]
    pub const fn kind(&self) -> u8 {
        self.kind
    }

    /// Returns the raw RADIUS attribute value bytes.
    #[must_use]
    pub fn value(&self) -> &[u8] {
        &self.value
    }
}

/// A well-formed RADIUS packet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RadiusPacket {
    code: RadiusCode,
    identifier: u8,
    authenticator: [u8; RADIUS_AUTHENTICATOR_LEN],
    attributes: Vec<RadiusAttribute>,
}

impl RadiusPacket {
    /// Returns the packet code.
    #[must_use]
    pub const fn code(&self) -> RadiusCode {
        self.code
    }

    /// Returns the packet identifier.
    #[must_use]
    pub const fn identifier(&self) -> u8 {
        self.identifier
    }

    /// Returns the 16-byte request authenticator from the packet header.
    #[must_use]
    pub const fn authenticator(&self) -> &[u8; RADIUS_AUTHENTICATOR_LEN] {
        &self.authenticator
    }

    /// Returns the decoded packet attributes.
    #[must_use]
    pub fn attributes(&self) -> &[RadiusAttribute] {
        &self.attributes
    }
}

/// A well-formed RADIUS Accounting-Request packet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountingRequest {
    packet: RadiusPacket,
}

impl AccountingRequest {
    /// Returns the decoded RADIUS packet.
    #[must_use]
    pub const fn packet(&self) -> &RadiusPacket {
        &self.packet
    }

    /// Consumes the accounting request and returns the decoded RADIUS packet.
    #[must_use]
    pub fn into_packet(self) -> RadiusPacket {
        self.packet
    }
}

/// Policy for accepting Accounting-Request packets without Message-Authenticator.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MessageAuthenticatorPolicy {
    /// Verify Message-Authenticator when present, but do not require it.
    #[default]
    Optional,
    /// Reject Accounting-Request packets that lack Message-Authenticator.
    Required,
}

impl MessageAuthenticatorPolicy {
    /// Returns true when the policy requires Message-Authenticator attribute 80.
    #[must_use]
    pub const fn requires_message_authenticator(self) -> bool {
        matches!(self, Self::Required)
    }
}

/// A RADIUS Accounting-Request whose authenticators matched the shared secret.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedAccountingRequest {
    request: AccountingRequest,
    has_message_authenticator: bool,
}

impl VerifiedAccountingRequest {
    /// Returns the verified Accounting-Request wrapper.
    #[must_use]
    pub const fn request(&self) -> &AccountingRequest {
        &self.request
    }

    /// Returns the verified decoded RADIUS packet.
    #[must_use]
    pub const fn packet(&self) -> &RadiusPacket {
        self.request.packet()
    }

    /// Returns true when the accepted packet contained Message-Authenticator.
    #[must_use]
    pub const fn has_message_authenticator(&self) -> bool {
        self.has_message_authenticator
    }

    /// Consumes the verified request and returns the Accounting-Request wrapper.
    #[must_use]
    pub fn into_request(self) -> AccountingRequest {
        self.request
    }

    /// Builds an Accounting-Response packet for this verified request.
    ///
    /// Side effects: none. The returned bytes are not sent to the network.
    #[must_use]
    pub fn build_response(&self, shared_secret: &[u8]) -> Vec<u8> {
        build_accounting_response(self, shared_secret)
    }
}

/// Errors returned while parsing RADIUS packets.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum PacketError {
    /// The datagram was shorter than the fixed RADIUS header.
    #[error("RADIUS packet is {actual} bytes; minimum is 20 bytes")]
    PacketTooShort {
        /// Number of bytes received in the datagram.
        actual: usize,
    },
    /// The packet length field was outside the RADIUS framing bounds.
    #[error("RADIUS packet length field is {declared} bytes; valid range is 20..=4096 bytes")]
    InvalidPacketLength {
        /// Length declared in the packet header.
        declared: usize,
    },
    /// The packet length field exceeded the RADIUS Accounting framing limit.
    #[error(
        "RADIUS accounting packet length field is {declared} bytes; valid range is 20..=4095 bytes"
    )]
    InvalidAccountingPacketLength {
        /// Length declared in the packet header.
        declared: usize,
    },
    /// The datagram ended before the declared RADIUS packet length.
    #[error(
        "RADIUS packet length field is {declared} bytes, but datagram contains only {actual} bytes"
    )]
    PacketTruncated {
        /// Length declared in the packet header.
        declared: usize,
        /// Number of bytes received in the datagram.
        actual: usize,
    },
    /// An attribute was missing its type and length bytes.
    #[error("RADIUS attribute at offset {offset} is missing its type/length header")]
    AttributeHeaderTruncated {
        /// Byte offset of the malformed attribute.
        offset: usize,
    },
    /// An attribute declared a length smaller than its own header.
    #[error(
        "RADIUS attribute at offset {offset} declares invalid length {declared}; minimum is 2 bytes"
    )]
    InvalidAttributeLength {
        /// Byte offset of the malformed attribute.
        offset: usize,
        /// Length declared by the attribute.
        declared: usize,
    },
    /// An attribute declared more bytes than remain in the packet.
    #[error(
        "RADIUS attribute at offset {offset} declares length {declared}, but only {remaining} bytes remain"
    )]
    AttributeTruncated {
        /// Byte offset of the malformed attribute.
        offset: usize,
        /// Length declared by the attribute.
        declared: usize,
        /// Remaining bytes in the packet from the attribute offset.
        remaining: usize,
    },
    /// The packet code is not supported by the accounting handler.
    #[error("RADIUS code {code} is not supported by the accounting handler")]
    UnsupportedCode {
        /// Unsupported raw packet code.
        code: u8,
    },
    /// The Accounting-Request authenticator did not match the shared secret.
    #[error("RADIUS Accounting-Request authenticator does not match the shared secret")]
    InvalidRequestAuthenticator,
    /// Message-Authenticator is required by policy but absent from the packet.
    #[error("RADIUS Message-Authenticator is required but missing")]
    MissingMessageAuthenticator,
    /// Message-Authenticator had an invalid value length.
    #[error("RADIUS Message-Authenticator value is {actual} bytes; expected 16 bytes")]
    InvalidMessageAuthenticatorLength {
        /// Actual Message-Authenticator value length.
        actual: usize,
    },
    /// More than one Message-Authenticator attribute was present.
    #[error("RADIUS packet contains more than one Message-Authenticator attribute")]
    MultipleMessageAuthenticators,
    /// The Message-Authenticator HMAC did not match the shared secret.
    #[error("RADIUS Message-Authenticator does not match the shared secret")]
    InvalidMessageAuthenticator,
}

/// Parses the RADIUS packet framing in one UDP datagram.
///
/// Extra bytes beyond the declared RADIUS length are ignored as UDP padding.
/// This function performs no shared-secret verification and does not mutate
/// process or system state.
pub fn parse_packet(datagram: &[u8]) -> Result<RadiusPacket, PacketError> {
    if datagram.len() < RADIUS_HEADER_LEN {
        return Err(PacketError::PacketTooShort {
            actual: datagram.len(),
        });
    }

    let declared_len = u16::from_be_bytes([datagram[2], datagram[3]]) as usize;
    if !(RADIUS_HEADER_LEN..=RADIUS_MAX_PACKET_LEN).contains(&declared_len) {
        return Err(PacketError::InvalidPacketLength {
            declared: declared_len,
        });
    }
    if datagram.len() < declared_len {
        return Err(PacketError::PacketTruncated {
            declared: declared_len,
            actual: datagram.len(),
        });
    }

    let packet_bytes = &datagram[..declared_len];
    let mut authenticator = [0_u8; RADIUS_AUTHENTICATOR_LEN];
    authenticator.copy_from_slice(&packet_bytes[4..RADIUS_HEADER_LEN]);
    let attributes = parse_attributes(&packet_bytes[RADIUS_HEADER_LEN..])?;

    Ok(RadiusPacket {
        code: RadiusCode::from(packet_bytes[0]),
        identifier: packet_bytes[1],
        authenticator,
        attributes,
    })
}

/// Parses a UDP datagram and returns only RADIUS Accounting-Request packets.
///
/// This function performs syntactic packet checks only. It does not verify
/// authenticators, build Accounting-Response packets, or update session state.
pub fn handle_accounting_request(datagram: &[u8]) -> Result<AccountingRequest, PacketError> {
    let packet = parse_packet(datagram)?;
    if packet.code() != RadiusCode::AccountingRequest {
        return Err(PacketError::UnsupportedCode {
            code: packet.code().as_u8(),
        });
    }
    let declared_len = u16::from_be_bytes([datagram[2], datagram[3]]) as usize;
    if declared_len > RADIUS_MAX_ACCOUNTING_PACKET_LEN {
        return Err(PacketError::InvalidAccountingPacketLength {
            declared: declared_len,
        });
    }

    Ok(AccountingRequest { packet })
}

/// Parses and verifies one RADIUS Accounting-Request datagram.
///
/// The caller is responsible for matching the UDP peer to a trusted client and
/// passing that client's shared secret. Packet framing and attribute bounds are
/// checked before authenticators are evaluated.
/// When Message-Authenticator is present, it is verified with the
/// Accounting-Request vector set to zero before the request authenticator is
/// verified with the received Message-Authenticator value present.
///
/// Side effects: none. This function does not send responses, update session
/// state, touch files, or modify host networking.
pub fn verify_accounting_request(
    datagram: &[u8],
    shared_secret: &[u8],
    message_authenticator_policy: MessageAuthenticatorPolicy,
) -> Result<VerifiedAccountingRequest, PacketError> {
    let request = handle_accounting_request(datagram)?;
    let message_authenticator_index = message_authenticator_index(request.packet())?;

    if message_authenticator_policy.requires_message_authenticator()
        && message_authenticator_index.is_none()
    {
        return Err(PacketError::MissingMessageAuthenticator);
    }

    if let Some(index) = message_authenticator_index {
        verify_message_authenticator(request.packet(), shared_secret, index)?;
    }
    verify_request_authenticator(request.packet(), shared_secret)?;

    Ok(VerifiedAccountingRequest {
        request,
        has_message_authenticator: message_authenticator_index.is_some(),
    })
}

/// Builds a minimal RADIUS Accounting-Response for a verified request.
///
/// The response uses code 5, copies the request identifier, preserves any
/// Proxy-State attributes from the request, and signs the response
/// authenticator with the shared secret.
///
/// Side effects: none. The returned bytes are not sent to the network.
#[must_use]
pub fn build_accounting_response(
    request: &VerifiedAccountingRequest,
    shared_secret: &[u8],
) -> Vec<u8> {
    let mut response = Vec::with_capacity(RADIUS_HEADER_LEN + shared_secret.len());
    response.push(ACCOUNTING_RESPONSE_CODE);
    response.push(request.packet().identifier());
    response.extend_from_slice(&[0, 0]);
    response.extend_from_slice(request.packet().authenticator());
    encode_proxy_state_attributes(request.packet(), &mut response);
    let response_len = response.len();
    response[2..4].copy_from_slice(&(response_len as u16).to_be_bytes());
    response.extend_from_slice(shared_secret);
    let authenticator = md5_digest(&response);

    response.truncate(response_len);
    response[4..RADIUS_HEADER_LEN].copy_from_slice(&authenticator);
    response
}

fn parse_attributes(mut attribute_bytes: &[u8]) -> Result<Vec<RadiusAttribute>, PacketError> {
    let mut attributes = Vec::new();
    let mut offset = RADIUS_HEADER_LEN;

    while !attribute_bytes.is_empty() {
        let (kind, value, remaining) = split_radius_tlv(attribute_bytes)
            .map_err(|error| packet_error_from_tlv_split(error, offset, attribute_bytes.len()))?;
        let declared_len = attribute_bytes.len() - remaining.len();

        attributes.push(RadiusAttribute {
            kind,
            value: value.to_vec(),
        });
        attribute_bytes = remaining;
        offset += declared_len;
    }

    Ok(attributes)
}

pub(crate) fn split_radius_tlv(value: &[u8]) -> Result<(u8, &[u8], &[u8]), RadiusTlvError> {
    if value.len() < RADIUS_ATTRIBUTE_HEADER_LEN {
        return Err(RadiusTlvError::HeaderTruncated);
    }

    let kind = value[0];
    let declared_len = usize::from(value[1]);
    if declared_len < RADIUS_MIN_ATTRIBUTE_LEN {
        return Err(RadiusTlvError::InvalidLength { declared_len });
    }
    if declared_len > value.len() {
        return Err(RadiusTlvError::Truncated { declared_len });
    }

    Ok((
        kind,
        &value[RADIUS_ATTRIBUTE_HEADER_LEN..declared_len],
        &value[declared_len..],
    ))
}

pub(crate) enum RadiusTlvError {
    HeaderTruncated,
    InvalidLength { declared_len: usize },
    Truncated { declared_len: usize },
}

fn packet_error_from_tlv_split(
    error: RadiusTlvError,
    offset: usize,
    remaining: usize,
) -> PacketError {
    match error {
        RadiusTlvError::HeaderTruncated => PacketError::AttributeHeaderTruncated { offset },
        RadiusTlvError::InvalidLength { declared_len } => PacketError::InvalidAttributeLength {
            offset,
            declared: declared_len,
        },
        RadiusTlvError::Truncated { declared_len } => PacketError::AttributeTruncated {
            offset,
            declared: declared_len,
            remaining,
        },
    }
}

fn verify_request_authenticator(
    packet: &RadiusPacket,
    shared_secret: &[u8],
) -> Result<(), PacketError> {
    let expected = expected_request_authenticator(packet, shared_secret);

    if constant_time_eq(packet.authenticator(), &expected) {
        Ok(())
    } else {
        Err(PacketError::InvalidRequestAuthenticator)
    }
}

fn verify_message_authenticator(
    packet: &RadiusPacket,
    shared_secret: &[u8],
    message_authenticator_index: usize,
) -> Result<(), PacketError> {
    let expected =
        expected_message_authenticator(packet, shared_secret, message_authenticator_index);
    let actual = packet.attributes()[message_authenticator_index].value();

    if constant_time_eq(actual, &expected) {
        Ok(())
    } else {
        Err(PacketError::InvalidMessageAuthenticator)
    }
}

pub(crate) fn message_authenticator_index(
    packet: &RadiusPacket,
) -> Result<Option<usize>, PacketError> {
    let mut found = None;
    for (index, attribute) in packet.attributes().iter().enumerate() {
        if attribute.kind() != MESSAGE_AUTHENTICATOR_TYPE {
            continue;
        }
        if attribute.value().len() != MESSAGE_AUTHENTICATOR_VALUE_LEN {
            return Err(PacketError::InvalidMessageAuthenticatorLength {
                actual: attribute.value().len(),
            });
        }
        if found.replace(index).is_some() {
            return Err(PacketError::MultipleMessageAuthenticators);
        }
    }

    Ok(found)
}

pub(crate) fn expected_request_authenticator(
    packet: &RadiusPacket,
    shared_secret: &[u8],
) -> [u8; RADIUS_AUTHENTICATOR_LEN] {
    let mut signed_packet = encode_packet(packet, [0_u8; RADIUS_AUTHENTICATOR_LEN], None);
    signed_packet.extend_from_slice(shared_secret);
    md5_digest(&signed_packet)
}

pub(crate) fn expected_message_authenticator(
    packet: &RadiusPacket,
    shared_secret: &[u8],
    message_authenticator_index: usize,
) -> [u8; MESSAGE_AUTHENTICATOR_VALUE_LEN] {
    let signed_packet = encode_packet(
        packet,
        [0_u8; RADIUS_AUTHENTICATOR_LEN],
        Some(message_authenticator_index),
    );
    hmac_md5(shared_secret, &signed_packet)
}

fn encode_packet(
    packet: &RadiusPacket,
    authenticator: [u8; RADIUS_AUTHENTICATOR_LEN],
    zero_message_authenticator_index: Option<usize>,
) -> Vec<u8> {
    let packet_len = RADIUS_HEADER_LEN
        + packet
            .attributes()
            .iter()
            .map(encoded_attribute_len)
            .sum::<usize>();

    let mut encoded = Vec::with_capacity(packet_len);
    encoded.push(packet.code().as_u8());
    encoded.push(packet.identifier());
    encoded.extend_from_slice(&(packet_len as u16).to_be_bytes());
    encoded.extend_from_slice(&authenticator);
    for (index, attribute) in packet.attributes().iter().enumerate() {
        if Some(index) == zero_message_authenticator_index {
            encode_attribute_value(
                attribute.kind(),
                &[0_u8; MESSAGE_AUTHENTICATOR_VALUE_LEN],
                &mut encoded,
            );
        } else {
            encode_attribute(attribute, &mut encoded);
        }
    }
    encoded
}

fn encode_proxy_state_attributes(packet: &RadiusPacket, encoded: &mut Vec<u8>) {
    for attribute in packet.attributes() {
        if attribute.kind() != PROXY_STATE_TYPE {
            continue;
        }
        encode_attribute(attribute, encoded);
    }
}

fn encode_attribute(attribute: &RadiusAttribute, encoded: &mut Vec<u8>) {
    encode_attribute_value(attribute.kind(), attribute.value(), encoded);
}

#[cfg(test)]
pub(crate) fn encode_radius_tlv(kind: u8, value: &[u8]) -> Vec<u8> {
    let encoded_len = RADIUS_ATTRIBUTE_HEADER_LEN + value.len();
    assert!(encoded_len <= u8::MAX as usize);
    let mut encoded = Vec::with_capacity(encoded_len);
    encode_attribute_value(kind, value, &mut encoded);
    encoded
}

fn encode_attribute_value(kind: u8, value: &[u8], encoded: &mut Vec<u8>) {
    encoded.push(kind);
    encoded.push((RADIUS_ATTRIBUTE_HEADER_LEN + value.len()) as u8);
    encoded.extend_from_slice(value);
}

fn encoded_attribute_len(attribute: &RadiusAttribute) -> usize {
    RADIUS_ATTRIBUTE_HEADER_LEN + attribute.value().len()
}

pub(crate) fn md5_digest(bytes: &[u8]) -> [u8; RADIUS_AUTHENTICATOR_LEN] {
    let mut hasher = Md5::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

pub(crate) fn hmac_md5(key: &[u8], bytes: &[u8]) -> [u8; MESSAGE_AUTHENTICATOR_VALUE_LEN] {
    let mut mac = HmacMd5::new_from_slice(key).expect("HMAC-MD5 accepts any key length");
    mac.update(bytes);
    mac.finalize().into_bytes().into()
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    left.ct_eq(right).into()
}

#[cfg(test)]
mod tests;
