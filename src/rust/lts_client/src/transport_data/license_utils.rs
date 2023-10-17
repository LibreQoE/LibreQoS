//! Functions for talking to the license server
//! 
//! License requests use the following format:
//! `u16` containing the version number (currently 1), in big-endian (network order)
//! `u64` containing the size of the payload, in big-endian (network order)
//! `payload` containing the actual payload. The payload is a CBOR-encoded.
//! 
//! License requests are not expected to be frequent, and the connection is
//! not reused. We use a simple framing protocol, and terminate the connection
//! after use.

use super::{LicenseCheckError, LicenseRequest, LicenseReply, LICENSE_SERVER};
use dryoc::dryocbox::PublicKey;
use tokio::{net::TcpStream, io::{AsyncReadExt, AsyncWriteExt}};

fn build_license_request(key: String) -> Result<Vec<u8>, LicenseCheckError> {
    let mut result = Vec::new();
    let payload = serde_cbor::to_vec(&LicenseRequest::LicenseCheck { key });
    if let Err(e) = payload {
        log::warn!("Unable to serialize license request. Not sending them.");
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

fn build_activation_query(node_id: String) -> Result<Vec<u8>, LicenseCheckError> {
    let mut result = Vec::new();
    let payload = serde_cbor::to_vec(&LicenseRequest::PendingLicenseRequest { node_id } );
    if let Err(e) = payload {
        log::warn!("Unable to serialize license request. Not sending them.");
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

fn build_key_exchange_request(
    node_id: String,
    node_name: String,
    license_key: String,
    public_key: PublicKey,
) -> Result<Vec<u8>, LicenseCheckError> {
    let mut result = Vec::new();
    let payload = serde_cbor::to_vec(&LicenseRequest::KeyExchange {
        node_id,
        node_name,
        license_key,
        public_key,
    });
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
        let stream = stream;
        match stream {
            Ok(mut stream) => {
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
            }
            Err(e) => {
                log::warn!("TCP stream failed to connect: {:?}", e);
                Err(LicenseCheckError::ReceiveFail)
            }
        }
    } else {
        Err(LicenseCheckError::SerializeFail)
    }
}

pub async fn ask_license_server_for_new_account(
    node_id: String,
) -> Result<LicenseReply, LicenseCheckError>
{
    if let Ok(buffer) = build_activation_query(node_id) {
        let stream = TcpStream::connect(LICENSE_SERVER).await;
        if let Err(e) = &stream {
            if e.kind() == std::io::ErrorKind::NotFound {
                log::error!("Unable to access {LICENSE_SERVER}. Check that lqosd is running and you have appropriate permissions.");
                return Err(LicenseCheckError::SendFail);
            }
        }
        let stream = stream;
        match stream {
            Ok(mut stream) => {
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
            }
            Err(e) => {
                log::warn!("TCP stream failed to connect: {:?}", e);
                Err(LicenseCheckError::ReceiveFail)
            }
        }
    } else {
        Err(LicenseCheckError::SerializeFail)
    }
}

/// Ask the license server for the public key
pub async fn exchange_keys_with_license_server(
    node_id: String,
    node_name: String,
    license_key: String,
    public_key: PublicKey,
) -> Result<LicenseReply, LicenseCheckError> {
    if let Ok(buffer) = build_key_exchange_request(node_id, node_name, license_key, public_key) {
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
    if buf.len() < 2 + std::mem::size_of::<u64>() {
        log::error!("License server returned an invalid response");
        return Err(LicenseCheckError::DeserializeFail);
    }
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
