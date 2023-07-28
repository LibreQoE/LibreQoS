use std::collections::VecDeque;

use wasm_pipe_types::{WasmResponse, WasmRequest};
use web_sys::{WebSocket, BinaryType, ErrorEvent, MessageEvent};
use thiserror::Error;
use wasm_bindgen::prelude::*;
use crate::{message::{WsResponseMessage, WsRequestMessage}, onAuthOk, onAuthFail, onMessage, get_token};
use super::log;

static mut CONDUIT: Option<Conduit> = None;

#[wasm_bindgen]
pub fn initialize_wss(url: String) {
    log(&format!("Initializing WSS to: {url}"));
    unsafe {
        if CONDUIT.is_none() {
            CONDUIT = Some(Conduit::new(url));

            if let Some(conduit) = &mut CONDUIT {
                match conduit.connect() {
                    Ok(_) => log("Connection requested."),
                    Err(e) => log(&format!("Error connecting: {:?}", e)),
                }
            }
        } else {
            log("Conduit already initialized");
        }
    }
}

#[wasm_bindgen]
pub fn is_wasm_connected() -> bool {
    unsafe {
        if let Some(conduit) = &CONDUIT {
            conduit.is_connected()
        } else {
            false
        }
    }
}

#[wasm_bindgen]
pub fn send_wss_queue() {
    unsafe {
        if let Some(conduit) = &mut CONDUIT {
            conduit.send_queue();
        } else {
            log("No conduit");
        }
    }
}

pub fn send_message(msg: WasmRequest) {
    unsafe {
        if let Some(conduit) = &mut CONDUIT {
            conduit.enqueue_raw(msg);
        } else {
            log("No conduit");
        }
    }
}

#[derive(Error, Debug)]
enum WebSocketError {
    #[error("URL is empty")]
    NoURL,
    #[error("Already connected")]
    AlreadyConnected,
    #[error("WebSocket already exists")]
    AlreadyExists,
    #[error("WebSocket Creation Error")]
    CreationError,
}

#[derive(PartialEq, Eq)]
enum ConnectionStatus {
    New,
    Connected,
}

/// Handles WS connection to the server.
struct Conduit {
    status: ConnectionStatus,
    socket: Option<WebSocket>,
    url: String,
    message_queue: VecDeque<WsRequestMessage>,
}

impl Conduit {
    fn new(url: String) -> Self {
        Self {
            status: ConnectionStatus::New,
            socket: None,
            url,
            message_queue: VecDeque::new(),
        }
    }

    fn connect(&mut self) -> Result<(), WebSocketError> {
        // Precondition testing
        if self.url.is_empty() { return Err(WebSocketError::NoURL); }
        if self.status != ConnectionStatus::New { return Err(WebSocketError::AlreadyConnected); }
        if self.socket.is_some() { return Err(WebSocketError::AlreadyExists); }
        self.socket = Some(WebSocket::new(&self.url).map_err(|_| WebSocketError::CreationError)?);
        if let Some(socket) = &mut self.socket {
            socket.set_binary_type(BinaryType::Arraybuffer);
            
            // Wire up on_close
            let onclose_callback = Closure::<dyn FnMut(_)>::new(move |_e: ErrorEvent| {
                on_close();
            });
            socket.set_onclose(Some(onclose_callback.as_ref().unchecked_ref()));
            onclose_callback.forget();

            // Wire up on_error
            let onerror_callback = Closure::<dyn FnMut(_)>::new(move |e: ErrorEvent| {
                log(&format!("Error Received: {e:?}"));
                on_error()
            });
            socket.set_onerror(Some(onerror_callback.as_ref().unchecked_ref()));
            onerror_callback.forget();

            // Wire up on_open
            let onopen_callback = Closure::<dyn FnMut(_)>::new(move |_e: ErrorEvent| {
                log("Open Received");
                on_open();
            });
            socket.set_onopen(Some(onopen_callback.as_ref().unchecked_ref()));
            onopen_callback.forget();

            // Wire up on message
            let onmessage_callback = Closure::<dyn FnMut(_)>::new(move |e: MessageEvent| {
                log("Message Received");
                if let Ok(abuf) = e.data().dyn_into::<js_sys::ArrayBuffer>() {
                    let response = WsResponseMessage::from_array_buffer(abuf);
                    match response {
                        Err(e) => log(&format!("Error parsing message: {:?}", e)),
                        Ok(WsResponseMessage(msg)) => {
                            match msg {
                                WasmResponse::AuthOk { token, name, license_key } => {
                                    onAuthOk(token, name, license_key);
                                }
                                WasmResponse::AuthFail => {
                                    onAuthFail();
                                }
                                _ => {
                                    let json = serde_json::to_string(&msg).unwrap();
                                    onMessage(json);
                                }
                            }
                        }
                    }
                }
            });
            socket.set_onmessage(Some(onmessage_callback.as_ref().unchecked_ref()));
            onmessage_callback.forget();
        }

        Ok(())        
    }

    fn is_connected(&self) -> bool {
        self.status == ConnectionStatus::Connected
    }

    fn enqueue_raw(&mut self, message: WasmRequest) {
        let msg = WsRequestMessage::new(message);
        self.enqueue_message(msg);
    }

    fn enqueue_message(&mut self, message: WsRequestMessage) {
        self.message_queue.push_back(message);
    }

    fn send_queue(&mut self) {
        // Bail out if there's nothing to do
        if self.message_queue.is_empty() {
            return;
        }

        // Kill old messages, to avoid a flood on reconnect
        self.message_queue.retain(|msg| msg.submitted.elapsed().as_secs_f32() < 10.0);
        log(&format!("{} Enqueued Messages", self.message_queue.len()));

        // Send queued messages
        if let Some(ws) = &mut self.socket {
            while let Some(msg) = self.message_queue.pop_front() {
                let msg = msg.serialize();
                log("Message Serialized");
                match msg {
                    Ok(msg) => {
                        if let Err(e) = ws.send_with_u8_array(&msg) {
                            log(&format!("Error sending message: {e:?}"));
                            self.status = ConnectionStatus::New;
                            break;
                        }
                    }
                    Err(e) => {
                        log(&format!("Serialization error: {e:?}"));
                    }
                }
                
            }
        } else {
            log("No WebSocket connection");
            let _  = self.connect();
        }
    }
}

fn on_close() {
    unsafe {
        if let Some(conduit) = &mut CONDUIT {
            conduit.socket = None;
            conduit.status = ConnectionStatus::New;
        }
    }
}

fn on_error() {
    unsafe {
        if let Some(conduit) = &mut CONDUIT {
            conduit.socket = None;
            conduit.status = ConnectionStatus::New;
        }
    }
}

fn on_open() {
    unsafe {
        if let Some(conduit) = &mut CONDUIT {
            conduit.status = ConnectionStatus::Connected;
            conduit.enqueue_raw(WasmRequest::Auth { token: get_token() });
        }
    }
}