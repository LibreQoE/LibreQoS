import {saveConfig, loadConfig, renderConfigMenu} from "./config/config_helper";

function isValidIPv4(ip) {
    if (!/^(\d{1,3}\.){3}\d{1,3}$/.test(ip)) return false;
    const parts = ip.split('.').map((p) => parseInt(p, 10));
    return parts.length === 4 && !parts.some((p) => Number.isNaN(p) || p < 0 || p > 255);
}

function isValidIPv6(ip) {
    return ip.includes(':') && /^[0-9a-fA-F:]+$/.test(ip);
}

function normalizeSubnetInput(value) {
    const raw = String(value || "").trim();
    if (!raw) return "";
    if (raw.includes('/')) return raw;
    if (isValidIPv4(raw)) return `${raw}/32`;
    if (isValidIPv6(raw)) return `${raw}/128`;
    return raw;
}

function isValidCIDR(cidr) {
    try {
        const [ip, mask, extra] = String(cidr).trim().split('/');
        if (!ip || !mask || extra !== undefined) return false;

        if (ip.includes(':')) {
            if (!isValidIPv6(ip)) return false;
        } else if (!isValidIPv4(ip)) {
            return false;
        }

        const maskNum = parseInt(mask, 10);
        if (Number.isNaN(maskNum)) return false;
        if (ip.includes(':')) {
            if (maskNum < 0 || maskNum > 128) return false;
        } else {
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
    const cidr = normalizeSubnetInput(input.value);

    if (!isValidCIDR(cidr)) {
        alert('Please enter a valid IP or CIDR notation (e.g. 192.168.1.0/24, 2803:16d0:40::/48, or 2001:db8::1)');
        return;
    }

    const select = document.getElementById(listId);
    for (let i = 0; i < select.options.length; i++) {
        if (select.options[i].value.toLowerCase() === cidr.toLowerCase()) {
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

// Render the configuration menu
renderConfigMenu('iprange');

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
