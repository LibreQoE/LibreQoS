// SPDX-FileCopyrightText: 2025 LibreQoE support@libreqos.io
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-LibreQoS-Exception

//! The `lqos_bus` crate provides the data-transfer back-end for communication
//! between the various parts of LibreQoS. `lqosd` listens on `localhost`
//! for requests. Any tool may use the daemon services locally for interaction
//! with the LibreQoS system.
//!
//! A normal session consists of connecting and sending a single `BusSession`
//! object (serialized with CBOR), that must contain one or more
//! `BusRequest` objects. Payloads are framed with a header and chunked
//! into length-prefixed blocks for transport. Replies are then batched
//! inside a `BusReply` object, containing one or more `BusResponse`
//! detail objects. The session then terminates.
//!
//! Protocol versioning/negotiation is intentionally skipped.

#![deny(clippy::unwrap_used)]
#![warn(missing_docs)]
mod bus;
mod ip_stats;
pub use ip_stats::{
    Circuit, FlowbeeProtocol, FlowbeeSummaryData, IpMapping, IpStats, PacketHeader, XdpPpingResult,
    tos_parser,
};
mod tc_handle;
pub use bus::response::{
    AsnHeatmapData, AsnListEntry, BakeryStatsSnapshot, CircuitCount, CircuitHeatmapData,
    CircuitCapacityRow, CountryListEntry, DeviceCounts, ExecutiveSummaryHeader, FlowMapPoint,
    FlowTimelineEntry, InsightLicenseSummary, NodeCapacity, ProtocolListEntry, QueueStatsTotal,
    RetransmitSummary, SchedulerDetails, SearchResultEntry, SiteHeatmapData, StormguardDebugDirection,
    StormguardDebugEntry, UrgentIssue, WarningLevel,
};
pub use bus::{
    BUS_SOCKET_PATH, BlackboardSystem, BusReply, BusRequest, BusResponse, BusSession,
    CakeDiffTinTransit, CakeDiffTransit, CakeTransit, LibreqosBusClient, QueueStoreTransit,
    TopFlowType, UnixSocketServer, UrgentSeverity, UrgentSource, bus_request,
};
pub use tc_handle::TcHandle;

/// Re-export CBOR
pub mod cbor {
    pub use serde_cbor::*;
}
