import {saveConfig, loadConfig} from "./config/config_helper";

function validateConfig() {
    // Validate required fields when enabled
    if (document.getElementById("enableWispgate").checked) {
        const apiKey = document.getElementById("wispgate_api_token").value.trim();
        if (!apiKey) {
            alert("API Key is required when WispGate integration is enabled");
            return false;
        }

        const apiUrl = document.getElementById("wispgate_api_url").value.trim();
        if (!apiUrl) {
            alert("API URL is required when WispGate integration is enabled");
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
    window.config.wispgate_integration = {
        enable_wispgate: document.getElementById("enableWispgate").checked,
        wispgate_api_token: document.getElementById("wispgate_api_token").value.trim(),
        wispgate_api_url: document.getElementById("wispgate_api_url").value.trim()
    };
}

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    if (window.config && window.config.wispgate_integration) {
        const wispgateIntegration = window.config.wispgate_integration;

        // Boolean field
        document.getElementById("enableWispgate").checked =
            wispgateIntegration.enable_wispgate ?? false;

        // String fields
        document.getElementById("wispgate_api_token").value =
            wispgateIntegration.wispgate_api_token ?? "";
        document.getElementById("wispgate_api_url").value =
            wispgateIntegration.wispgate_api_url ?? "";

        // Add save button click handler
        document.getElementById('saveButton').addEventListener('click', () => {
            if (validateConfig()) {
                updateConfig();
                saveConfig(() => {
                    alert("Configuration saved successfully!");
                });
            }
        });
    }
});
