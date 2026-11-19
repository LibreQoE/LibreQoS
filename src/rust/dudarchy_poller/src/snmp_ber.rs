//! The SNMP wire codec: BER encoding of GET/GETNEXT request messages and
//! decoding of response messages, plus the wire shapes both sides share
//! (tag constants, the fixed request-id, the well-known system OIDs the
//! single GET carries).
//!
//! Pure and synchronous: bytes in, values out — no socket, no timeout, no
//! clock. The UDP transport lives in [`crate::snmp_transport`]; the walk
//! state machine and reply validation in [`crate::snmp`]. The codec is
//! exercised from fixture bytes.

use crate::snmp::SnmpVersion;

/// `sysDescr.0`.
pub(crate) const SYS_DESCR: [u32; 9] = [1, 3, 6, 1, 2, 1, 1, 1, 0];

/// `sysObjectID.0` — the agent's hardware/family OID; the adapter
/// documents select on it.
pub(crate) const SYS_OBJECT_ID: [u32; 9] = [1, 3, 6, 1, 2, 1, 1, 2, 0];

/// `sysName.0`.
pub(crate) const SYS_NAME: [u32; 9] = [1, 3, 6, 1, 2, 1, 1, 5, 0];

/// Request-id carried by the single GET; the reply must echo it.
pub(crate) const REQUEST_ID: i64 = 1;

const INTEGER_TAG: u8 = 0x02;
const OCTET_STRING_TAG: u8 = 0x04;
const NULL_TAG: u8 = 0x05;
const OID_TAG: u8 = 0x06;
const SEQUENCE_TAG: u8 = 0x30;
/// TimeTicks tag: ages and upticks in hundredths of a second.
const TIME_TICKS_TAG: u8 = 0x43;
/// Counter64 tag: wide byte/rate counters.
const COUNTER64_TAG: u8 = 0x46;
/// IpAddress tag: a four-octet IPv4 address.
const IP_ADDRESS_TAG: u8 = 0x40;

/// v2c exception value: the object does not exist (context tag 0).
const NO_SUCH_OBJECT_TAG: u8 = 0x80;
/// v2c exception value: the instance does not exist (context tag 1).
const NO_SUCH_INSTANCE_TAG: u8 = 0x81;
/// v2c exception value: the walk passed the end of the MIB view
/// (context tag 2).
const END_OF_MIB_VIEW_TAG: u8 = 0x82;
/// GetRequest-PDU (the single-GET request).
const GET_REQUEST_PDU_TAG: u8 = 0xA0;
/// GetNextRequest-PDU (the walk-step request).
const GET_NEXT_PDU_TAG: u8 = 0xA1;
/// Response-PDU (the v2c reply).
const RESPONSE_PDU_TAG: u8 = 0xA2;

/// Builds a GET request for `sysDescr.0` + `sysName.0` with `community`
/// at `version`.
pub(crate) fn get_request_bytes(community: &[u8], version: SnmpVersion) -> Vec<u8> {
    request_bytes(
        community,
        version,
        GET_REQUEST_PDU_TAG,
        &[&SYS_DESCR, &SYS_OBJECT_ID, &SYS_NAME],
    )
}

/// Builds a GET request for one OID with `community` at `version`.
pub(crate) fn get_oid_request_bytes(
    community: &[u8],
    version: SnmpVersion,
    oid: &[u32],
) -> Vec<u8> {
    request_bytes(community, version, GET_REQUEST_PDU_TAG, &[oid])
}

/// Builds a GETNEXT request asking for the OID after `oid` at `version`.
pub(crate) fn next_request_bytes(community: &[u8], version: SnmpVersion, oid: &[u32]) -> Vec<u8> {
    request_bytes(community, version, GET_NEXT_PDU_TAG, &[oid])
}

/// Builds a request at `version` with `pdu_tag` carrying one NULL-valued
/// varbind per OID, request-id [`REQUEST_ID`].
fn request_bytes(community: &[u8], version: SnmpVersion, pdu_tag: u8, oids: &[&[u32]]) -> Vec<u8> {
    let mut varbinds = Vec::new();
    for oid in oids {
        let mut oid_bytes = Vec::new();
        push_oid(&mut oid_bytes, oid);
        let mut varbind = Vec::new();
        push_tlv(&mut varbind, OID_TAG, &oid_bytes);
        push_tlv(&mut varbind, NULL_TAG, &[]);
        push_tlv(&mut varbinds, SEQUENCE_TAG, &varbind);
    }
    let mut pdu = Vec::new();
    push_integer(&mut pdu, REQUEST_ID);
    push_integer(&mut pdu, 0); // error-status
    push_integer(&mut pdu, 0); // error-index
    push_tlv(&mut pdu, SEQUENCE_TAG, &varbinds);
    let mut message = Vec::new();
    push_integer(&mut message, version.wire());
    push_tlv(&mut message, OCTET_STRING_TAG, community);
    push_tlv(&mut message, pdu_tag, &pdu);
    let mut out = Vec::new();
    push_tlv(&mut out, SEQUENCE_TAG, &message);
    out
}

/// Appends one BER element: `tag`, definite length, `content`.
fn push_tlv(out: &mut Vec<u8>, tag: u8, content: &[u8]) {
    out.push(tag);
    push_length(out, content.len());
    out.extend_from_slice(content);
}

/// Appends a definite BER length: short form below 128, long form above.
fn push_length(out: &mut Vec<u8>, length: usize) {
    if length < 0x80 {
        out.push(length as u8); // guarded by the branch above
        return;
    }
    let mut bytes = [0u8; 8];
    let mut count = 0;
    let mut remaining = length;
    while remaining > 0 {
        bytes[count] = (remaining & 0xff) as u8; // masked to one byte
        count += 1;
        remaining >>= 8;
    }
    out.push(0x80 | count as u8); // count <= 8
    out.extend(bytes[..count].iter().rev().copied());
}

/// Appends an INTEGER in minimal two's-complement encoding.
fn push_integer(out: &mut Vec<u8>, value: i64) {
    let be = value.to_be_bytes();
    let mut start = 0;
    while start < be.len() - 1 {
        let sign = be[start + 1] & 0x80;
        if (be[start] == 0 && sign == 0) || (be[start] == 0xff && sign != 0) {
            start += 1;
        } else {
            break;
        }
    }
    push_tlv(out, INTEGER_TAG, &be[start..]);
}

/// Appends an object identifier, BER base-128 encoded. Called only with the
/// module's own OIDs; `debug_assert` documents the two-component minimum.
fn push_oid(out: &mut Vec<u8>, oid: &[u32]) {
    debug_assert!(oid.len() >= 2, "OID needs at least two components");
    let mut components = oid.iter().copied();
    let first = components.next().unwrap_or_default();
    let second = components.next().unwrap_or_default();
    push_subidentifier(out, 40 * first + second);
    for component in components {
        push_subidentifier(out, component);
    }
}

/// Appends one base-128 subidentifier, high-order byte first.
fn push_subidentifier(out: &mut Vec<u8>, mut value: u32) {
    let mut bytes = [0u8; 5];
    let mut count = 1;
    bytes[4] = (value & 0x7f) as u8; // masked to seven bits
    value >>= 7;
    while value > 0 {
        count += 1;
        bytes[5 - count] = (value & 0x7f) as u8 | 0x80; // masked to seven bits
        value >>= 7;
    }
    out.extend_from_slice(&bytes[5 - count..]);
}

/// A decoded SNMP response.
#[derive(Debug)]
pub(crate) struct Response<'a> {
    pub(crate) community: &'a [u8],
    /// The version INTEGER the agent echoed (0 = v1, 1 = v2c).
    pub(crate) version: i64,
    pub(crate) request_id: i64,
    pub(crate) error_status: i64,
    pub(crate) varbinds: Vec<(Vec<u32>, BerValue<'a>)>,
}

/// A varbind value: an octet string, an integer, a counter, a v2c
/// exception (which the walk treats as an absent column or the end of the
/// table), or anything else, which value extraction rejects.
#[derive(Debug)]
pub(crate) enum BerValue<'a> {
    OctetString(&'a [u8]),
    Integer(i64),
    Counter64(u64),
    TimeTicks(u64),
    /// An IpAddress (tag 0x40) as dotted text.
    IpAddress(String),
    /// An OID value (sysObjectID-class), kept as dotted text.
    Oid(String),
    NoSuchObject,
    NoSuchInstance,
    EndOfMibView,
    Other,
}

/// Decodes one SNMP v1/v2c response message, consuming `bytes` exactly.
/// Failure reasons are static so tests can assert them.
pub(crate) fn decode_response(bytes: &[u8]) -> Result<Response<'_>, &'static str> {
    let (tag, start, end) = element(bytes, 0)?;
    if tag != SEQUENCE_TAG {
        return Err("outer element is not a sequence");
    }
    if end != bytes.len() {
        return Err("trailing bytes after the message");
    }
    let mut message = Reader {
        bytes,
        pos: start,
        end,
    };
    let version = message.integer()?;
    if version != 0 && version != 1 {
        return Err("not an SNMP v1/v2c message");
    }
    let community = message.octet_string()?;
    let (pdu_tag, pdu_start, pdu_end) = message.element()?;
    if pdu_tag != RESPONSE_PDU_TAG {
        return Err("unexpected PDU type");
    }
    let mut pdu = Reader {
        bytes,
        pos: pdu_start,
        end: pdu_end,
    };
    let request_id = pdu.integer()?;
    let error_status = pdu.integer()?;
    pdu.integer()?; // error-index: not meaningful for a successful reply
    let mut varbinds = pdu.sequence()?;
    let mut parsed = Vec::new();
    while varbinds.pos < varbinds.end {
        let (tag, start, end) = varbinds.element()?;
        if tag != SEQUENCE_TAG {
            return Err("varbind is not a sequence");
        }
        let mut varbind = Reader {
            bytes,
            pos: start,
            end,
        };
        let name_oid = varbind.oid()?;
        let (tag, start, end) = varbind.element()?;
        let value = match tag {
            OID_TAG => BerValue::Oid({
                // Re-encode the OID content as dotted text via the shared
                // decoder: decode into components, then join.
                let components = oid_components(&bytes[start..end])?;
                components
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(".")
            }),
            OCTET_STRING_TAG => BerValue::OctetString(&bytes[start..end]),
            INTEGER_TAG => BerValue::Integer(integer_value(&bytes[start..end])?),
            TIME_TICKS_TAG => BerValue::TimeTicks(unsigned_value(&bytes[start..end], 4)?),
            IP_ADDRESS_TAG => BerValue::IpAddress({
                let content = &bytes[start..end];
                if content.len() != 4 {
                    return Err("ip address is not four octets");
                }
                format!(
                    "{}.{}.{}.{}",
                    content[0], content[1], content[2], content[3]
                )
            }),
            COUNTER64_TAG => BerValue::Counter64(unsigned_value(&bytes[start..end], 8)?),
            NO_SUCH_OBJECT_TAG => BerValue::NoSuchObject,
            NO_SUCH_INSTANCE_TAG => BerValue::NoSuchInstance,
            END_OF_MIB_VIEW_TAG => BerValue::EndOfMibView,
            _ => BerValue::Other,
        };
        parsed.push((name_oid, value));
    }
    Ok(Response {
        community,
        version,
        request_id,
        error_status,
        varbinds: parsed,
    })
}

/// Interprets BER unsigned-integer content (counters, TimeTicks) as a
/// `u64`; `max_bytes` bounds the encoding length (4 for TimeTicks, 8 for
/// Counter64).
fn unsigned_value(content: &[u8], max_bytes: usize) -> Result<u64, &'static str> {
    if content.is_empty() || content.len() > max_bytes {
        return Err("unsigned value out of range");
    }
    let mut value = 0u64;
    for &byte in content {
        value = (value << 8) | u64::from(byte);
    }
    Ok(value)
}

/// A cursor over one BER-structured region of a message.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    end: usize,
}

impl<'a> Reader<'a> {
    /// The next element inside this region as `(tag, content start, content end)`.
    fn element(&mut self) -> Result<(u8, usize, usize), &'static str> {
        if self.pos >= self.end {
            return Err("message content ends early");
        }
        let (tag, start, end) = element(self.bytes, self.pos)?;
        if end > self.end {
            return Err("element extends past its parent");
        }
        self.pos = end;
        Ok((tag, start, end))
    }

    /// The next element as an INTEGER value.
    fn integer(&mut self) -> Result<i64, &'static str> {
        let (tag, start, end) = self.element()?;
        if tag != INTEGER_TAG {
            return Err("expected an INTEGER");
        }
        integer_value(&self.bytes[start..end])
    }

    /// The next element as an OCTET STRING.
    fn octet_string(&mut self) -> Result<&'a [u8], &'static str> {
        let (tag, start, end) = self.element()?;
        if tag != OCTET_STRING_TAG {
            return Err("expected an OCTET STRING");
        }
        Ok(&self.bytes[start..end])
    }

    /// The next element as an object identifier.
    fn oid(&mut self) -> Result<Vec<u32>, &'static str> {
        let (tag, start, end) = self.element()?;
        if tag != OID_TAG {
            return Err("expected an OID");
        }
        oid_components(&self.bytes[start..end])
    }

    /// The next element as a SEQUENCE, returning a reader over its content.
    fn sequence(&mut self) -> Result<Reader<'a>, &'static str> {
        let (tag, start, end) = self.element()?;
        if tag != SEQUENCE_TAG {
            return Err("expected a SEQUENCE");
        }
        Ok(Reader {
            bytes: self.bytes,
            pos: start,
            end,
        })
    }
}

/// Reads the BER element at `bytes[pos..]`: tag, definite length, content.
/// Returns `(tag, content start, content end)`.
fn element(bytes: &[u8], pos: usize) -> Result<(u8, usize, usize), &'static str> {
    let &tag = bytes.get(pos).ok_or("truncated element")?;
    let &first = bytes.get(pos + 1).ok_or("truncated length")?;
    let (header, length) = match first {
        0x80 => return Err("indefinite length"),
        short if short < 0x80 => (2, usize::from(short)),
        long => {
            let count = usize::from(long & 0x7f);
            let start = pos + 2;
            let end = start.checked_add(count).ok_or("length overflows")?;
            let digits = bytes.get(start..end).ok_or("truncated long-form length")?;
            let mut length = 0usize;
            for &digit in digits {
                length = length
                    .checked_mul(0x100)
                    .and_then(|value| value.checked_add(usize::from(digit)))
                    .ok_or("length overflows")?;
            }
            (2 + count, length)
        }
    };
    let start = pos + header;
    let end = start.checked_add(length).ok_or("length overflows")?;
    if end > bytes.len() {
        return Err("element length exceeds the input");
    }
    Ok((tag, start, end))
}

/// Interprets BER INTEGER content as a two's-complement `i64`.
fn integer_value(content: &[u8]) -> Result<i64, &'static str> {
    if content.is_empty() || content.len() > 8 {
        return Err("INTEGER out of range");
    }
    let mut value = if content[0] & 0x80 != 0 { -1i64 } else { 0i64 };
    for &byte in content {
        value = (value << 8) | i64::from(byte);
    }
    Ok(value)
}

/// Decodes BER OID content into its components.
fn oid_components(content: &[u8]) -> Result<Vec<u32>, &'static str> {
    let Some(&first) = content.first() else {
        return Err("empty OID");
    };
    let lead = u32::from(first);
    let top = (lead / 40).min(2);
    let mut components = vec![top, lead - 40 * top];
    let mut value = 0u32;
    let mut in_continuation = false;
    for &byte in &content[1..] {
        value = value
            .checked_mul(128)
            .and_then(|part| part.checked_add(u32::from(byte & 0x7f)))
            .ok_or("OID subidentifier too large")?;
        if byte & 0x80 == 0 {
            components.push(value);
            value = 0;
            in_continuation = false;
        } else {
            in_continuation = true;
        }
    }
    if in_continuation {
        return Err("truncated OID subidentifier");
    }
    Ok(components)
}

#[cfg(test)]
/// Hand-built v2c GET for community `public`: version 1, request-id 1,
/// empty error fields, `sysDescr.0` and `sysName.0` with NULL values.
pub(crate) const GET_PUBLIC: [u8; 68] = [
    0x30, 0x42, 0x02, 0x01, 0x01, 0x04, 0x06, 0x70, 0x75, 0x62, 0x6c, 0x69, 0x63, 0xa0, 0x35, 0x02,
    0x01, 0x01, 0x02, 0x01, 0x00, 0x02, 0x01, 0x00, 0x30, 0x2a, 0x30, 0x0c, 0x06, 0x08, 0x2b, 0x06,
    0x01, 0x02, 0x01, 0x01, 0x01, 0x00, 0x05, 0x00, 0x30, 0x0c, 0x06, 0x08, 0x2b, 0x06, 0x01, 0x02,
    0x01, 0x01, 0x02, 0x00, 0x05, 0x00, 0x30, 0x0c, 0x06, 0x08, 0x2b, 0x06, 0x01, 0x02, 0x01, 0x01,
    0x05, 0x00, 0x05, 0x00,
];

#[cfg(test)]
/// Hand-built v2c response for community `public`: request-id 1, error
/// status 0, `sysDescr.0` = "RouterOS CCR1009-8G-1S",
/// `sysName.0` = "FSW-Office-Core".
pub(crate) const RESPONSE_PUBLIC: [u8; 113] = [
    0x30, 0x6f, 0x02, 0x01, 0x01, 0x04, 0x06, 0x70, 0x75, 0x62, 0x6c, 0x69, 0x63, 0xa2, 0x62, 0x02,
    0x01, 0x01, 0x02, 0x01, 0x00, 0x02, 0x01, 0x00, 0x30, 0x57, 0x30, 0x22, 0x06, 0x08, 0x2b, 0x06,
    0x01, 0x02, 0x01, 0x01, 0x01, 0x00, 0x04, 0x16, 0x52, 0x6f, 0x75, 0x74, 0x65, 0x72, 0x4f, 0x53,
    0x20, 0x43, 0x43, 0x52, 0x31, 0x30, 0x30, 0x39, 0x2d, 0x38, 0x47, 0x2d, 0x31, 0x53, 0x30, 0x14,
    0x06, 0x08, 0x2b, 0x06, 0x01, 0x02, 0x01, 0x01, 0x02, 0x00, 0x06, 0x08, 0x2b, 0x06, 0x01, 0x04,
    0x01, 0xf5, 0x0c, 0x01, 0x30, 0x1b, 0x06, 0x08, 0x2b, 0x06, 0x01, 0x02, 0x01, 0x01, 0x05, 0x00,
    0x04, 0x0f, 0x46, 0x53, 0x57, 0x2d, 0x4f, 0x66, 0x66, 0x69, 0x63, 0x65, 0x2d, 0x43, 0x6f, 0x72,
    0x65,
];

#[cfg(test)]
/// Hand-built v2c response reporting error-status 5 (genErr), no varbinds.
pub(crate) const RESPONSE_GEN_ERR: [u8; 26] = [
    0x30, 0x18, 0x02, 0x01, 0x01, 0x04, 0x06, 0x70, 0x75, 0x62, 0x6c, 0x69, 0x63, 0xa2, 0x0b, 0x02,
    0x01, 0x01, 0x02, 0x01, 0x05, 0x02, 0x01, 0x00, 0x30, 0x00,
];

#[cfg(test)]
/// Hand-built v2c response whose `sysDescr.0` value is the v2c exception
/// `noSuchInstance`.
pub(crate) const RESPONSE_NO_SUCH_INSTANCE: [u8; 69] = [
    0x30, 0x43, 0x02, 0x01, 0x01, 0x04, 0x06, 0x70, 0x75, 0x62, 0x6c, 0x69, 0x63, 0xa2, 0x36, 0x02,
    0x01, 0x01, 0x02, 0x01, 0x00, 0x02, 0x01, 0x00, 0x30, 0x2b, 0x30, 0x0c, 0x06, 0x08, 0x2b, 0x06,
    0x01, 0x02, 0x01, 0x01, 0x01, 0x00, 0x81, 0x00, 0x30, 0x1b, 0x06, 0x08, 0x2b, 0x06, 0x01, 0x02,
    0x01, 0x01, 0x05, 0x00, 0x04, 0x0f, 0x46, 0x53, 0x57, 0x2d, 0x4f, 0x66, 0x66, 0x69, 0x63, 0x65,
    0x2d, 0x43, 0x6f, 0x72, 0x65,
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-built v2c GETNEXT for community `public` asking past
    /// `lldpRemTable`: version 1, request-id 1, PDU tag 0xA1, one varbind
    /// with a NULL value.
    const NEXT_REQUEST_PUBLIC: [u8; 41] = [
        0x30, 0x27, 0x02, 0x01, 0x01, 0x04, 0x06, 0x70, 0x75, 0x62, 0x6c, 0x69, 0x63, 0xa1, 0x1a,
        0x02, 0x01, 0x01, 0x02, 0x01, 0x00, 0x02, 0x01, 0x00, 0x30, 0x0f, 0x30, 0x0d, 0x06, 0x09,
        0x28, 0xc4, 0x62, 0x01, 0x01, 0x02, 0x01, 0x04, 0x01, 0x05, 0x00,
    ];

    /// Hand-built v2c GET for community `public` asking for `sysName.0`
    /// (`.1.3.6.1.2.1.1.5.0`): version 1, request-id 1, PDU tag 0xA0, one
    /// varbind with a NULL value.
    const GET_OID_REQUEST_PUBLIC: [u8; 40] = [
        0x30, 0x26, 0x02, 0x01, 0x01, 0x04, 0x06, 0x70, 0x75, 0x62, 0x6c, 0x69, 0x63, 0xa0, 0x19,
        0x02, 0x01, 0x01, 0x02, 0x01, 0x00, 0x02, 0x01, 0x00, 0x30, 0x0e, 0x30, 0x0c, 0x06, 0x08,
        0x2b, 0x06, 0x01, 0x02, 0x01, 0x01, 0x05, 0x00, 0x05, 0x00,
    ];

    #[test]
    fn request_matches_fixture_bytes() {
        assert_eq!(
            get_request_bytes(b"public", SnmpVersion::V2c),
            GET_PUBLIC.to_vec()
        );
    }

    #[test]
    fn get_oid_request_matches_fixture_bytes() {
        // The fixture request asks for `sysName.0` (.1.3.6.1.2.1.1.5.0).
        let sys_name = [1, 3, 6, 1, 2, 1, 1, 5, 0];
        assert_eq!(
            get_oid_request_bytes(b"public", SnmpVersion::V2c, &sys_name),
            GET_OID_REQUEST_PUBLIC.to_vec()
        );
    }

    #[test]
    fn next_request_matches_fixture_bytes() {
        // The fixture request asks past `lldpRemTable`
        // (.1.0.8802.1.1.2.1.4.1).
        let lldp_rem_table = [1, 0, 8802, 1, 1, 2, 1, 4, 1];
        assert_eq!(
            next_request_bytes(b"public", SnmpVersion::V2c, &lldp_rem_table),
            NEXT_REQUEST_PUBLIC.to_vec()
        );
    }

    #[test]
    fn v1_request_encodes_version_zero() {
        // The version INTEGER is the message's first field: v1 encodes 0,
        // v2c 1 (pinned by GET_PUBLIC above).
        let v1 = get_request_bytes(b"public", SnmpVersion::V1);
        assert_eq!(&v1[..8], &[0x30, 0x42, 0x02, 0x01, 0x00, 0x04, 0x06, 0x70]);
        let v2c = get_request_bytes(b"public", SnmpVersion::V2c);
        assert_eq!(&v2c[..5], &[0x30, 0x42, 0x02, 0x01, 0x01]);
    }

    /// Assembles a full response message from raw varbind values
    /// (pre-encoded tag + content), for decoder tests.
    fn response_bytes(
        version: i64,
        error_status: i64,
        varbinds: &[(Vec<u32>, u8, Vec<u8>)],
    ) -> Vec<u8> {
        let mut bindings = Vec::new();
        for (oid, tag, content) in varbinds {
            let mut oid_bytes = Vec::new();
            push_oid(&mut oid_bytes, oid);
            let mut varbind = Vec::new();
            push_tlv(&mut varbind, OID_TAG, &oid_bytes);
            push_tlv(&mut varbind, *tag, content);
            push_tlv(&mut bindings, SEQUENCE_TAG, &varbind);
        }
        let mut pdu = Vec::new();
        push_integer(&mut pdu, REQUEST_ID);
        push_integer(&mut pdu, error_status);
        push_integer(&mut pdu, 0); // error-index
        push_tlv(&mut pdu, SEQUENCE_TAG, &bindings);
        let mut message = Vec::new();
        push_integer(&mut message, version);
        push_tlv(&mut message, OCTET_STRING_TAG, b"public");
        push_tlv(&mut message, RESPONSE_PDU_TAG, &pdu);
        let mut out = Vec::new();
        push_tlv(&mut out, SEQUENCE_TAG, &message);
        out
    }

    #[test]
    fn counter64_and_timeticks_decode() {
        let oid = [1, 3, 6, 1, 2, 1, 99, 1];
        let bytes = response_bytes(
            1,
            0,
            &[(oid.to_vec(), COUNTER64_TAG, {
                let mut content = vec![0x00, 0xde, 0xad, 0xbe, 0xef, 0xca, 0xfe, 0x00, 0x01];
                content.remove(0);
                content
            })],
        );
        let response = decode_response(&bytes).expect("decodes");
        match &response.varbinds[0].1 {
            BerValue::Counter64(value) => assert_eq!(*value, 0xdead_beef_cafe_0001),
            other => panic!("expected Counter64, got {other:?}"),
        }

        let bytes = response_bytes(
            1,
            0,
            &[(oid.to_vec(), TIME_TICKS_TAG, vec![0x00, 0x01, 0xe2, 0x40])],
        );
        let response = decode_response(&bytes).expect("decodes");
        match &response.varbinds[0].1 {
            BerValue::TimeTicks(value) => assert_eq!(*value, 123_456),
            other => panic!("expected TimeTicks, got {other:?}"),
        }

        // A Counter64 body longer than eight bytes is out of range.
        let bytes = response_bytes(
            1,
            0,
            &[(oid.to_vec(), COUNTER64_TAG, vec![1, 2, 3, 4, 5, 6, 7, 8, 9])],
        );
        assert_eq!(
            decode_response(&bytes).unwrap_err(),
            "unsigned value out of range"
        );
    }

    #[test]
    fn v2c_exceptions_are_classified() {
        let response = decode_response(&RESPONSE_NO_SUCH_INSTANCE).expect("fixture decodes");
        assert!(matches!(response.varbinds[0].1, BerValue::NoSuchInstance));
    }

    #[test]
    fn long_community_switches_to_long_form_length() {
        let request = get_request_bytes(&[0x41; 200], SnmpVersion::V2c);
        // With three varbinds plus a 200-byte community the message body
        // passes 255 bytes: the outer length takes two long-form length
        // bytes.
        assert_eq!(request[1], 0x82, "long form marker with two length bytes");
        let content = ((request[2] as usize) << 8) | request[3] as usize;
        assert_eq!(request.len(), 4 + content);
    }

    #[test]
    fn response_with_long_form_outer_length_decodes_the_same() {
        let mut long_form = vec![0x30, 0x81];
        long_form.push(RESPONSE_PUBLIC.len() as u8 - 2);
        long_form.extend_from_slice(&RESPONSE_PUBLIC[2..]);
        let response = decode_response(&long_form).expect("long form decodes");
        assert_eq!(response.request_id, 1);
    }

    #[test]
    fn every_truncated_prefix_is_rejected() {
        for len in 0..RESPONSE_PUBLIC.len() {
            assert!(
                decode_response(&RESPONSE_PUBLIC[..len]).is_err(),
                "prefix of {len} bytes should not decode"
            );
        }
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        let mut trailing = RESPONSE_PUBLIC.to_vec();
        trailing.push(0x00);
        assert_eq!(
            decode_response(&trailing).unwrap_err(),
            "trailing bytes after the message"
        );
    }

    #[test]
    fn malformed_responses_are_rejected_with_reasons() {
        let mut not_a_sequence = RESPONSE_PUBLIC;
        not_a_sequence[0] = 0x31;
        assert_eq!(
            decode_response(&not_a_sequence).unwrap_err(),
            "outer element is not a sequence"
        );

        let mut indefinite = RESPONSE_PUBLIC;
        indefinite[1] = 0x80;
        assert_eq!(
            decode_response(&indefinite).unwrap_err(),
            "indefinite length"
        );

        let mut too_long = RESPONSE_PUBLIC;
        too_long[1] = 0x7f;
        assert_eq!(
            decode_response(&too_long).unwrap_err(),
            "element length exceeds the input"
        );

        let mut not_integer = RESPONSE_PUBLIC;
        not_integer[2] = 0x05;
        assert_eq!(
            decode_response(&not_integer).unwrap_err(),
            "expected an INTEGER"
        );

        // A version neither v1 (0) nor v2c (1) is not an SNMP message this
        // client speaks. (Version 0 on this wire position would be a valid
        // v1 message now.)
        let mut unsupported = RESPONSE_PUBLIC;
        unsupported[4] = 0x02;
        assert_eq!(
            decode_response(&unsupported).unwrap_err(),
            "not an SNMP v1/v2c message"
        );

        let mut wrong_pdu = RESPONSE_PUBLIC;
        wrong_pdu[13] = 0xa1;
        assert_eq!(
            decode_response(&wrong_pdu).unwrap_err(),
            "unexpected PDU type"
        );
    }

    #[test]
    fn oid_encoding_round_trips_standard_and_multi_byte() {
        for oid in [&SYS_DESCR, &SYS_NAME, &[1, 3, 1000, 200_000, 0][..]] {
            let mut encoded = Vec::new();
            push_oid(&mut encoded, oid);
            let decoded = oid_components(&encoded).expect("encoded OID decodes");
            assert_eq!(decoded, oid);
        }
    }

    #[test]
    fn malformed_oid_content_is_rejected() {
        assert_eq!(oid_components(&[]).unwrap_err(), "empty OID");
        assert_eq!(
            oid_components(&[0x2b, 0x86]).unwrap_err(),
            "truncated OID subidentifier"
        );
        let huge = [0x2b, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f];
        assert_eq!(
            oid_components(&huge).unwrap_err(),
            "OID subidentifier too large"
        );
    }
}
