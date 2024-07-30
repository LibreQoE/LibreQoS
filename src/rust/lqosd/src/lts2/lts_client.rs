use std::sync::atomic::AtomicU32;
use log::info;
use lqos_config::load_config;
use tonic::transport::{Certificate, Channel, ClientTlsConfig};
use crate::lts2::lts_client::lts2_grpc::lts2_client::Lts2Client;

#[allow(missing_docs)]
pub mod lts2_grpc {
    tonic::include_proto!("lts2");
}

pub async fn get_lts_client() -> Option<Lts2Client<Channel>> {
    if let Ok(cfg) = load_config() {
        info!("Loading LTS client");
        let mut remote_domain = "stats.libreqos.io".to_string();
        let tls = ClientTlsConfig::new();

        // If needed, add a root certificate to the TLS configuration
        let tls = if let Some(pem) = &cfg.long_term_stats.lts_root_pem {
            info!("Adding root certificate to TLS configuration");
            let cert = std::fs::read_to_string(pem).unwrap();
            tls.ca_certificate(Certificate::from_pem(cert))
        } else {
            tls
        };

        let tls = if let Some(domain) = &cfg.long_term_stats.lts_url {
            info!("Setting domain name to {}", domain);
            remote_domain = domain.to_string();
            tls.domain_name(domain)
        } else {
            tls
        };
        let tls = tls.assume_http2(true);

        info!("Connecting to LTS server");
        let url = format!("https://{}:443/", remote_domain);
        let channel = Channel::from_shared(url).unwrap()
            .tls_config(tls).unwrap()
            .connect()
            .await.unwrap();
        let client = Lts2Client::new(channel);
        info!("Connected to LTS server");
        return Some(client)
    }
    None
}