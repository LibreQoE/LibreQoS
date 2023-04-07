use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

/// Type that provides a minimum, maximum and average value
/// for a given statistic within the associated time period.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StatsSummary {
    /// Minimum value
    pub min: (u64, u64),
    /// Maximum value
    pub max: (u64, u64),
    /// Average value
    pub avg: (u64, u64),
}

/// Type that provides a minimum, maximum and average value
/// for a given RTT value within the associated time period.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StatsRttSummary {
    /// Minimum value
    pub min: u32,
    /// Maximum value
    pub max: u32,
    /// Average value
    pub avg: u32,
}

/// Type that holds total traffic statistics for a given time period
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StatsTotals {
    /// Total number of packets
    pub packets: StatsSummary,
    /// Total number of bits
    pub bits: StatsSummary,
    /// Total number of shaped bits
    pub shaped_bits: StatsSummary,
}

/// Type that holds per-host statistics for a given stats collation
/// period.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StatsHost {
    /// Host circuit_id as it appears in ShapedDevices.csv
    pub circuit_id: String,
    /// Host's IP address
    pub ip_address: String,
    /// Host's traffic statistics
    pub bits: StatsSummary,
    /// Host's RTT statistics
    pub rtt: StatsRttSummary,
    /// Positional arguments indicating which tree entries apply
    pub tree_indices: Vec<usize>,
}

/// Node inside a traffic summary tree
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StatsTreeNode {
    /// Name (from network.json)
    pub name: String,
    /// Maximum allowed throughput (from network.json)
    pub max_throughput: (u32, u32),
    /// Indices of parents in the tree
    pub parents: Vec<usize>,
    /// Index of immediate parent in the tree
    pub immediate_parent: Option<usize>,
}

/// Network-transmitted query to ask the status of a license
/// key.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LicenseCheck {
    /// The key to check
    pub key: String,
}

/// License server responses for a key
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum LicenseReply {
    /// The license is denied
    Denied,
    /// The license is valid
    Valid,
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

fn build_license_request(key: String) -> Result<Vec<u8>, LicenseCheckError> {
    let mut result = Vec::new();
    let payload = serde_cbor::to_vec(&LicenseCheck { key });
    if let Err(e) = payload {
        log::warn!("Unable to serialize statistics. Not sending them.");
        log::warn!("{e:?}");
        return Err(LicenseCheckError::SerializeFail);
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

const LICENSE_SERVER: &str = "127.0.0.1:9126";

/// Ask the license server if the license is valid
/// 
/// # Arguments
/// 
/// * `key` - The license key to check
pub async fn ask_license_server(key: String) -> Result<LicenseReply, LicenseCheckError> {
    if let Ok(buffer) = build_license_request(key) {
        let stream = TcpStream::connect(LICENSE_SERVER).await;
        if let Err(e) = &stream {
            if e.kind() == std::io::ErrorKind::NotFound {
                log::error!("Unable to access {LICENSE_SERVER}. Check that lqosd is running and you have appropriate permissions.");
                return Err(LicenseCheckError::SendFail);
            }
        }
        let mut stream = stream.unwrap(); // This unwrap is safe, we checked that it exists previously
        let ret = stream.write(&buffer).await;
        if ret.is_err() {
            log::error!("Unable to write to {LICENSE_SERVER} stream.");
            log::error!("{:?}", ret);
            return Err(LicenseCheckError::SendFail);
        }
        let mut buf = Vec::with_capacity(10240);
        let ret = stream.read_to_end(&mut buf).await;
        if ret.is_err() {
            log::error!("Unable to read from {LICENSE_SERVER} stream.");
            log::error!("{:?}", ret);
            return Err(LicenseCheckError::SendFail);
        }

        decode_response(&buf)
    } else {
        Err(LicenseCheckError::SerializeFail)
    }
}

fn decode_response(buf: &[u8]) -> Result<LicenseReply, LicenseCheckError> {
    const U64SIZE: usize = std::mem::size_of::<u64>();
    let version_buf = &buf[0..2]
        .try_into()
        .map_err(|_| LicenseCheckError::DeserializeFail)?;
    let version = u16::from_be_bytes(*version_buf);
    let size_buf = &buf[2..2 + U64SIZE]
        .try_into()
        .map_err(|_| LicenseCheckError::DeserializeFail)?;
    let size = u64::from_be_bytes(*size_buf);

    if version != 1 {
        log::error!("License server returned an unknown version: {}", version);
        return Err(LicenseCheckError::DeserializeFail);
    }

    let start = 2 + U64SIZE;
    let end = start + size as usize;
    let payload: Result<LicenseReply, _> = serde_cbor::from_slice(&buf[start..end]);
    match payload {
        Ok(payload) => Ok(payload),
        Err(e) => {
            log::error!("Unable to deserialize license result");
            log::error!("{e:?}");
            Err(LicenseCheckError::DeserializeFail)
        }
    }
}
