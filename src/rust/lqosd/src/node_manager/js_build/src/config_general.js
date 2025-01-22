import {saveConfig, loadConfig} from "./config/config_helper";

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
    } else {
        console.error("Configuration not found in window.config");
    }
});
