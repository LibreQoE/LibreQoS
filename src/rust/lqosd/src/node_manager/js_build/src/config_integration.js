import {saveConfig, loadConfig} from "./config/config_helper";

function validateConfig() {
    // Validate queue refresh interval
    const interval = parseInt(document.getElementById("queueRefreshInterval").value);
    if (isNaN(interval) || interval < 1) {
        alert("Queue Refresh Interval must be a number greater than 0");
        return false;
    }
    
    // Validate promote_to_root entries
    const rawPromote = document.getElementById("promoteToRoot").value;
    const hasInvalidEntries = rawPromote.split('\n')
        .some(line => line.trim().length === 0 && rawPromote.trim().length > 0);
    if (hasInvalidEntries) {
        alert("Please remove empty lines from Promote to Root Nodes");
        return false;
    }
    return true;
}

function updateConfig() {
    // Update only the integration_common section
    window.config.integration_common = {
        circuit_name_as_address: document.getElementById("circuitNameAsAddress").checked,
        always_overwrite_network_json: document.getElementById("alwaysOverwriteNetworkJson").checked,
        queue_refresh_interval_mins: parseInt(document.getElementById("queueRefreshInterval").value),
        promote_to_root: (() => {
            const raw = document.getElementById("promoteToRoot").value;
            const list = raw.split('\n')
                .map(line => line.trim())
                .filter(line => line.length > 0);
            return list.length > 0 ? list : null;
        })()
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

        // Promote to root field
        const promoteRoot = integration.promote_to_root ? integration.promote_to_root.join('\n') : '';
        document.getElementById("promoteToRoot").value = promoteRoot;

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
