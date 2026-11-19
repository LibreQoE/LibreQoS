import {
    bindSecretField,
    loadConfig,
    renderConfigMenu,
    saveConfig,
    secretWillExistAfterSave,
} from "./config/config_helper";
import {t} from "./i18n/index.js";

function validateConfig() {
    const enabled = document.getElementById("enableNetzur").checked;
    if (!enabled) {
        return true;
    }

    if (!secretWillExistAfterSave("netzur_integration", "api_key", "netzurApiKey")) {
        alert(t("config.netzur.validation.key_required"));
        return false;
    }

    const urlValue = document.getElementById("netzurApiUrl").value.trim();
    if (!urlValue) {
        alert(t("config.netzur.validation.url_required"));
        return false;
    }

    try {
        new URL(urlValue);
    } catch (_) {
        alert(t("config.netzur.validation.url_invalid"));
        return false;
    }

    const timeout = parseInt(document.getElementById("netzurTimeout").value, 10);
    if (Number.isNaN(timeout) || timeout <= 0) {
        alert(t("config.netzur.validation.timeout_positive"));
        return false;
    }

    return true;
}

function updateConfig() {
    window.config.netzur_integration = {
        enable_netzur: document.getElementById("enableNetzur").checked,
        api_key: document.getElementById("netzurApiKey").value.trim(),
        api_url: document.getElementById("netzurApiUrl").value.trim(),
        timeout_secs: parseInt(document.getElementById("netzurTimeout").value, 10) || 60
    };
}

renderConfigMenu('netzur');

loadConfig(() => {
    if (!window.config) {
        // translatable: false — "Configuration not loaded" is a console diagnostic, never rendered
        console.error("Configuration not loaded");
        return;
    }

    const cfg = window.config.netzur_integration ?? {};
    document.getElementById("enableNetzur").checked = cfg.enable_netzur ?? false;
    document.getElementById("netzurApiUrl").value = cfg.api_url ?? "";
    document.getElementById("netzurTimeout").value = cfg.timeout_secs ?? 60;
    bindSecretField({
        section: "netzur_integration",
        field: "api_key",
        inputId: "netzurApiKey",
        statusId: "netzurApiKeyStatus",
        clearButtonId: "clearNetzurApiKey",
    });
    document.getElementById("saveNetzur").addEventListener('click', () => {
        if (validateConfig()) {
            updateConfig();
            saveConfig(() => alert(t("config.netzur.saved")));
        }
    });
});
