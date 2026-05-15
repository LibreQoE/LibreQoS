//! Shared helpers for NetFlow wire-field conversions.

pub(crate) const NANOS_PER_MILLI: u64 = 1_000_000;

pub(crate) fn clamp_i64_to_u32(protocol: &str, field: &str, value: i64) -> u32 {
    if value < 0 {
        tracing::warn!("{protocol} {field} value {value} is negative; clamping to 0");
        return 0;
    }

    clamp_u64_to_u32(protocol, field, value as u64)
}

pub(crate) fn clamp_u64_to_u32(protocol: &str, field: &str, value: u64) -> u32 {
    match u32::try_from(value) {
        Ok(value) => value,
        Err(_) => {
            tracing::warn!(
                "{protocol} {field} value {value} exceeds u32::MAX; clamping to u32::MAX"
            );
            u32::MAX
        }
    }
}

pub(crate) fn boot_nanos_to_netflow_millis(protocol: &str, field: &str, value: u64) -> u32 {
    clamp_u64_to_u32(protocol, field, value / NANOS_PER_MILLI)
}
