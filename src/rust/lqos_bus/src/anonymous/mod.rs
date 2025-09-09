// SPDX-FileCopyrightText: 2025 LibreQoE support@libreqos.io
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-LibreQoS-Exception

mod v1;
use allocative::Allocative;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::warn;
pub use v1::*;

#[derive(Debug, Clone, Serialize, Deserialize, Allocative)]
/// Header for stats submission
pub struct StatsHeader {
    /// The version to use (should be 1)
    pub version: u16,
    /// The number of bytes being submitted following the header
    pub size: usize,
}

/// Build an anonymous usage statistics buffer.
/// Transforms `stats` (`AnonymousUsageV1`) into a matching
/// header and payload, in a single buffer ready to send.
pub fn build_stats(stats: &AnonymousUsageV1) -> Result<Vec<u8>, StatsError> {
    let mut result = Vec::new();
    let payload = serde_cbor::to_vec(stats);
    if let Err(e) = payload {
        warn!("Unable to serialize statistics. Not sending them.");
        warn!("{e:?}");
        return Err(StatsError::SerializeFail);
    }
    let payload = payload.unwrap();

    // Store the version as network order
    result.extend(1u16.to_be_bytes());
    // Store the payload size as network order
    result.extend((payload.len() as u64).to_be_bytes());
    // Store the payload itself
    result.extend(payload);

    Ok(result)
}

/// Errors for anonymous usage statistics failure
#[derive(Error, Debug)]
pub enum StatsError {
    /// Serializing the object failed
    #[error("Unable to serialize object")]
    SerializeFail,
}
