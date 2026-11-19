import {
    saveConfig,
    loadConfig,
    renderConfigMenu,
    TOPOLOGY_SOURCES,
    buildTopologyConfigUpdate,
    topologySource,
    topologySourceDefinition,
    topologySourceLabel,
} from "./config/config_helper";
import {renderMarkupText} from "./config/markup_text.mjs";
import {
    capacitySourceBadge,
    driftClass,
    driftText,
    intervalValid,
    isUnavailable,
    statusBadge,
    summaryCounts,
    unavailableKey,
    validateFactorRows,
} from "./config/ap_polling.mjs";
import {formatDateTime, formatNumber} from "./i18n/format.js";
import {t, tHtml} from "./i18n/index.js";

const COMPILE_MODES = [    {value: "flat", label: t("config.integration.mode.flat")},
    {value: "ap_only", label: t("config.integration.mode.ap_only")},
    {value: "ap_site", label: t("config.integration.mode.ap_site")},
    {value: "full", label: t("config.integration.mode.full")},
];

function populateTopologySources() {
    const select = document.getElementById("topologySource");
    if (!select) return;
    select.innerHTML = "";
    const placeholder = document.createElement("option");
    placeholder.value = "";
    placeholder.textContent = t("config.integration.select_source");
    placeholder.disabled = true;
    select.appendChild(placeholder);
    for (const source of TOPOLOGY_SOURCES) {
        const option = document.createElement("option");
        option.value = source.value;
        option.textContent = source.label;
        select.appendChild(option);
    }
}

function selectedSourceDefinition() {
    const select = document.getElementById("topologySource");
    return topologySourceDefinition(select?.value || "");
}

function renderCompileModes() {
    const select = document.getElementById("topologyCompileMode");
    if (!select) return;
    const definition = selectedSourceDefinition();
    const modes = definition?.modes || COMPILE_MODES.map((entry) => entry.value);
    const current = window.config?.topology?.compile_mode || "";
    const preferred = modes.includes(current) ? current : "";
    select.innerHTML = "";
    if (!preferred) {
        const placeholder = document.createElement("option");
        placeholder.value = "";
        placeholder.textContent = t("config.integration.select_mode");
        placeholder.disabled = true;
        select.appendChild(placeholder);
    }
    for (const value of modes) {
        const meta = COMPILE_MODES.find((entry) => entry.value === value);
        const option = document.createElement("option");
        option.value = value;
        option.textContent = meta?.label || value;
        option.selected = value === preferred;
        select.appendChild(option);
    }
    select.value = preferred;
}

function renderTopologyStatus() {
    const status = document.getElementById("topologySourceStatus");
    const conflict = document.getElementById("topologySourceConflict");
    const help = document.getElementById("topologySourceHelp");
    const definition = selectedSourceDefinition();
    if (help) {
        help.textContent = definition?.description || "";
    }
    const view = window.configView || {};
    const active = view.topology_source || null;
    const label = view.topology_source_label || (active ? topologySourceLabel(active) : null);
    const parts = [];
    parts.push(label
        ? t("config.integration.status.active_source", {source: label})
        : t("config.integration.status.no_source"));
    if (view.topology_source_origin) {
        parts.push(t("config.integration.status.origin", {origin: view.topology_source_origin}));
    }
    if (view.topology_source_last_update) {
        parts.push(t("config.integration.status.last_update", {time: view.topology_source_last_update}));
    }
    if (view.topology_source_last_result) {
        parts.push(t("config.integration.status.last_result", {result: view.topology_source_last_result}));
    }
    if (view.topology_source_reject_reason) {
        parts.push(t("config.integration.status.reject", {reason: view.topology_source_reject_reason}));
    }
    const ingest = view.libreqos_network_status;
    if (ingest) {
        // `state` is the value the server reports; the two fallbacks below are
        // this page's own wording for a payload that carries only the gate flag.
        const state = ingest.state
            || (ingest.gate_active
                ? t("config.integration.status.gated")
                : t("config.integration.status.inert"));
        parts.push(t("config.integration.status.ingest", {state: state}));
        if (typeof ingest.gate_active === "boolean") {
            parts.push(ingest.gate_active
                ? t("config.integration.status.gate_active")
                : t("config.integration.status.gate_inactive"));
        }
        if (ingest.last_accepted_unix) {
            parts.push(t("config.integration.status.last_accepted", {time: ingest.last_accepted_unix}));
        }
        if (ingest.last_reject_reason) {
            parts.push(ingest.last_reject_unix
                ? t("config.integration.status.last_rejected_at", {
                    reason: ingest.last_reject_reason,
                    time: ingest.last_reject_unix,
                })
                : t("config.integration.status.last_rejected", {reason: ingest.last_reject_reason}));
        }
    }
    if (status) {
        status.textContent = parts.join(" · ");
    }
    if (conflict) {
        const message = view.topology_source_conflict?.message || "";
        conflict.textContent = message;
        conflict.classList.toggle("d-none", !message);
    }
}

// ---- AP Polling (step 6b) -------------------------------------------------
// The table is fed by GET /local-api/ap-polling (step 6a). Every label is
// cataloged; the endpoint's unavailable states are rendered explicitly, never
// as an empty all-zero table.

const AP_POLLING_REFRESH_MS = 60000;
let apPollingRefreshTimer = null;

// The catalog scanner only sees literal keys, so these switches are also the
// single place where the panel's dynamic keys are declared. The pure helper
// modules keep the token-to-key mapping under test.
function apCapacitySourceTitle(token) {
    switch (token) {
        case "phy": return t("config.integration.ap_polling.source.phy");
        case "per_sm_rate": return t("config.integration.ap_polling.source.per_sm_rate");
        case "crm": return t("config.integration.ap_polling.source.crm");
        default: return t("config.integration.ap_polling.source.unknown");
    }
}

function apStateLabel(token) {
    switch (token) {
        case "ok": return t("config.integration.ap_polling.state.ok");
        case "failing": return t("config.integration.ap_polling.state.failing");
        default: return t("config.integration.ap_polling.state.observing");
    }
}

function apUnavailableMessage(reasons) {
    switch (unavailableKey(reasons)) {
        case "config.integration.ap_polling.unavailable.not_started":
            return t("config.integration.ap_polling.unavailable.not_started");
        case "config.integration.ap_polling.unavailable.disabled":
            return t("config.integration.ap_polling.unavailable.disabled");
        case "config.integration.ap_polling.unavailable.pending":
            return t("config.integration.ap_polling.unavailable.pending");
        case "config.integration.ap_polling.unavailable.stopped":
            return t("config.integration.ap_polling.unavailable.stopped");
        case "config.integration.ap_polling.unavailable.request_failed":
            return t("config.integration.ap_polling.unavailable.request_failed");
        default:
            return t("config.integration.ap_polling.unavailable.unknown", {
                reason: (reasons || [])[0] || "",
            });
    }
}

function apFactorErrorMessage(result) {
    switch (result.errorKey) {
        case "config.integration.ap_polling.validation.factor_model":
            return t("config.integration.ap_polling.validation.factor_model");
        case "config.integration.ap_polling.validation.factor_range":
            return t("config.integration.ap_polling.validation.factor_range");
        case "config.integration.ap_polling.validation.factor_duplicate":
            return t("config.integration.ap_polling.validation.factor_duplicate");
        default:
            return "";
    }
}

function buildFactorRow(model = "", factor = "") {
    const row = document.createElement("tr");
    row.innerHTML = `
        <td><input type="text" class="form-control form-control-sm ap-polling-factor-model"></td>
        <td><input type="number" min="0.01" max="1" step="0.01" class="form-control form-control-sm ap-polling-factor-value"></td>
        <td class="text-end"><button type="button" class="btn btn-outline-danger btn-sm ap-polling-factor-remove"><i class="fa fa-trash"></i></button></td>
    `;
    const modelInput = row.querySelector(".ap-polling-factor-model");
    const factorInput = row.querySelector(".ap-polling-factor-value");
    const remove = row.querySelector(".ap-polling-factor-remove");
    modelInput.value = model;
    factorInput.value = factor;
    modelInput.setAttribute("aria-label", t("config.integration.ap_polling.factors.model_aria"));
    factorInput.setAttribute("aria-label", t("config.integration.ap_polling.factors.factor_aria"));
    remove.setAttribute("aria-label", t("config.integration.ap_polling.factors.remove"));
    remove.title = t("config.integration.ap_polling.factors.remove");
    remove.addEventListener("click", () => row.remove());
    return row;
}

function renderFactorRows(factors = {}) {
    const body = document.getElementById("apPollingFactorBody");
    if (!body) return;
    body.innerHTML = "";
    for (const [model, factor] of Object.entries(factors || {})) {
        body.appendChild(buildFactorRow(model, String(factor)));
    }
}

function collectFactorRows() {
    // translatable: false — "#apPollingFactorBody tr" is a DOM selector, not a label
    return Array.from(document.querySelectorAll("#apPollingFactorBody tr")).map((row) => ({
        model: row.querySelector(".ap-polling-factor-model").value,
        factor: row.querySelector(".ap-polling-factor-value").value,
    }));
}

function polledDeviceRow(device) {
    const row = document.createElement("tr");

    const deviceCell = document.createElement("td");
    const node = document.createElement("div");
    node.className = "fw-semibold";
    node.textContent = device.node_id || "—";
    const meta = document.createElement("div");
    meta.className = "small text-muted";
    meta.textContent = [device.model, device.mgmt_ip].filter(Boolean).join(" · ") || "—";
    deviceCell.append(node, meta);
    row.appendChild(deviceCell);

    const adapterCell = document.createElement("td");
    adapterCell.textContent = device.adapter_id || "—";
    row.appendChild(adapterCell);

    const capacityCell = document.createElement("td");
    if (typeof device.capacity_mbps === "number" && Number.isFinite(device.capacity_mbps)) {
        capacityCell.appendChild(document.createTextNode(
            formatNumber(device.capacity_mbps, {maximumFractionDigits: 1}),
        ));
        capacityCell.appendChild(document.createTextNode(" "));
    } else {
        capacityCell.appendChild(document.createTextNode("—"));
        capacityCell.appendChild(document.createTextNode(" "));
    }
    const source = capacitySourceBadge(device.capacity_source);
    const badge = document.createElement("span");
    badge.className = `badge ${source.className}`;
    badge.textContent = source.token;
    badge.title = apCapacitySourceTitle(source.token);
    capacityCell.appendChild(badge);
    row.appendChild(capacityCell);

    const numberCell = (value) => {
        const cell = document.createElement("td");
        cell.textContent = typeof value === "number" && Number.isFinite(value)
            ? formatNumber(value)
            : "—";
        return cell;
    };
    row.appendChild(numberCell(device.station_count));
    row.appendChild(numberCell(device.crm_pinned_count));

    const driftCell = document.createElement("td");
    driftCell.textContent = driftText(device.drift_delta);
    driftCell.className = driftClass(device.drift_delta);
    row.appendChild(driftCell);

    const lastPollCell = document.createElement("td");
    lastPollCell.textContent = Number(device.last_poll) > 0
        ? formatDateTime(Number(device.last_poll) * 1000)
        : "—";
    row.appendChild(lastPollCell);

    const stateCell = document.createElement("td");
    const state = statusBadge(device.state);
    const stateBadge = document.createElement("span");
    stateBadge.className = `badge ${state.className}`;
    stateBadge.textContent = apStateLabel(state.token);
    stateCell.appendChild(stateBadge);
    row.appendChild(stateCell);

    return row;
}

function renderApPolling(payload) {
    const box = document.getElementById("apPollingUnavailable");
    const table = document.getElementById("apPollingTable");
    const line = document.getElementById("apPollingStatusLine");
    const body = document.getElementById("apPollingRows");
    if (!box || !table || !line || !body) return;

    if (isUnavailable(payload)) {
        table.classList.add("d-none");
        box.classList.remove("d-none");
        const reasons = Array.isArray(payload?.unavailable_reasons) ? payload.unavailable_reasons : [];
        box.textContent = apUnavailableMessage(reasons);
        line.textContent = "";
        body.innerHTML = "";
        return;
    }

    box.classList.add("d-none");
    table.classList.remove("d-none");
    body.innerHTML = "";
    const devices = Array.isArray(payload.devices) ? payload.devices : [];
    if (devices.length === 0) {
        line.textContent = t("config.integration.ap_polling.status.empty");
        return;
    }
    for (const device of devices) {
        body.appendChild(polledDeviceRow(device));
    }
    line.textContent = t("config.integration.ap_polling.status.summary", summaryCounts(payload.aggregate));
}

async function refreshApPolling() {
    try {
        const response = await fetch("/local-api/ap-polling", {
            method: "GET",
            // translatable: false — "Accept" is an HTTP header name, not a label
            headers: {"Accept": "application/json"},
            credentials: "same-origin",
        });
        if (!response.ok) {
            renderApPolling({state: "unavailable", unavailable_reasons: ["request_failed"]});
            return;
        }
        renderApPolling(await response.json());
    } catch {
        renderApPolling({state: "unavailable", unavailable_reasons: ["request_failed"]});
    }
}

function populateApPolling(config) {
    const section = config?.ap_polling || {};
    const enabled = document.getElementById("apPollingEnabled");
    if (enabled) enabled.checked = section.enabled === true;
    const interval = document.getElementById("apPollingInterval");
    if (interval) interval.value = section.interval_s ?? 300;
    renderFactorRows(section.capacity_factor || {});
}

function startApPollingStatus() {
    document.getElementById("apPollingAddFactor")?.addEventListener("click", () => {
        document.getElementById("apPollingFactorBody")?.appendChild(buildFactorRow());
    });
    document.getElementById("apPollingRefresh")?.addEventListener("click", refreshApPolling);
    refreshApPolling();
    if (apPollingRefreshTimer === null) {
        apPollingRefreshTimer = setInterval(refreshApPolling, AP_POLLING_REFRESH_MS);
    }
}

function validateConfig() {
    // Validate queue refresh interval
    const interval = parseInt(document.getElementById("queueRefreshInterval").value);
    if (isNaN(interval) || interval < 1) {
        alert(t("config.integration.validation.queue_refresh"));
        return false;
    }
    
    // Validate promote_to_root entries
    const rawPromote = document.getElementById("promoteToRoot").value;
    const hasInvalidEntries = rawPromote.split('\n')
        .some(line => line.trim().length === 0 && rawPromote.trim().length > 0);
    if (hasInvalidEntries) {
        alert(t("config.integration.validation.promote_empty_lines"));
        return false;
    }
    
    // Validate multiplier
    const multiplier = parseFloat(document.getElementById("clientBandwidthMultiplier").value);
    if (isNaN(multiplier) || multiplier <= 0) {
        alert(t("config.integration.validation.client_multiplier"));
        return false;
    }

    const ethernetMultiplierRaw = document.getElementById("ethernetPortLimitMultiplier").value.trim();
    if (ethernetMultiplierRaw.length > 0) {
        const ethernetMultiplier = parseFloat(ethernetMultiplierRaw);
        if (isNaN(ethernetMultiplier) || ethernetMultiplier <= 0 || ethernetMultiplier > 1) {
            alert(t("config.integration.validation.ethernet_multiplier"));
            return false;
        }
    }

    const sourceValue = document.getElementById("topologySource").value;
    const definition = topologySourceDefinition(sourceValue);
    if (!definition) {
        alert(t("config.integration.select_source"));
        return false;
    }
    const topologyMode = document.getElementById("topologyCompileMode").value.trim();
    if (!definition.modes.includes(topologyMode)) {
        alert(t("config.integration.validation.mode_unsupported", {
            source: definition.label,
            modes: definition.modes.join(", "),
        }));
        return false;
    }

    const apInterval = document.getElementById("apPollingInterval");
    if (apInterval && !intervalValid(apInterval.value)) {
        alert(t("config.integration.ap_polling.validation.interval"));
        return false;
    }
    const apFactors = validateFactorRows(collectFactorRows());
    if (!apFactors.ok) {
        alert(apFactorErrorMessage(apFactors));
        return false;
    }

    return true;
}

function updateConfig() {
    // Update only the integration_common section
    window.config.integration_common = {
        ...(window.config.integration_common || {}),
        circuit_name_as_address: document.getElementById("circuitNameAsAddress").checked,
        use_mikrotik_ipv6: document.getElementById("useMikrotikIpv6").checked,
        queue_refresh_interval_mins: parseInt(document.getElementById("queueRefreshInterval").value),
        promote_to_root: (() => {
            const raw = document.getElementById("promoteToRoot").value;
            const list = raw.split('\n')
                .map(line => line.trim())
                .filter(line => line.length > 0);
            return list.length > 0 ? list : null;
        })(),
        client_bandwidth_multiplier: (() => {
            const value = parseFloat(document.getElementById("clientBandwidthMultiplier").value);
            return value === 1.0 ? null : value; // Store as null for default to save space
        })(),
        ethernet_port_limits_enabled: document.getElementById("ethernetPortLimitsEnabled").checked,
        ethernet_port_limit_multiplier: (() => {
            const raw = document.getElementById("ethernetPortLimitMultiplier").value.trim();
            if (!raw.length) {
                return null;
            }
            const value = parseFloat(raw);
            return value === 0.94 ? null : value;
        })(),
    };
    window.config.topology = buildTopologyConfigUpdate(
        window.config,
        document.getElementById("topologySource").value,
        document.getElementById("topologyCompileMode").value.trim(),
    );
    const apFactors = validateFactorRows(collectFactorRows());
    window.config.ap_polling = {
        ...(window.config.ap_polling || {}),
        enabled: document.getElementById("apPollingEnabled").checked,
        interval_s: Number(document.getElementById("apPollingInterval").value),
        capacity_factor: apFactors.ok
            ? apFactors.factors
            : (window.config.ap_polling?.capacity_factor || {}),
    };
    if (window.config.uisp_integration && typeof window.config.uisp_integration === "object") {
        window.config.uisp_integration.strategy = window.config.topology.compile_mode;
    }
    if (window.config.splynx_integration && typeof window.config.splynx_integration === "object") {
        window.config.splynx_integration.strategy = window.config.topology.compile_mode;
    }
}

// Render the configuration menu
renderConfigMenu('integration');
// The help paragraphs wrap a literal file or column name in `<code>`, which the
// DOM applier cannot keep: they are empty in the page and rendered here.
renderMarkupText({
    circuitNameAsAddressHelp: tHtml("config.integration.circuit_name_as_address_help"),
    mikrotikIpv6Help: tHtml("config.integration.mikrotik_ipv6_help"),
    clientMultiplierHelp: tHtml("config.integration.client_multiplier_help"),
    ethernetMultiplierHelp: tHtml("config.integration.ethernet_multiplier_help"),
});

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    populateTopologySources();
    if (window.config && window.config.integration_common) {
        const integration = window.config.integration_common;
        
        // Boolean fields
        document.getElementById("circuitNameAsAddress").checked = 
            integration.circuit_name_as_address ?? false;
        document.getElementById("useMikrotikIpv6").checked = 
            integration.use_mikrotik_ipv6 ?? false;

        // Numeric field
        document.getElementById("queueRefreshInterval").value = 
            integration.queue_refresh_interval_mins ?? 30;

        // Promote to root field
        const promoteRoot = integration.promote_to_root ? integration.promote_to_root.join('\n') : '';
        document.getElementById("promoteToRoot").value = promoteRoot;
        document.getElementById("clientBandwidthMultiplier").value = 
            (integration.client_bandwidth_multiplier ?? 1.0).toFixed(1);
        document.getElementById("ethernetPortLimitsEnabled").checked =
            integration.ethernet_port_limits_enabled ?? true;
        document.getElementById("ethernetPortLimitMultiplier").value =
            (integration.ethernet_port_limit_multiplier ?? 0.94).toFixed(2);
        const topologySelect = document.getElementById("topologySource");
        topologySelect.value = topologySource(window.config) || "";
        renderCompileModes();
        renderTopologyStatus();
        topologySelect.addEventListener("change", () => {
            renderCompileModes();
            renderTopologyStatus();
        });

        populateApPolling(window.config);
        startApPollingStatus();

        // Add save button click handler
        document.getElementById('saveButton').addEventListener('click', () => {
            if (validateConfig()) {
                updateConfig();
                saveConfig(() => {
                    alert(t("config.integration.saved"));
                    loadConfig(() => {
                        renderCompileModes();
                        renderTopologyStatus();
                        populateApPolling(window.config);
                        refreshApPolling();
                    });
                });
            }
        });
    } else {
        // translatable: false — "Integration configuration not found in window.config" is a console diagnostic, never rendered
        console.error("Integration configuration not found in window.config");
    }
});
