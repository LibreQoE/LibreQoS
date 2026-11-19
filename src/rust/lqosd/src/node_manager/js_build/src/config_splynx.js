import {
    bindSecretField,
    loadConfig,
    renderConfigMenu,
    saveConfig,
    secretWillExistAfterSave,
} from "./config/config_helper";
import {renderMarkupText} from "./config/markup_text.mjs";
import {t, tHtml} from "./i18n/index.js";

function validateConfig() {
    // Validate required fields when enabled
    if (document.getElementById("enableSplynx").checked) {
        if (!secretWillExistAfterSave("splynx_integration", "api_key", "apiKey")) {
            alert(t("config.splynx.validation.key_required"));
            return false;
        }

        if (!secretWillExistAfterSave("splynx_integration", "api_secret", "apiSecret")) {
            alert(t("config.splynx.validation.secret_required"));
            return false;
        }

        const url = document.getElementById("splynxUrl").value.trim();
        if (!url) {
            alert(t("config.splynx.validation.url_required"));
            return false;
        }
        try {
            new URL(url);
        } catch {
            alert(t("config.splynx.validation.url_invalid"));
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
// The compile-mode hint wraps `Integration - Common` in `<code>`, which the DOM
// applier cannot keep: the paragraph is empty in the page and rendered here.
renderMarkupText({splynxCompileModeHelp: tHtml("config.splynx.compile_mode_help")});

loadConfig(() => {
    if (!window.config) {
        // translatable: false — "Configuration not loaded" is a console diagnostic, never rendered
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
                alert(t("config.splynx.saved"));
            });
        }
    });
});
