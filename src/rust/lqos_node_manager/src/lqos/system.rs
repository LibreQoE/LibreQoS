use default_net::get_interfaces;

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