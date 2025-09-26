// SPDX-FileCopyrightText: 2025 LibreQoE support@libreqos.io
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-LibreQoS-Exception

use crate::{
    BUS_SOCKET_PATH, BusReply, BusRequest, BusResponse, BusSession,
    bus::client::{MAGIC_NUMBER, MAGIC_RESPONSE},
};
use std::{ffi::CString, fs::remove_file};
use thiserror::Error;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixListener,
};
use tracing::{debug, error, info, warn};

use super::BUS_SOCKET_DIRECTORY;

/// Implements a Tokio-friendly server using Unix Sockets and the bus protocol.
/// Requests are handled and then forwarded to the handler.
pub struct UnixSocketServer {}

static RECEIVE_BUFFER: tokio::sync::OnceCell<tokio::sync::Mutex<Vec<u8>>> =
    tokio::sync::OnceCell::const_new();
const RECEIVE_BUFFER_SIZE: usize = 1024 * 1024 * 512; // 512 MiB

impl UnixSocketServer {
    /// Creates a new `UnixSocketServer`. Will delete any pre-existing
    /// socket file.
    pub fn new() -> Result<Self, UnixSocketServerError> {
        Self::delete_local_socket()?;
        Self::check_directory()?;
        Self::path_permissions()?;
        Ok(Self {})
    }

    /// We can't guaranty that Drop will be called on a process exit
    /// (doing so is considered unsound), so provide a mechanism
    /// to explicitly call the cleanup for signal handling.
    pub fn signal_cleanup() {
        let _ = UnixSocketServer::delete_local_socket(); // Ignore result
    }

    fn check_directory() -> Result<(), UnixSocketServerError> {
        let dir_path = std::path::Path::new(BUS_SOCKET_DIRECTORY);
        if dir_path.exists() && dir_path.is_dir() {
            Ok(())
        } else {
            let ret = std::fs::create_dir(dir_path);
            if ret.is_err() {
                error!("Unable to create {}", dir_path.display());
                error!("{:?}", ret);
                return Err(UnixSocketServerError::MkDirFail);
            }
            Ok(())
        }
    }

    fn path_permissions() -> Result<(), UnixSocketServerError> {
        let unix_path = CString::new(BUS_SOCKET_DIRECTORY);
        if unix_path.is_err() {
            error!("Unable to create C-compatible path string. This should never happen.");
            return Err(UnixSocketServerError::CString);
        }
        let unix_path = unix_path.unwrap();
        unsafe {
            nix::libc::chmod(unix_path.as_ptr(), 777);
        }
        Ok(())
    }

    fn delete_local_socket() -> Result<(), UnixSocketServerError> {
        let socket_path = std::path::Path::new(BUS_SOCKET_PATH);
        if socket_path.exists() {
            let ret = remove_file(socket_path);
            if ret.is_err() {
                error!("Unable to remove {BUS_SOCKET_PATH}");
                return Err(UnixSocketServerError::RmDirFail);
            }
        }
        Ok(())
    }

    fn make_socket_public() -> Result<(), UnixSocketServerError> {
        let _ = lqos_utils::run_success!("/bin/chmod", "-R", "a+rwx", BUS_SOCKET_DIRECTORY);
        Ok(())
    }

    /// Start listening for bus traffic, forward requests to the `handle_bus_requests`
    /// function for procesing.
    pub async fn listen(
        &self,
        handle_bus_requests: fn(&[BusRequest], &mut Vec<BusResponse>),
        mut bus_rx: tokio::sync::mpsc::Receiver<(
            tokio::sync::oneshot::Sender<BusReply>,
            BusRequest,
        )>,
    ) -> Result<(), UnixSocketServerError> {
        RECEIVE_BUFFER
            .set(tokio::sync::Mutex::new(vec![0; RECEIVE_BUFFER_SIZE])) // 512 MiB
            .map_err(|_| UnixSocketServerError::MkDirFail)?;

        // Set up the listener and grant permissions to it
        let listener = UnixListener::bind(BUS_SOCKET_PATH);
        if listener.is_err() {
            error!("Unable to bind to {BUS_SOCKET_PATH}");
            error!("{:?}", listener);
            return Err(UnixSocketServerError::BindFail);
        }
        let listener = listener.unwrap();
        Self::make_socket_public()?;
        info!("Listening on: {}", BUS_SOCKET_PATH);
        loop {
            tokio::select!(
              ret = bus_rx.recv() => {
                // We received a channel-based message
                if let Some((reply_channel, msg)) = ret {
                  let mut response = BusReply { responses: Vec::with_capacity(8) };
                  handle_bus_requests(&[msg], &mut response.responses);
                  if let Err(e) = reply_channel.send(response) {
                      warn!("Unable to send response back to client: {:?}", e);
                  }
                }
              },
              ret = listener.accept() => {
                // We received a UNIX socket message
                if ret.is_err() {
                  error!("Unable to listen for requests on bound {BUS_SOCKET_PATH}");
                  error!("{:?}", ret);
                  return Err(UnixSocketServerError::ListenFail);
                }
                let (mut socket, _) = ret.unwrap();
                tokio::spawn(async move {
                    // Listen for the magic number
                    let mut magic_buf = [0; 4];
                    let bytes_read = socket.read_exact(&mut magic_buf).await;
                    if bytes_read.is_err() {
                        debug!("Unable to read magic number from client socket. Server remains alive.");
                        debug!("This is probably harmless.");
                        debug!("{:?}", bytes_read);
                        return;
                    }
                    if magic_buf != MAGIC_NUMBER {
                        warn!("Received invalid magic number from client socket.");
                        return;
                    }

                    // Send the magic number back to the client
                    if let Err(e) = socket.write_all(&MAGIC_RESPONSE).await {
                        debug!("Unable to write magic number to client socket. Server remains alive.");
                        debug!("This is probably harmless.");
                        debug!("{:?}", e);
                        return;
                    }

                    loop {
                        // Read the request ID (64 bits) and the Size (64 bits)
                        // This is the size of the request in bytes, not the number of requests
                        let mut id_buf = [0; 16];
                        let bytes_read = socket.read_exact(&mut id_buf).await;
                        if bytes_read.is_err() {
                            debug!("Unable to read request ID from client socket. Server remains alive.");
                            debug!("This is probably harmless.");
                            debug!("{:?}", bytes_read);
                            break; // Escape out of the thread
                        }
                        let request_id = u64::from_le_bytes(id_buf[0..8].try_into().unwrap());
                        let request_size = u64::from_le_bytes(id_buf[8..16].try_into().unwrap());
                        debug!("Received request ID: {request_id}, Size: {request_size}");
                        if request_size > RECEIVE_BUFFER_SIZE as u64 {
                            warn!("Received request size ({request_size}) exceeds buffer size ({RECEIVE_BUFFER_SIZE}).");
                            break; // Escape out of the thread
                        }

                        // Read the request data
                        //let mut buf = vec![0; request_size as usize];
                        let mut buf = RECEIVE_BUFFER.get().unwrap().lock().await;

                        let bytes_read = socket.read_exact(&mut buf[0 .. request_size as usize]).await;
                        if bytes_read.is_err() {
                            debug!("Unable to read request data from client socket. Server remains alive.");
                            debug!("This is probably harmless.");
                            debug!("{:?}", bytes_read);
                            break; // Escape out of the thread
                        }

                        // Decode the request
                        let Ok(request) = bincode::deserialize::<BusSession>(&buf[0..request_size as usize]) else {
                            warn!("Invalid data on local socket");
                            break;
                        };
                        debug!("Received request: {:?}", request);
                        drop(buf); // Release the lock on the buffer

                        // Handle the request and build the response
                        let mut response = BusReply { responses: Vec::with_capacity(8) };
                        handle_bus_requests(&request.requests, &mut response.responses);

                        // Encode the response
                        let Ok(encoded_response) = bincode::serialize(&response) else {
                            warn!("Unable to encode response for request ID: {request_id}");
                            break;
                        };
                        debug!("Sending response for request ID: {request_id}");

                        // Send the response back to the client
                        let response_size = encoded_response.len() as u64;
                        let mut response_buf = vec![0; 16 + encoded_response.len()];
                        response_buf[0..8].copy_from_slice(&request_id.to_le_bytes());
                        response_buf[8..16].copy_from_slice(&response_size.to_le_bytes());
                        response_buf[16..].copy_from_slice(&encoded_response);
                        if let Err(e) = socket.write_all(&response_buf).await {
                            debug!("Unable to write response to client socket. Server remains alive.");
                            debug!("This is probably harmless.");
                            debug!("{:?}", e);
                            break; // Escape out of the thread
                        }
                        debug!("Response sent for request ID: {request_id}");

                    } // End of request handling loop
                });
              },
            );
        }
        //Ok(()) // unreachable
    }
}

impl Drop for UnixSocketServer {
    fn drop(&mut self) {
        let _ = UnixSocketServer::delete_local_socket(); // Ignore result
    }
}

#[derive(Error, Debug)]
pub enum UnixSocketServerError {
    #[error("Unable to create directory")]
    MkDirFail,
    #[error("Unable to create C-Compatible String")]
    CString,
    #[error("Unable to remove directory")]
    RmDirFail,
    #[error("Cannot bind unix socket")]
    BindFail,
    #[error("Cannot listen to socket")]
    ListenFail,
    #[error("Unable to write to socket")]
    WriteFail,
}
