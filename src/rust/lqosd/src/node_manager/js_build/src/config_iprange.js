import {loadConfig} from "./config/config_helper";

function populateSubnetList(selectId, subnets) {
    const select = document.getElementById(selectId);
    select.innerHTML = ''; // Clear existing options
    subnets.forEach(subnet => {
        const option = document.createElement('option');
        option.value = subnet;
        option.text = subnet;
        select.appendChild(option);
    });
}

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    if (window.config && window.config.ip_ranges) {
        const ipRanges = window.config.ip_ranges;
        
        // Populate subnet lists
        populateSubnetList('ignoredSubnets', ipRanges.ignore_subnets);
        populateSubnetList('allowedSubnets', ipRanges.allow_subnets);

        // Set checkbox states
        document.getElementById('unknownHonorsIgnore').checked = 
            ipRanges.unknown_ip_honors_ignore ?? true;
        document.getElementById('unknownHonorsAllow').checked = 
            ipRanges.unknown_ip_honors_allow ?? true;
    } else {
        console.error("IP Ranges configuration not found in window.config");
    }
});
