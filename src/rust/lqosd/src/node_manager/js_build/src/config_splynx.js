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

        const url = document.getElementById("splynxUrl").value.trim();
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
    // Update only the splynx_integration section
    window.config.splynx_integration = {
        enable_splynx: document.getElementById("enableSplynx").checked,
        api_key: document.getElementById("apiKey").value.trim(),
        api_secret: document.getElementById("apiSecret").value.trim(),
        url: document.getElementById("splynxUrl").value.trim(),
        strategy: document.getElementById("topologyStrategy").value
    };
}

// Render the configuration menu
renderConfigMenu('splynx');

loadConfig(() => {
    if (!window.config) {
        console.error("Configuration not loaded");
        return;
    }

    // window.config now contains the configuration.
    // Populate form fields with config values
    const splynx = window.config.splynx_integration;
    document.getElementById("enableSplynx").checked = splynx.enable_splynx ?? false;
    document.getElementById("apiKey").value = splynx.api_key ?? "";
    document.getElementById("apiSecret").value = splynx.api_secret ?? "";
    document.getElementById("splynxUrl").value = splynx.url ?? "";
    document.getElementById("topologyStrategy").value = splynx.strategy ?? "ap_only";

    // Add save button click handler
    document.getElementById('saveButton').addEventListener('click', () => {
        if (validateConfig()) {
            updateConfig();
            saveConfig(() => {
                alert("Configuration saved successfully!");
            });
        }
    });
});
