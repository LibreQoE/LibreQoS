// SPDX-FileCopyrightText: 2025 LibreQoE support@libreqos.io
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-LibreQoS-Exception

use crate::{
    BUS_SOCKET_PATH, BusReply, BusRequest, BusResponse,
    bus::client::{MAGIC_NUMBER, MAGIC_RESPONSE},
};
use std::{
    collections::BTreeMap,
    ffi::CString,
    fmt::Write,
    fs::remove_file,
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixListener,
    sync::{OwnedSemaphorePermit, Semaphore},
    task::spawn_blocking,
    time::timeout,
};
use tracing::{debug, error, info, warn};

use super::BUS_SOCKET_DIRECTORY;
use super::protocol::{decode_session_cbor, encode_reply_cbor, read_frame, write_frame};

const BUS_HANDLER_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_CONCURRENT_BUS_HANDLERS: usize = 16;

#[derive(Clone)]
struct BusHandlerLimiter {
    permits: std::sync::Arc<Semaphore>,
    limit: usize,
}

impl BusHandlerLimiter {
    fn new(limit: usize) -> Self {
        Self {
            permits: std::sync::Arc::new(Semaphore::new(limit)),
            limit,
        }
    }

    fn try_acquire(&self) -> Option<OwnedSemaphorePermit> {
        self.permits.clone().try_acquire_owned().ok()
    }

    fn active_handlers(&self) -> usize {
        self.limit.saturating_sub(self.permits.available_permits())
    }
}

fn dropped_reply_response_count(reply: &BusReply) -> usize {
    reply.responses.len()
}

fn timeout_reply(request_count: usize) -> BusReply {
    BusReply {
        responses: (0..request_count)
            .map(|_| BusResponse::Fail("Bus request handler timed out".to_string()))
            .collect(),
    }
}

fn busy_reply(request_count: usize) -> BusReply {
    BusReply {
        responses: (0..request_count)
            .map(|_| BusResponse::Fail("Bus request handler busy".to_string()))
            .collect(),
    }
}

fn request_kind_summary(request_kinds: &[&'static str]) -> String {
    if request_kinds.is_empty() {
        return "<none>".to_string();
    }

    let mut counts = BTreeMap::new();
    for kind in request_kinds {
        *counts.entry(*kind).or_insert(0usize) += 1;
    }

    let mut summary = String::new();
    for (idx, (kind, count)) in counts.into_iter().enumerate() {
        if idx > 0 {
            summary.push_str(", ");
        }
        let _ = write!(summary, "{kind}={count}");
    }
    summary
}

async fn handle_requests_with_deadline(
    handle_bus_requests: fn(&[BusRequest], &mut Vec<BusResponse>),
    requests: Vec<BusRequest>,
    request_source: &'static str,
    limiter: &BusHandlerLimiter,
) -> BusReply {
    handle_requests_with_limiter_for_duration(
        handle_bus_requests,
        requests,
        request_source,
        BUS_HANDLER_TIMEOUT,
        limiter,
    )
    .await
}

#[cfg(test)]
async fn handle_requests_with_deadline_for_duration(
    handle_bus_requests: fn(&[BusRequest], &mut Vec<BusResponse>),
    requests: Vec<BusRequest>,
    request_source: &'static str,
    timeout_duration: Duration,
) -> BusReply {
    let limiter = BusHandlerLimiter::new(MAX_CONCURRENT_BUS_HANDLERS);
    handle_requests_with_limiter_for_duration(
        handle_bus_requests,
        requests,
        request_source,
        timeout_duration,
        &limiter,
    )
    .await
}

async fn handle_requests_with_limiter_for_duration(
    handle_bus_requests: fn(&[BusRequest], &mut Vec<BusResponse>),
    requests: Vec<BusRequest>,
    request_source: &'static str,
    timeout_duration: Duration,
    limiter: &BusHandlerLimiter,
) -> BusReply {
    let request_count = requests.len();
    let request_kinds = requests.iter().map(BusRequest::kind).collect::<Vec<_>>();
    let can_fail_fast = requests.iter().all(BusRequest::can_fail_fast_on_timeout);
    let Some(permit) = limiter.try_acquire() else {
        warn!(
            source = request_source,
            request_count,
            request_kinds = %request_kind_summary(&request_kinds),
            active_handlers = limiter.active_handlers(),
            handler_limit = limiter.limit,
            "Bus request handler capacity exhausted"
        );
        return busy_reply(request_count);
    };
    let start = Instant::now();
    let mut handler = spawn_blocking(move || {
        let _permit = permit;
        let mut response = BusReply {
            responses: Vec::with_capacity(request_count),
        };
        handle_bus_requests(&requests, &mut response.responses);
        response
    });

    if can_fail_fast {
        return match timeout(timeout_duration, handler).await {
            Ok(result) => handle_bus_result(
                result,
                request_source,
                request_count,
                &request_kinds,
                "Bus request handler task failed",
            ),
            Err(_) => {
                warn!(
                    source = request_source,
                    request_count,
                    request_kinds = %request_kind_summary(&request_kinds),
                    elapsed_ms = start.elapsed().as_millis(),
                    timeout_ms = timeout_duration.as_millis(),
                    "Bus request handler timed out"
                );
                timeout_reply(request_count)
            }
        };
    }

    tokio::select! {
        result = &mut handler => {
            handle_bus_result(
                result,
                request_source,
                request_count,
                &request_kinds,
                "Side-effecting bus request handler task failed",
            )
        }
        _ = tokio::time::sleep(timeout_duration) => {
            warn!(
                source = request_source,
                request_count,
                request_kinds = %request_kind_summary(&request_kinds),
                elapsed_ms = start.elapsed().as_millis(),
                timeout_ms = timeout_duration.as_millis(),
                "Side-effecting bus request handler exceeded deadline; waiting for completion"
            );
            handle_bus_result(
                handler.await,
                request_source,
                request_count,
                &request_kinds,
                "Side-effecting bus request handler task failed after deadline",
            )
        }
    }
}

fn handle_bus_result(
    result: Result<BusReply, tokio::task::JoinError>,
    request_source: &'static str,
    request_count: usize,
    request_kinds: &[&'static str],
    failure_message: &'static str,
) -> BusReply {
    match result {
        Ok(response) => response,
        Err(err) => {
            warn!(
                source = request_source,
                request_count,
                request_kinds = %request_kind_summary(request_kinds),
                error = %err,
                "{failure_message}"
            );
            BusReply {
                responses: (0..request_count)
                    .map(|_| BusResponse::Fail("Bus request handler failed".to_string()))
                    .collect(),
            }
        }
    }
}

/// Implements a Tokio-friendly server using Unix Sockets and the bus protocol.
/// Requests are handled and then forwarded to the handler.
pub struct UnixSocketServer {
    handler_limiter: BusHandlerLimiter,
}

impl UnixSocketServer {
    /// Creates a new `UnixSocketServer`. Will delete any pre-existing
    /// socket file.
    pub fn new() -> Result<Self, UnixSocketServerError> {
        Self::delete_local_socket()?;
        Self::check_directory()?;
        Self::path_permissions()?;
        Ok(Self {
            handler_limiter: BusHandlerLimiter::new(MAX_CONCURRENT_BUS_HANDLERS),
        })
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
        let Ok(unix_path) = unix_path else {
            if unix_path.is_err() {
                error!("Unable to create C-compatible path string. This should never happen.");
            }
            return Err(UnixSocketServerError::CString);
        };
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
        // Set up the listener and grant permissions to it
        let listener = UnixListener::bind(BUS_SOCKET_PATH);
        let Ok(listener) = listener else {
            if listener.is_err() {
                error!("Unable to bind to {BUS_SOCKET_PATH}");
                error!("{:?}", listener);
            }
            return Err(UnixSocketServerError::BindFail);
        };
        Self::make_socket_public()?;
        info!("Listening on: {}", BUS_SOCKET_PATH);
        loop {
            tokio::select!(
              ret = bus_rx.recv() => {
                // We received a channel-based message
                if let Some((reply_channel, msg)) = ret {
                  let response = handle_requests_with_deadline(
                      handle_bus_requests,
                      vec![msg],
                      "internal_channel",
                      &self.handler_limiter,
                  ).await;
                  if let Err(reply) = reply_channel.send(response) {
                      warn!(
                          dropped_response_count = dropped_reply_response_count(&reply),
                          "Unable to send response back to client; receiver dropped"
                      );
                  }
                }
              },
              ret = listener.accept() => {
                // We received a UNIX socket message
                let Ok((mut socket, _)) = ret else {
                    if ret.is_err() {
                      error!("Unable to listen for requests on bound {BUS_SOCKET_PATH}");
                      error!("{:?}", ret);
                    }
                    return Err(UnixSocketServerError::ListenFail);
                };
                let handler_limiter = self.handler_limiter.clone();
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
                        let (request_id, request_bytes) = match read_frame(&mut socket).await {
                            Ok(frame) => frame,
                            Err(e) => {
                                debug!("Unable to read request frame from client socket.");
                                debug!("This is probably harmless.");
                                debug!("{:?}", e);
                                break;
                            }
                        };
                        if request_bytes.is_empty() {
                            warn!("Received empty request payload; closing client socket.");
                            break;
                        }
                        debug!(
                            "Received request ID: {request_id}, Size: {}",
                            request_bytes.len()
                        );

                        // Decode the request
                        let Ok(request) = decode_session_cbor(&request_bytes) else {
                            warn!("Invalid data on local socket");
                            break;
                        };
                        debug!("Received request: {:?}", request);

                        // Handle the request and build the response
                        let response = handle_requests_with_deadline(
                            handle_bus_requests,
                            request.requests,
                            "unix_socket",
                            &handler_limiter,
                        )
                        .await;

                        // Encode the response
                        let Ok(encoded_response) = encode_reply_cbor(&response) else {
                            warn!("Unable to encode response for request ID: {request_id}");
                            break;
                        };
                        debug!("Sending response for request ID: {request_id}");

                        // Send the response back to the client
                        if let Err(e) =
                            write_frame(&mut socket, request_id, &encoded_response).await
                        {
                            debug!("Unable to write response to client socket. Server remains alive.");
                            debug!("This is probably harmless.");
                            debug!("{:?}", e);
                            break; // Escape out of the thread
                        }
                        debug!("Response sent for request ID: {request_id}");

                    } // End of the request handling loop
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

#[cfg(test)]
mod tests {
    use super::{
        BusHandlerLimiter, MAX_CONCURRENT_BUS_HANDLERS, dropped_reply_response_count,
        handle_requests_with_deadline_for_duration, handle_requests_with_limiter_for_duration,
        request_kind_summary,
    };
    use crate::{BusReply, BusRequest, BusResponse};
    use std::sync::{
        Barrier, OnceLock,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;

    static TIMEOUT_HANDLER_INVOCATIONS: AtomicUsize = AtomicUsize::new(0);
    static BURST_HANDLER_INVOCATIONS: AtomicUsize = AtomicUsize::new(0);
    static BURST_ACTIVE_HANDLERS: AtomicUsize = AtomicUsize::new(0);
    static BURST_MAX_ACTIVE_HANDLERS: AtomicUsize = AtomicUsize::new(0);

    fn timeout_handler_started() -> &'static Barrier {
        static BARRIER: OnceLock<Barrier> = OnceLock::new();
        BARRIER.get_or_init(|| Barrier::new(2))
    }

    fn timeout_handler_release() -> &'static Barrier {
        static BARRIER: OnceLock<Barrier> = OnceLock::new();
        BARRIER.get_or_init(|| Barrier::new(2))
    }

    fn burst_handlers_started() -> &'static Barrier {
        static BARRIER: OnceLock<Barrier> = OnceLock::new();
        BARRIER.get_or_init(|| Barrier::new(MAX_CONCURRENT_BUS_HANDLERS + 1))
    }

    fn burst_handlers_release() -> &'static Barrier {
        static BARRIER: OnceLock<Barrier> = OnceLock::new();
        BARRIER.get_or_init(|| Barrier::new(MAX_CONCURRENT_BUS_HANDLERS + 1))
    }

    fn coordinated_timeout_handler(_requests: &[BusRequest], responses: &mut Vec<BusResponse>) {
        TIMEOUT_HANDLER_INVOCATIONS.fetch_add(1, Ordering::SeqCst);
        timeout_handler_started().wait();
        timeout_handler_release().wait();
        responses.push(BusResponse::Ack);
    }

    fn coordinated_burst_handler(_requests: &[BusRequest], responses: &mut Vec<BusResponse>) {
        BURST_HANDLER_INVOCATIONS.fetch_add(1, Ordering::SeqCst);
        let active = BURST_ACTIVE_HANDLERS.fetch_add(1, Ordering::SeqCst) + 1;
        BURST_MAX_ACTIVE_HANDLERS.fetch_max(active, Ordering::SeqCst);
        burst_handlers_started().wait();
        burst_handlers_release().wait();
        BURST_ACTIVE_HANDLERS.fetch_sub(1, Ordering::SeqCst);
        responses.push(BusResponse::Ack);
    }

    #[test]
    fn dropped_reply_summary_only_counts_responses() {
        let reply = BusReply {
            responses: vec![BusResponse::Ack, BusResponse::Ack, BusResponse::Ack],
        };

        assert_eq!(dropped_reply_response_count(&reply), 3);
    }

    #[test]
    fn request_kind_summary_counts_by_request_type() {
        assert_eq!(
            request_kind_summary(&["Ping", "GetNetworkMap", "Ping"]),
            "GetNetworkMap=1, Ping=2"
        );
    }

    #[tokio::test]
    async fn request_handler_success_returns_handler_responses_in_order() {
        fn success_handler(requests: &[BusRequest], responses: &mut Vec<BusResponse>) {
            for request in requests {
                responses.push(BusResponse::Fail(format!("handled {}", request.kind())));
            }
        }

        let response = handle_requests_with_deadline_for_duration(
            success_handler,
            vec![BusRequest::Ping, BusRequest::GetCurrentThroughput],
            "test",
            Duration::from_millis(50),
        )
        .await;

        assert_eq!(
            response.responses,
            vec![
                BusResponse::Fail("handled Ping".to_string()),
                BusResponse::Fail("handled GetCurrentThroughput".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn request_handler_timeout_returns_failure_per_request() {
        fn slow_handler(_requests: &[BusRequest], _responses: &mut Vec<BusResponse>) {
            std::thread::sleep(Duration::from_millis(50));
        }

        let response = handle_requests_with_deadline_for_duration(
            slow_handler,
            vec![BusRequest::Ping, BusRequest::GetCurrentThroughput],
            "test",
            Duration::from_millis(5),
        )
        .await;

        assert_eq!(
            response.responses,
            vec![
                BusResponse::Fail("Bus request handler timed out".to_string()),
                BusResponse::Fail("Bus request handler timed out".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn timed_out_handler_retains_capacity_until_it_finishes() {
        fn fast_handler(_requests: &[BusRequest], responses: &mut Vec<BusResponse>) {
            responses.push(BusResponse::Ack);
        }

        TIMEOUT_HANDLER_INVOCATIONS.store(0, Ordering::SeqCst);
        let limiter = BusHandlerLimiter::new(1);
        let timed_limiter = limiter.clone();
        let timed_handler = tokio::spawn(async move {
            handle_requests_with_limiter_for_duration(
                coordinated_timeout_handler,
                vec![BusRequest::Ping],
                "test",
                Duration::from_millis(10),
                &timed_limiter,
            )
            .await
        });
        tokio::task::spawn_blocking(|| timeout_handler_started().wait())
            .await
            .expect("test coordinator should join");
        let timed_out = timed_handler.await.expect("timed handler should join");
        assert_eq!(
            timed_out.responses,
            vec![BusResponse::Fail(
                "Bus request handler timed out".to_string()
            )]
        );

        let rejected = handle_requests_with_limiter_for_duration(
            fast_handler,
            vec![BusRequest::Ping],
            "test",
            Duration::from_millis(5),
            &limiter,
        )
        .await;
        assert_eq!(
            rejected.responses,
            vec![BusResponse::Fail("Bus request handler busy".to_string())]
        );
        assert_eq!(TIMEOUT_HANDLER_INVOCATIONS.load(Ordering::SeqCst), 1);

        tokio::task::spawn_blocking(|| timeout_handler_release().wait())
            .await
            .expect("test coordinator should join");
        tokio::time::timeout(Duration::from_secs(1), async {
            while limiter.active_handlers() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("timed-out handler should eventually release capacity");
        let recovered = handle_requests_with_limiter_for_duration(
            fast_handler,
            vec![BusRequest::Ping],
            "test",
            Duration::from_millis(20),
            &limiter,
        )
        .await;
        assert_eq!(recovered.responses, vec![BusResponse::Ack]);
    }

    #[tokio::test]
    async fn bus_handler_limiter_caps_running_handlers_at_configured_limit() {
        BURST_HANDLER_INVOCATIONS.store(0, Ordering::SeqCst);
        BURST_ACTIVE_HANDLERS.store(0, Ordering::SeqCst);
        BURST_MAX_ACTIVE_HANDLERS.store(0, Ordering::SeqCst);
        let limiter = BusHandlerLimiter::new(MAX_CONCURRENT_BUS_HANDLERS);
        let mut handlers = Vec::with_capacity(MAX_CONCURRENT_BUS_HANDLERS);
        for _ in 0..MAX_CONCURRENT_BUS_HANDLERS {
            let handler_limiter = limiter.clone();
            handlers.push(tokio::spawn(async move {
                handle_requests_with_limiter_for_duration(
                    coordinated_burst_handler,
                    vec![BusRequest::Ping],
                    "test",
                    Duration::from_secs(5),
                    &handler_limiter,
                )
                .await
            }));
        }

        tokio::task::spawn_blocking(|| burst_handlers_started().wait())
            .await
            .expect("test coordinator should join");
        assert_eq!(limiter.active_handlers(), MAX_CONCURRENT_BUS_HANDLERS);
        assert_eq!(
            BURST_HANDLER_INVOCATIONS.load(Ordering::SeqCst),
            MAX_CONCURRENT_BUS_HANDLERS
        );

        let rejected = handle_requests_with_limiter_for_duration(
            coordinated_burst_handler,
            vec![BusRequest::Ping],
            "test",
            Duration::from_millis(10),
            &limiter,
        )
        .await;
        assert_eq!(
            rejected.responses,
            vec![BusResponse::Fail("Bus request handler busy".to_string())]
        );
        assert_eq!(
            BURST_HANDLER_INVOCATIONS.load(Ordering::SeqCst),
            MAX_CONCURRENT_BUS_HANDLERS
        );

        tokio::task::spawn_blocking(|| burst_handlers_release().wait())
            .await
            .expect("test coordinator should join");
        for handler in handlers {
            let response = handler.await.expect("burst handler should join");
            assert_eq!(response.responses, vec![BusResponse::Ack]);
        }
        assert_eq!(limiter.active_handlers(), 0);
        assert_eq!(BURST_ACTIVE_HANDLERS.load(Ordering::SeqCst), 0);
        assert_eq!(
            BURST_MAX_ACTIVE_HANDLERS.load(Ordering::SeqCst),
            MAX_CONCURRENT_BUS_HANDLERS
        );
    }

    #[tokio::test]
    async fn side_effecting_request_waits_for_handler_after_deadline() {
        fn slow_mutating_handler(_requests: &[BusRequest], responses: &mut Vec<BusResponse>) {
            std::thread::sleep(Duration::from_millis(20));
            responses.push(BusResponse::Ack);
        }

        let response = handle_requests_with_deadline_for_duration(
            slow_mutating_handler,
            vec![BusRequest::ClearHotCache],
            "test",
            Duration::from_millis(5),
        )
        .await;

        assert_eq!(response.responses, vec![BusResponse::Ack]);
    }

    #[tokio::test]
    async fn request_handler_panic_returns_failure_per_request() {
        fn panic_handler(_requests: &[BusRequest], _responses: &mut Vec<BusResponse>) {
            panic!("bus handler test panic");
        }

        let response = handle_requests_with_deadline_for_duration(
            panic_handler,
            vec![BusRequest::Ping],
            "test",
            Duration::from_millis(50),
        )
        .await;

        assert_eq!(
            response.responses,
            vec![BusResponse::Fail("Bus request handler failed".to_string())]
        );
    }
}
