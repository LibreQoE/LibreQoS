import { activeTopologySourceIntegrations, loadConfig } from "./config/config_helper";

function setText(id, text) {
    const element = document.getElementById(id);
    if (!element) return;
    element.textContent = text;
}

function setStatusAlert(state) {
    const element = document.getElementById("runtimeSetupStatus");
    if (!element) return;

    const label = state?.status_label || "Setup Required";
    const summary = state?.summary || "Choose a topology source before expecting scheduler activity.";
    const alertClass = state?.required ? "alert-warning" : "alert-success";
    element.className = `alert ${alertClass} mb-4`;
    element.innerHTML = `
        <div class="fw-semibold mb-1">${label}</div>
        <div>${summary}</div>`;
}

function renderState(state) {
    const config = window.config || {};
    const activeIntegrations = state?.active_integrations?.length
        ? state.active_integrations
        : activeTopologySourceIntegrations(config);

    setStatusAlert(state);
    setText(
        "runtimeActiveIntegrations",
        activeIntegrations.length ? activeIntegrations.join(", ") : "None configured",
    );
    setText("runtimeNetworkJson", state?.network_json_present ? "Present" : "Missing");
    setText("runtimeShapedDevices", state?.shaped_devices_present ? "Present" : "Missing");
}

function initActions() {
    const button = document.getElementById("btnOpenIntegrationProvider");
    const select = document.getElementById("runtimeIntegrationProvider");
    if (!button || !select) return;
    button.addEventListener("click", () => {
        window.location.href = select.value;
    });
}

loadConfig((msg) => {
    renderState(msg?.data?.runtime_onboarding || {});
    initActions();
}, () => {
    setStatusAlert({
        required: true,
        status_label: "Setup Required",
        summary: "Unable to load runtime setup status right now.",
    });
    initActions();
});
