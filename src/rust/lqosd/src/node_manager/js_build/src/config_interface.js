import {loadConfig, renderConfigMenu} from "./config/config_helper";

const DRAFT_KEY = "lqos-network-mode-draft";
const PENDING_OPERATION_KEY = "lqos-network-mode-pending";
const NETPLAN_TRY_TIMEOUT_MS = 30_000;
const RECONNECT_POLL_INTERVAL_MS = 2_000;
let currentHelperStatus = null;
let currentInspection = null;
let currentPendingOperation = null;
let reconnectPollTimer = null;
let reconnectProbeInFlight = false;
let recoveryCountdownTimer = null;
let recoveryReachable = true;

function selectedInterfaceValuesFromConfig(config) {
    return {
        toInternet: config?.bridge?.to_internet ?? "",
        toNetwork: config?.bridge?.to_network ?? "",
        singleInterface: config?.single_interface?.interface ?? "",
    };
}

function escapeHtml(value) {
    return String(value ?? "")
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;")
        .replace(/"/g, "&quot;")
        .replace(/'/g, "&#039;");
}

function cloneConfig(config) {
    return JSON.parse(JSON.stringify(config || {}));
}

function configModeKind(config) {
    if (config?.bridge?.use_xdp_bridge) return "xdp";
    if (config?.bridge) return "bridge";
    if (config?.single_interface) return "single";
    return "unknown";
}

function getJson(url) {
    return fetch(url, {
        headers: { "Accept": "application/json" },
        credentials: "same-origin",
    }).then(async (response) => {
        if (!response.ok) {
            const body = await response.text();
            throw new Error(body || `Request failed: ${response.status}`);
        }
        return response.json();
    });
}

function postJson(url, body) {
    return fetch(url, {
        method: "POST",
        headers: {
            "Accept": "application/json",
            "Content-Type": "application/json",
        },
        credentials: "same-origin",
        body: JSON.stringify(body),
    }).then(async (response) => {
        const data = await response.json().catch(() => ({}));
        if (!response.ok) {
            throw new Error(data?.message || `Request failed: ${response.status}`);
        }
        return data;
    });
}

function validateConfig() {
    if (document.getElementById("bridgeMode").checked) {
        const toInternet = document.getElementById("toInternet").value.trim();
        const toNetwork = document.getElementById("toNetwork").value.trim();
        if (!toInternet || !toNetwork) {
            alert("Both interface names are required in bridge mode");
            return false;
        }
        if (toInternet === toNetwork) {
            alert("Bridge mode requires two different interfaces");
            return false;
        }
    } else if (document.getElementById("singleInterfaceMode").checked) {
        const interfaceName = document.getElementById("interface").value.trim();
        const internetVlan = parseInt(document.getElementById("internetVlan").value, 10);
        const networkVlan = parseInt(document.getElementById("networkVlan").value, 10);
        if (!interfaceName) {
            alert("Interface name is required in single interface mode");
            return false;
        }
        if (isNaN(internetVlan) || internetVlan < 1 || internetVlan > 4094) {
            alert("Internet VLAN must be between 1 and 4094");
            return false;
        }
        if (isNaN(networkVlan) || networkVlan < 1 || networkVlan > 4094) {
            alert("Network VLAN must be between 1 and 4094");
            return false;
        }
    } else {
        alert("Please select either bridge or single interface mode");
        return false;
    }
    return true;
}

function buildCandidateConfig() {
    const next = cloneConfig(window.config);
    next.bridge = null;
    next.single_interface = null;

    if (document.getElementById("bridgeMode").checked) {
        next.bridge = {
            use_xdp_bridge: document.getElementById("useXdpBridge").checked,
            to_internet: document.getElementById("toInternet").value.trim(),
            to_network: document.getElementById("toNetwork").value.trim(),
        };
    } else {
        next.single_interface = {
            interface: document.getElementById("interface").value.trim(),
            internet_vlan: parseInt(document.getElementById("internetVlan").value, 10),
            network_vlan: parseInt(document.getElementById("networkVlan").value, 10),
        };
    }

    return next;
}

function interfaceCandidates() {
    return Array.isArray(currentInspection?.interface_candidates) ? currentInspection.interface_candidates : [];
}

function optionLabel(candidate, selectedValue) {
    if (!candidate) return selectedValue;
    if (candidate.bridge_eligible || candidate.single_interface_eligible) {
        return candidate.name;
    }
    if (candidate.current_selection || candidate.name === selectedValue) {
        return `${candidate.name} (current selection; unavailable)`;
    }
    return `${candidate.name} (unavailable)`;
}

function buildSelectOptions(selectElement, modeKey, selectedValue, excludedValue = null) {
    if (!selectElement) return;
    const eligibilityField = modeKey === "single" ? "single_interface_eligible" : "bridge_eligible";
    const candidates = interfaceCandidates();
    const options = [`<option value="">Select an eligible interface</option>`];
    const seen = new Set();

    candidates.forEach((candidate) => {
        const selected = candidate.name === selectedValue;
        const eligible = Boolean(candidate[eligibilityField]);
        if (!eligible && !selected) return;
        if (excludedValue && candidate.name === excludedValue && !selected) return;
        options.push(
            `<option value="${escapeHtml(candidate.name)}">${escapeHtml(optionLabel(candidate, selectedValue))}</option>`
        );
        seen.add(candidate.name);
    });

    if (selectedValue && !seen.has(selectedValue)) {
        options.push(
            `<option value="${escapeHtml(selectedValue)}">${escapeHtml(`${selectedValue} (current selection; unavailable)`)}</option>`
        );
    }

    selectElement.innerHTML = options.join("");
    selectElement.value = selectedValue || "";
}

function renderInterfaceHelp(helpElementId, modeKey, selectedValues = {}) {
    const element = document.getElementById(helpElementId);
    if (!element) return;

    const eligibilityField = modeKey === "single" ? "single_interface_eligible" : "bridge_eligible";
    const selectedSet = new Set(
        Object.values(selectedValues)
            .map((value) => String(value || "").trim())
            .filter(Boolean)
    );
    const unavailable = interfaceCandidates().filter((candidate) => {
        if (candidate[eligibilityField]) return false;
        return !selectedSet.has(candidate.name);
    });

    if (unavailable.length === 0) {
        element.innerHTML = "";
        return;
    }

    element.innerHTML = `
        <details class="lqos-config-disclosure mt-2">
            <summary>Why some interfaces are unavailable</summary>
            <ul class="mb-0 mt-2">
            ${unavailable.map((candidate) => {
                const reason = Array.isArray(candidate.details) && candidate.details.length > 0
                    ? candidate.details.join(" ")
                    : "Unavailable for managed LibreQoS setup.";
                return `<li><code>${escapeHtml(candidate.name)}</code>: ${escapeHtml(reason)}</li>`;
            }).join("")}
            </ul>
        </details>`;
}

function setReviewTab(tabName) {
    document.querySelectorAll("[data-review-tab]").forEach((button) => {
        const active = button.dataset.reviewTab === tabName;
        button.classList.toggle("active", active);
        button.setAttribute("aria-selected", active ? "true" : "false");
    });
    document.querySelectorAll("[data-review-pane]").forEach((pane) => {
        pane.classList.toggle("d-none", pane.dataset.reviewPane !== tabName);
    });
}

function wireReviewTabs() {
    document.querySelectorAll("[data-review-tab]").forEach((button) => {
        if (button.dataset.wired === "true") return;
        button.dataset.wired = "true";
        button.addEventListener("click", () => {
            setReviewTab(button.dataset.reviewTab || "files");
        });
    });
}

function renderInterfaceSelectors(config = null) {
    const selected = selectedInterfaceValuesFromConfig(config || buildCandidateConfig());
    buildSelectOptions(document.getElementById("toInternet"), "bridge", selected.toInternet, selected.toNetwork);
    buildSelectOptions(document.getElementById("toNetwork"), "bridge", selected.toNetwork, selected.toInternet);
    buildSelectOptions(document.getElementById("interface"), "single", selected.singleInterface);
    renderInterfaceHelp("bridgeInterfaceHelp", "bridge", {
        toInternet: selected.toInternet,
        toNetwork: selected.toNetwork,
    });
    renderInterfaceHelp("singleInterfaceHelp", "single", {
        interface: selected.singleInterface,
    });
}

function populateFormFromConfig(config) {
    renderInterfaceSelectors(config);
    if (config?.bridge) {
        document.getElementById("bridgeMode").checked = true;
        document.getElementById("useXdpBridge").checked = config.bridge.use_xdp_bridge ?? true;
        document.getElementById("toInternet").value = config.bridge.to_internet ?? "";
        document.getElementById("toNetwork").value = config.bridge.to_network ?? "";
    } else if (config?.single_interface) {
        document.getElementById("singleInterfaceMode").checked = true;
        document.getElementById("interface").value = config.single_interface.interface ?? "";
        document.getElementById("internetVlan").value = config.single_interface.internet_vlan ?? 2;
        document.getElementById("networkVlan").value = config.single_interface.network_vlan ?? 3;
    }

    const event = new Event("change");
    document.querySelector('input[name="networkMode"]:checked')?.dispatchEvent(event);
}

function loadDraft() {
    try {
        const raw = localStorage.getItem(DRAFT_KEY);
        if (!raw) return null;
        return JSON.parse(raw);
    } catch (_) {
        return null;
    }
}

function saveDraft() {
    const draft = buildCandidateConfig();
    localStorage.setItem(DRAFT_KEY, JSON.stringify(draft));
    return draft;
}

function loadPendingOperation() {
    try {
        const raw = localStorage.getItem(PENDING_OPERATION_KEY);
        if (!raw) return null;
        return JSON.parse(raw);
    } catch (_) {
        return null;
    }
}

function pendingDeadlineMs(operation) {
    if (!operation) return null;
    if (Number.isFinite(operation.created_unix)) {
        return (operation.created_unix * 1000) + NETPLAN_TRY_TIMEOUT_MS;
    }
    if (Number.isFinite(operation.saved_at_ms)) {
        return operation.saved_at_ms + NETPLAN_TRY_TIMEOUT_MS;
    }
    return null;
}

function formatCountdown(remainingMs) {
    if (!Number.isFinite(remainingMs)) {
        return "Waiting for reconnect status...";
    }
    if (remainingMs <= 0) {
        return "Rollback deadline reached. Waiting for LibreQoS to report the final state.";
    }
    const totalSeconds = Math.ceil(remainingMs / 1000);
    const seconds = totalSeconds % 60;
    const minutes = Math.floor(totalSeconds / 60);
    return `Confirm or revert within ${minutes}:${String(seconds).padStart(2, "0")}.`;
}

function setPendingOperation(operation, extra = {}) {
    if (!operation?.operation_id) return;
    currentPendingOperation = {
        ...currentPendingOperation,
        ...operation,
        ...extra,
        saved_at_ms: Date.now(),
    };
    localStorage.setItem(PENDING_OPERATION_KEY, JSON.stringify(currentPendingOperation));
    renderRecoveryPanel();
    ensureRecoveryPolling();
}

function clearPendingOperation() {
    currentPendingOperation = null;
    localStorage.removeItem(PENDING_OPERATION_KEY);
    if (reconnectPollTimer) {
        clearInterval(reconnectPollTimer);
        reconnectPollTimer = null;
    }
    if (recoveryCountdownTimer) {
        clearInterval(recoveryCountdownTimer);
        recoveryCountdownTimer = null;
    }
    renderRecoveryPanel();
}

function setRecoveryReachable(isReachable) {
    recoveryReachable = Boolean(isReachable);
    renderRecoveryPanel();
}

function activePendingOperationId() {
    return currentHelperStatus?.pending_operation?.operation_id || currentPendingOperation?.operation_id || null;
}

function renderRecoveryPanel() {
    const panel = document.getElementById("networkRecoveryPanel");
    const summary = document.getElementById("networkRecoverySummary");
    const detail = document.getElementById("networkRecoveryDetail");
    const countdown = document.getElementById("networkRecoveryCountdown");
    const confirmButton = document.getElementById("recoveryConfirmButton");
    const revertButton = document.getElementById("recoveryRevertButton");
    if (!panel || !summary || !detail || !countdown || !confirmButton || !revertButton) return;

    const pending = currentPendingOperation;
    if (!pending?.operation_id) {
        panel.classList.add("d-none");
        confirmButton.disabled = true;
        revertButton.disabled = true;
        return;
    }

    panel.classList.remove("d-none");
    panel.classList.toggle("alert-warning", !recoveryReachable);
    panel.classList.toggle("alert-success", recoveryReachable);

    const deadline = pendingDeadlineMs(pending);
    const remainingMs = deadline == null ? null : deadline - Date.now();
    const summaryText = pending.summary || "LibreQoS staged a network change and is waiting for confirmation.";
    const reconnectText = recoveryReachable
        ? "LibreQoS is reachable again. Confirm the network change if access looks correct, or revert it if not."
        : "Connection is temporarily unavailable while LibreQoS tests the network change. This page will keep polling and show the confirm buttons again as soon as the box comes back.";

    summary.textContent = summaryText;
    detail.textContent = `${reconnectText} Pending operation: ${pending.operation_id}`;
    countdown.textContent = formatCountdown(remainingMs);

    confirmButton.disabled = !recoveryReachable;
    revertButton.disabled = !recoveryReachable;
}

function ensureRecoveryPolling() {
    if (!currentPendingOperation?.operation_id) return;
    if (!recoveryCountdownTimer) {
        recoveryCountdownTimer = setInterval(() => {
            renderRecoveryPanel();
        }, 1000);
    }
    if (!reconnectPollTimer) {
        reconnectPollTimer = setInterval(() => {
            pollRecoveryStatus();
        }, RECONNECT_POLL_INTERVAL_MS);
    }
}

function probeHealth() {
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 1500);
    return fetch("/health", {
        method: "GET",
        cache: "no-store",
        credentials: "same-origin",
        signal: controller.signal,
    })
        .then((response) => response.ok)
        .catch(() => false)
        .finally(() => {
            clearTimeout(timeout);
        });
}

function pollRecoveryStatus() {
    if (!currentPendingOperation?.operation_id || reconnectProbeInFlight) return;
    reconnectProbeInFlight = true;
    probeHealth()
        .then((reachable) => {
            setRecoveryReachable(reachable);
            if (!reachable) return null;
            return refreshHelperStatus();
        })
        .catch(() => {
            setRecoveryReachable(false);
        })
        .finally(() => {
            reconnectProbeInFlight = false;
        });
}

function badgeClassForShapingState(state) {
    switch (state) {
        case "Active":
            return "text-bg-success";
        case "Starting":
            return "text-bg-info";
        case "Inactive":
            return "text-bg-secondary";
        case "ErrorPreflight":
        case "ErrorKernelAttach":
        case "ErrorInterfaceMissing":
        case "ErrorConfig":
            return "text-bg-danger";
        case "Degraded":
            return "text-bg-warning";
        default:
            return "text-bg-secondary";
    }
}

function badgeClassForInspectionState(state) {
    switch (state) {
        case "ManagedByLibreQoS":
            return "text-bg-success";
        case "ExternalCompatible":
            return "text-bg-info";
        case "Ready":
            return "text-bg-primary";
        case "PendingTry":
            return "text-bg-warning";
        case "Missing":
        case "Conflict":
        case "ComplexUnsupported":
            return "text-bg-danger";
        default:
            return "text-bg-secondary";
    }
}

function renderAlertList(containerId, tone, messages) {
    const container = document.getElementById(containerId);
    if (!container) return;
    if (!Array.isArray(messages) || messages.length === 0) {
        container.innerHTML = "";
        return;
    }
    container.innerHTML = messages
        .map((message) => `<div class="alert alert-${tone} py-2 mb-2">${escapeHtml(message)}</div>`)
        .join("");
}

function renderShapingStatus(status) {
    const badge = document.getElementById("shapingStatusBadge");
    const summary = document.getElementById("shapingStatusSummary");
    const detail = document.getElementById("shapingStatusDetail");
    const banner = document.getElementById("shapingBanner");
    const retryButton = document.getElementById("retryShapingButton");
    if (!badge || !summary || !detail || !banner || !retryButton) return;

    const state = status?.state || "Unknown";
    badge.className = `badge ${badgeClassForShapingState(state)}`;
    badge.textContent = state;
    summary.textContent = status?.summary || "No shaping status available.";

    if (status?.detail) {
        detail.classList.remove("d-none");
        detail.textContent = status.detail;
    } else {
        detail.classList.add("d-none");
        detail.textContent = "";
    }

    banner.innerHTML = status?.degraded
        ? `<div class="alert alert-danger" role="alert"><strong>Shaping is degraded.</strong> ${escapeHtml(status.summary || "")}</div>`
        : "";

    const canRetry = Boolean(status?.can_retry);
    retryButton.disabled = !canRetry;
    retryButton.classList.toggle("d-none", !canRetry);
}

function renderDetectedFiles(files) {
    const container = document.getElementById("netplanFiles");
    if (!container) return;
    if (!Array.isArray(files) || files.length === 0) {
        container.innerHTML = `<div class="text-secondary">No relevant netplan files were detected for the current selection.</div>`;
        return;
    }

    container.innerHTML = files.map((file) => {
        const details = Array.isArray(file.details) && file.details.length > 0
            ? `<ul class="mb-0 mt-2">${file.details.map((detail) => `<li>${escapeHtml(detail)}</li>`).join("")}</ul>`
            : `<div class="text-secondary mt-2">No extra details.</div>`;
        const interfaces = Array.isArray(file.relevant_interfaces) && file.relevant_interfaces.length > 0
            ? file.relevant_interfaces.map((iface) => `<code>${escapeHtml(iface)}</code>`).join(", ")
            : "None";
        const badgeClass = file.compatible ? "text-bg-success" : badgeClassForInspectionState(file.classification);
        return `
            <div class="border rounded p-3 mb-3">
                <div class="d-flex justify-content-between flex-wrap gap-2 align-items-center">
                    <div>
                        <div class="fw-semibold">${escapeHtml(file.path)}</div>
                        <div class="text-secondary">Relevant interfaces: ${interfaces}</div>
                    </div>
                    <span class="badge ${badgeClass}">${escapeHtml(file.classification)}</span>
                </div>
                ${details}
            </div>`;
    }).join("");
}

function renderInspection(inspection) {
    currentInspection = inspection || null;
    renderInterfaceSelectors();
    const badge = document.getElementById("netplanStateBadge");
    const summary = document.getElementById("netplanSummary");
    const preview = document.getElementById("managedPreview");
    const previewMeta = document.getElementById("managedPreviewMeta");
    const previewNote = document.getElementById("managedPreviewNote");
    const diffPreview = document.getElementById("diffPreview");
    const diffPreviewMeta = document.getElementById("diffPreviewMeta");
    const applyButton = document.getElementById("applyButton");
    const adoptButton = document.getElementById("adoptButton");
    const takeoverButton = document.getElementById("takeoverButton");
    if (!badge || !summary || !preview || !previewMeta || !previewNote || !diffPreview || !diffPreviewMeta || !applyButton || !adoptButton || !takeoverButton) return;

    const state = inspection?.inspector_state || "Unknown";
    badge.className = `badge ${badgeClassForInspectionState(state)}`;
    badge.textContent = state;
    summary.textContent = inspection?.summary || "No netplan inspection data available.";

    const warnings = Array.isArray(inspection?.warnings) ? [...inspection.warnings] : [];
    const dangerousChanges = Array.isArray(inspection?.dangerous_changes) ? inspection.dangerous_changes : [];
    if (dangerousChanges.length > 0) {
        warnings.unshift(...dangerousChanges);
    }
    renderAlertList("netplanWarnings", dangerousChanges.length > 0 ? "warning" : "warning", warnings);
    renderAlertList("netplanConflicts", "danger", inspection?.conflicts || []);
    renderDetectedFiles(inspection?.detected_files || []);

    previewMeta.innerHTML = `
        <div><strong>Mode:</strong> ${escapeHtml(inspection?.mode_label || "Unknown")}</div>
        <div><strong>Managed file:</strong> <code>${escapeHtml(inspection?.managed_file_path || "/etc/netplan/libreqos.yaml")}</code></div>`;
    previewNote.innerHTML = [
        inspection?.strong_confirmation_text
            ? `<div class="alert alert-danger py-2">${escapeHtml(inspection.strong_confirmation_text)}</div>`
            : "",
        inspection?.preview_note
            ? `<div class="alert alert-info py-2">${escapeHtml(inspection.preview_note)}</div>`
            : "",
    ].join("");
    preview.textContent = inspection?.managed_preview_yaml || "";
    diffPreviewMeta.innerHTML = inspection?.diff_preview_label
        ? `<div><strong>Diff:</strong> ${escapeHtml(inspection.diff_preview_label)}</div>`
        : `<div class="text-secondary">No diff preview available.</div>`;
    diffPreview.textContent = inspection?.diff_preview || "No diff preview available.";

    [
        "bridgeMode",
        "singleInterfaceMode",
        "useXdpBridge",
        "toInternet",
        "toNetwork",
        "interface",
        "internetVlan",
        "networkVlan",
        "saveButton",
    ].forEach((id) => {
        const element = document.getElementById(id);
        if (element) {
            element.disabled = Boolean(inspection?.editing_locked);
        }
    });
    applyButton.disabled = !inspection?.can_apply;
    adoptButton.disabled = !inspection?.can_adopt;
    takeoverButton.disabled = !inspection?.can_take_over;
}

function renderHelperStatus(status) {
    currentHelperStatus = status || null;
    const pendingEl = document.getElementById("pendingChange");
    const lastBackupEl = document.getElementById("lastBackup");
    const confirmButton = document.getElementById("confirmPendingButton");
    const revertButton = document.getElementById("revertPendingButton");
    const restoreButton = document.getElementById("restoreLatestBackupButton");
    if (!pendingEl || !lastBackupEl || !confirmButton || !revertButton || !restoreButton) return;

    const pending = status?.pending_operation || null;
    if (pending) {
        setPendingOperation(pending);
        pendingEl.innerHTML = `
            <div><strong>${escapeHtml(pending.state)}</strong></div>
            <div class="text-secondary mt-1">${escapeHtml(pending.summary)}</div>
            <div class="small mt-2"><code>${escapeHtml(pending.operation_id)}</code></div>`;
        confirmButton.disabled = false;
        revertButton.disabled = false;
    } else {
        clearPendingOperation();
        pendingEl.innerHTML = `<div class="text-secondary">No pending network change.</div>`;
        confirmButton.disabled = true;
        revertButton.disabled = true;
    }

    const recentBackups = Array.isArray(status?.recent_backups) ? status.recent_backups : [];
    if (recentBackups.length > 0) {
        const recent = recentBackups.map((backup) => {
            const warnings = Array.isArray(backup.warnings_present) && backup.warnings_present.length > 0
                ? `<div class="small text-warning mt-1">${backup.warnings_present.map(escapeHtml).join(" | ")}</div>`
                : "";
            return `
                <div class="border rounded p-2 mt-2 lqos-backup-entry">
                    <div><code class="lqos-break-anywhere">${escapeHtml(backup.backup_id)}</code></div>
                    <div class="small text-secondary mt-1">${escapeHtml(backup.old_mode)} -> ${escapeHtml(backup.new_mode)}</div>
                    ${warnings}
                </div>`;
        }).join("");
        lastBackupEl.innerHTML = recent;
        restoreButton.disabled = Boolean(pending);
    } else {
        lastBackupEl.innerHTML = `<div class="text-secondary">No helper backup recorded yet.</div>`;
        restoreButton.disabled = true;
    }
}

function refreshHelperStatus() {
    return getJson("/local-api/network-mode/status")
        .then((data) => {
            renderHelperStatus(data?.helper_status || {});
            setRecoveryReachable(true);
            return data?.helper_status || {};
        })
        .catch(() => {
            if (!currentPendingOperation?.operation_id) {
                renderHelperStatus(null);
            }
            setRecoveryReachable(false);
            throw new Error("Unable to refresh helper status");
        });
}

function inspectCandidate() {
    if (!validateConfig()) return Promise.resolve();
    const candidate = buildCandidateConfig();
    const button = document.getElementById("inspectButton");
    if (button) {
        button.disabled = true;
        button.textContent = "Inspecting...";
    }

    return postJson("/local-api/network-mode/inspect", { config: candidate })
        .then((inspection) => {
            renderInspection(inspection);
            return refreshHelperStatus();
        })
        .catch((err) => {
            alert(err.message || "Unable to inspect current netplan");
        })
        .finally(() => {
            if (button) {
                button.disabled = false;
                button.textContent = "Inspect Current Netplan";
            }
        });
}

function confirmDangerousChange(actionLabel, candidate) {
    const warnings = Array.isArray(currentInspection?.dangerous_changes) ? currentInspection.dangerous_changes.slice() : [];
    if (configModeKind(window.config) !== configModeKind(candidate)) {
        warnings.push("Switching between Linux bridge and single-interface modes requires strong confirmation.");
    }
    if (warnings.length === 0 && actionLabel === "Apply") {
        return true;
    }

    const intro = currentInspection?.strong_confirmation_text
        || "This change may interrupt access to this system. LibreQoS will automatically roll back if you do not confirm within 30 seconds. If your browser disconnects, stay on this page and it will keep trying to reconnect.";
    return window.confirm(`${actionLabel} requires confirmation:\n\n- ${warnings.join("\n- ")}\n\n${intro}`);
}

function applyNetworkChanges(mode = "Apply") {
    if (!validateConfig()) return;
    const candidate = buildCandidateConfig();
    const actionLabel = mode === "Adopt" ? "Adopt into libreqos.yaml" : mode === "TakeOver" ? "Take Over libreqos.yaml" : "Apply Network Changes";
    if (!confirmDangerousChange(actionLabel, candidate)) {
        return;
    }
    const button = document.getElementById("applyButton");
    if (button) {
        button.disabled = true;
        button.textContent = "Applying...";
    }

    postJson("/local-api/network-mode/apply", {
        config: candidate,
        mode,
        confirm_dangerous_changes: true,
    })
        .then((response) => {
            if (response?.operation) {
                setPendingOperation(response.operation, {
                    action_label: actionLabel,
                });
            }
            localStorage.removeItem(DRAFT_KEY);
            window.config = candidate;
            renderRecoveryPanel();
            window.scrollTo({ top: 0, behavior: "smooth" });
            return Promise.allSettled([inspectCandidate(), refreshHelperStatus()]);
        })
        .catch((err) => {
            alert(err.message || "Unable to apply network changes");
        })
        .finally(() => {
            if (button) {
                button.disabled = false;
                button.textContent = "Apply Network Changes";
            }
        });
}

function actOnPending(endpoint) {
    const operationId = activePendingOperationId();
    if (!operationId) {
        alert("No pending network operation is available");
        return;
    }

    postJson(endpoint, { operation_id: operationId })
        .then((response) => {
            clearPendingOperation();
            alert(response?.message || "Operation completed");
            return Promise.allSettled([refreshHelperStatus(), inspectCandidate()]);
        })
        .catch((err) => {
            alert(err.message || "Operation failed");
        });
}

function rollbackLatestBackup() {
    const pending = currentHelperStatus?.pending_operation;
    if (pending?.operation_id) {
        alert("Confirm or revert the pending network change before restoring an older backup.");
        return;
    }

    const backupId = currentHelperStatus?.recent_backups?.[0]?.backup_id;
    if (!backupId) {
        alert("No rollback bundle is available.");
        return;
    }

    const confirmed = window.confirm(
        "Restore the previous managed LibreQoS network configuration?\n\n"
        + "This will overwrite the current managed netplan state with the most recent backup and may interrupt access to this system."
    );
    if (!confirmed) {
        return;
    }

    postJson("/local-api/network-mode/rollback", { backup_id: backupId })
        .then((response) => {
            alert(response?.message || "Rollback completed");
            return Promise.all([refreshHelperStatus(), inspectCandidate()]);
        })
        .catch((err) => {
            alert(err.message || "Unable to restore the previous managed config");
        });
}

function retryShaping() {
    const button = document.getElementById("retryShapingButton");
    if (button) {
        button.disabled = true;
        button.textContent = "Retrying...";
    }

    postJson("/local-api/network-mode/retry-shaping", {})
        .then((response) => {
            alert(response?.message || "Shaping retry requested");
            return Promise.all([refreshHelperStatus(), inspectCandidate()]);
        })
        .catch((err) => {
            alert(err.message || "Unable to retry shaping");
        })
        .finally(() => {
            if (button) {
                button.disabled = false;
                button.textContent = "Retry Shaping";
            }
        });
}

function wireActions() {
    wireReviewTabs();
    [
        "bridgeMode",
        "singleInterfaceMode",
        "toInternet",
        "toNetwork",
        "interface",
    ].forEach((id) => {
        document.getElementById(id).addEventListener("change", () => {
            renderInterfaceSelectors();
        });
    });
    document.getElementById("saveButton").addEventListener("click", () => {
        if (!validateConfig()) return;
        saveDraft();
        inspectCandidate();
        alert("Network mode draft saved in this browser. Use Apply Network Changes to commit both lqos.conf and netplan together.");
    });
    document.getElementById("inspectButton").addEventListener("click", inspectCandidate);
    document.getElementById("applyButton").addEventListener("click", () => applyNetworkChanges("Apply"));
    document.getElementById("adoptButton").addEventListener("click", () => applyNetworkChanges("Adopt"));
    document.getElementById("takeoverButton").addEventListener("click", () => applyNetworkChanges("TakeOver"));
    document.getElementById("retryShapingButton").addEventListener("click", retryShaping);
    document.getElementById("confirmPendingButton").addEventListener("click", () => {
        actOnPending("/local-api/network-mode/confirm");
    });
    document.getElementById("revertPendingButton").addEventListener("click", () => {
        actOnPending("/local-api/network-mode/revert");
    });
    document.getElementById("recoveryConfirmButton").addEventListener("click", () => {
        actOnPending("/local-api/network-mode/confirm");
    });
    document.getElementById("recoveryRevertButton").addEventListener("click", () => {
        actOnPending("/local-api/network-mode/revert");
    });
    document.getElementById("restoreLatestBackupButton").addEventListener("click", rollbackLatestBackup);
}

renderConfigMenu("interface");

loadConfig((msg) => {
    const payload = msg?.data || {};
    window.config = payload.config || window.config || {};
    currentPendingOperation = loadPendingOperation();

    renderShapingStatus(payload.shaping_status || {});
    renderInspection(payload.network_mode_inspection || {});
    const draft = loadDraft();
    populateFormFromConfig(draft || window.config);
    renderRecoveryPanel();
    wireActions();
    refreshHelperStatus().catch(() => {});
    ensureRecoveryPolling();
}, () => {
    alert("Unable to load LibreQoS configuration");
});
