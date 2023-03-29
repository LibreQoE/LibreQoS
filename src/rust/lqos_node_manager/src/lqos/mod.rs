pub mod tracker;
mod config;
mod bus;

use default_net::get_interfaces;
use lqos_bus::{bus_request, BusRequest};
use lqos_config::{EtcLqos, LibreQoSConfig, Tunables, WebUsers};
use axum::Json;

use crate::error::AppError;

//Get lqos configs
pub fn allow_anonymous() -> bool {
	if let Some(users) = Some(WebUsers::load_or_create().unwrap()) {
		return users.do_we_allow_anonymous()
	}
	false
}

//Get lqos configs
pub async fn get_ispconfig() -> Result<Json<LibreQoSConfig>, AppError> {
	let config = LibreQoSConfig::load().unwrap();
	Ok(Json(config))
}

pub async fn get_lqos() -> Result<Json<EtcLqos>, AppError> {
	let config = EtcLqos::load().unwrap();
	Ok(Json(config))
}

//Update lqos configs
pub async fn update_ispconfig(config: Json<LibreQoSConfig>) -> Result<Json<String>, AppError> {
	config.save().unwrap();
	Ok(Json("OK".to_string()))
}

pub async fn update_lqos(period: u64, tuning: Json<Tunables>) -> Result<Json<String>, AppError> {
	// Send the update to the server
	bus_request(vec![BusRequest::UpdateLqosDTuning(period, (*tuning).clone())])
		.await
		.unwrap();
	// For now, ignore the reply.
	Ok(Json("OK".to_string()))
}

//Get NIC list
pub async fn nic_list<'a>() -> Json<Vec<(String, String, String)>> {
	let result = get_interfaces()
		.iter()
		.map(|eth| {
			let mac = if let Some(mac) = &eth.mac_addr {
				mac.to_string()
			} else {
				String::new()
			};
		(eth.name.clone(), format!("{:?}", eth.if_type), mac)
		})
		.collect();
	Json(result)
}