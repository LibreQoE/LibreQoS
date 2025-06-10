import {saveConfig, loadConfig, renderConfigMenu} from "./config/config_helper";

function validateConfig() {
    // Validate server address format if provided
    const server = document.getElementById("anonymousServer").value.trim();
    if (server) {
        const parts = server.split(':');
        if (parts.length !== 2 || isNaN(parseInt(parts[1]))) {
            alert("Statistics Server must be in format HOST:PORT");
            return false;
        }
    }
    return true;
}

function updateConfig() {
    // Update only the usage stats section
    window.config.usage_stats.send_anonymous = document.getElementById("sendAnonymous").checked;
    window.config.usage_stats.anonymous_server = document.getElementById("anonymousServer").value.trim();
}

// Render the configuration menu
renderConfigMenu('anon');

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    if (window.config && window.config.usage_stats) {
        // Required fields
        document.getElementById("sendAnonymous").checked = window.config.usage_stats.send_anonymous ?? true;
        document.getElementById("anonymousServer").value = window.config.usage_stats.anonymous_server ?? "stats.libreqos.io:9125";

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
        console.error("Usage statistics configuration not found in window.config");
    }
});
