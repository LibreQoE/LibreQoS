import {saveConfig, loadConfig, renderConfigMenu} from "./config/config_helper";

function validateConfig() {
    // Validate required fields when enabled
    if (document.getElementById("enableSplynx").checked) {
        const apiKey = document.getElementById("apiKey").value.trim();
        if (!apiKey) {
            alert("API Key is required when Splynx integration is enabled");
            return false;
        }

        const apiSecret = document.getElementById("apiSecret").value.trim();
        if (!apiSecret) {
            alert("API Secret is required when Splynx integration is enabled");
            return false;
        }

        const url = document.getElementById("spylnxUrl").value.trim();
        if (!url) {
            alert("Splynx URL is required when Splynx integration is enabled");
            return false;
        }
        try {
            new URL(url);
        } catch {
            alert("Splynx URL must be a valid URL");
            return false;
        }
    }
    return true;
}

function updateConfig() {
    // Update only the spylnx_integration section
    window.config.spylnx_integration = {
        enable_spylnx: document.getElementById("enableSplynx").checked,
        api_key: document.getElementById("apiKey").value.trim(),
        api_secret: document.getElementById("apiSecret").value.trim(),
        url: document.getElementById("spylnxUrl").value.trim()
    };
}

// Render the configuration menu
renderConfigMenu('spylnx');

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    if (window.config && window.config.spylnx_integration) {
        const spylnx = window.config.spylnx_integration;
        
        // Boolean field
        document.getElementById("enableSplynx").checked = 
            spylnx.enable_spylnx ?? false;

        // String fields
        document.getElementById("apiKey").value =
            spylnx.api_key ?? "";
        document.getElementById("apiSecret").value =
            spylnx.api_secret ?? "";
        document.getElementById("spylnxUrl").value = 
            spylnx.url ?? "";

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
        console.error("Splynx integration configuration not found in window.config");
    }
});
