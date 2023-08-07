use thiserror::Error;
use wasm_pipe_types::{WasmRequest, WasmResponse};

#[derive(Error, Debug)]
pub enum MessageError {
    #[error("Unable to decompress stream")]
    DecompressError,
    #[error("Unable to de-serialize CBOR into native type")]
    DeserializeError,
    #[error("Unable to serialize CBOR from native type")]
    SerializeError,
}

pub struct WsResponseMessage(pub WasmResponse);

impl WsResponseMessage {
    pub fn from_array_buffer(buffer: js_sys::ArrayBuffer) -> Result<Self, MessageError> {
        // Convert the array buffer into a strongly typed buffer
        let array = js_sys::Uint8Array::new(&buffer);
        let raw = array.to_vec();
        let decompressed = miniz_oxide::inflate::decompress_to_vec(&raw)
            .map_err(|_| MessageError::DecompressError)?;
        let msg: WasmResponse =
            serde_cbor::from_slice(&decompressed).map_err(|_| MessageError::DeserializeError)?;
        Ok(Self(msg))
    }
}

pub struct WsRequestMessage(pub WasmRequest);

impl WsRequestMessage {
    pub fn new(msg: WasmRequest) -> Self {
        Self(msg)
    }

    pub fn serialize(&self) -> Result<Vec<u8>, MessageError> {
        let cbor = serde_cbor::to_vec(&self.0).map_err(|_| MessageError::SerializeError)?;
        Ok(miniz_oxide::deflate::compress_to_vec(&cbor, 8))
    }
}
