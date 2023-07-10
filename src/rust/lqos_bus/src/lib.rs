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

#![warn(missing_docs)]
mod bus;
mod ip_stats;
pub use ip_stats::{
  tos_parser, FlowProto, FlowTransport, IpMapping, IpStats, PacketHeader,
  XdpPpingResult,
};
mod tc_handle;
pub use bus::{
  bus_request, decode_request, decode_response, encode_request,
  encode_response, BusClient, BusReply, BusRequest, BusResponse, BusSession,
  CakeDiffTinTransit, CakeDiffTransit, CakeTransit, QueueStoreTransit,
  UnixSocketServer, BUS_SOCKET_PATH, StatsRequest
};
pub use tc_handle::TcHandle;

/// Anonymous Usage Statistics Data Types
pub mod anonymous;

/// Re-export bincode
pub mod bincode {
  pub use bincode::*;
}

/// Re-export CBOR
pub mod cbor {
  pub use serde_cbor::*;
}