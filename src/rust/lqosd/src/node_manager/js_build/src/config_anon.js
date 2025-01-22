import {loadConfig} from "./config/config_helper";

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    if (window.config && window.config.usage_stats) {
        // Required fields
        document.getElementById("sendAnonymous").checked = window.config.usage_stats.send_anonymous ?? true;
        document.getElementById("anonymousServer").value = window.config.usage_stats.anonymous_server ?? "stats.libreqos.io:9125";
    } else {
        console.error("Usage statistics configuration not found in window.config");
    }
});
