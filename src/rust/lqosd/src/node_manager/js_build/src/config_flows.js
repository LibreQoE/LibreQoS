import {saveConfig, loadConfig, renderConfigMenu} from "./config/config_helper";
import {t} from "./i18n/index.js";

function validateConfig() {
    // Validate optional fields if provided
    const netflowPort = document.getElementById("netflowPort").value;
    if (netflowPort && (isNaN(netflowPort) || netflowPort < 1 || netflowPort > 65535)) {
        alert(t("config.flows.validation.port_range"));
        return false;
    }

    const netflowIp = document.getElementById("netflowIP").value.trim();
    if (netflowIp) {
        try {
            new URL(`http://${netflowIp}`);
        } catch {
            alert(t("config.flows.validation.ip_invalid"));
            return false;
        }
    }

    return true;
}

function parseSubnets(raw) {
    return (raw || "")
        .split(/[\n,]+/)
        .map((entry) => entry.trim())
        .filter((entry) => entry.length > 0);
}

function updateConfig() {
    if (!window.config || typeof window.config !== "object") {
        return;
    }

    // Update only the flows section
    window.config.flows = {
        netflow_enabled: document.getElementById("enableNetflow").checked,
        netflow_port: document.getElementById("netflowPort").value ?
            parseInt(document.getElementById("netflowPort").value) : null,
        netflow_ip: document.getElementById("netflowIP").value.trim() || null,
        netflow_version: document.getElementById("netflowVersion").value ?
            parseInt(document.getElementById("netflowVersion").value) : null,
        do_not_track_subnets: parseSubnets(document.getElementById("doNotTrackSubnets").value),
    };
}

// Render the configuration menu
renderConfigMenu('flows');

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    if (window.config) {
        const flows = window.config.flows || {
            netflow_enabled: false,
            netflow_port: null,
            netflow_ip: null,
            netflow_version: null,
            do_not_track_subnets: [],
        };

        // Fields
        document.getElementById("enableNetflow").checked = flows.netflow_enabled ?? false;
        document.getElementById("netflowPort").value = flows.netflow_port ?? "";
        document.getElementById("netflowIP").value = flows.netflow_ip ?? "";
        document.getElementById("netflowVersion").value = flows.netflow_version ?? "5";

        // §`lqos-74t.31` W1: the do-not-track set + the RTT-exclude wizard deep link
        // (`?prefillDoNotTrack=<cidr>`, emitted by flow_rtt_exclude_wizard.js — no
        // longer a dead link).
        const doNotTrack = parseSubnets((flows.do_not_track_subnets || []).join("\n"));
        const prefill = new URLSearchParams(window.location.search).get("prefillDoNotTrack");
        for (const cidr of parseSubnets(prefill)) {
            if (!doNotTrack.includes(cidr)) {
                doNotTrack.push(cidr);
            }
        }
        document.getElementById("doNotTrackSubnets").value = doNotTrack.join("\n");

        // Add save button click handler
        document.getElementById('saveButton').addEventListener('click', () => {
            if (validateConfig()) {
                updateConfig();
                saveConfig(() => {
                    alert(t("config.flows.saved"));
                });
            }
        });
    } else {
        // translatable: false — "Flows configuration not found in window.config" is a console diagnostic, never rendered
        console.error("Flows configuration not found in window.config");
    }
});
