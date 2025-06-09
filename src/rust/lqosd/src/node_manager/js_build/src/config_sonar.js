import {saveConfig, loadConfig, renderConfigMenu} from "./config/config_helper";

function arrayToString(arr) {
    return arr ? arr.join(', ') : '';
}

function stringToArray(str) {
    return str ? str.split(',').map(s => s.trim()).filter(s => s.length > 0) : [];
}

function validateConfig() {
    // Validate required fields when enabled
    if (document.getElementById("enableSonar").checked) {
        const apiUrl = document.getElementById("sonarApiUrl").value.trim();
        if (!apiUrl) {
            alert("API URL is required when Sonar integration is enabled");
            return false;
        }
        try {
            new URL(apiUrl);
        } catch {
            alert("API URL must be a valid URL");
            return false;
        }

        const apiKey = document.getElementById("sonarApiKey").value.trim();
        if (!apiKey) {
            alert("API Key is required when Sonar integration is enabled");
            return false;
        }

        const snmpCommunity = document.getElementById("snmpCommunity").value.trim();
        if (!snmpCommunity) {
            alert("SNMP Community is required when Sonar integration is enabled");
            return false;
        }
    }
    return true;
}

function updateConfig() {
    // Update only the sonar_integration section
    window.config.sonar_integration = {
        enable_sonar: document.getElementById("enableSonar").checked,
        sonar_api_url: document.getElementById("sonarApiUrl").value.trim(),
        sonar_api_key: document.getElementById("sonarApiKey").value.trim(),
        snmp_community: document.getElementById("snmpCommunity").value.trim(),
        airmax_model_ids: stringToArray(document.getElementById("airmaxModelIds").value),
        ltu_model_ids: stringToArray(document.getElementById("ltuModelIds").value),
        active_status_ids: stringToArray(document.getElementById("activeStatusIds").value)
    };
}

// Render the configuration menu
renderConfigMenu('sonar');

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    if (window.config && window.config.sonar_integration) {
        const sonar = window.config.sonar_integration;
        
        // Boolean field
        document.getElementById("enableSonar").checked = 
            sonar.enable_sonar ?? false;

        // String fields
        document.getElementById("sonarApiUrl").value = 
            sonar.sonar_api_url ?? "";
        document.getElementById("sonarApiKey").value = 
            sonar.sonar_api_key ?? "";
        document.getElementById("snmpCommunity").value = 
            sonar.snmp_community ?? "public";

        // Array fields (convert to comma-separated strings)
        document.getElementById("airmaxModelIds").value = 
            arrayToString(sonar.airmax_model_ids);
        document.getElementById("ltuModelIds").value = 
            arrayToString(sonar.ltu_model_ids);
        document.getElementById("activeStatusIds").value = 
            arrayToString(sonar.active_status_ids);

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
        console.error("Sonar integration configuration not found in window.config");
    }
});
