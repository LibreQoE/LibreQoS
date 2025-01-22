import {loadConfig} from "./config/config_helper";

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

        // Numeric fields
        if (queues.uplink_bandwidth_mbps) {
            document.getElementById("uplinkBandwidth").value = queues.uplink_bandwidth_mbps;
        }
        if (queues.downlink_bandwidth_mbps) {
            document.getElementById("downlinkBandwidth").value = queues.downlink_bandwidth_mbps;
        }
        if (queues.generated_pn_download_mbps) {
            document.getElementById("generatedPnDownload").value = queues.generated_pn_download_mbps;
        }
        if (queues.generated_pn_upload_mbps) {
            document.getElementById("generatedPnUpload").value = queues.generated_pn_upload_mbps;
        }

        // Optional numeric field
        if (queues.override_available_queues) {
            document.getElementById("overrideQueues").value = queues.override_available_queues;
        }
    } else {
        console.error("Queue configuration not found in window.config");
    }
});
