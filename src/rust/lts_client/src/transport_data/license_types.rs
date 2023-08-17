//! Data-types used for license key exchange and lookup.

use serde::{Serialize, Deserialize};
use dryoc::dryocbox::PublicKey;
use thiserror::Error;

/// Network-transmitted query to ask the status of a license
/// key.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum LicenseRequest {
    /// Check the validity of a key
    LicenseCheck {
        /// The Key to Check
        key: String,
    },
    /// Exchange Keys
    KeyExchange {
        /// The node ID of the requesting shaper node
        node_id: String,
        /// The pretty name of the requesting shaper node
        node_name: String,
        /// The license key of the requesting shaper node
        license_key: String,
        /// The sodium-style public key of the requesting shaper node
        public_key: PublicKey,
    },
}

/// License server responses for a key
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum LicenseReply {
    /// The license is denied
    Denied,
    /// The license is valid
    Valid {
        /// When does the license expire?
        expiry: u64,
        /// Address to which statistics should be submitted
        stats_host: String,
    },
    /// Key Exchange
    MyPublicKey {
        /// The server's public key
        public_key: PublicKey,
    },
}

/// Errors that can occur when checking licenses
#[derive(Debug, Error)]
pub enum LicenseCheckError {
    /// Serialization error
    #[error("Unable to serialize license check")]
    SerializeFail,
    /// Network error
    #[error("Unable to send license check")]
    SendFail,
    /// Network error
    #[error("Unable to receive license result")]
    ReceiveFail,
    /// Deserialization error
    #[error("Unable to deserialize license result")]
    DeserializeFail,
}

/// Stores a license id and node id for transport
#[derive(Debug, Serialize, Deserialize)]
pub struct NodeIdAndLicense {
    /// The node id
    pub node_id: String,
    /// The license key
    pub license_key: String,
    /// The Sodium Nonce
    pub nonce: [u8; 24],
}