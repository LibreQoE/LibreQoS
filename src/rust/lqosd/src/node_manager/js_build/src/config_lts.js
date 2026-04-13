import {
    loadConfig,
    renderConfigMenu,
    saveConfig,
    sendWsRequest,
} from "./config/config_helper";

const licenseKeyInput = document.getElementById("licenseKey");
const toggleLicenseKeyButton = document.getElementById("toggleLicenseKey");
const clearLicenseKeyButton = document.getElementById("clearLicenseKey");
const saveButton = document.getElementById("saveButton");
const retryButton = document.getElementById("retryLicenseCheck");

function validateConfig() {
    const collationPeriod = parseInt(document.getElementById("collationPeriod").value, 10);
    if (Number.isNaN(collationPeriod) || collationPeriod < 1) {
        alert("Collation Period must be a number greater than 0");
        return false;
    }

    const uispInterval = parseInt(document.getElementById("uispInterval").value, 10);
    if (Number.isNaN(uispInterval) || uispInterval < 0) {
        alert("UISP Reporting Interval must be a number of at least 0");
        return false;
    }

    const ltsUrl = document.getElementById("ltsUrl").value.trim();
    if (ltsUrl) {
        try {
            new URL(ltsUrl);
        } catch {
            alert("Insight Server URL must be a valid URL");
            return false;
        }
    }

    return true;
}

function updateConfig() {
    window.config.long_term_stats = {
        gather_stats: document.getElementById("gatherStats").checked,
        collation_period_seconds: parseInt(document.getElementById("collationPeriod").value, 10),
        license_key: licenseKeyInput.value.trim() || null,
        uisp_reporting_interval_seconds: parseInt(document.getElementById("uispInterval").value, 10) || null,
        lts_url: document.getElementById("ltsUrl").value.trim() || null,
    };
}

function setLicenseKeyVisibility(revealed) {
    licenseKeyInput.type = revealed ? "text" : "password";
    toggleLicenseKeyButton.innerHTML = revealed
        ? '<i class="fa fa-eye-slash"></i>'
        : '<i class="fa fa-eye"></i>';
    toggleLicenseKeyButton.setAttribute(
        "aria-label",
        revealed ? "Hide license key" : "Reveal license key",
    );
}

function formatMappedCircuitLimit(limit) {
    if (limit === null || limit === undefined) {
        return "Unlimited";
    }
    return Number(limit).toLocaleString();
}

function renderCapabilityBadges(capabilities) {
    const container = document.getElementById("licenseCapabilityBadges");
    if (!container) {
        return;
    }

    const badgeSpec = [
        {
            enabled: capabilities.can_view_insight_ui,
            label: "Insight UI",
        },
        {
            enabled: capabilities.can_use_api_link,
            label: "API Docs",
        },
        {
            enabled: capabilities.can_use_support_tickets,
            label: "Support",
        },
        {
            enabled: capabilities.can_use_chatbot,
            label: "Libby",
        },
        {
            enabled: capabilities.can_receive_remote_commands,
            label: "Remote Control",
        },
        {
            enabled: capabilities.can_submit_long_term_stats,
            label: "Stats Submit",
        },
    ];

    container.innerHTML = badgeSpec
        .map((badge) => {
            const css = badge.enabled
                ? "badge rounded-pill text-bg-success-subtle text-success-emphasis border border-success-subtle"
                : "badge rounded-pill text-bg-light text-secondary border";
            return `<span class="${css}">${badge.label}</span>`;
        })
        .join("");
}

function renderAvailability(capabilities) {
    document.getElementById("licenseStateLabel").textContent = capabilities.license_state_label || "Unknown";
    document.getElementById("licenseAuthorityBadge").textContent = capabilities.authority_label || "Unknown";
    document.getElementById("controlServiceStatus").textContent = capabilities.control_service_reachable
        ? "Reachable"
        : "Unavailable";
    document.getElementById("mappedCircuitLimit").textContent = formatMappedCircuitLimit(
        capabilities.mapped_circuit_limit,
    );

    renderCapabilityBadges(capabilities);

    const alert = document.getElementById("licenseAvailabilityAlert");
    if (!alert) {
        return;
    }

    let message = "";
    if (
        !capabilities.control_service_reachable
        && (capabilities.can_use_support_tickets || capabilities.can_use_chatbot)
    ) {
        message = "license valid, control service unavailable";
    } else if (capabilities.bootstrap_suppressed) {
        message = "Automatic bootstrap retries are currently suppressed for the configured key.";
    } else if (
        capabilities.bootstrap_intent
        && !capabilities.control_service_reachable
        && capabilities.authority_label === "Bootstrap pending"
    ) {
        message = "Bootstrap is pending. Save a corrected key or use Retry License Check to try again.";
    }

    if (message) {
        alert.textContent = message;
        alert.classList.remove("d-none");
    } else {
        alert.textContent = "";
        alert.classList.add("d-none");
    }
}

function fetchCapabilities(request = { LtsCapabilities: {} }) {
    sendWsRequest(
        "LtsCapabilitiesResult",
        request,
        (msg) => {
            renderAvailability(msg.data || {});
        },
        (msg) => {
            const alert = document.getElementById("licenseAvailabilityAlert");
            if (!alert) {
                return;
            }
            alert.textContent = msg?.message || "Unable to load license status.";
            alert.classList.remove("d-none");
        },
    );
}

function wireActions() {
    setLicenseKeyVisibility(false);

    toggleLicenseKeyButton.addEventListener("click", () => {
        setLicenseKeyVisibility(licenseKeyInput.type === "password");
    });

    clearLicenseKeyButton.addEventListener("click", () => {
        licenseKeyInput.value = "";
    });

    retryButton.addEventListener("click", () => {
        retryButton.disabled = true;
        sendWsRequest(
            "LtsCapabilitiesResult",
            { LtsRetryLicenseCheck: {} },
            (msg) => {
                retryButton.disabled = false;
                renderAvailability(msg.data || {});
            },
            (msg) => {
                retryButton.disabled = false;
                alert(msg?.message || "Unable to retry license check.");
            },
        );
    });

    saveButton.addEventListener("click", () => {
        if (!validateConfig()) {
            return;
        }

        updateConfig();
        saveButton.disabled = true;
        saveConfig(
            () => {
                saveButton.disabled = false;
                fetchCapabilities();
                alert("Configuration saved successfully!");
            },
            (msg) => {
                saveButton.disabled = false;
                alert(msg?.message || "That didn't work");
            },
        );
    });
}

renderConfigMenu("lts");

loadConfig(() => {
    if (!window.config || !window.config.long_term_stats) {
        console.error("Long-term stats configuration not found in window.config");
        return;
    }

    const lts = window.config.long_term_stats;
    document.getElementById("gatherStats").checked = lts.gather_stats ?? true;
    document.getElementById("collationPeriod").value = lts.collation_period_seconds ?? 60;
    document.getElementById("uispInterval").value = lts.uisp_reporting_interval_seconds ?? 300;
    document.getElementById("ltsUrl").value = lts.lts_url ?? "";
    licenseKeyInput.value = lts.license_key ?? "";

    wireActions();
    fetchCapabilities();
});
