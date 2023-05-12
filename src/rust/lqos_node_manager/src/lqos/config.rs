use lqos_config::{EtcLqos, LibreQoSConfig, NetworkJsonTransport, ShapedDevice, Tunables, WebUsers};

fn get_current_lqosd_config() {
    lqos_config::EtcLqos::load().unwrap();
}

fn get_isp_config() {
    LibreQoSConfig::load().unwrap();
}

fn update_python_config(config: LibreQoSConfig) {
    config.save().unwrap();
}

pub fn requires_setup() -> bool {
	!WebUsers::does_users_file_exist().unwrap()
}

pub fn allow_anonymous() -> bool {
	if let Some(users) = Some(WebUsers::load_or_create().unwrap()) {
		return users.do_we_allow_anonymous()
	}
	false
}