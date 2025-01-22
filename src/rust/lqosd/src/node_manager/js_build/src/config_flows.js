import {loadConfig} from "./config/config_helper";

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

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    if (window.config && window.config.flows) {
        const flows = window.config.flows;
        
        // Required fields
        document.getElementById("flowTimeout").value = flows.flow_timeout_seconds;
        document.getElementById("enableNetflow").checked = flows.netflow_enabled ?? false;

        // Optional fields
        if (flows.netflow_port) {
            document.getElementById("netflowPort").value = flows.netflow_port;
        }
        if (flows.netflow_ip) {
            document.getElementById("netflowIp").value = flows.netflow_ip;
        }
        if (flows.netflow_version) {
            document.getElementById("netflowVersion").value = flows.netflow_version;
        }

        // Populate do not track list
        populateDoNotTrackList('doNotTrackSubnets', flows.do_not_track_subnets);
    } else {
        console.error("Flows configuration not found in window.config");
    }
});
