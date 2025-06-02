import {saveConfig, loadConfig} from "./config/config_helper";

function validateConfig() {
    // Validate numeric fields
    const uplink = parseInt(document.getElementById("uplinkBandwidth").value);
    if (isNaN(uplink) || uplink < 1) {
        alert("Uplink Bandwidth must be a number greater than 0");
        return false;
    }

    const downlink = parseInt(document.getElementById("downlinkBandwidth").value);
    if (isNaN(downlink) || downlink < 1) {
        alert("Downlink Bandwidth must be a number greater than 0");
        return false;
    }

    const pnDownload = parseInt(document.getElementById("generatedPnDownload").value);
    if (isNaN(pnDownload) || pnDownload < 1) {
        alert("Per-Node Download must be a number greater than 0");
        return false;
    }

    const pnUpload = parseInt(document.getElementById("generatedPnUpload").value);
    if (isNaN(pnUpload) || pnUpload < 1) {
        alert("Per-Node Upload must be a number greater than 0");
        return false;
    }

    const overrideQueues = document.getElementById("overrideQueues").value;
    if (overrideQueues && (isNaN(overrideQueues) || overrideQueues < 1)) {
        alert("Override Queues must be a number greater than 0");
        return false;
    }

    // Validate lazy queue expiration seconds
    const lazyExpireSeconds = document.getElementById("lazyExpireSeconds").value;
    if (lazyExpireSeconds && (isNaN(lazyExpireSeconds) || lazyExpireSeconds < 30)) {
        alert("Lazy Queue Expiration must be at least 30 seconds");
        return false;
    }

    return true;
}

function updateConfig() {
    // Update only the queues section
    window.config.queues = {
        default_sqm: document.getElementById("defaultSqm").value,
        monitor_only: document.getElementById("monitorOnly").checked,
        uplink_bandwidth_mbps: parseInt(document.getElementById("uplinkBandwidth").value),
        downlink_bandwidth_mbps: parseInt(document.getElementById("downlinkBandwidth").value),
        generated_pn_download_mbps: parseInt(document.getElementById("generatedPnDownload").value),
        generated_pn_upload_mbps: parseInt(document.getElementById("generatedPnUpload").value),
        dry_run: document.getElementById("dryRun").checked,
        sudo: document.getElementById("sudo").checked,
        override_available_queues: document.getElementById("overrideQueues").value ? 
            parseInt(document.getElementById("overrideQueues").value) : null,
        use_binpacking: document.getElementById("useBinpacking").checked,
        lazy_queues: document.getElementById("lazyQueues").checked ? true : null,
        lazy_expire_seconds: document.getElementById("lazyExpireSeconds").value ? 
            parseInt(document.getElementById("lazyExpireSeconds").value) : null
    };
}

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    if (window.config && window.config.queues) {
        const queues = window.config.queues;
        
        // Select field
        document.getElementById("defaultSqm").value = queues.default_sqm;

        // Boolean fields
        document.getElementById("monitorOnly").checked = queues.monitor_only ?? false;
        document.getElementById("dryRun").checked = queues.dry_run ?? false;
        document.getElementById("sudo").checked = queues.sudo ?? false;
        document.getElementById("useBinpacking").checked = queues.use_binpacking ?? false;
        document.getElementById("lazyQueues").checked = queues.lazy_queues ?? false;

        // Numeric fields
        document.getElementById("uplinkBandwidth").value = queues.uplink_bandwidth_mbps ?? 1000;
        document.getElementById("downlinkBandwidth").value = queues.downlink_bandwidth_mbps ?? 1000;
        document.getElementById("generatedPnDownload").value = queues.generated_pn_download_mbps ?? 1000;
        document.getElementById("generatedPnUpload").value = queues.generated_pn_upload_mbps ?? 1000;

        // Optional numeric fields
        document.getElementById("overrideQueues").value = queues.override_available_queues ?? "";
        document.getElementById("lazyExpireSeconds").value = queues.lazy_expire_seconds ?? "";

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
        console.error("Queue configuration not found in window.config");
    }
});
