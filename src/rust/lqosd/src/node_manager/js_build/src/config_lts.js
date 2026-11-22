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
import {t, tHtml} from "./i18n/index.js";
import {formatMappedCircuitLimit, renderAvailabilityAlert} from "./config/license_availability.mjs";

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

// Three paragraphs wrap a literal header or token name in <code>, so a `data-i18n`
// attribute cannot carry them (that applier writes text): the elements ship empty
// and the messages supply both the prose and the literal.
function renderTranslatedProse() {
    const apiKeysSubtitle = document.getElementById("localApiKeysSubtitle");
    if (apiKeysSubtitle) apiKeysSubtitle.innerHTML = tHtml("config.lts.api_keys.subtitle", {header: "x-bearer"});
    const legacyHelp = document.getElementById("legacyLocalApiKeyHelp");
    if (legacyHelp) legacyHelp.innerHTML = tHtml("config.lts.api_keys.legacy_help", {token: "bearer_token"});
    const newKeyHelp = document.getElementById("newLocalApiKeyHelp");
    if (newKeyHelp) newKeyHelp.innerHTML = tHtml("config.lts.modal.copy_help", {header: "x-bearer"});
}

function beginLocalApiManagement(message) {
    if (localApiManagementInFlight) {
        document.getElementById("localApiKeysStatus").textContent = t("config.lts.api_keys.busy");
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
            document.getElementById("localApiKeysStatus").textContent = t("config.lts.api_keys.refresh_failed");
        },
    );
}

function validateConfig() {
    const collationPeriod = parseInt(document.getElementById("collationPeriod").value, 10);
    if (Number.isNaN(collationPeriod) || collationPeriod < 1) {
        alert(t("config.lts.validation.period_invalid", {field: t("config.lts.collection.period")}));
        return false;
    }

    const ltsUrl = document.getElementById("ltsUrl").value.trim();
    if (ltsUrl) {
        try {
            new URL(ltsUrl);
        } catch {
            alert(t("config.lts.validation.server_url_invalid"));
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
        lts_url: document.getElementById("ltsUrl").value.trim() || null,
    };
    window.config.local_api = window.config.local_api || {};
}

function setSecretVisibility(input, button, revealed, revealLabel, hideLabel) {
    input.type = revealed ? "text" : "password";
    button.innerHTML = revealed
        ? '<i class="fa fa-eye-slash"></i>'
        : '<i class="fa fa-eye"></i>';
    button.setAttribute("aria-label", revealed ? hideLabel : revealLabel);
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
        name.textContent = key.name || t("config.lts.api_keys.unnamed");
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
        revoke.innerHTML = `<i class="fa fa-trash me-1"></i> ${t("config.lts.api_keys.revoke")}`;
        revoke.setAttribute("aria-label", t("config.lts.api_keys.revoke_aria", {name: key.name || key.id}));
        revoke.addEventListener("click", () => revokeLocalApiKey(key, revoke));
        action.append(revoke);
        row.append(name, created, id, action);
        body.append(row);
    }
    document.getElementById("localApiKeysEmpty").classList.toggle("d-none", keys.length > 0);
    document.getElementById("localApiKeyCount").textContent = t("config.lts.api_keys.count", {
        count: keys.length,
        max: MAX_LOCAL_API_KEYS,
    });
    document.getElementById("generateLocalApiKey").disabled = keys.length >= MAX_LOCAL_API_KEYS;
    const legacy = !!window.configSecretState?.local_api?.bearer_token;
    document.getElementById("legacyLocalApiKey").classList.toggle("d-none", !legacy);
}

function revokeLocalApiKey(key, button) {
    const label = key.name || key.id;
    if (!window.confirm(t("config.lts.api_keys.revoke_confirm", {name: key.name || t("config.lts.api_keys.unnamed")}))) return;
    if (!beginLocalApiManagement(t("config.lts.api_keys.revoking", {name: label}))) return;
    button.disabled = true;
    sendWsRequest("RevokeLocalApiKeyResult", { RevokeLocalApiKey: { id: key.id } }, (msg) => {
        finishLocalApiManagement();
        if (!msg?.ok) {
            button.disabled = false;
            alert(msg?.message || t("config.lts.api_keys.revoke_failed"));
            return;
        }
        window.config.local_api.keys = localApiKeys().filter((entry) => entry.id !== key.id);
        renderLocalApiKeys();
        document.getElementById("localApiKeysStatus").textContent = t("config.lts.api_keys.revoked", {name: label});
        focusLocalApiKeySection();
    }, (msg) => {
        finishLocalApiManagement();
        button.disabled = false;
        alert(msg?.message || t("config.lts.api_keys.revoke_failed"));
        refreshLocalApiKeys();
    }, {
        timeoutMs: 15000,
        timeoutMessage: t("config.lts.api_keys.revoke_timeout"),
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
    copyNewLocalApiKeyButton.innerHTML = `<i class="fa fa-copy me-1"></i> ${t("config.lts.api_key.copy")}`;
    document.getElementById("localApiKeyNameStep").classList.remove("d-none");
    document.getElementById("localApiKeyResultStep").classList.add("d-none");
    confirmGenerateLocalApiKeyButton.classList.remove("d-none");
    setLocalApiKeyCreationPending(
        confirmGenerateLocalApiKeyButton,
        localApiKeyModalElement,
        operationStatus,
        false,
    );
    setSecretVisibility(
        newLocalApiKeyInput,
        toggleNewLocalApiKeyButton,
        false,
        t("config.lts.api_key.reveal"),
        t("config.lts.api_key.hide"),
    );
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
    if (!beginLocalApiManagement(t("config.lts.api_keys.creating", {name: validation.name}))) return;
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
            showLocalApiOperationError(msg?.message || t("config.lts.api_keys.generate_failed"));
            return;
        }
        window.config.local_api.keys = [...localApiKeys(), {
            id: msg.key.id,
            name: msg.key.name,
            token_sha256: "",
            created_at_unix: msg.key.created_at_unix,
        }];
        renderLocalApiKeys();
        document.getElementById("localApiKeysStatus").textContent = t("config.lts.api_keys.created", {name: msg.key.name});
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
        showLocalApiOperationError(msg?.message || t("config.lts.api_keys.generate_failed"));
        refreshLocalApiKeys();
    }, {
        timeoutMs: 15000,
        timeoutMessage: t("config.lts.api_keys.create_timeout"),
    });
}

function removeLegacyLocalApiKey(button) {
    if (!window.confirm(t("config.lts.api_keys.legacy_remove_confirm"))) return;
    if (!beginLocalApiManagement(t("config.lts.api_keys.legacy_removing"))) return;
    button.disabled = true;
    sendWsRequest("RemoveLegacyLocalApiKeyResult", { RemoveLegacyLocalApiKey: {} }, (msg) => {
        finishLocalApiManagement();
        button.disabled = false;
        if (!msg?.ok) {
            alert(msg?.message || t("config.lts.api_keys.legacy_remove_failed"));
            return;
        }
        window.configSecretState.local_api = window.configSecretState.local_api || {};
        window.configSecretState.local_api.bearer_token = false;
        renderLocalApiKeys();
        document.getElementById("localApiKeysStatus").textContent = t("config.lts.api_keys.legacy_removed");
        focusLocalApiKeySection();
    }, (msg) => {
        finishLocalApiManagement();
        button.disabled = false;
        alert(msg?.message || t("config.lts.api_keys.legacy_remove_failed"));
        refreshLocalApiKeys();
    }, {
        timeoutMs: 15000,
        timeoutMessage: t("config.lts.api_keys.legacy_remove_timeout"),
    });
}

function renderCapabilityBadges(capabilities) {
    const container = document.getElementById("licenseCapabilityBadges");
    if (!container) {
        return;
    }

    const apiDocsStatus = capabilities.can_use_api_link
        ? (window.apiServiceAvailable
            ? { label: t("config.lts.badge.api_docs_available"), tone: "success" }
            : { label: t("config.lts.badge.api_docs_unavailable"), tone: "warning" })
        : { label: t("config.lts.badge.api_docs_license"), tone: "secondary" };
    const badgeSpec = [
        {
            tone: capabilities.can_view_insight_ui ? "success" : "secondary",
            label: t("config.lts.badge.insight_ui"),
        },
        apiDocsStatus,
        {
            tone: capabilities.can_use_support_tickets ? "success" : "secondary",
            label: t("config.lts.badge.support"),
        },
        {
            tone: capabilities.can_use_chatbot ? "success" : "secondary",
            // translatable: false — "Libby" is the assistant's product name, kept literal (GLOSSARY §6)
            label: "Libby",
        },
        {
            tone: capabilities.can_receive_remote_commands ? "success" : "secondary",
            label: t("config.lts.badge.remote_control"),
        },
        {
            tone: capabilities.can_submit_long_term_stats ? "success" : "secondary",
            label: t("config.lts.badge.stats_submit"),
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
    document.getElementById("licenseStateLabel").textContent = capabilities.license_state_label || t("config.lts.status.unknown");
    document.getElementById("licenseAuthorityBadge").textContent = capabilities.authority_label || t("config.lts.status.unknown");
    document.getElementById("controlServiceStatus").textContent = capabilities.control_service_reachable
        ? t("config.lts.status.reachable")
        : t("config.lts.status.unavailable");
    document.getElementById("mappedCircuitLimit").textContent = formatMappedCircuitLimit(
        capabilities.mapped_circuit_limit,
    );

    renderCapabilityBadges(capabilities);

    const alert = document.getElementById("licenseAvailabilityAlert");
    if (!alert) {
        return;
    }

    renderAvailabilityAlert(alert, capabilities);
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
            alert.textContent = msg?.message || t("config.lts.status.load_failed");
            alert.classList.remove("d-none");
        },
    );
}

function wireActions() {
    setSecretVisibility(
        licenseKeyInput,
        toggleLicenseKeyButton,
        false,
        t("config.lts.license_key.reveal"),
        t("config.lts.license_key.hide"),
    );
    setSecretVisibility(
        newLocalApiKeyInput,
        toggleNewLocalApiKeyButton,
        false,
        t("config.lts.api_key.reveal"),
        t("config.lts.api_key.hide"),
    );

    toggleLicenseKeyButton.addEventListener("click", () => {
        setSecretVisibility(
            licenseKeyInput,
            toggleLicenseKeyButton,
            licenseKeyInput.type === "password",
            t("config.lts.license_key.reveal"),
            t("config.lts.license_key.hide"),
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
    // translatable: false — "Enter" is the KeyboardEvent.key token compared here, never rendered
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
            t("config.lts.api_key.reveal"),
            t("config.lts.api_key.hide"),
        );
    });
    copyNewLocalApiKeyButton.addEventListener("click", async () => {
        const copyStatus = document.getElementById("newLocalApiKeyCopyStatus");
        try {
            await copyLocalApiToken(newLocalApiKeyInput);
            copyStatus.textContent = t("config.lts.api_key.copied_status");
            copyNewLocalApiKeyButton.innerHTML = `<i class="fa fa-check me-1"></i> ${t("config.lts.api_key.copied")}`;
            setTimeout(() => {
                copyNewLocalApiKeyButton.innerHTML = `<i class="fa fa-copy me-1"></i> ${t("config.lts.api_key.copy")}`;
            }, 1500);
        } catch (error) {
            copyStatus.textContent = "";
            alert(error instanceof Error ? error.message : t("config.lts.api_key.copy_failed"));
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
                alert(msg?.message || t("config.lts.status.retry_failed"));
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
                alert(t("config.lts.saved"));
            },
            (msg) => {
                saveButton.disabled = false;
                alert(msg?.message || t("config.lts.save_failed"));
            },
        );
    });
}

renderConfigMenu("lts");
renderTranslatedProse();

loadConfig(() => {
    if (!window.config || !window.config.long_term_stats) {
        // translatable: false — "Long-term stats configuration not found in window.config" is a console diagnostic, never rendered
        console.error("Long-term stats configuration not found in window.config");
        return;
    }

    const lts = window.config.long_term_stats;
    document.getElementById("gatherStats").checked = lts.gather_stats ?? true;
    document.getElementById("collationPeriod").value = lts.collation_period_seconds ?? 60;
    document.getElementById("ltsUrl").value = lts.lts_url ?? "";
    licenseKeyInput.value = lts.license_key ?? "";

    wireActions();
    renderLocalApiKeys();
    fetchCapabilities();
});
