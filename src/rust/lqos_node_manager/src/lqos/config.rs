use lqos_config::{EtcLqos, LibreQoSConfig, Tunables, WebUsers};

fn get_current_lqosd_config() {
    lqos_config::EtcLqos::load().unwrap();
}

fn get_isp_config() {
    LibreQoSConfig::load().unwrap();
}

fn update_python_config(config: LibreQoSConfig) {
    config.save().unwrap();
}