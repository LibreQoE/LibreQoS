import {saveConfig, loadConfig} from "./config/config_helper";

function populateDoNotTrackList(selectId, subnets) {
    const select = document.getElementById(selectId);
    select.innerHTML = ''; // Clear existing options
    if (subnets) {
        subnets.forEach(subnet => {
            const option = document.createElement('option');
            option.value = subnet;
            option.text = subnet;
            select.appendChild(option);
        });
    }
}

function validateConfig() {
    // Validate required fields
    const flowTimeout = parseInt(document.getElementById("flowTimeout").value);
    if (isNaN(flowTimeout) || flowTimeout < 1) {
        alert("Flow Timeout must be a number greater than 0");
        return false;
    }

    // Validate optional fields if provided
    const netflowPort = document.getElementById("netflowPort").value;
    if (netflowPort && (isNaN(netflowPort) || netflowPort < 1 || netflowPort > 65535)) {
        alert("Netflow Port must be a number between 1 and 65535");
        return false;
    }

    const netflowIp = document.getElementById("netflowIp").value.trim();
    if (netflowIp) {
        try {
            new URL(`http://${netflowIp}`);
        } catch {
            alert("Netflow IP must be a valid IP address");
            return false;
        }
    }

    return true;
}

function updateConfig() {
    // Update only the flows section
    window.config.flows = {
        flow_timeout_seconds: parseInt(document.getElementById("flowTimeout").value),
        netflow_enabled: document.getElementById("enableNetflow").checked,
        netflow_port: document.getElementById("netflowPort").value ? 
            parseInt(document.getElementById("netflowPort").value) : null,
        netflow_ip: document.getElementById("netflowIp").value.trim() || null,
        netflow_version: document.getElementById("netflowVersion").value ?
            parseInt(document.getElementById("netflowVersion").value) : null,
        do_not_track_subnets: getSubnetsFromList('doNotTrackSubnets')
    };
}

function getSubnetsFromList(listId) {
    const select = document.getElementById(listId);
    return Array.from(select.options).map(option => option.value);
}

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    if (window.config && window.config.flows) {
        const flows = window.config.flows;
        
        // Required fields
        document.getElementById("flowTimeout").value = flows.flow_timeout_seconds;
        document.getElementById("enableNetflow").checked = flows.netflow_enabled ?? false;

        // Optional fields
        document.getElementById("netflowPort").value = flows.netflow_port ?? "";
        document.getElementById("netflowIP").value = flows.netflow_ip ?? "";
        document.getElementById("netflowVersion").value = flows.netflow_version ?? "5";

        // Populate do not track list
        populateDoNotTrackList('doNotTrackSubnets', flows.do_not_track_subnets);

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
        console.error("Flows configuration not found in window.config");
    }
});
