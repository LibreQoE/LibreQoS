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

const COMPILE_MODES = [
    {value: "flat", label: "Flat - Only shapes subscribers, no hierarchy"},
    {value: "ap_only", label: "AP Only - AP to clients"},
    {value: "ap_site", label: "AP Site - Site to AP to clients"},
    {value: "full", label: "Full - Preserve the richest hierarchy"},
];

function populateTopologySources() {
    const select = document.getElementById("topologySource");
    if (!select) return;
    select.innerHTML = "";
    const placeholder = document.createElement("option");
    placeholder.value = "";
    placeholder.textContent = "Select a topology source";
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
        placeholder.textContent = "Select a valid mode";
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
    parts.push(label ? `Active source: ${label}` : "No topology source is selected.");
    if (view.topology_source_origin) {
        parts.push(`origin: ${view.topology_source_origin}`);
    }
    if (view.topology_source_last_update) {
        parts.push(`last update: ${view.topology_source_last_update}`);
    }
    if (view.topology_source_last_result) {
        parts.push(`last result: ${view.topology_source_last_result}`);
    }
    if (view.topology_source_reject_reason) {
        parts.push(`reject: ${view.topology_source_reject_reason}`);
    }
    const ingest = view.libreqos_network_status;
    if (ingest) {
        const state = ingest.state || (ingest.gate_active ? "Gated" : "Inert");
        parts.push(`libreqos_network: ${state}`);
        if (typeof ingest.gate_active === "boolean") {
            parts.push(`gate ${ingest.gate_active ? "active" : "inactive"}`);
        }
        if (ingest.last_accepted_unix) {
            parts.push(`last accepted: ${ingest.last_accepted_unix}`);
        }
        if (ingest.last_reject_reason) {
            const suffix = ingest.last_reject_unix
                ? ` @ ${ingest.last_reject_unix}`
                : "";
            parts.push(`last rejected: ${ingest.last_reject_reason}${suffix}`);
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

function validateConfig() {
    // Validate queue refresh interval
    const interval = parseInt(document.getElementById("queueRefreshInterval").value);
    if (isNaN(interval) || interval < 1) {
        alert("Queue Refresh Interval must be a number greater than 0");
        return false;
    }
    
    // Validate promote_to_root entries
    const rawPromote = document.getElementById("promoteToRoot").value;
    const hasInvalidEntries = rawPromote.split('\n')
        .some(line => line.trim().length === 0 && rawPromote.trim().length > 0);
    if (hasInvalidEntries) {
        alert("Please remove empty lines from Promote to Root Nodes");
        return false;
    }
    
    // Validate multiplier
    const multiplier = parseFloat(document.getElementById("clientBandwidthMultiplier").value);
    if (isNaN(multiplier) || multiplier <= 0) {
        alert("Client Bandwidth Multiplier must be a number greater than 0");
        return false;
    }

    const ethernetMultiplierRaw = document.getElementById("ethernetPortLimitMultiplier").value.trim();
    if (ethernetMultiplierRaw.length > 0) {
        const ethernetMultiplier = parseFloat(ethernetMultiplierRaw);
        if (isNaN(ethernetMultiplier) || ethernetMultiplier <= 0 || ethernetMultiplier > 1) {
            alert("Ethernet Port Limit Multiplier must be a number greater than 0 and less than or equal to 1");
            return false;
        }
    }

    const sourceValue = document.getElementById("topologySource").value;
    const definition = topologySourceDefinition(sourceValue);
    if (!definition) {
        alert("Select a topology source");
        return false;
    }
    const topologyMode = document.getElementById("topologyCompileMode").value.trim();
    if (!definition.modes.includes(topologyMode)) {
        alert(`Topology Compile Mode is not supported by ${definition.label}. Choose one of: ${definition.modes.join(", ")}`);
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
    if (window.config.uisp_integration && typeof window.config.uisp_integration === "object") {
        window.config.uisp_integration.strategy = window.config.topology.compile_mode;
    }
    if (window.config.splynx_integration && typeof window.config.splynx_integration === "object") {
        window.config.splynx_integration.strategy = window.config.topology.compile_mode;
    }
}

// Render the configuration menu
renderConfigMenu('integration');

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

        // Add save button click handler
        document.getElementById('saveButton').addEventListener('click', () => {
            if (validateConfig()) {
                updateConfig();
                saveConfig(() => {
                    alert("Configuration saved successfully!");
                    loadConfig(() => {
                        renderCompileModes();
                        renderTopologyStatus();
                    });
                });
            }
        });
    } else {
        console.error("Integration configuration not found in window.config");
    }
});
