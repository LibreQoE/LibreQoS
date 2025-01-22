import {loadConfig} from "./config/config_helper";

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    document.getElementById("nodeId").value = window.config.node_id;
    document.getElementById("nodeName").value = window.config.node_name;
    document.getElementById("packetCaptureTime").value = window.config.packet_capture_time;
    document.getElementById("queueCheckPeriod").value = window.config.queue_check_period_ms;
    
    // Handle optional boolean with nullish coalescing
    document.getElementById("disableWebserver").checked = window.config.disable_webserver ?? false;
    
    // Handle optional string with nullish coalescing
    document.getElementById("webserverListen").value = window.config.webserver_listen ?? "";
});
