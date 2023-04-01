pub mod tracker;
mod config;
mod bus;

use default_net::get_interfaces;
use lqos_config::WebUsers;

//Get lqos configs
pub fn allow_anonymous() -> bool {
	if let Some(users) = Some(WebUsers::load_or_create().unwrap()) {
		return users.do_we_allow_anonymous()
	}
	false
}

//Get NIC list
pub async fn nic_list<'a>() -> Vec<(String, String, String)> {
	get_interfaces()
		.iter()
		.map(|eth| {
			let mac = if let Some(mac) = &eth.mac_addr {
				mac.to_string()
			} else {
				String::new()
			};
		(eth.name.clone(), format!("{:?}", eth.if_type), mac)
		})
		.collect()
}