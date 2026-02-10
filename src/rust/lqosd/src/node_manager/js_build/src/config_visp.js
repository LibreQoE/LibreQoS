import {saveConfig, loadConfig, renderConfigMenu} from "./config/config_helper";

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

    const clientSecret = document.getElementById("vispClientSecret").value.trim();
    if (!clientSecret) {
        alert("Client Secret is required when VISP integration is enabled");
        return false;
    }

    const username = document.getElementById("vispUsername").value.trim();
    if (!username) {
        alert("Appuser Username is required when VISP integration is enabled");
        return false;
    }

    const password = document.getElementById("vispPassword").value.trim();
    if (!password) {
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
    document.getElementById("vispClientSecret").value = cfg.client_secret ?? "";
    document.getElementById("vispUsername").value = cfg.username ?? "";
    document.getElementById("vispPassword").value = cfg.password ?? "";
    document.getElementById("vispIspId").value = cfg.isp_id ?? "";
    document.getElementById("vispTimeout").value = cfg.timeout_secs ?? 20;
    document.getElementById("vispOnlineUsersDomain").value = cfg.online_users_domain ?? "";

    document.getElementById("saveVisp").addEventListener('click', () => {
        if (validateConfig()) {
            updateConfig();
            saveConfig(() => alert("VISP configuration saved"));
        }
    });
});
