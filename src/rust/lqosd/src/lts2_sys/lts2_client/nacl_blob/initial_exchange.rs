use crate::lts2_sys::lts2_client::nacl_blob::size_info::SizeInfo;
use crate::lts2_sys::lts2_client::nacl_blob::{KeyStore, initial_exchange};
use dryoc::dryocbox::PublicKey;

pub fn transmit_hello(
    keys: &KeyStore,
    magic_number: u16,
    version: u16,
    tcp_stream: &mut std::net::TcpStream,
) -> anyhow::Result<SizeInfo> {
    use std::io::Write;
    let mut size_info = SizeInfo::default();
    let key_bytes = keys.public_key_as_cbor_bytes();
    let key_len = key_bytes.len() as u32;

    // Transmit the magic number, version, and key length to the other party in BE order
    tcp_stream.write_all(&magic_number.to_be_bytes())?;
    tcp_stream.flush()?;
    tcp_stream.write_all(&version.to_be_bytes())?;
    tcp_stream.flush()?;
    tcp_stream.write_all(&key_len.to_be_bytes())?;
    tcp_stream.flush()?;
    size_info.raw_size += 8 + key_len as u64;
    size_info.final_size += 8 + key_len as u64;

    // Transmit the public key to the other party
    tcp_stream.write_all(&key_bytes)?;
    tcp_stream.flush()?;

    Ok(size_info)
}

pub fn receive_hello(
    tcp_stream: &mut std::net::TcpStream,
) -> anyhow::Result<(initial_exchange::InitialExchange, SizeInfo)> {
    use std::io::Read;
    let mut size_info = SizeInfo::default();

    // Read the fields in turn
    let mut buf = [0u8; 2];
    let received = tcp_stream.read(&mut buf)?;
    if received < 2 {
        return Err(anyhow::anyhow!("Failed to read magic number"));
    }
    let magic_number = u16::from_be_bytes([buf[0], buf[1]]);

    let received = tcp_stream.read(&mut buf)?;
    if received < 2 {
        return Err(anyhow::anyhow!("Failed to read version"));
    }
    let version = u16::from_be_bytes([buf[0], buf[1]]);

    let mut buf = [0u8; 4];
    let received = tcp_stream.read(&mut buf)?;
    if received < 4 {
        return Err(anyhow::anyhow!("Failed to read key length"));
    }
    let key_len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);

    let mut key_bytes = vec![0; key_len as usize];
    tcp_stream.read_exact(&mut key_bytes)?;
    let public_key = serde_cbor::from_slice(&key_bytes)?;
    size_info.raw_size += 8 + key_len as u64;
    size_info.final_size += 8 + key_len as u64;

    Ok((
        InitialExchange {
            magic_number,
            version,
            public_key,
        },
        size_info,
    ))
}

#[allow(dead_code)]
pub struct InitialExchange {
    pub magic_number: u16,
    pub version: u16,
    pub public_key: PublicKey,
}
