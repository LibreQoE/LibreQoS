import {
    bindSecretField,
    loadConfig,
    renderConfigMenu,
    saveConfig,
    secretWillExistAfterSave,
} from "./config/config_helper";
import {t} from "./i18n/index.js";

function validateConfig() {
    // Validate required fields when enabled
    if (document.getElementById("enablePowercode").checked) {
        if (!secretWillExistAfterSave("powercode_integration", "powercode_api_key", "powercodeApiKey")) {
            alert(t("config.powercode.validation.key_required"));
            return false;
        }

        const apiUrl = document.getElementById("powercodeApiUrl").value.trim();
        if (!apiUrl) {
            alert(t("config.powercode.validation.url_required"));
            return false;
        }
        try {
            new URL(apiUrl);
        } catch {
            alert(t("config.powercode.validation.url_invalid"));
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

// Render the configuration menu
renderConfigMenu('powercode');

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    if (window.config && window.config.powercode_integration) {
        const powercode = window.config.powercode_integration;
        
        // Boolean field
        document.getElementById("enablePowercode").checked = 
            powercode.enable_powercode ?? false;

        // String fields
        document.getElementById("powercodeApiUrl").value = 
            powercode.powercode_api_url ?? "";
        bindSecretField({
            section: "powercode_integration",
            field: "powercode_api_key",
            inputId: "powercodeApiKey",
            statusId: "powercodeApiKeyStatus",
            clearButtonId: "clearPowercodeApiKey",
        });

        // Add save button click handler
        document.getElementById('saveButton').addEventListener('click', () => {
            if (validateConfig()) {
                updateConfig();
                saveConfig(() => {
                    alert(t("config.powercode.saved"));
                });
            }
        });
    } else {
        // translatable: false — "Powercode integration configuration not found in window.config" is a console diagnostic, never rendered
        console.error("Powercode integration configuration not found in window.config");
    }
});
