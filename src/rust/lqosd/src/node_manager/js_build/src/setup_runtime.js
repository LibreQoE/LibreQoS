import {t} from "./i18n/index.js";
import {
    activeTopologySourceIntegrations,
    loadConfig,
    topologySourceLabel,
} from "./config/config_helper";

function setText(id, text) {
    const element = document.getElementById(id);
    if (!element) return;
    element.textContent = text;
}

// The banner's label and summary are the server's own text when it sends them
// (`runtime_onboarding.rs`), the same way the licence page shows the authority's
// label; only the fallbacks this page supplies are catalogued.
function setStatusAlert(state) {
    const element = document.getElementById("runtimeSetupStatus");
    if (!element) return;

    const label = state?.status_label || t("setup_runtime.status.setup_required");
    const summary = state?.summary || t("setup_runtime.summary.choose_source");
    const severity = state?.status_severity || (state?.required ? "warning" : "success");
    const alertClass = severity === "danger"
        ? "alert-danger"
        : (severity === "warning"
            ? "alert-warning"
            : (severity === "info" ? "alert-info" : "alert-success"));
    element.className = `alert ${alertClass} mb-4`;
    element.innerHTML = `
        <div class="fw-semibold mb-1">${label}</div>
        <div>${summary}</div>`;
}

function renderState(state) {
    const config = window.config || {};
    const sourceLabel = topologySourceLabel(config?.topology?.source);
    const activeIntegrations = state?.active_integrations?.length
        ? state.active_integrations
        : (sourceLabel ? [sourceLabel] : activeTopologySourceIntegrations(config));

    setStatusAlert(state);
    setText(
        "runtimeActiveIntegrations",
        activeIntegrations.length ? activeIntegrations.join(", ") : t("setup_runtime.none_configured"),
    );
    setText("runtimeNetworkJson", state?.network_json_present ? t("setup_runtime.present") : t("setup_runtime.missing"));
    setText("runtimeShapedDevices", state?.shaped_devices_present ? t("setup_runtime.present") : t("setup_runtime.missing"));
}

loadConfig((msg) => {
    renderState(msg?.data?.runtime_onboarding || {});
}, () => {
    setStatusAlert({
        required: true,
        status_label: t("setup_runtime.status.setup_required"),
        summary: t("setup_runtime.load_failed"),
    });
});
