import {loadConfig} from "./config/config_helper";

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    if (window.config && window.config.integration_common) {
        const integration = window.config.integration_common;
        
        // Boolean fields
        document.getElementById("circuitNameAsAddress").checked = 
            integration.circuit_name_as_address ?? false;
        document.getElementById("alwaysOverwriteNetworkJson").checked = 
            integration.always_overwrite_network_json ?? false;

        // Numeric field
        if (integration.queue_refresh_interval_mins) {
            document.getElementById("queueRefreshInterval").value = 
                integration.queue_refresh_interval_mins;
        }
    } else {
        console.error("Integration configuration not found in window.config");
    }
});
