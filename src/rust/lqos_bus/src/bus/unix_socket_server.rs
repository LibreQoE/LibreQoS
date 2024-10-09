use crate::{
  decode_request, encode_response, BusReply, BusRequest, BusResponse,
  BUS_SOCKET_PATH,
};
use tracing::{debug, error, info, warn};
use std::{ffi::CString, fs::remove_file};
use thiserror::Error;
use tokio::{
  io::{AsyncReadExt, AsyncWriteExt},
  net::{UnixListener, UnixStream},
};

use super::BUS_SOCKET_DIRECTORY;

const READ_BUFFER_SIZE: usize = 20_480;

/// Implements a Tokio-friendly server using Unix Sockets and the bus protocol.
/// Requests are handled and then forwarded to the handler.
pub struct UnixSocketServer {}

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
      error!(
        "Unable to create C-compatible path string. This should never happen."
      );
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
    let _ = lqos_utils::run_success!(
      "/bin/chmod",
      "-R",
      "a+rwx",
      BUS_SOCKET_DIRECTORY
    );
    Ok(())
  }

  /// Start listening for bus traffic, forward requests to the `handle_bus_requests`
  /// function for procesing.
  pub async fn listen(
    &self,
    handle_bus_requests: fn(&[BusRequest], &mut Vec<BusResponse>),
    mut bus_rx: tokio::sync::mpsc::Receiver<(tokio::sync::oneshot::Sender<BusReply>, BusRequest)>,
  ) -> Result<(), UnixSocketServerError> {
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
            loop {
              let mut buf = vec![0; READ_BUFFER_SIZE];

              let bytes_read = socket.read(&mut buf).await;
              if bytes_read.is_err() {
                debug!("Unable to read from client socket. Server remains alive.");
                debug!("This is probably harmless.");
                debug!("{:?}", bytes_read);
                break; // Escape out of the thread
              }

              if let Ok(request) = decode_request(&buf) {
                let mut response = BusReply { responses: Vec::with_capacity(8) };
                handle_bus_requests(&request.requests, &mut response.responses);
                let _ =
                  reply_unix(&encode_response(&response).unwrap(), &mut socket)
                    .await;
                if !request.persist {
                  break;
                }
              } else {
                warn!("Invalid data on local socket");
                break;
              }
            }
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

async fn reply_unix(
  response: &[u8],
  socket: &mut UnixStream,
) -> Result<(), UnixSocketServerError> {
  let ret = socket.write_all(response).await;
  if ret.is_err() {
    debug!("Unable to write to UNIX socket. This is usually harmless, meaning the client went away.");
    debug!("{:?}", ret);
    return Err(UnixSocketServerError::WriteFail);
  };
  Ok(())
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
