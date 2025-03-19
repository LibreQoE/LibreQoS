use crate::lts2_sys::lts2_client::nacl_blob::KeyStore;
use crate::lts2_sys::lts2_client::{get_node_id, get_remote_host, nacl_blob};
use crate::lts2_sys::shared_types::FreeTrialDetails;
use serde::{Deserialize, Serialize};
use std::net::TcpStream;
use tokio::sync::oneshot;

#[derive(Serialize)]
pub struct FreeTrialRequest {
    pub node_id: String,
    pub name: String,
    pub email: String,
    pub business_name: String,
    pub address1: String,
    pub address2: String,
    pub city: String,
    pub state: String,
    pub zip: String,
    pub country: String,
    pub phone: String,
    pub website: String,
}

#[derive(Deserialize)]
pub struct FreeTrialResponse {
    pub success: bool,
    pub license_key: String,
}

pub fn request_free_trial(trial: FreeTrialDetails, sender: oneshot::Sender<String>) {
    let keys = KeyStore::new();
    let remote_host = get_remote_host();
    if let Ok(mut socket) = TcpStream::connect(format!("{}:9122", remote_host)) {
        if let Err(e) = nacl_blob::transmit_hello(&keys, 0x8342, 2, &mut socket) {
            println!("Failed to send hello to license server. {e:?}");
            sender.send("FAIL".to_string()).unwrap();
            return;
        }

        if let Ok((server_hello, _)) = nacl_blob::receive_hello(&mut socket) {
            let node_id = get_node_id();
            let req = FreeTrialRequest {
                node_id,
                name: trial.name.clone(),
                email: trial.email.clone(),
                business_name: trial.business_name.clone(),
                address1: trial.address1.clone(),
                address2: trial.address2.clone(),
                city: trial.city.clone(),
                state: trial.state.clone(),
                zip: trial.zip.clone(),
                country: trial.country.clone(),
                phone: trial.phone.clone(),
                website: trial.website.clone(),
            };
            if let Err(e) =
                nacl_blob::transmit_payload(&keys, &server_hello.public_key, &req, &mut socket)
            {
                println!("Failed to send license key to license server. {e:?}");
                sender.send("FAIL".to_string()).unwrap();
                return;
            }

            if let Ok((response, _)) = nacl_blob::receive_payload::<FreeTrialResponse>(
                &keys,
                &server_hello.public_key,
                &mut socket,
            ) {
                if response.success {
                    sender.send(response.license_key).unwrap();
                } else {
                    sender.send("FAIL".to_string()).unwrap();
                }
            } else {
                sender.send("FAIL".to_string()).unwrap();
            }
        } else {
            println!("Failed to receive hello from license server.");
            sender.send("FAIL".to_string()).unwrap();
            return;
        }
    }
}
