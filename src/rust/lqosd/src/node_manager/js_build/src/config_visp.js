import {
    bindSecretField,
    loadConfig,
    renderConfigMenu,
    saveConfig,
    secretWillExistAfterSave,
} from "./config/config_helper";

function validateConfig() {
    const enabled = document.getElementById("enableVisp").checked;
    if (!enabled) {
        return true;
    }

    const clientId = document.getElementById("vispClientId").value.trim();
    if (!clientId) {
        alert("Client ID is required when VISP integration is enabled");
        return false;
    }

    if (!secretWillExistAfterSave("visp_integration", "client_secret", "vispClientSecret")) {
        alert("Client Secret is required when VISP integration is enabled");
        return false;
    }

    const username = document.getElementById("vispUsername").value.trim();
    if (!username) {
        alert("Appuser Username is required when VISP integration is enabled");
        return false;
    }

    if (!secretWillExistAfterSave("visp_integration", "password", "vispPassword")) {
        alert("Appuser Password is required when VISP integration is enabled");
        return false;
    }

    const timeout = parseInt(document.getElementById("vispTimeout").value, 10);
    if (Number.isNaN(timeout) || timeout <= 0) {
        alert("Timeout must be a positive number of seconds");
        return false;
    }

    return true;
}

function updateConfig() {
    const ispIdRaw = document.getElementById("vispIspId").value;
    const ispIdParsed = ispIdRaw !== "" ? parseInt(ispIdRaw, 10) : null;

    window.config.visp_integration = {
        enable_visp: document.getElementById("enableVisp").checked,
        client_id: document.getElementById("vispClientId").value.trim(),
        client_secret: document.getElementById("vispClientSecret").value.trim(),
        username: document.getElementById("vispUsername").value.trim(),
        password: document.getElementById("vispPassword").value.trim(),
        isp_id: (ispIdParsed !== null && !Number.isNaN(ispIdParsed) && ispIdParsed > 0) ? ispIdParsed : null,
        online_users_domain: document.getElementById("vispOnlineUsersDomain").value.trim() || null,
        timeout_secs: parseInt(document.getElementById("vispTimeout").value, 10) || 20
    };
}

renderConfigMenu('visp');

loadConfig(() => {
    if (!window.config) {
        console.error("Configuration not loaded");
        return;
    }

    const cfg = window.config.visp_integration ?? {};
    document.getElementById("enableVisp").checked = cfg.enable_visp ?? false;
    document.getElementById("vispClientId").value = cfg.client_id ?? "";
    document.getElementById("vispUsername").value = cfg.username ?? "";
    document.getElementById("vispIspId").value = cfg.isp_id ?? "";
    document.getElementById("vispTimeout").value = cfg.timeout_secs ?? 20;
    document.getElementById("vispOnlineUsersDomain").value = cfg.online_users_domain ?? "";
    bindSecretField({
        section: "visp_integration",
        field: "client_secret",
        inputId: "vispClientSecret",
        statusId: "vispClientSecretStatus",
        clearButtonId: "clearVispClientSecret",
    });
    bindSecretField({
        section: "visp_integration",
        field: "password",
        inputId: "vispPassword",
        statusId: "vispPasswordStatus",
        clearButtonId: "clearVispPassword",
    });

    document.getElementById("saveVisp").addEventListener('click', () => {
        if (validateConfig()) {
            updateConfig();
            saveConfig(() => alert("VISP configuration saved"));
        }
    });
});
