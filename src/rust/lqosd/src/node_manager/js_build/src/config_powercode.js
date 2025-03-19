import {saveConfig, loadConfig} from "./config/config_helper";

function validateConfig() {
    // Validate required fields when enabled
    if (document.getElementById("enablePowercode").checked) {
        const apiKey = document.getElementById("powercodeApiKey").value.trim();
        if (!apiKey) {
            alert("API Key is required when Powercode integration is enabled");
            return false;
        }

        const apiUrl = document.getElementById("powercodeApiUrl").value.trim();
        if (!apiUrl) {
            alert("API URL is required when Powercode integration is enabled");
            return false;
        }
        try {
            new URL(apiUrl);
        } catch {
            alert("API URL must be a valid URL");
            return false;
        }
    }
    return true;
}

function updateConfig() {
    // Update only the powercode_integration section
    window.config.powercode_integration = {
        enable_powercode: document.getElementById("enablePowercode").checked,
        powercode_api_key: document.getElementById("powercodeApiKey").value.trim(),
        powercode_api_url: document.getElementById("powercodeApiUrl").value.trim()
    };
}

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    if (window.config && window.config.powercode_integration) {
        const powercode = window.config.powercode_integration;
        
        // Boolean field
        document.getElementById("enablePowercode").checked = 
            powercode.enable_powercode ?? false;

        // String fields
        document.getElementById("powercodeApiKey").value = 
            powercode.powercode_api_key ?? "";
        document.getElementById("powercodeApiUrl").value = 
            powercode.powercode_api_url ?? "";

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
        console.error("Powercode integration configuration not found in window.config");
    }
});
