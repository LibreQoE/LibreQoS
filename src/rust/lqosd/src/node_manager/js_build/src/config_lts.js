import { loadConfig, renderConfigMenu, saveConfig, sendWsRequest } from "./config/config_helper";
import {
    MAX_LOCAL_API_KEYS,
    abbreviateLocalApiKeyId,
    clearLocalApiKeyInput,
    copyLocalApiToken,
    formatLocalApiKeyCreatedAt,
    setLocalApiKeyCreationPending,
    validateLocalApiKeyName,
} from "./config/local_api_token";

const licenseKeyInput = document.getElementById("licenseKey");
const toggleLicenseKeyButton = document.getElementById("toggleLicenseKey");
const clearLicenseKeyButton = document.getElementById("clearLicenseKey");
const localApiKeyModalElement = document.getElementById("localApiKeyModal");
const localApiKeyNameInput = document.getElementById("localApiKeyName");
const newLocalApiKeyInput = document.getElementById("newLocalApiKey");
const confirmGenerateLocalApiKeyButton = document.getElementById("confirmGenerateLocalApiKey");
const copyNewLocalApiKeyButton = document.getElementById("copyNewLocalApiKey");
const toggleNewLocalApiKeyButton = document.getElementById("toggleNewLocalApiKey");
const saveButton = document.getElementById("saveButton");
const retryButton = document.getElementById("retryLicenseCheck");
let localApiManagementInFlight = false;

function beginLocalApiManagement(message) {
    if (localApiManagementInFlight) {
        document.getElementById("localApiKeysStatus").textContent = "Another local API key operation is still in progress.";
        return false;
    }
    localApiManagementInFlight = true;
    document.getElementById("localApiKeysStatus").textContent = message;
    return true;
}

function finishLocalApiManagement() {
    localApiManagementInFlight = false;
}

function focusLocalApiKeySection() {
    const generateButton = document.getElementById("generateLocalApiKey");
    const target = generateButton.disabled
        ? document.getElementById("localApiKeysHeading")
        : generateButton;
    target.focus();
}

function refreshLocalApiKeys() {
    loadConfig(
        () => renderLocalApiKeys(),
        () => {
            document.getElementById("localApiKeysStatus").textContent = "Unable to refresh the local API key list.";
        },
    );
}

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
    window.config.local_api = window.config.local_api || {};
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

function localApiKeys() {
    return Array.isArray(window.config?.local_api?.keys) ? window.config.local_api.keys : [];
}

function renderLocalApiKeys() {
    const body = document.getElementById("localApiKeysTableBody");
    const keys = localApiKeys();
    body.replaceChildren();
    for (const key of keys) {
        const row = document.createElement("tr");
        const name = document.createElement("td");
        name.textContent = key.name || "Unnamed key";
        const created = document.createElement("td");
        created.textContent = formatLocalApiKeyCreatedAt(key.created_at_unix);
        const id = document.createElement("td");
        const code = document.createElement("code");
        code.textContent = abbreviateLocalApiKeyId(key.id);
        code.title = key.id || "";
        id.append(code);
        const action = document.createElement("td");
        action.className = "text-end";
        const revoke = document.createElement("button");
        revoke.type = "button";
        revoke.className = "btn btn-sm btn-outline-danger";
        revoke.innerHTML = '<i class="fa fa-trash me-1"></i> Revoke';
        revoke.setAttribute("aria-label", `Revoke API key ${key.name || key.id}`);
        revoke.addEventListener("click", () => revokeLocalApiKey(key, revoke));
        action.append(revoke);
        row.append(name, created, id, action);
        body.append(row);
    }
    document.getElementById("localApiKeysEmpty").classList.toggle("d-none", keys.length > 0);
    document.getElementById("localApiKeyCount").textContent = `${keys.length} of ${MAX_LOCAL_API_KEYS} keys`;
    document.getElementById("generateLocalApiKey").disabled = keys.length >= MAX_LOCAL_API_KEYS;
    const legacy = !!window.configSecretState?.local_api?.bearer_token;
    document.getElementById("legacyLocalApiKey").classList.toggle("d-none", !legacy);
}

function revokeLocalApiKey(key, button) {
    if (!window.confirm(`Revoke the local API key “${key.name}”?`)) return;
    if (!beginLocalApiManagement(`Revoking API key ${key.name}…`)) return;
    button.disabled = true;
    sendWsRequest("RevokeLocalApiKeyResult", { RevokeLocalApiKey: { id: key.id } }, (msg) => {
        finishLocalApiManagement();
        if (!msg?.ok) {
            button.disabled = false;
            alert(msg?.message || "Unable to revoke the API key.");
            return;
        }
        window.config.local_api.keys = localApiKeys().filter((entry) => entry.id !== key.id);
        renderLocalApiKeys();
        document.getElementById("localApiKeysStatus").textContent = `API key ${key.name} revoked. It may take up to 30 seconds to stop working.`;
        focusLocalApiKeySection();
    }, (msg) => {
        finishLocalApiManagement();
        button.disabled = false;
        alert(msg?.message || "Unable to revoke the API key.");
        refreshLocalApiKeys();
    }, {
        timeoutMs: 15000,
        timeoutMessage: "Timed out while revoking the API key. Refresh the key list before trying again.",
    });
}

function resetLocalApiKeyModal() {
    clearLocalApiKeyInput(newLocalApiKeyInput);
    localApiKeyNameInput.value = "";
    localApiKeyNameInput.classList.remove("is-invalid");
    document.getElementById("localApiKeyNameError").textContent = "";
    document.getElementById("newLocalApiKeyCopyStatus").textContent = "";
    const operationStatus = document.getElementById("localApiKeyOperationStatus");
    operationStatus.className = "small text-secondary mt-3";
    operationStatus.textContent = "";
    copyNewLocalApiKeyButton.innerHTML = '<i class="fa fa-copy me-1"></i> Copy';
    document.getElementById("localApiKeyNameStep").classList.remove("d-none");
    document.getElementById("localApiKeyResultStep").classList.add("d-none");
    confirmGenerateLocalApiKeyButton.classList.remove("d-none");
    setLocalApiKeyCreationPending(
        confirmGenerateLocalApiKeyButton,
        localApiKeyModalElement,
        operationStatus,
        false,
    );
    setSecretVisibility(newLocalApiKeyInput, toggleNewLocalApiKeyButton, false, "generated API key");
}

function showNameError(message) {
    document.getElementById("localApiKeyNameError").textContent = message;
    localApiKeyNameInput.classList.add("is-invalid");
    localApiKeyNameInput.focus();
}

function showLocalApiOperationError(message) {
    const status = document.getElementById("localApiKeyOperationStatus");
    status.className = "alert alert-danger mt-3 py-2";
    status.textContent = message;
    status.focus();
}

function createLocalApiKey() {
    const validation = validateLocalApiKeyName(
        localApiKeyNameInput.value,
        localApiKeys().map((key) => key.name),
    );
    if (!validation.ok) {
        showNameError(validation.message);
        return;
    }
    if (!beginLocalApiManagement(`Creating API key ${validation.name}…`)) return;
    localApiKeyNameInput.classList.remove("is-invalid");
    const operationStatus = document.getElementById("localApiKeyOperationStatus");
    setLocalApiKeyCreationPending(
        confirmGenerateLocalApiKeyButton,
        localApiKeyModalElement,
        operationStatus,
        true,
    );
    sendWsRequest("CreateLocalApiKeyResult", { CreateLocalApiKey: { name: validation.name } }, (msg) => {
        finishLocalApiManagement();
        setLocalApiKeyCreationPending(
            confirmGenerateLocalApiKeyButton,
            localApiKeyModalElement,
            operationStatus,
            false,
        );
        if (!msg?.ok || !msg.key?.api_key) {
            showLocalApiOperationError(msg?.message || "Unable to generate the API key.");
            return;
        }
        window.config.local_api.keys = [...localApiKeys(), {
            id: msg.key.id,
            name: msg.key.name,
            token_sha256: "",
            created_at_unix: msg.key.created_at_unix,
        }];
        renderLocalApiKeys();
        document.getElementById("localApiKeysStatus").textContent = `API key ${msg.key.name} created.`;
        document.getElementById("localApiKeyNameStep").classList.add("d-none");
        document.getElementById("localApiKeyResultStep").classList.remove("d-none");
        confirmGenerateLocalApiKeyButton.classList.add("d-none");
        newLocalApiKeyInput.value = msg.key.api_key;
        newLocalApiKeyInput.focus();
    }, (msg) => {
        finishLocalApiManagement();
        setLocalApiKeyCreationPending(
            confirmGenerateLocalApiKeyButton,
            localApiKeyModalElement,
            operationStatus,
            false,
        );
        showLocalApiOperationError(msg?.message || "Unable to generate the API key.");
        refreshLocalApiKeys();
    }, {
        timeoutMs: 15000,
        timeoutMessage: "Timed out while creating the API key. It may have been created, but its raw value cannot be recovered. Refresh the key list and revoke it before trying again.",
    });
}

function removeLegacyLocalApiKey(button) {
    if (!window.confirm("Remove the legacy local API key? Clients using it will stop authenticating.")) return;
    if (!beginLocalApiManagement("Removing the legacy local API key…")) return;
    button.disabled = true;
    sendWsRequest("RemoveLegacyLocalApiKeyResult", { RemoveLegacyLocalApiKey: {} }, (msg) => {
        finishLocalApiManagement();
        button.disabled = false;
        if (!msg?.ok) {
            alert(msg?.message || "Unable to remove the legacy local API key.");
            return;
        }
        window.configSecretState.local_api = window.configSecretState.local_api || {};
        window.configSecretState.local_api.bearer_token = false;
        renderLocalApiKeys();
        document.getElementById("localApiKeysStatus").textContent = "Legacy local API key removed. It may take up to 30 seconds to stop working.";
        focusLocalApiKeySection();
    }, (msg) => {
        finishLocalApiManagement();
        button.disabled = false;
        alert(msg?.message || "Unable to remove the legacy local API key.");
        refreshLocalApiKeys();
    }, {
        timeoutMs: 15000,
        timeoutMessage: "Timed out while removing the legacy local API key. Refresh the key list before trying again.",
    });
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
    setSecretVisibility(newLocalApiKeyInput, toggleNewLocalApiKeyButton, false, "generated API key");

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

    document.getElementById("generateLocalApiKey").addEventListener("click", () => {
        resetLocalApiKeyModal();
        bootstrap.Modal.getOrCreateInstance(localApiKeyModalElement, {
            backdrop: "static",
            keyboard: false,
        }).show();
    });
    localApiKeyModalElement.addEventListener("shown.bs.modal", () => localApiKeyNameInput.focus());
    localApiKeyModalElement.addEventListener("hidden.bs.modal", resetLocalApiKeyModal);
    localApiKeyNameInput.addEventListener("input", () => {
        localApiKeyNameInput.classList.remove("is-invalid");
        document.getElementById("localApiKeyNameError").textContent = "";
    });
    localApiKeyNameInput.addEventListener("keydown", (event) => {
        if (event.key === "Enter") {
            event.preventDefault();
            createLocalApiKey();
        }
    });
    confirmGenerateLocalApiKeyButton.addEventListener("click", createLocalApiKey);
    toggleNewLocalApiKeyButton.addEventListener("click", () => {
        setSecretVisibility(
            newLocalApiKeyInput,
            toggleNewLocalApiKeyButton,
            newLocalApiKeyInput.type === "password",
            "generated API key",
        );
    });
    copyNewLocalApiKeyButton.addEventListener("click", async () => {
        const copyStatus = document.getElementById("newLocalApiKeyCopyStatus");
        try {
            await copyLocalApiToken(newLocalApiKeyInput);
            copyStatus.textContent = "Generated API key copied.";
            copyNewLocalApiKeyButton.innerHTML = '<i class="fa fa-check me-1"></i> Copied';
            setTimeout(() => {
                copyNewLocalApiKeyButton.innerHTML = '<i class="fa fa-copy me-1"></i> Copy';
            }, 1500);
        } catch (error) {
            copyStatus.textContent = "";
            alert(error instanceof Error ? error.message : "Unable to copy the API key.");
        }
    });
    const removeLegacyButton = document.getElementById("removeLegacyLocalApiKey");
    removeLegacyButton.addEventListener("click", () => removeLegacyLocalApiKey(removeLegacyButton));

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
    renderLocalApiKeys();
    fetchCapabilities();
});
