const HEX_LOWER: &[u8; 16] = b"0123456789abcdef";
const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";

pub(crate) fn encode_hex_lower(bytes: impl AsRef<[u8]>) -> String {
    encode_hex(bytes.as_ref(), HEX_LOWER)
}

pub(crate) fn encode_hex_upper(bytes: impl AsRef<[u8]>) -> String {
    encode_hex(bytes.as_ref(), HEX_UPPER)
}

fn encode_hex(bytes: &[u8], table: &[u8; 16]) -> String {
    let mut encoded = String::with_capacity(bytes.len().saturating_mul(2));
    for &byte in bytes {
        encoded.push(table[(byte >> 4) as usize] as char);
        encoded.push(table[(byte & 0x0f) as usize] as char);
    }
    encoded
}
