//! Tests for RADIUS UDP listener behavior.

use super::*;
use crate::attribute_type::ACCT_STATUS_TYPE;
use crate::packet::{RADIUS_HEADER_LEN, RADIUS_MAX_ACCOUNTING_PACKET_LEN, RADIUS_MAX_PACKET_LEN};
use crate::test_support::{
    SHARED_SECRET, accounting_request_packet, accounting_request_packet_with_message_authenticator,
    max_sized_accounting_request_packet, radius_attributes, radius_packet, radius_text_attribute,
    radius_u32_attribute, signed_accounting_request_packet,
};
use crate::{PacketError, RadiusCode, parse_packet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::time::timeout;

const ACCT_SESSION_ID: u8 = 44;
const UDP_NO_RESPONSE_TIMEOUT: Duration = Duration::from_millis(25);
const UDP_TEST_TIMEOUT: Duration = Duration::from_secs(1);

#[test]
fn default_listener_address_is_rootless_loopback() {
    let config = ListenerConfig::default();

    assert!(config.listen_addr.ip().is_loopback());
    assert!(config.listen_addr.port() > 1024);
}

mod udp_listener {
    //! Loopback UDP listener tests.

    use super::*;

    #[tokio::test]
    async fn receives_accounting_request_on_loopback_ephemeral_port() {
        let fixture = LoopbackUdpFixture::bind().await;

        fixture.send(&accounting_request_packet(11, &[])).await;
        let received = fixture.receive_next().await.unwrap();

        assert_eq!(received.peer, fixture.sender_addr());
        assert_eq!(received.received_len, RADIUS_HEADER_LEN);
        assert_eq!(received.request.packet().identifier(), 11);
    }

    #[tokio::test]
    async fn receives_maximum_sized_accounting_request() {
        let fixture = LoopbackUdpFixture::bind().await;

        fixture.send(&max_sized_accounting_request_packet(14)).await;
        let received = fixture.receive_next().await.unwrap();

        assert_eq!(received.received_len, RADIUS_MAX_ACCOUNTING_PACKET_LEN);
        assert_eq!(received.request.packet().identifier(), 14);
        assert_eq!(received.request.packet().attributes().len(), 16);
    }

    #[tokio::test]
    async fn reports_packet_errors_with_sender_address() {
        let fixture = LoopbackUdpFixture::bind().await;

        fixture
            .send(&radius_packet(RadiusCode::AccessRequest, 12, &[]))
            .await;
        let err = fixture.receive_next().await.unwrap_err();

        let ListenerError::Packet { peer, source } = err else {
            panic!("expected packet parsing error, got {err:?}");
        };
        assert_eq!(peer, fixture.sender_addr());
        assert_eq!(source, PacketError::UnsupportedCode { code: 1 });
    }

    #[tokio::test]
    async fn reports_malformed_datagrams_with_sender_address() {
        let fixture = LoopbackUdpFixture::bind().await;

        fixture
            .send(&[RadiusCode::AccountingRequest.as_u8(), 13])
            .await;
        let err = fixture.receive_next().await.unwrap_err();

        let ListenerError::Packet { peer, source } = err else {
            panic!("expected packet parsing error, got {err:?}");
        };
        assert_eq!(peer, fixture.sender_addr());
        assert_eq!(source, PacketError::PacketTooShort { actual: 2 });

        fixture.send(&accounting_request_packet(15, &[])).await;
        let received = fixture.receive_next().await.unwrap();

        assert_eq!(received.peer, fixture.sender_addr());
        assert_eq!(received.request.packet().identifier(), 15);
    }

    #[tokio::test]
    async fn verified_listener_accepts_request_and_sends_response() {
        let fixture = LoopbackUdpFixture::bind().await;
        let clients = trusted_loopback_clients(SHARED_SECRET);
        let attributes = radius_attributes(&[
            radius_u32_attribute(ACCT_STATUS_TYPE, 1),
            radius_text_attribute(ACCT_SESSION_ID, "session-accepted"),
        ]);
        let request = signed_accounting_request_packet(21, &attributes, SHARED_SECRET);

        let accepted = expect_accepted(fixture.verified_outcome(&request, &clients).await);
        let response = fixture.receive_response().await;

        assert_eq!(accepted.peer, fixture.sender_addr());
        assert_eq!(accepted.received_len, request.len());
        assert_eq!(accepted.response_len, response.len());
        assert_eq!(accepted.request.packet().identifier(), 21);
        let event = crate::AccountingEvent::from_verified(&accepted.request);
        assert_eq!(event.acct_session_id.as_deref(), Some("session-accepted"));
        assert_eq!(response, accepted.request.build_response(SHARED_SECRET));
        assert_accounting_response(&response, 21);
    }

    #[tokio::test]
    async fn verified_listener_accepts_later_source_in_client_allow_list() {
        let fixture = LoopbackUdpFixture::bind().await;
        let clients = vec![
            TrustedRadiusClient::new(
                vec![alternate_loopback_source(), trusted_loopback_source()],
                SHARED_SECRET,
            )
            .unwrap(),
        ];
        let request = signed_accounting_request_packet(31, &[], SHARED_SECRET);

        let accepted = expect_accepted(fixture.verified_outcome(&request, &clients).await);
        let response = fixture.receive_response().await;

        assert_eq!(accepted.request.packet().identifier(), 31);
        assert_eq!(response, accepted.request.build_response(SHARED_SECRET));
        assert_accounting_response(&response, 31);
    }

    #[tokio::test]
    async fn verified_listener_accepts_message_authenticator_when_required() {
        let fixture = LoopbackUdpFixture::bind().await;
        let clients = trusted_loopback_clients_with_policy(
            SHARED_SECRET,
            MessageAuthenticatorPolicy::Required,
        );
        let request = accounting_request_packet_with_message_authenticator(28, &[], SHARED_SECRET);

        let accepted = expect_accepted(fixture.verified_outcome(&request, &clients).await);
        let response = fixture.receive_response().await;

        assert_eq!(accepted.request.packet().identifier(), 28);
        assert!(accepted.request.has_message_authenticator());
        assert_eq!(accepted.response_len, response.len());
        assert_eq!(response, accepted.request.build_response(SHARED_SECRET));
        assert_accounting_response(&response, 28);
    }

    #[tokio::test]
    async fn verified_listener_rejects_missing_message_authenticator_when_required() {
        let fixture = LoopbackUdpFixture::bind().await;
        let clients = trusted_loopback_clients_with_policy(
            SHARED_SECRET,
            MessageAuthenticatorPolicy::Required,
        );
        let request = signed_accounting_request_packet(29, &[], SHARED_SECRET);

        let outcome = fixture
            .verified_outcome_without_response(&request, &clients)
            .await;

        assert_eq!(
            outcome,
            AccountingListenerOutcome::RejectedPacket {
                peer: fixture.sender_addr(),
                received_len: request.len(),
                source: PacketError::MissingMessageAuthenticator,
            }
        );
    }

    #[tokio::test]
    async fn verified_listener_rejects_ambiguous_source_before_parsing() {
        let fixture = LoopbackUdpFixture::bind().await;
        let clients = vec![
            TrustedRadiusClient::new(
                vec![
                    TrustedClientSource::network(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 0)), 8)
                        .unwrap(),
                ],
                SHARED_SECRET,
            )
            .unwrap(),
            TrustedRadiusClient::new(vec![trusted_loopback_source()], SHARED_SECRET).unwrap(),
        ];
        let request = [RadiusCode::AccountingRequest.as_u8(), 24];

        let outcome = fixture
            .verified_outcome_without_response(&request, &clients)
            .await;

        assert_eq!(
            outcome,
            AccountingListenerOutcome::RejectedAmbiguousSource {
                peer: fixture.sender_addr(),
                received_len: request.len(),
            }
        );
    }

    #[tokio::test]
    async fn verified_listener_rejects_unconfigured_source_without_event_or_response() {
        let fixture = LoopbackUdpFixture::bind().await;
        let clients = alternate_loopback_clients();
        let request = signed_accounting_request_packet(22, &[], SHARED_SECRET);

        let outcome = fixture
            .verified_outcome_without_response(&request, &clients)
            .await;

        assert_eq!(
            outcome,
            AccountingListenerOutcome::RejectedSource {
                peer: fixture.sender_addr(),
                received_len: request.len(),
            }
        );
    }

    #[tokio::test]
    async fn verified_listener_rejects_malformed_untrusted_source_before_parsing() {
        let fixture = LoopbackUdpFixture::bind().await;
        let clients = alternate_loopback_clients();
        let request = [RadiusCode::AccountingRequest.as_u8(), 27];

        let outcome = fixture
            .verified_outcome_without_response(&request, &clients)
            .await;

        assert_eq!(
            outcome,
            AccountingListenerOutcome::RejectedSource {
                peer: fixture.sender_addr(),
                received_len: request.len(),
            }
        );
    }

    #[tokio::test]
    async fn verified_listener_rejects_malformed_trusted_packet_without_response() {
        let fixture = LoopbackUdpFixture::bind().await;
        let clients = trusted_loopback_clients(SHARED_SECRET);
        let request = [RadiusCode::AccountingRequest.as_u8(), 26];

        let outcome = fixture
            .verified_outcome_without_response(&request, &clients)
            .await;

        assert_eq!(
            outcome,
            AccountingListenerOutcome::RejectedPacket {
                peer: fixture.sender_addr(),
                received_len: request.len(),
                source: PacketError::PacketTooShort { actual: 2 },
            }
        );
    }

    #[tokio::test]
    async fn verified_listener_rejects_unsupported_trusted_packet_without_response() {
        let fixture = LoopbackUdpFixture::bind().await;
        let clients = trusted_loopback_clients(SHARED_SECRET);
        let request = radius_packet(RadiusCode::AccessRequest, 30, &[]);

        let outcome = fixture
            .verified_outcome_without_response(&request, &clients)
            .await;

        assert_eq!(
            outcome,
            AccountingListenerOutcome::RejectedPacket {
                peer: fixture.sender_addr(),
                received_len: request.len(),
                source: PacketError::UnsupportedCode { code: 1 },
            }
        );
    }

    #[tokio::test]
    async fn verified_listener_rejects_invalid_authenticator_without_event_or_response() {
        let fixture = LoopbackUdpFixture::bind().await;
        let clients = trusted_loopback_clients(SHARED_SECRET);
        let request = signed_accounting_request_packet(23, &[], b"wrong-secret");

        let outcome = fixture
            .verified_outcome_without_response(&request, &clients)
            .await;

        assert_eq!(
            outcome,
            AccountingListenerOutcome::RejectedPacket {
                peer: fixture.sender_addr(),
                received_len: request.len(),
                source: PacketError::InvalidRequestAuthenticator,
            }
        );
    }
}

#[test]
fn trusted_client_source_matches_exact_hosts_and_networks() {
    let host = TrustedClientSource::host(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));
    let network =
        TrustedClientSource::network(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 0)), 24).unwrap();
    let ipv6_network = TrustedClientSource::network("2001:db8::".parse().unwrap(), 64).unwrap();

    assert!(host.contains(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
    assert!(!host.contains(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2))));
    assert_eq!(network.address(), IpAddr::V4(Ipv4Addr::new(192, 0, 2, 0)));
    assert!(network.contains(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 25))));
    assert!(!network.contains(IpAddr::V4(Ipv4Addr::new(192, 0, 3, 25))));
    assert_eq!(
        ipv6_network.address(),
        "2001:db8::".parse::<IpAddr>().unwrap()
    );
    assert!(ipv6_network.contains("2001:db8::abcd".parse().unwrap()));
    assert!(!ipv6_network.contains("2001:db9::abcd".parse().unwrap()));
}

#[test]
fn trusted_client_source_parses_hosts_and_cidr_networks() {
    assert_eq!(
        "127.0.0.1".parse::<TrustedClientSource>().unwrap(),
        trusted_loopback_source()
    );
    assert_eq!(
        "2001:db8::/64".parse::<TrustedClientSource>().unwrap(),
        TrustedClientSource::network("2001:db8::".parse().unwrap(), 64).unwrap()
    );
    assert_eq!(
        "192.0.2.1/33".parse::<TrustedClientSource>(),
        Err(TrustedClientSourceError::InvalidPrefixLength {
            address: IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
            prefix_len: 33,
            max_prefix_len: 32,
        })
    );
    assert_eq!(
        "2001:db8::/129".parse::<TrustedClientSource>(),
        Err(TrustedClientSourceError::InvalidPrefixLength {
            address: "2001:db8::".parse().unwrap(),
            prefix_len: 129,
            max_prefix_len: 128,
        })
    );
}

#[test]
fn trusted_client_source_rejects_invalid_text() {
    assert_eq!(
        "".parse::<TrustedClientSource>(),
        Err(TrustedClientSourceError::Empty)
    );
    assert_eq!(
        "not-an-address".parse::<TrustedClientSource>(),
        Err(TrustedClientSourceError::InvalidAddress {
            raw: "not-an-address".to_string(),
        })
    );
    assert_eq!(
        "192.0.2.1/nope".parse::<TrustedClientSource>(),
        Err(TrustedClientSourceError::InvalidPrefixLengthText {
            raw: "nope".to_string(),
        })
    );
    assert_eq!(
        "192.0.2.129/24".parse::<TrustedClientSource>(),
        Err(TrustedClientSourceError::HostBitsSet {
            address: IpAddr::V4(Ipv4Addr::new(192, 0, 2, 129)),
            prefix_len: 24,
        })
    );
    assert_eq!(
        TrustedClientSource::network(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 129)), 24),
        Err(TrustedClientSourceError::HostBitsSet {
            address: IpAddr::V4(Ipv4Addr::new(192, 0, 2, 129)),
            prefix_len: 24,
        })
    );
}

#[test]
fn trusted_radius_client_rejects_missing_sources_or_secret() {
    assert_eq!(
        TrustedRadiusClient::with_message_authenticator_policy(
            Vec::new(),
            SHARED_SECRET,
            MessageAuthenticatorPolicy::Required,
        ),
        Err(TrustedRadiusClientError::NoSources)
    );
    assert_eq!(
        TrustedRadiusClient::with_message_authenticator_policy(
            vec![trusted_loopback_source()],
            Vec::new(),
            MessageAuthenticatorPolicy::Required,
        ),
        Err(TrustedRadiusClientError::EmptySharedSecret)
    );
}

struct LoopbackUdpFixture {
    listener: RadiusListener,
    listener_addr: SocketAddr,
    sender: UdpSocket,
}

impl LoopbackUdpFixture {
    async fn bind() -> Self {
        let listener = start_listener(ListenerConfig {
            listen_addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        })
        .await
        .unwrap();
        let listener_addr = listener.local_addr().unwrap();
        let sender = UdpSocket::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();

        Self {
            listener,
            listener_addr,
            sender,
        }
    }

    fn sender_addr(&self) -> SocketAddr {
        self.sender.local_addr().unwrap()
    }

    async fn send(&self, packet: &[u8]) {
        self.sender
            .send_to(packet, self.listener_addr)
            .await
            .unwrap();
    }

    async fn receive_next(&self) -> Result<ReceivedAccountingPacket, ListenerError> {
        timeout(UDP_TEST_TIMEOUT, self.listener.receive_next())
            .await
            .unwrap()
    }

    async fn verified_outcome(
        &self,
        packet: &[u8],
        clients: &[TrustedRadiusClient],
    ) -> AccountingListenerOutcome {
        self.send(packet).await;
        timeout(
            UDP_TEST_TIMEOUT,
            self.listener.receive_next_verified(clients),
        )
        .await
        .unwrap()
        .unwrap()
    }

    async fn verified_outcome_without_response(
        &self,
        packet: &[u8],
        clients: &[TrustedRadiusClient],
    ) -> AccountingListenerOutcome {
        let outcome = self.verified_outcome(packet, clients).await;
        self.assert_no_response().await;
        outcome
    }

    async fn receive_response(&self) -> Vec<u8> {
        let mut response = [0_u8; RADIUS_MAX_PACKET_LEN];
        let (response_len, peer) = timeout(UDP_TEST_TIMEOUT, self.sender.recv_from(&mut response))
            .await
            .unwrap()
            .unwrap();

        assert!(peer.ip().is_loopback());
        response[..response_len].to_vec()
    }

    async fn assert_no_response(&self) {
        let mut response = [0_u8; RADIUS_MAX_PACKET_LEN];

        match timeout(
            UDP_NO_RESPONSE_TIMEOUT,
            self.sender.recv_from(&mut response),
        )
        .await
        {
            Err(_) => {}
            Ok(Ok((response_len, peer))) => {
                panic!("rejected packet received {response_len} response bytes from {peer}");
            }
            Ok(Err(source)) => panic!("failed to check for rejected-packet response: {source}"),
        }
    }
}

fn trusted_loopback_clients(shared_secret: &[u8]) -> Vec<TrustedRadiusClient> {
    trusted_loopback_clients_with_policy(shared_secret, MessageAuthenticatorPolicy::Optional)
}

fn trusted_loopback_clients_with_policy(
    shared_secret: &[u8],
    policy: MessageAuthenticatorPolicy,
) -> Vec<TrustedRadiusClient> {
    vec![
        TrustedRadiusClient::with_message_authenticator_policy(
            vec![trusted_loopback_source()],
            shared_secret,
            policy,
        )
        .unwrap(),
    ]
}

fn alternate_loopback_clients() -> Vec<TrustedRadiusClient> {
    vec![TrustedRadiusClient::new(vec![alternate_loopback_source()], SHARED_SECRET).unwrap()]
}

fn trusted_loopback_source() -> TrustedClientSource {
    TrustedClientSource::host(IpAddr::V4(Ipv4Addr::LOCALHOST))
}

fn alternate_loopback_source() -> TrustedClientSource {
    TrustedClientSource::host(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2)))
}

fn expect_accepted(outcome: AccountingListenerOutcome) -> ReceivedVerifiedAccountingPacket {
    match outcome {
        AccountingListenerOutcome::Accepted(accepted) => accepted,
        other => panic!("expected accepted packet, got {other:?}"),
    }
}

fn assert_accounting_response(response: &[u8], identifier: u8) {
    let parsed = parse_packet(response).unwrap();

    assert_eq!(parsed.code(), RadiusCode::AccountingResponse);
    assert_eq!(parsed.identifier(), identifier);
    assert_eq!(parsed.attributes(), &[]);
}
