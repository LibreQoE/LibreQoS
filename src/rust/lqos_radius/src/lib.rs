//! RADIUS accounting packet parsing and rootless listener support.
//!
//! The crate checks RADIUS packet framing, parses and verifies
//! Accounting-Request packets, builds Accounting-Response packets, tracks
//! decoded accounting sessions in memory, emits dynamic-circuit command intents
//! through an explicit sink boundary, and exposes a UDP listener that can bind
//! to non-privileged loopback addresses for development and automated tests.

#![warn(missing_docs)]

mod accounting_event;
mod attribute_type;
mod dynamic_circuit;
mod listener;
mod packet;
mod session;
#[cfg(test)]
mod test_support;

pub use accounting_event::{
    AccountingEvent, AccountingEventOptions, AcctStatusType, Ipv6Prefix, MikrotikRateLimit,
    MikrotikRateLimitDirection, UnknownVendorAttribute,
};
pub use dynamic_circuit::{
    DynamicCircuitCommandSink, DynamicCircuitIntent, DynamicCircuitRemoval,
    DynamicCircuitRemovalReason, DynamicCircuitUpsert,
};
pub use listener::{
    AccountingListenerOutcome, DEFAULT_LISTEN_ADDR, ListenerConfig, ListenerError, RadiusListener,
    ReceivedAccountingPacket, ReceivedVerifiedAccountingPacket, TrustedClientSource,
    TrustedClientSourceError, TrustedRadiusClient, TrustedRadiusClientError, start_listener,
};
pub use packet::{
    AccountingRequest, MessageAuthenticatorPolicy, PacketError, RadiusAttribute, RadiusCode,
    RadiusPacket, VerifiedAccountingRequest, build_accounting_response, handle_accounting_request,
    parse_packet, verify_accounting_request,
};
pub use session::{
    AccountingSession, AccountingSessionIgnoreReason, AccountingSessionKey, AccountingSessionState,
    AccountingSessionStore, AccountingSessionUpdate, DynamicCircuitMapping, NasIdentity,
    NasResetStatus, PendingSessionFingerprint, PendingSessionReason,
};
