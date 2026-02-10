// SPDX-FileCopyrightText: 2025 LibreQoE support@libreqos.io
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-LibreQoS-Exception

use super::{BusClientError, BusReply, BusSession};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tracing::error;

pub(crate) const BUS_CHUNK_SIZE: usize = 8 * 1024;
pub(crate) const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const CHUNK_LEN_BYTES: usize = 4;

pub(crate) fn encode_session_cbor(session: &BusSession) -> Result<Vec<u8>, BusClientError> {
    serde_cbor::to_vec(session).map_err(|e| {
        error!("Unable to serialize session to CBOR: {:?}", e);
        BusClientError::EncodingError
    })
}

pub(crate) fn decode_session_cbor(bytes: &[u8]) -> Result<BusSession, BusClientError> {
    serde_cbor::from_slice(bytes).map_err(|e| {
        error!("Unable to deserialize session from CBOR: {:?}", e);
        BusClientError::DecodingError
    })
}

pub(crate) fn encode_reply_cbor(reply: &BusReply) -> Result<Vec<u8>, BusClientError> {
    serde_cbor::to_vec(reply).map_err(|e| {
        error!("Unable to serialize reply to CBOR: {:?}", e);
        BusClientError::EncodingError
    })
}

pub(crate) fn decode_reply_cbor(bytes: &[u8]) -> Result<BusReply, BusClientError> {
    serde_cbor::from_slice(bytes).map_err(|e| {
        error!("Unable to deserialize reply from CBOR: {:?}", e);
        BusClientError::DecodingError
    })
}

pub(crate) async fn write_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    request_id: u64,
    payload: &[u8],
) -> Result<(), BusClientError> {
    if payload.len() > MAX_FRAME_BYTES {
        error!(
            "Payload size {} exceeds MAX_FRAME_BYTES {}.",
            payload.len(),
            MAX_FRAME_BYTES
        );
        return Err(BusClientError::EncodingError);
    }

    writer
        .write_u64_le(request_id)
        .await
        .map_err(|_| BusClientError::StreamWriteError)?;
    writer
        .write_u64_le(payload.len() as u64)
        .await
        .map_err(|_| BusClientError::StreamWriteError)?;

    if payload.is_empty() {
        return Ok(());
    }

    for chunk in payload.chunks(BUS_CHUNK_SIZE) {
        let chunk_len = u32::try_from(chunk.len()).map_err(|_| {
            error!("Chunk length exceeds u32 capacity.");
            BusClientError::EncodingError
        })?;
        let mut len_buf = [0u8; CHUNK_LEN_BYTES];
        len_buf.copy_from_slice(&chunk_len.to_le_bytes());
        writer
            .write_all(&len_buf)
            .await
            .map_err(|_| BusClientError::StreamWriteError)?;
        writer
            .write_all(chunk)
            .await
            .map_err(|_| BusClientError::StreamWriteError)?;
    }

    Ok(())
}

pub(crate) async fn read_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> Result<(u64, Vec<u8>), BusClientError> {
    let request_id = reader
        .read_u64_le()
        .await
        .map_err(|_| BusClientError::StreamReadError)?;
    let payload_len = reader
        .read_u64_le()
        .await
        .map_err(|_| BusClientError::StreamReadError)?;

    if payload_len == 0 {
        return Ok((request_id, Vec::new()));
    }

    let payload_len = usize::try_from(payload_len).map_err(|_| {
        error!("Payload size exceeds usize capacity.");
        BusClientError::DecodingError
    })?;
    if payload_len > MAX_FRAME_BYTES {
        error!(
            "Payload size {} exceeds MAX_FRAME_BYTES {}.",
            payload_len, MAX_FRAME_BYTES
        );
        return Err(BusClientError::DecodingError);
    }

    let mut payload = Vec::with_capacity(payload_len);
    let mut remaining = payload_len;
    let mut chunk_buf = vec![0u8; BUS_CHUNK_SIZE];

    while remaining > 0 {
        let mut len_buf = [0u8; CHUNK_LEN_BYTES];
        reader
            .read_exact(&mut len_buf)
            .await
            .map_err(|_| BusClientError::StreamReadError)?;
        let chunk_len = u32::from_le_bytes(len_buf) as usize;
        if chunk_len == 0 {
            error!("Chunk length of 0 is invalid for non-empty payloads.");
            return Err(BusClientError::DecodingError);
        }
        if chunk_len > BUS_CHUNK_SIZE {
            error!(
                "Chunk length {} exceeds BUS_CHUNK_SIZE {}.",
                chunk_len, BUS_CHUNK_SIZE
            );
            return Err(BusClientError::DecodingError);
        }
        if chunk_len > remaining {
            error!(
                "Chunk length {} exceeds remaining payload {}.",
                chunk_len, remaining
            );
            return Err(BusClientError::DecodingError);
        }
        reader
            .read_exact(&mut chunk_buf[..chunk_len])
            .await
            .map_err(|_| BusClientError::StreamReadError)?;
        payload.extend_from_slice(&chunk_buf[..chunk_len]);
        remaining -= chunk_len;
    }

    Ok((request_id, payload))
}

#[cfg(test)]
mod tests {
    use super::{
        BUS_CHUNK_SIZE, MAX_FRAME_BYTES, decode_reply_cbor, decode_session_cbor, encode_reply_cbor,
        encode_session_cbor, read_frame, write_frame,
    };
    use crate::{BusReply, BusRequest, BusResponse, BusSession, bus::BusClientError};
    use tokio::io::{AsyncWriteExt, duplex};

    #[test]
    fn cbor_round_trip_session() {
        let session = BusSession {
            requests: vec![BusRequest::Ping],
        };
        let bytes = encode_session_cbor(&session).expect("encode_session_cbor");
        let decoded = decode_session_cbor(&bytes).expect("decode_session_cbor");
        assert_eq!(decoded.requests, session.requests);
    }

    #[test]
    fn cbor_round_trip_reply() {
        let reply = BusReply {
            responses: vec![BusResponse::Ack],
        };
        let bytes = encode_reply_cbor(&reply).expect("encode_reply_cbor");
        let decoded = decode_reply_cbor(&bytes).expect("decode_reply_cbor");
        assert_eq!(decoded.responses, reply.responses);
    }

    #[tokio::test]
    async fn frame_round_trip_small_payload() {
        let (mut client, mut server) = duplex(128 * 1024);
        let payload = vec![0xAB; BUS_CHUNK_SIZE / 2];
        let expected = payload.clone();

        let write = async {
            write_frame(&mut client, 7, &payload)
                .await
                .expect("write_frame");
        };
        let read = async {
            read_frame(&mut server).await.expect("read_frame")
        };

        let (_, (request_id, read_payload)) = tokio::join!(write, read);
        assert_eq!(request_id, 7);
        assert_eq!(read_payload, expected);
    }

    #[tokio::test]
    async fn frame_round_trip_large_payload() {
        let (mut client, mut server) = duplex(256 * 1024);
        let payload = vec![0xCD; BUS_CHUNK_SIZE * 10 + 123];
        let expected = payload.clone();

        let write = async {
            write_frame(&mut client, 11, &payload)
                .await
                .expect("write_frame");
        };
        let read = async {
            read_frame(&mut server).await.expect("read_frame")
        };

        let (_, (request_id, read_payload)) = tokio::join!(write, read);
        assert_eq!(request_id, 11);
        assert_eq!(read_payload, expected);
    }

    #[tokio::test]
    async fn frame_rejects_oversized_on_write() {
        let (mut client, _server) = duplex(128 * 1024);
        let payload = vec![0xEF; MAX_FRAME_BYTES + 1];
        let result = write_frame(&mut client, 1, &payload).await;
        assert!(matches!(result, Err(BusClientError::EncodingError)));
    }

    #[tokio::test]
    async fn frame_rejects_oversized_on_read() {
        let (mut client, mut server) = duplex(128 * 1024);
        let write = async {
            client
                .write_u64_le(5)
                .await
                .expect("write request id");
            client
                .write_u64_le((MAX_FRAME_BYTES as u64) + 1)
                .await
                .expect("write oversized len");
        };
        let read = async {
            read_frame(&mut server).await
        };

        let (_, result) = tokio::join!(write, read);
        assert!(matches!(result, Err(BusClientError::DecodingError)));
    }

    #[tokio::test]
    async fn frame_multiple_back_to_back() {
        let (mut client, mut server) = duplex(256 * 1024);
        let payload_a = vec![0x01; BUS_CHUNK_SIZE + 5];
        let payload_b = vec![0x02; BUS_CHUNK_SIZE * 2 + 7];

        let write = async {
            write_frame(&mut client, 100, &payload_a)
                .await
                .expect("write_frame a");
            write_frame(&mut client, 101, &payload_b)
                .await
                .expect("write_frame b");
        };
        let read = async {
            let first = read_frame(&mut server).await.expect("read_frame a");
            let second = read_frame(&mut server).await.expect("read_frame b");
            (first, second)
        };

        let (_, ((id_a, data_a), (id_b, data_b))) = tokio::join!(write, read);
        assert_eq!(id_a, 100);
        assert_eq!(data_a, payload_a);
        assert_eq!(id_b, 101);
        assert_eq!(data_b, payload_b);
    }
}
