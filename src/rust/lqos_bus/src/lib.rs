// SPDX-FileCopyrightText: 2025 LibreQoE support@libreqos.io
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-LibreQoS-Exception

//! The `lqos_bus` crate provides the data-transfer back-end for communication
//! between the various parts of LibreQoS. `lqosd` listens on `localhost`
//! for requests. Any tool may use the daemon services locally for interaction
//! with the LibreQoS system.
//!
//! A normal session consists of connecting and sending a single `BusSession`
//! object (serialized with `bincode`), that must contain one or more
//! `BusRequest` objects. Replies are then batched inside a `BusReply`
//! object, containing one or more `BusResponse` detail objects.
//! The session then terminates.

#![deny(clippy::unwrap_used)]
#![warn(missing_docs)]
mod bus;
mod ip_stats;
pub use ip_stats::{
    Circuit, FlowbeeProtocol, FlowbeeSummaryData, IpMapping, IpStats, PacketHeader, XdpPpingResult,
    tos_parser,
};
mod tc_handle;
pub use bus::response::{BakeryStatsSnapshot, CircuitHeatmapData, UrgentIssue};
pub use bus::{
    BUS_SOCKET_PATH, BlackboardSystem, BusReply, BusRequest, BusResponse, BusSession,
    CakeDiffTinTransit, CakeDiffTransit, CakeTransit, LibreqosBusClient, QueueStoreTransit,
    TopFlowType, UnixSocketServer, UrgentSeverity, UrgentSource, bus_request,
};
pub use tc_handle::TcHandle;

/// Re-export bincode
pub mod bincode {
    pub use bincode::*;
}

/// Re-export CBOR
pub mod cbor {
    pub use serde_cbor::*;
}
