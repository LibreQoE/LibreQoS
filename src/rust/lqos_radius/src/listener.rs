//! UDP listener startup for RADIUS Accounting-Request packets.

use crate::VerifiedAccountingRequest;
use crate::packet::{
    MessageAuthenticatorPolicy, PacketError, RADIUS_MAX_PACKET_LEN, handle_accounting_request,
    verify_accounting_request,
};
use ip_network::{IpNetwork, IpNetworkError};
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::str::FromStr;
use thiserror::Error;
use tokio::net::UdpSocket;

/// Default non-privileged loopback listen address for local use and tests.
pub const DEFAULT_LISTEN_ADDR: SocketAddr =
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 18130));

/// Configuration used to start a RADIUS accounting listener.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ListenerConfig {
    /// UDP address the listener will bind.
    pub listen_addr: SocketAddr,
}

impl Default for ListenerConfig {
    fn default() -> Self {
        Self {
            listen_addr: DEFAULT_LISTEN_ADDR,
        }
    }
}

/// A started UDP listener for RADIUS accounting datagrams.
pub struct RadiusListener {
    socket: UdpSocket,
}

impl RadiusListener {
    /// Returns the local UDP address bound by the listener.
    ///
    /// Side effects: queries the operating system for the socket's local
    /// address. It does not read packets or change socket state.
    pub fn local_addr(&self) -> Result<SocketAddr, ListenerError> {
        match self.socket.local_addr() {
            Ok(addr) => Ok(addr),
            Err(source) => Err(ListenerError::LocalAddr { source }),
        }
    }

    /// Waits for and parses the next RADIUS Accounting-Request datagram.
    ///
    /// Side effects: awaits network input on the listener's UDP socket.
    /// Malformed or unsupported packets are returned as errors and no response
    /// packets are sent. This diagnostic listener does not verify shared-secret
    /// authenticators; callers that know the trusted client secret should use
    /// `verify_accounting_request` before accepting a packet.
    pub async fn receive_next(&self) -> Result<ReceivedAccountingPacket, ListenerError> {
        let mut datagram = [0_u8; RADIUS_MAX_PACKET_LEN];
        let (received_len, peer) = receive_datagram(&self.socket, &mut datagram).await?;

        let request = match handle_accounting_request(&datagram[..received_len]) {
            Ok(request) => request,
            Err(source) => return Err(ListenerError::Packet { peer, source }),
        };

        Ok(ReceivedAccountingPacket {
            peer,
            received_len,
            request,
        })
    }

    /// Waits for the next datagram, verifies it against trusted clients, and
    /// sends Accounting-Response packets for accepted Accounting-Requests.
    ///
    /// Side effects: awaits network input on the listener socket and sends one
    /// UDP Accounting-Response when the packet source and authenticators are
    /// trusted. Rejected packets do not produce accounting events and do not
    /// receive responses.
    pub async fn receive_next_verified(
        &self,
        clients: &[TrustedRadiusClient],
    ) -> Result<AccountingListenerOutcome, ListenerError> {
        let mut datagram = [0_u8; RADIUS_MAX_PACKET_LEN];
        let (received_len, peer) = receive_datagram(&self.socket, &mut datagram).await?;

        let mut matched_client = None;
        for client in clients {
            if !client.source_matches(peer.ip()) {
                continue;
            }
            if matched_client.replace(client).is_some() {
                return Ok(AccountingListenerOutcome::RejectedAmbiguousSource {
                    peer,
                    received_len,
                });
            }
        }
        let Some(client) = matched_client else {
            return Ok(AccountingListenerOutcome::RejectedSource { peer, received_len });
        };

        let request = match verify_accounting_request(
            &datagram[..received_len],
            client.shared_secret(),
            client.message_authenticator_policy(),
        ) {
            Ok(request) => request,
            Err(source) => {
                return Ok(AccountingListenerOutcome::RejectedPacket {
                    peer,
                    received_len,
                    source,
                });
            }
        };
        let response = request.build_response(client.shared_secret());
        let response_len = match self.socket.send_to(&response, peer).await {
            Ok(response_len) => response_len,
            Err(source) => return Err(ListenerError::Send { peer, source }),
        };

        Ok(AccountingListenerOutcome::Accepted(
            ReceivedVerifiedAccountingPacket {
                peer,
                received_len,
                response_len,
                request,
            },
        ))
    }
}

/// One parse-only Accounting-Request datagram received by the listener.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReceivedAccountingPacket {
    /// UDP peer address that sent the datagram.
    pub peer: SocketAddr,
    /// Number of bytes received in the datagram.
    pub received_len: usize,
    /// Parsed, unverified Accounting-Request packet.
    pub request: crate::AccountingRequest,
}

/// Runtime allow-list entry for a trusted RADIUS client source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrustedClientSource {
    network: IpNetwork,
}

impl TrustedClientSource {
    /// Creates a source matcher for one host IP address.
    #[must_use]
    pub fn host(address: IpAddr) -> Self {
        Self {
            network: IpNetwork::from(address),
        }
    }

    /// Creates a source matcher for an IP network.
    ///
    /// Side effects: none. The supplied address must be the network address for
    /// the prefix length.
    pub fn network(address: IpAddr, prefix_len: u8) -> Result<Self, TrustedClientSourceError> {
        source_network(address, prefix_len).map(|network| Self { network })
    }

    /// Returns the canonical source address or network address.
    #[must_use]
    pub fn address(&self) -> IpAddr {
        self.network.network_address()
    }

    /// Returns the source prefix length.
    #[must_use]
    pub fn prefix_len(&self) -> u8 {
        self.network.netmask()
    }

    /// Returns true when `address` belongs to this source allow-list entry.
    #[must_use]
    pub fn contains(&self, address: IpAddr) -> bool {
        self.network.contains(address)
    }
}

impl FromStr for TrustedClientSource {
    type Err = TrustedClientSourceError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(TrustedClientSourceError::Empty);
        }

        let Some((address, prefix_len)) = trimmed.split_once('/') else {
            return trimmed.parse::<IpAddr>().map(Self::host).map_err(|_| {
                TrustedClientSourceError::InvalidAddress {
                    raw: trimmed.to_string(),
                }
            });
        };

        let address =
            address
                .parse::<IpAddr>()
                .map_err(|_| TrustedClientSourceError::InvalidAddress {
                    raw: address.to_string(),
                })?;
        let prefix_len = prefix_len.parse::<u8>().map_err(|_| {
            TrustedClientSourceError::InvalidPrefixLengthText {
                raw: prefix_len.to_string(),
            }
        })?;

        source_network(address, prefix_len).map(|network| Self { network })
    }
}

/// Errors returned while building trusted RADIUS client source matchers.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum TrustedClientSourceError {
    /// The supplied source text is empty.
    #[error("RADIUS client source must not be empty")]
    Empty,
    /// The supplied source text does not contain a valid IP address.
    #[error("RADIUS client source address '{raw}' is invalid")]
    InvalidAddress {
        /// Source address text supplied by the caller.
        raw: String,
    },
    /// The supplied CIDR prefix is not numeric.
    #[error("RADIUS client source prefix '{raw}' is invalid")]
    InvalidPrefixLengthText {
        /// Prefix text supplied by the caller.
        raw: String,
    },
    /// The supplied prefix length is outside the address-family range.
    #[error(
        "RADIUS client source prefix length {prefix_len} is invalid for {address}; valid range is 0..={max_prefix_len}"
    )]
    InvalidPrefixLength {
        /// Address supplied for the source matcher.
        address: IpAddr,
        /// Prefix length supplied for the source matcher.
        prefix_len: u8,
        /// Maximum prefix length for the address family.
        max_prefix_len: u8,
    },
    /// The supplied CIDR string contains host bits in the network address.
    #[error("RADIUS client source '{address}/{prefix_len}' has host bits set")]
    HostBitsSet {
        /// Address supplied for the source matcher.
        address: IpAddr,
        /// Prefix length supplied for the source matcher.
        prefix_len: u8,
    },
}

/// Runtime trusted RADIUS client with source matching and shared secret.
#[derive(Clone, Eq, PartialEq)]
pub struct TrustedRadiusClient {
    sources: Vec<TrustedClientSource>,
    shared_secret: Vec<u8>,
    message_authenticator_policy: MessageAuthenticatorPolicy,
}

impl TrustedRadiusClient {
    /// Creates a trusted RADIUS client using optional Message-Authenticator
    /// policy.
    ///
    /// Side effects: none. The shared secret is copied into owned memory for
    /// packet verification and response signing.
    pub fn new(
        sources: Vec<TrustedClientSource>,
        shared_secret: impl Into<Vec<u8>>,
    ) -> Result<Self, TrustedRadiusClientError> {
        Self::with_message_authenticator_policy(
            sources,
            shared_secret,
            MessageAuthenticatorPolicy::Optional,
        )
    }

    /// Creates a trusted RADIUS client with an explicit Message-Authenticator
    /// policy.
    ///
    /// Side effects: none. The shared secret is copied into owned memory for
    /// packet verification and response signing.
    pub fn with_message_authenticator_policy(
        sources: Vec<TrustedClientSource>,
        shared_secret: impl Into<Vec<u8>>,
        message_authenticator_policy: MessageAuthenticatorPolicy,
    ) -> Result<Self, TrustedRadiusClientError> {
        if sources.is_empty() {
            return Err(TrustedRadiusClientError::NoSources);
        }

        let shared_secret = shared_secret.into();
        if shared_secret.is_empty() {
            return Err(TrustedRadiusClientError::EmptySharedSecret);
        }

        Ok(Self {
            sources,
            shared_secret,
            message_authenticator_policy,
        })
    }

    /// Returns the source allow-list for this client.
    #[must_use]
    pub fn sources(&self) -> &[TrustedClientSource] {
        &self.sources
    }

    /// Returns the shared secret used for packet verification and response
    /// signing.
    #[must_use]
    pub fn shared_secret(&self) -> &[u8] {
        &self.shared_secret
    }

    /// Returns the Message-Authenticator policy for this client.
    #[must_use]
    pub const fn message_authenticator_policy(&self) -> MessageAuthenticatorPolicy {
        self.message_authenticator_policy
    }

    fn source_matches(&self, address: IpAddr) -> bool {
        self.sources.iter().any(|source| source.contains(address))
    }
}

impl fmt::Debug for TrustedRadiusClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TrustedRadiusClient")
            .field("sources", &self.sources)
            .field("shared_secret", &"REDACTED")
            .field(
                "message_authenticator_policy",
                &self.message_authenticator_policy,
            )
            .finish()
    }
}

/// Errors returned while building trusted RADIUS clients.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum TrustedRadiusClientError {
    /// The client has no source allow-list entries.
    #[error("trusted RADIUS client must include at least one source")]
    NoSources,
    /// The client has no shared secret bytes.
    #[error("trusted RADIUS client shared secret must not be empty")]
    EmptySharedSecret,
}

/// Result of handling one verified-listener UDP datagram.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AccountingListenerOutcome {
    /// The datagram was verified and received an Accounting-Response.
    Accepted(ReceivedVerifiedAccountingPacket),
    /// The datagram source IP did not match any trusted client.
    RejectedSource {
        /// UDP peer address that sent the rejected datagram.
        peer: SocketAddr,
        /// Number of bytes received in the rejected datagram.
        received_len: usize,
    },
    /// The datagram source IP matched more than one trusted client, so the
    /// listener rejected it instead of guessing which shared secret should apply.
    RejectedAmbiguousSource {
        /// UDP peer address that sent the rejected datagram.
        peer: SocketAddr,
        /// Number of bytes received in the rejected datagram.
        received_len: usize,
    },
    /// The datagram source matched a trusted client, but packet parsing or
    /// authenticator verification failed.
    RejectedPacket {
        /// UDP peer address that sent the rejected datagram.
        peer: SocketAddr,
        /// Number of bytes received in the rejected datagram.
        received_len: usize,
        /// Packet parsing or authenticator verification error.
        source: PacketError,
    },
}

/// One accepted and verified Accounting-Request handled by the listener.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReceivedVerifiedAccountingPacket {
    /// UDP peer address that sent the datagram.
    pub peer: SocketAddr,
    /// Number of bytes received in the accepted datagram.
    pub received_len: usize,
    /// Number of bytes sent in the Accounting-Response datagram.
    pub response_len: usize,
    /// Verified Accounting-Request packet.
    pub request: VerifiedAccountingRequest,
}

/// Errors returned while starting or receiving from the RADIUS listener.
#[derive(Debug, Error)]
pub enum ListenerError {
    /// The UDP listener could not bind the requested address.
    #[error("failed to bind RADIUS UDP listener to {addr}: {source}")]
    Bind {
        /// Address that failed to bind.
        addr: SocketAddr,
        /// Operating-system bind error.
        #[source]
        source: std::io::Error,
    },
    /// The UDP listener could not receive a datagram.
    #[error("failed to receive RADIUS UDP datagram: {source}")]
    Receive {
        /// Operating-system receive error.
        #[source]
        source: std::io::Error,
    },
    /// The UDP listener could not send a response datagram.
    #[error("failed to send RADIUS Accounting-Response to {peer}: {source}")]
    Send {
        /// UDP peer address that should have received the response.
        peer: SocketAddr,
        /// Operating-system send error.
        #[source]
        source: std::io::Error,
    },
    /// The listener's local address could not be queried.
    #[error("failed to read RADIUS UDP listener local address: {source}")]
    LocalAddr {
        /// Operating-system local-address error.
        #[source]
        source: std::io::Error,
    },
    /// The listener received a malformed or unsupported packet.
    #[error("received malformed or unsupported RADIUS accounting packet from {peer}: {source}")]
    Packet {
        /// UDP peer address that sent the rejected datagram.
        peer: SocketAddr,
        /// Packet parsing error.
        #[source]
        source: PacketError,
    },
}

async fn receive_datagram(
    socket: &UdpSocket,
    datagram: &mut [u8; RADIUS_MAX_PACKET_LEN],
) -> Result<(usize, SocketAddr), ListenerError> {
    match socket.recv_from(datagram).await {
        Ok(received) => Ok(received),
        Err(source) => Err(ListenerError::Receive { source }),
    }
}

const fn max_prefix_len(address: IpAddr) -> u8 {
    match address {
        IpAddr::V4(_) => 32,
        IpAddr::V6(_) => 128,
    }
}

fn network_error(
    address: IpAddr,
    prefix_len: u8,
    source: IpNetworkError,
) -> TrustedClientSourceError {
    match source {
        IpNetworkError::NetmaskError(_) => TrustedClientSourceError::InvalidPrefixLength {
            address,
            prefix_len,
            max_prefix_len: max_prefix_len(address),
        },
        IpNetworkError::HostBitsSet => TrustedClientSourceError::HostBitsSet {
            address,
            prefix_len,
        },
    }
}

fn source_network(address: IpAddr, prefix_len: u8) -> Result<IpNetwork, TrustedClientSourceError> {
    IpNetwork::new(address, prefix_len).map_err(|source| network_error(address, prefix_len, source))
}

/// Starts a UDP listener for RADIUS accounting packets.
///
/// Side effects: binds a UDP socket to `config.listen_addr`. This function does
/// not touch TC/XDP state, services, files, or privileged ports unless the caller
/// explicitly supplies such an address.
pub async fn start_listener(config: ListenerConfig) -> Result<RadiusListener, ListenerError> {
    let socket = match UdpSocket::bind(config.listen_addr).await {
        Ok(socket) => socket,
        Err(source) => {
            return Err(ListenerError::Bind {
                addr: config.listen_addr,
                source,
            });
        }
    };

    Ok(RadiusListener { socket })
}

#[cfg(test)]
mod tests;
