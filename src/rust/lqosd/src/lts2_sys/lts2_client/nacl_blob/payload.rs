use dryoc::dryocbox::{NewByteArray, Nonce, PublicKey};
use serde::Serialize;
use serde::de::DeserializeOwned;
use crate::lts2_sys::lts2_client::nacl_blob::KeyStore;
use crate::lts2_sys::lts2_client::nacl_blob::size_info::SizeInfo;

pub fn transmit_payload<T: Serialize>(
    our_keys: &KeyStore,
    their_public_key: &PublicKey,
    payload: &T,
    tcp_stream: &mut std::net::TcpStream,
) -> anyhow::Result<SizeInfo> {
    use std::io::Write;
    let mut size_info = SizeInfo::default();

    // Make the nonce, serialize it
    let nonce = Nonce::gen();
    let serialized_nonce = serde_cbor::to_vec(&nonce)?;
    let compressed_nonce = miniz_oxide::deflate::compress_to_vec(&serialized_nonce, 10);
    // Transmit the nonce size, then the nonce
    tcp_stream.write_all(&(compressed_nonce.len() as u32).to_be_bytes())?;
    tcp_stream.write_all(&compressed_nonce)?;
    size_info.raw_size += 4 + compressed_nonce.len() as u64;
    size_info.final_size += 4 + compressed_nonce.len() as u64;

    // Encrypt and compress the payload
    let serialized_payload = serde_cbor::to_vec(payload)?;
    let compressed_bytes = miniz_oxide::deflate::compress_to_vec(&serialized_payload, 10);
    let dryocbox = dryoc::dryocbox::DryocBox::encrypt_to_vecbox(
        &compressed_bytes,
        &nonce,
        their_public_key,
        &our_keys.keys.secret_key,
    )?;
    let encrypted_bytes = dryocbox.to_vec();

    // Transmit the size of the encrypted payload, then the payload
    tcp_stream.write_all(&(encrypted_bytes.len() as u64).to_be_bytes())?;
    tcp_stream.write_all(&encrypted_bytes)?;
    tcp_stream.flush()?;
    size_info.final_size += 8 + serialized_payload.len() as u64;
    size_info.raw_size += 8 + encrypted_bytes.len() as u64;
    Ok(size_info)
}

pub fn receive_payload<T: DeserializeOwned>(
    our_keys: &KeyStore,
    their_public_key: &PublicKey,
    tcp_stream: &mut std::net::TcpStream,
) -> anyhow::Result<(T, SizeInfo)> {
    use std::io::Read;
    
    let mut size_info = SizeInfo::default();
    // Receive the size of the nonce (u32)
    let mut nonce_size_bytes = [0; 4];
    tcp_stream.read_exact(&mut nonce_size_bytes)?;
    let nonce_size = u32::from_be_bytes(nonce_size_bytes);
    // Receive the nonce
    let mut nonce_bytes = vec![0; nonce_size as usize];
    tcp_stream.read_exact(&mut nonce_bytes)?;
    let decompressed_nonce = miniz_oxide::inflate::decompress_to_vec(&nonce_bytes)
        .map_err(|e| {
            println!("Failed to decompress nonce: {:?}", e);
            anyhow::Error::msg("Failed to decompress nonce")
        })?;
    let nonce: Nonce = serde_cbor::from_slice(&decompressed_nonce)?;
    size_info.raw_size += 4 + nonce_size as u64;
    size_info.final_size += 4 + nonce_size as u64;

    // Receive the size of the encrypted payload (u64)
    let mut encrypted_size_bytes = [0; 8];
    tcp_stream.read_exact(&mut encrypted_size_bytes)?;
    let encrypted_size = u64::from_be_bytes(encrypted_size_bytes);
    // Receive the encrypted payload
    let mut encrypted_bytes = vec![0; encrypted_size as usize];
    tcp_stream.read_exact(&mut encrypted_bytes)?;
    let salted_box = dryoc::dryocbox::DryocBox::from_bytes(&encrypted_bytes)?;
    let decrypted_bytes = salted_box.decrypt_to_vec(
        &nonce,
        their_public_key,
        &our_keys.keys.secret_key,
    )?;
    let decompressed_bytes = miniz_oxide::inflate::decompress_to_vec(&decrypted_bytes)
        .map_err(|e| {
            println!("Failed to decompress nonce: {:?}", e);
            anyhow::Error::msg("Failed to decompress nonce")
        })?;
    let result: T = serde_cbor::from_slice(&decompressed_bytes)?;
    size_info.raw_size += 8 + decompressed_bytes.len() as u64;
    size_info.final_size += 8 + encrypted_size;
    Ok((result, size_info))
}