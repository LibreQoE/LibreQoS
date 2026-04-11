use lqos_bus::BusResponse;
use lqos_config::ConfigShapedDevices;
use lqos_topology_compile::TopologyImportFile;

pub fn validate_shaped_devices_csv() -> BusResponse {
    let result: anyhow::Result<ConfigShapedDevices> = match lqos_config::load_config() {
        Ok(config) => {
            if integration_ingress_enabled(config.as_ref()) {
                match TopologyImportFile::load(config.as_ref()) {
                    Ok(Some(topology_import)) => {
                        Ok(topology_import.into_imported_bundle().shaped_devices)
                    }
                    Ok(None) => Ok(ConfigShapedDevices::default()),
                    Err(err) => Err(err),
                }
            } else {
                ConfigShapedDevices::load().map_err(anyhow::Error::from)
            }
        }
        Err(err) => Err(anyhow::Error::from(err)),
    };
    match result {
        Ok(..) => BusResponse::Ack,
        Err(e) => BusResponse::ShapedDevicesValidation(format!("{e:#?}")),
    }
}

fn integration_ingress_enabled(config: &lqos_config::Config) -> bool {
    config.uisp_integration.enable_uisp
        || config.splynx_integration.enable_splynx
        || config
            .netzur_integration
            .as_ref()
            .is_some_and(|integration| integration.enable_netzur)
        || config
            .visp_integration
            .as_ref()
            .is_some_and(|integration| integration.enable_visp)
        || config.powercode_integration.enable_powercode
        || config.sonar_integration.enable_sonar
        || config
            .wispgate_integration
            .as_ref()
            .is_some_and(|integration| integration.enable_wispgate)
}
