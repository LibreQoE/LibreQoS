import {saveConfig, loadConfig} from "./config/config_helper";

function validateConfig() {
    // Validate queue refresh interval
    const interval = parseInt(document.getElementById("queueRefreshInterval").value);
    if (isNaN(interval) || interval < 1) {
        alert("Queue Refresh Interval must be a number greater than 0");
        return false;
    }
    return true;
}

function updateConfig() {
    // Update only the integration_common section
    window.config.integration_common = {
        circuit_name_as_address: document.getElementById("circuitNameAsAddress").checked,
        always_overwrite_network_json: document.getElementById("alwaysOverwriteNetworkJson").checked,
        queue_refresh_interval_mins: parseInt(document.getElementById("queueRefreshInterval").value)
    };
}

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
        document.getElementById("queueRefreshInterval").value = 
            integration.queue_refresh_interval_mins ?? 30;

        // Add save button click handler
        document.getElementById('saveButton').addEventListener('click', () => {
            if (validateConfig()) {
                updateConfig();
                saveConfig(() => {
                    alert("Configuration saved successfully!");
                });
            }
        });
    } else {
        console.error("Integration configuration not found in window.config");
    }
});
