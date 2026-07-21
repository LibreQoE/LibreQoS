//! Rootless diagnostic executable for the LibreQoS RADIUS accounting listener.

use clap::{ArgGroup, Parser};
use lqos_radius::{
    AccountingListenerOutcome, DEFAULT_LISTEN_ADDR, ListenerConfig, ListenerError, RadiusListener,
    TrustedClientSource, TrustedRadiusClient, start_listener,
};
use std::net::SocketAddr;
use thiserror::Error;

#[derive(Parser)]
#[command(
    name = "lqos_radius",
    about = "Run the LibreQoS RADIUS accounting diagnostic listener.",
    group(
        ArgGroup::new("run_mode")
            .required(true)
            .args(["parse_only", "shared_secret"])
    )
)]
struct Args {
    #[arg(long, value_name = "ADDR", default_value_t = DEFAULT_LISTEN_ADDR)]
    listen: SocketAddr,
    #[arg(long, conflicts_with_all = ["client_sources", "shared_secret"])]
    parse_only: bool,
    #[arg(
        long = "client-source",
        value_name = "IP_OR_CIDR",
        requires = "shared_secret"
    )]
    client_sources: Vec<TrustedClientSource>,
    #[arg(
        long,
        value_name = "SECRET",
        requires = "client_sources",
        value_parser = clap::builder::NonEmptyStringValueParser::new()
    )]
    shared_secret: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), MainError> {
    let args = Args::parse();
    let listen = args.listen;
    let run_mode = args.run_mode();
    let listener = start_listener(ListenerConfig {
        listen_addr: listen,
    })
    .await?;
    let local_addr = listener.local_addr()?;

    println!("Listening for RADIUS accounting packets on {local_addr}");
    match run_mode {
        RunMode::ParseOnly => parse_only_loop(&listener).await,
        RunMode::Verified(clients) => verified_loop(&listener, &clients).await,
    }
}

impl Args {
    fn run_mode(self) -> RunMode {
        let Self {
            listen: _,
            parse_only,
            client_sources,
            shared_secret,
        } = self;

        if parse_only {
            return RunMode::ParseOnly;
        }
        let shared_secret = shared_secret.expect("clap requires --shared-secret in verified mode");
        let client = TrustedRadiusClient::new(client_sources, shared_secret.into_bytes()).expect(
            "clap requires at least one --client-source and a non-empty --shared-secret in verified mode",
        );

        RunMode::Verified(vec![client])
    }
}

enum RunMode {
    ParseOnly,
    Verified(Vec<TrustedRadiusClient>),
}

#[derive(Debug, Error)]
enum MainError {
    #[error(transparent)]
    Listener(#[from] ListenerError),
}

async fn parse_only_loop(listener: &RadiusListener) -> Result<(), MainError> {
    loop {
        match listener.receive_next().await {
            Ok(received) => println!(
                "Parsed Accounting-Request from {}: identifier={}, attributes={}",
                received.peer,
                received.request.packet().identifier(),
                received.request.packet().attributes().len()
            ),
            Err(ListenerError::Packet { peer, source }) => {
                eprintln!("Discarded malformed or unsupported RADIUS packet from {peer}: {source}");
            }
            Err(err) => return Err(err.into()),
        }
    }
}

async fn verified_loop(
    listener: &RadiusListener,
    clients: &[TrustedRadiusClient],
) -> Result<(), MainError> {
    loop {
        match listener.receive_next_verified(clients).await {
            Ok(AccountingListenerOutcome::Accepted(accepted)) => println!(
                "Accepted Accounting-Request from {}: identifier={}, attributes={}, response_bytes={}",
                accepted.peer,
                accepted.request.packet().identifier(),
                accepted.request.packet().attributes().len(),
                accepted.response_len
            ),
            Ok(AccountingListenerOutcome::RejectedSource { peer, .. }) => {
                eprintln!("Discarded RADIUS packet from unconfigured source {peer}");
            }
            Ok(AccountingListenerOutcome::RejectedAmbiguousSource { peer, .. }) => {
                eprintln!("Discarded RADIUS packet from ambiguous trusted source {peer}");
            }
            Ok(AccountingListenerOutcome::RejectedPacket { peer, source, .. }) => {
                eprintln!("Rejected RADIUS packet from {peer}: {source}");
            }
            Err(ListenerError::Send { peer, source }) => {
                eprintln!("Failed to send RADIUS Accounting-Response to {peer}: {source}");
            }
            Err(err) => return Err(err.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn cli_parser_accepts_parse_only_mode() {
        let args = Args::try_parse_from(["lqos_radius", "--parse-only"]).unwrap();

        assert_eq!(args.listen, DEFAULT_LISTEN_ADDR);
        assert!(matches!(args.run_mode(), RunMode::ParseOnly));
    }

    #[test]
    fn cli_parser_accepts_verified_mode_with_multiple_sources() {
        let args = Args::try_parse_from([
            "lqos_radius",
            "--listen",
            "127.0.0.1:0",
            "--client-source",
            "127.0.0.1",
            "--client-source",
            "192.0.2.0/24",
            "--shared-secret",
            "radius-secret",
        ])
        .unwrap();
        assert_eq!(args.listen, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)));

        let RunMode::Verified(clients) = args.run_mode() else {
            panic!("expected verified run mode");
        };
        assert_eq!(clients.len(), 1);
        assert_eq!(clients[0].sources().len(), 2);
        assert!(clients[0].sources()[0].contains(Ipv4Addr::LOCALHOST.into()));
        assert!(clients[0].sources()[1].contains(Ipv4Addr::new(192, 0, 2, 99).into()));
        assert_eq!(clients[0].shared_secret(), b"radius-secret");
    }

    #[test]
    fn cli_parser_rejects_missing_or_conflicting_modes() {
        assert!(Args::try_parse_from(["lqos_radius"]).is_err());
        assert!(
            Args::try_parse_from(["lqos_radius", "--parse-only", "--shared-secret", "secret"])
                .is_err()
        );
        assert!(
            Args::try_parse_from([
                "lqos_radius",
                "--parse-only",
                "--client-source",
                "127.0.0.1"
            ])
            .is_err()
        );
        assert!(Args::try_parse_from(["lqos_radius", "--client-source", "127.0.0.1"]).is_err());
        assert!(Args::try_parse_from(["lqos_radius", "--shared-secret", "secret"]).is_err());
        assert!(
            Args::try_parse_from([
                "lqos_radius",
                "--client-source",
                "127.0.0.1",
                "--shared-secret",
                "",
            ])
            .is_err()
        );
    }
}
