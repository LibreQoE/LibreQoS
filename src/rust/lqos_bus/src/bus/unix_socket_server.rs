use std::{fs::remove_file, ffi::CString};
use crate::{BUS_SOCKET_PATH, decode_request, BusReply, encode_response, BusRequest, BusResponse};
use anyhow::Result;
use tokio::{net::{UnixListener, UnixStream}, io::{AsyncReadExt, AsyncWriteExt}};
use log::warn;

use super::BUS_SOCKET_DIRECTORY;

/// Implements a Tokio-friendly server using Unix Sockets and the bus protocol.
/// Requests are handled and then forwarded to the handler.
pub struct UnixSocketServer {}

impl UnixSocketServer {
    /// Creates a new `UnixSocketServer`. Will delete any pre-existing
    /// socket file.
    pub fn new() -> Result<Self> {
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

    fn check_directory() -> Result<()> {
        let dir_path = std::path::Path::new(BUS_SOCKET_DIRECTORY);
        if dir_path.exists() && dir_path.is_dir() {
            Ok(())
        } else {
            std::fs::create_dir(dir_path)?;            
            Ok(())
        }
    }

    fn path_permissions() -> Result<()> {
        let unix_path = CString::new(BUS_SOCKET_DIRECTORY)?;
        unsafe {
            nix::libc::chmod(unix_path.as_ptr(), 777);
        }
        Ok(())
    }

    fn delete_local_socket() -> Result<()> {
        let socket_path = std::path::Path::new(BUS_SOCKET_PATH);
        if socket_path.exists() {
            remove_file(socket_path)?;
        }
        Ok(())
    }

    fn make_socket_public() -> Result<()> {
        lqos_utils::run_success!("/bin/chmod", "-R", "a+rwx", BUS_SOCKET_DIRECTORY);
        Ok(())
    }

    /// Start listening for bus traffic, forward requests to the `handle_bus_requests`
    /// function for procesing.
    pub async fn listen(&self, handle_bus_requests: fn(&[BusRequest], &mut Vec<BusResponse>)) -> Result<()> 
    {
        // Setup the listener and grant permissions to it
        let listener = UnixListener::bind(BUS_SOCKET_PATH)?;
        Self::make_socket_public()?;
        warn!("Listening on: {}", BUS_SOCKET_PATH);
        loop {
            let (mut socket, _) = listener.accept().await?;            
            tokio::spawn(async move {
                loop {
                    let mut buf = vec![0; 1024];

                    let _bytes_read = socket
                        .read(&mut buf)
                        .await
                        .expect("failed to read data from socket");

                    if let Ok(request) = decode_request(&buf) {
                        let mut response = BusReply {
                            responses: Vec::with_capacity(8),
                        };
                        handle_bus_requests(&request.requests, &mut response.responses);
                        let _ = reply_unix(&encode_response(&response).unwrap(), &mut socket).await;
                        if !request.persist {
                            break;
                        }
                    } else {
                        warn!("Invalid data on local socket");
                        break;
                    }
                }
            });
        }
        //Ok(()) // unreachable
    }
}

impl Drop for UnixSocketServer {
    fn drop(&mut self) {
        let _ = UnixSocketServer::delete_local_socket(); // Ignore result
    }
}

async fn reply_unix(response: &[u8], socket: &mut UnixStream) -> Result<()> {
    socket.write_all(&response).await?;
    Ok(())
}