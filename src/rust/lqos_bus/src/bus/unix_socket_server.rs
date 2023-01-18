use std::{fs::remove_file, ffi::CString};
use crate::{BUS_SOCKET_PATH, decode_request, BusReply, encode_response, BusRequest, BusResponse};
use anyhow::Result;
use nix::libc::mode_t;
use tokio::{net::{UnixListener, UnixStream}, io::{AsyncReadExt, AsyncWriteExt}};
use log::warn;

pub struct UnixSocketServer {}

impl UnixSocketServer {
    pub fn new() -> Result<Self> {
        Self::delete_local_socket()?;
        Ok(Self {})
    }

    fn delete_local_socket() -> Result<()> {
        let socket_path = std::path::Path::new(BUS_SOCKET_PATH);
        if socket_path.exists() {
            remove_file(socket_path)?;
        }
        Ok(())
    }

    fn make_socket_public() -> Result<()> {
        let unix_path = CString::new(BUS_SOCKET_PATH)?;
        unsafe {
            nix::libc::chmod(unix_path.as_ptr(), mode_t::from_le(666));
        }
        Ok(())
    }

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
                            responses: Vec::new(),
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