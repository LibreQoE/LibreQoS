//! The SNMP transport boundary: how one request datagram actually reaches
//! the agent and its reply comes back. [`exchange`] owns the socket —
//! bind, connect, send, and one deadline-bounded receive;
//! [`exchange_retried`] adds the walk's single lost-datagram retry;
//! [`resolve_socket`] owns address parsing and resolution.
//!
//! This module is the whole socket surface of the SNMP client. Every wait
//! is an await point: a dropped or aborted caller ends the receive at
//! once, and each request carries its own explicit deadline, so a silent
//! peer surfaces as [`SnmpError::Timeout`] instead of a hang. One request
//! is outstanding per exchange (each exchange binds its own socket, as
//! the client always has). The async phase (P2) replaced the std UDP
//! loop here and left every parser, validator, and assembler — and their
//! tests — untouched.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::UdpSocket;

use crate::snmp::SnmpError;

/// Standard SNMP agent port, used when a target address carries no port.
const SNMP_PORT: u16 = 161;

/// Largest response datagram read from the agent.
const MAX_RESPONSE_BYTES: usize = 4096;

/// Sends `request` to `address` over a fresh UDP socket and returns the
/// single reply datagram. No retries: an expired transaction deadline is
/// [`SnmpError::Timeout`]. Dropping the returned future (or aborting its
/// task) cancels the wait immediately.
pub(crate) async fn exchange(
    address: &str,
    request: &[u8],
    timeout: Duration,
) -> Result<Vec<u8>, SnmpError> {
    let into_io = |source: std::io::Error| SnmpError::Io {
        address: address.to_owned(),
        source,
    };
    let target = resolve_socket(address).await?;
    let bind = if target.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let socket = UdpSocket::bind(bind).await.map_err(into_io)?;
    socket.connect(target).await.map_err(into_io)?;
    socket.send(request).await.map_err(into_io)?;
    let mut buffer = vec![0u8; MAX_RESPONSE_BYTES];
    match tokio::time::timeout(timeout, socket.recv(&mut buffer)).await {
        Ok(len) => {
            let len = len.map_err(into_io)?;
            buffer.truncate(len);
            Ok(buffer)
        }
        Err(_elapsed) => Err(SnmpError::Timeout {
            address: address.to_owned(),
            timeout,
        }),
    }
}

/// One walk step: [`exchange`] with the walk's lost-datagram policy — a
/// step whose reply times out is retried once, because one lost datagram
/// must not discard a multi-hundred-step walk. The original timeout error
/// stands when the retry also goes unanswered. Cancellation (dropping the
/// future) covers both the first wait and the retry.
pub(crate) async fn exchange_retried(
    address: &str,
    request: &[u8],
    timeout: Duration,
) -> Result<Vec<u8>, SnmpError> {
    match exchange(address, request, timeout).await {
        Ok(reply) => Ok(reply),
        Err(err @ SnmpError::Timeout { .. }) => {
            exchange(address, request, timeout).await.map_err(|_| err)
        }
        Err(err) => Err(err),
    }
}

/// Parses `address` as `host:port`, defaulting the port to the standard
/// SNMP port, and resolves hostnames via the runtime's resolver (numeric
/// addresses short-circuit before any lookup, as before).
async fn resolve_socket(address: &str) -> Result<SocketAddr, SnmpError> {
    let into_io = |source: std::io::Error| SnmpError::Io {
        address: address.to_owned(),
        source,
    };
    if let Ok(parsed) = address.parse() {
        return Ok(parsed);
    }
    let with_port = format!("{address}:{SNMP_PORT}");
    let mut addrs = tokio::net::lookup_host(with_port.as_str())
        .await
        .map_err(into_io)?;
    addrs.next().ok_or_else(|| {
        into_io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no addresses found",
        ))
    })
}
