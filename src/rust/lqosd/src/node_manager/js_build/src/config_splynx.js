import {
    bindSecretField,
    loadConfig,
    renderConfigMenu,
    saveConfig,
    secretWillExistAfterSave,
} from "./config/config_helper";

function validateConfig() {
    // Validate required fields when enabled
    if (document.getElementById("enableSplynx").checked) {
        if (!secretWillExistAfterSave("splynx_integration", "api_key", "apiKey")) {
            alert("API Key is required when Splynx integration is enabled");
            return false;
        }

        if (!secretWillExistAfterSave("splynx_integration", "api_secret", "apiSecret")) {
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
        strategy: window.config.topology?.compile_mode ?? document.getElementById("topologyStrategy").value
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
    document.getElementById("splynxUrl").value = splynx.url ?? "";
    document.getElementById("topologyStrategy").value = window.config.topology?.compile_mode ?? splynx.strategy ?? "ap_site";
    bindSecretField({
        section: "splynx_integration",
        field: "api_key",
        inputId: "apiKey",
        statusId: "apiKeyStatus",
        clearButtonId: "clearApiKey",
    });
    bindSecretField({
        section: "splynx_integration",
        field: "api_secret",
        inputId: "apiSecret",
        statusId: "apiSecretStatus",
        clearButtonId: "clearApiSecret",
    });

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
