import {saveConfig, loadConfig, renderConfigMenu} from "./config/config_helper";

function validateConfig() {
    // Validate required fields
    const nodeName = document.getElementById("nodeName").value.trim();
    if (!nodeName) {
        alert("Node Name is required");
        return false;
    }

    const packetCaptureTime = parseInt(document.getElementById("packetCaptureTime").value);
    if (isNaN(packetCaptureTime) || packetCaptureTime < 1) {
        alert("Packet Capture Time must be a number greater than 0");
        return false;
    }

    const queueCheckPeriod = parseInt(document.getElementById("queueCheckPeriod").value);
    if (isNaN(queueCheckPeriod) || queueCheckPeriod < 100) {
        alert("Queue Check Period must be a number of at least 100 milliseconds");
        return false;
    }

    // Validate webserver listen address if provided
    const webserverListen = document.getElementById("webserverListen").value.trim();
    if (webserverListen) {
        const parts = webserverListen.split(':');
        if (parts.length !== 2 || isNaN(parseInt(parts[1]))) {
            alert("Web Server Listen Address must be in format IP:PORT");
            return false;
        }
    }

    return true;
}

function updateConfig() {
    // Update only the general configuration section
    window.config.node_name = document.getElementById("nodeName").value.trim();
    window.config.packet_capture_time = parseInt(document.getElementById("packetCaptureTime").value);
    window.config.queue_check_period_ms = parseInt(document.getElementById("queueCheckPeriod").value);
    window.config.disable_webserver = document.getElementById("disableWebserver").checked;
    window.config.disable_icmp_ping = document.getElementById("disableIcmpPing").checked;
    window.config.enable_circuit_heatmaps = document.getElementById("enableCircuitHeatmaps").checked;
    window.config.enable_site_heatmaps = document.getElementById("enableSiteHeatmaps").checked;
    window.config.enable_asn_heatmaps = document.getElementById("enableAsnHeatmaps").checked;
    
    const webserverListen = document.getElementById("webserverListen").value.trim();
    window.config.webserver_listen = webserverListen ? webserverListen : null;
}

// Render the configuration menu
renderConfigMenu('general');

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    if (window.config) {
        // Required fields
        if (window.config.node_id) {
            document.getElementById("nodeId").value = window.config.node_id;
        }
        if (window.config.node_name) {
            document.getElementById("nodeName").value = window.config.node_name;
        }
        if (window.config.packet_capture_time) {
            document.getElementById("packetCaptureTime").value = window.config.packet_capture_time;
        }
        if (window.config.queue_check_period_ms) {
            document.getElementById("queueCheckPeriod").value = window.config.queue_check_period_ms;
        }

        // Optional fields with nullish coalescing
        document.getElementById("disableWebserver").checked = window.config.disable_webserver ?? false;
        document.getElementById("webserverListen").value = window.config.webserver_listen ?? "";
        document.getElementById("disableIcmpPing").checked = window.config.disable_icmp_ping ?? false;
        document.getElementById("enableCircuitHeatmaps").checked = window.config.enable_circuit_heatmaps ?? true;
        document.getElementById("enableSiteHeatmaps").checked = window.config.enable_site_heatmaps ?? true;
        document.getElementById("enableAsnHeatmaps").checked = window.config.enable_asn_heatmaps ?? true;

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
        console.error("Configuration not found in window.config");
    }
});
