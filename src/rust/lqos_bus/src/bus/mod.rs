// SPDX-FileCopyrightText: 2025 LibreQoE support@libreqos.io
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-LibreQoS-Exception

mod client;
mod protocol;
mod queue_data;
mod reply;
mod request;
pub mod response;
mod session;
mod unix_socket_server;
pub use client::{LibreqosBusClient, bus_request};
pub use queue_data::*;
pub use reply::BusReply;
pub use request::{BlackboardSystem, BusRequest, TopFlowType, UrgentSeverity, UrgentSource};
#[allow(unused_imports)]
pub use response::{
    AsnHeatmapData, BakeryStatsSnapshot, BusResponse, CircuitHeatmapData, SiteHeatmapData,
    StormguardDebugDirection, StormguardDebugEntry, UrgentIssue,
};
pub use session::BusSession;
use thiserror::Error;
pub use unix_socket_server::UnixSocketServer;

/// The local socket path to which `lqosd` will bind itself,
/// listening for requets.
pub const BUS_SOCKET_PATH: &str = "/run/lqos/bus";

/// The directory containing the bus socket. Used for ensuring
/// that the directory exists.
pub(crate) const BUS_SOCKET_DIRECTORY: &str = "/run/lqos";

/// Errors returned by local bus clients while connecting to or communicating
/// with the `lqosd` Unix socket.
#[derive(Error, Debug)]
pub enum BusClientError {
    /// The local bus socket does not yet exist or is not reachable.
    #[error(
        "Socket (typically /run/lqos/bus) not found. Check that lqosd is running, and you have permission to access the socket path."
    )]
    SocketNotFound,
    /// The request could not be encoded for transport.
    #[error("Unable to encode request")]
    EncodingError,
    /// The reply could not be decoded from the socket stream.
    #[error("Unable to decode request")]
    DecodingError,
    /// Writing the request to the socket failed.
    #[error("Cannot write to socket")]
    StreamWriteError,
    /// Reading the reply from the socket failed.
    #[error("Cannot read from socket")]
    StreamReadError,
    /// The socket connection is no longer usable.
    #[error("Stream is no longer connected")]
    StreamNotConnected,
}
