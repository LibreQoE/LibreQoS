import {
    bindSecretField,
    loadConfig,
    renderConfigMenu,
    saveConfig,
    sendWsRequest,
} from "./config/config_helper";
import {
    copyLocalApiToken,
    generateLocalApiToken,
} from "./config/local_api_token";

const licenseKeyInput = document.getElementById("licenseKey");
const toggleLicenseKeyButton = document.getElementById("toggleLicenseKey");
const clearLicenseKeyButton = document.getElementById("clearLicenseKey");
const localApiTokenInput = document.getElementById("localApiBearerToken");
const toggleLocalApiTokenButton = document.getElementById("toggleLocalApiBearerToken");
const generateLocalApiTokenButton = document.getElementById("generateLocalApiBearerToken");
const copyLocalApiTokenButton = document.getElementById("copyLocalApiBearerToken");
const clearLocalApiTokenButton = document.getElementById("clearLocalApiBearerToken");
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
    window.config.local_api = {
        bearer_token: localApiTokenInput.value.trim() || null,
    };
}

function setSecretVisibility(input, button, revealed, label) {
    input.type = revealed ? "text" : "password";
    button.innerHTML = revealed
        ? '<i class="fa fa-eye-slash"></i>'
        : '<i class="fa fa-eye"></i>';
    button.setAttribute(
        "aria-label",
        revealed ? `Hide ${label}` : `Reveal ${label}`,
    );
}

function updateLocalApiTokenActions() {
    const hasCurrentToken = localApiTokenInput.value.trim().length > 0;
    toggleLocalApiTokenButton.disabled = !hasCurrentToken;
    copyLocalApiTokenButton.disabled = !hasCurrentToken;
    if (!hasCurrentToken) {
        setSecretVisibility(
            localApiTokenInput,
            toggleLocalApiTokenButton,
            false,
            "local API bearer token",
        );
    }
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

    const apiDocsStatus = capabilities.can_use_api_link
        ? (window.apiServiceAvailable
            ? { label: "API Docs: available", tone: "success" }
            : { label: "API Docs: service unavailable", tone: "warning" })
        : { label: "API Docs: license required", tone: "secondary" };
    const badgeSpec = [
        {
            tone: capabilities.can_view_insight_ui ? "success" : "secondary",
            label: "Insight UI",
        },
        apiDocsStatus,
        {
            tone: capabilities.can_use_support_tickets ? "success" : "secondary",
            label: "Support",
        },
        {
            tone: capabilities.can_use_chatbot ? "success" : "secondary",
            label: "Libby",
        },
        {
            tone: capabilities.can_receive_remote_commands ? "success" : "secondary",
            label: "Remote Control",
        },
        {
            tone: capabilities.can_submit_long_term_stats ? "success" : "secondary",
            label: "Stats Submit",
        },
    ];

    container.innerHTML = badgeSpec
        .map((badge) => {
            const css = badge.tone === "success"
                ? "badge rounded-pill text-bg-success-subtle text-success-emphasis border border-success-subtle"
                : badge.tone === "warning"
                    ? "badge rounded-pill text-bg-warning-subtle text-warning-emphasis border border-warning-subtle"
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
    setSecretVisibility(licenseKeyInput, toggleLicenseKeyButton, false, "license key");
    setSecretVisibility(
        localApiTokenInput,
        toggleLocalApiTokenButton,
        false,
        "local API bearer token",
    );
    updateLocalApiTokenActions();

    toggleLicenseKeyButton.addEventListener("click", () => {
        setSecretVisibility(
            licenseKeyInput,
            toggleLicenseKeyButton,
            licenseKeyInput.type === "password",
            "license key",
        );
    });

    clearLicenseKeyButton.addEventListener("click", () => {
        licenseKeyInput.value = "";
    });

    toggleLocalApiTokenButton.addEventListener("click", () => {
        setSecretVisibility(
            localApiTokenInput,
            toggleLocalApiTokenButton,
            localApiTokenInput.type === "password",
            "local API bearer token",
        );
    });

    localApiTokenInput.addEventListener("input", updateLocalApiTokenActions);
    clearLocalApiTokenButton.addEventListener("click", updateLocalApiTokenActions);

    generateLocalApiTokenButton.addEventListener("click", () => {
        try {
            localApiTokenInput.value = generateLocalApiToken();
            setSecretVisibility(
                localApiTokenInput,
                toggleLocalApiTokenButton,
                true,
                "local API bearer token",
            );
            localApiTokenInput.dispatchEvent(new Event("input"));
        } catch (error) {
            alert(error instanceof Error ? error.message : "Unable to generate a secure token.");
        }
    });

    copyLocalApiTokenButton.addEventListener("click", async () => {
        const copyStatus = document.getElementById("localApiCopyStatus");
        try {
            await copyLocalApiToken(localApiTokenInput);
            if (copyStatus) {
                copyStatus.textContent = "Local API bearer token copied.";
            }
            copyLocalApiTokenButton.innerHTML = '<i class="fa fa-check me-1"></i> Copied';
            setTimeout(() => {
                copyLocalApiTokenButton.innerHTML = '<i class="fa fa-copy me-1"></i> Copy';
            }, 1500);
        } catch (error) {
            if (copyStatus) {
                copyStatus.textContent = "";
            }
            alert(error instanceof Error ? error.message : "Unable to copy the token.");
        }
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
                updateLocalApiTokenActions();
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
    localApiTokenInput.value = "";

    bindSecretField({
        section: "local_api",
        field: "bearer_token",
        inputId: "localApiBearerToken",
        statusId: "localApiBearerTokenStatus",
        clearButtonId: "clearLocalApiBearerToken",
        configuredMessage: "A local API token is stored but cannot be shown again. Leave blank to keep it, or generate a replacement.",
        emptyMessage: "No local API token is stored.",
    });

    wireActions();
    fetchCapabilities();
});
