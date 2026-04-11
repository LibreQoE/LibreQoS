import {saveConfig, loadConfig, renderConfigMenu} from "./config/config_helper";

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

    const topologyMode = document.getElementById("topologyCompileMode").value.trim();
    if (!["flat", "ap_only", "ap_site", "full"].includes(topologyMode)) {
        alert("Topology Compile Mode must be one of Flat, AP Only, AP Site, or Full");
        return false;
    }

    const queueAutoThreshold = parseInt(
        document.getElementById("queueAutoVirtualizeThresholdMbps").value,
        10,
    );
    if (isNaN(queueAutoThreshold) || queueAutoThreshold < 5001) {
        alert("Queue Auto-Virtualize Threshold must be a whole number greater than or equal to 5001");
        return false;
    }
    
    return true;
}

function updateConfig() {
    // Update only the integration_common section
    window.config.integration_common = {
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
    window.config.topology = {
        ...(window.config.topology || {}),
        compile_mode: document.getElementById("topologyCompileMode").value.trim(),
        queue_auto_virtualize_threshold_mbps: parseInt(
            document.getElementById("queueAutoVirtualizeThresholdMbps").value,
            10,
        ),
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

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
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
        document.getElementById("topologyCompileMode").value =
            window.config.topology?.compile_mode ?? "ap_site";
        document.getElementById("queueAutoVirtualizeThresholdMbps").value =
            String(window.config.topology?.queue_auto_virtualize_threshold_mbps ?? 5001);

        // Add save button click handler
        document.getElementById('saveButton').addEventListener('click', () => {
            if (validateConfig()) {
                updateConfig();
                saveConfig(() => {
                    alert("Configuration saved successfully!");
                });
            }
        });
    } else {
        console.error("Integration configuration not found in window.config");
    }
});
