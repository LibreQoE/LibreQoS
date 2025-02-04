import {saveConfig, loadConfig} from "./config/config_helper";

function isValidCIDR(cidr) {
    try {
        const [ip, mask] = cidr.split('/');
        if (!ip || !mask) return false;
        
        // Validate IP address
        if (ip.includes(':')) {
            // IPv6
            if (!/^([0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4}$/.test(ip)) return false;
        } else {
            // IPv4
            if (!/^(\d{1,3}\.){3}\d{1,3}$/.test(ip)) return false;
        }
        
        // Validate mask
        const maskNum = parseInt(mask);
        if (isNaN(maskNum)) return false;
        if (ip.includes(':')) {
            // IPv6
            if (maskNum < 0 || maskNum > 128) return false;
        } else {
            // IPv4
            if (maskNum < 0 || maskNum > 32) return false;
        }
        
        return true;
    } catch {
        return false;
    }
}

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

function addSubnet(listId, inputId) {
    const input = document.getElementById(inputId);
    const cidr = input.value.trim();
    
    if (!isValidCIDR(cidr)) {
        alert('Please enter a valid CIDR notation (e.g. 192.168.1.0/24 or 2001:db8::/32)');
        return;
    }
    
    const select = document.getElementById(listId);
    // Check for duplicates
    for (let i = 0; i < select.options.length; i++) {
        if (select.options[i].value === cidr) {
            alert('This CIDR is already in the list');
            return;
        }
    }
    
    const option = document.createElement('option');
    option.value = cidr;
    option.text = cidr;
    select.appendChild(option);
    input.value = ''; // Clear input
}

function removeSubnet(listId) {
    const select = document.getElementById(listId);
    const selected = Array.from(select.selectedOptions);
    selected.forEach(option => select.removeChild(option));
}

function getSubnetsFromList(listId) {
    const select = document.getElementById(listId);
    return Array.from(select.options).map(option => option.value);
}

function updateConfig() {
    // Update only the ip_ranges section
    window.config.ip_ranges = {
        ignore_subnets: getSubnetsFromList('ignoredSubnets'),
        allow_subnets: getSubnetsFromList('allowedSubnets'),
        unknown_ip_honors_ignore: document.getElementById('unknownHonorsIgnore').checked,
        unknown_ip_honors_allow: document.getElementById('unknownHonorsAllow').checked
    };
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
            
        // Add event listeners
        document.getElementById('addIgnoredSubnet').addEventListener('click', () => {
            addSubnet('ignoredSubnets', 'newIgnoredSubnet');
        });
        document.getElementById('removeIgnoredSubnet').addEventListener('click', () => {
            removeSubnet('ignoredSubnets');
        });
        document.getElementById('addAllowedSubnet').addEventListener('click', () => {
            addSubnet('allowedSubnets', 'newAllowedSubnet');
        });
        document.getElementById('removeAllowedSubnet').addEventListener('click', () => {
            removeSubnet('allowedSubnets');
        });

        // Add save button click handler
        document.getElementById('saveButton').addEventListener('click', () => {
            updateConfig();
            saveConfig(() => {
                alert("Configuration saved successfully!");
            });
        });
    } else {
        console.error("IP Ranges configuration not found in window.config");
    }
});
