import {saveConfig, loadConfig, renderConfigMenu} from "./config/config_helper";

function isValidCIDR(cidr) {
    try {
        const [ip, mask] = String(cidr).trim().split('/');
        if (!ip || !mask) return false;

        // Validate IP address (basic)
        if (ip.includes(':')) {
            // IPv6 (very permissive: 2+ hex chars and colons; exact validation happens in backend)
            if (!/^[0-9a-fA-F:]+$/.test(ip)) return false;
        } else {
            // IPv4
            if (!/^(\d{1,3}\.){3}\d{1,3}$/.test(ip)) return false;
            const parts = ip.split('.').map(p => parseInt(p, 10));
            if (parts.length !== 4 || parts.some(p => Number.isNaN(p) || p < 0 || p > 255)) return false;
        }

        // Validate mask
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

function populateDoNotTrackList(selectId, subnets) {
    const select = document.getElementById(selectId);
    select.innerHTML = '';
    (subnets || []).forEach((subnet) => {
        const option = document.createElement('option');
        option.value = subnet;
        option.text = subnet;
        select.appendChild(option);
    });
}

function addSubnet(listId, inputId) {
    const input = document.getElementById(inputId);
    const cidr = String(input.value || "").trim();
    if (!isValidCIDR(cidr)) {
        alert('Please enter a valid CIDR notation (e.g. 192.168.1.0/24 or 2001:db8::/32)');
        return;
    }

    const select = document.getElementById(listId);
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
    input.value = '';
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

function validateDoNotTrackList() {
    const items = getSubnetsFromList('doNotTrackSubnets');
    return items.filter((cidr) => !isValidCIDR(cidr));
}

function updateDoNotTrackValidationUi() {
    const invalid = validateDoNotTrackList();
    const holder = document.getElementById("doNotTrackValidation");
    const save = document.getElementById("saveButton");
    if (save) save.disabled = invalid.length > 0;
    if (!holder) return;

    if (invalid.length === 0) {
        holder.className = "small mt-3 text-success";
        holder.innerHTML = `All entries look like valid CIDR notation. The flow tracker will honor this ignore list.`;
        return;
    }

    holder.className = "small mt-3 text-danger";
    holder.innerHTML = `
        <div><strong>Invalid CIDR entries detected:</strong></div>
        <ul class="mb-0">${invalid.map(v => `<li><code>${v}</code></li>`).join("")}</ul>
        <div class="mt-1">Fix/remove these entries to enable saving.</div>
    `;
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

    const netflowIp = document.getElementById("netflowIP").value.trim();
    if (netflowIp) {
        try {
            new URL(`http://${netflowIp}`);
        } catch {
            alert("Netflow IP must be a valid IP address");
            return false;
        }
    }

    const invalid = validateDoNotTrackList();
    if (invalid.length > 0) {
        alert("Invalid CIDR entries:\n" + invalid.join("\n"));
        return false;
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
        netflow_ip: document.getElementById("netflowIP").value.trim() || null,
        netflow_version: document.getElementById("netflowVersion").value ?
            parseInt(document.getElementById("netflowVersion").value) : null,
        do_not_track_subnets: getSubnetsFromList('doNotTrackSubnets'),
    };
}

// Render the configuration menu
renderConfigMenu('flows');

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    if (window.config) {
        const flows = window.config.flows || {
            flow_timeout_seconds: 30,
            netflow_enabled: false,
            netflow_port: null,
            netflow_ip: null,
            netflow_version: null,
            do_not_track_subnets: [],
        };
        
        // Required fields
        document.getElementById("flowTimeout").value = flows.flow_timeout_seconds ?? 30;
        document.getElementById("enableNetflow").checked = flows.netflow_enabled ?? false;

        // Optional fields
        document.getElementById("netflowPort").value = flows.netflow_port ?? "";
        document.getElementById("netflowIP").value = flows.netflow_ip ?? "";
        document.getElementById("netflowVersion").value = flows.netflow_version ?? "5";

        // Populate do not track list
        populateDoNotTrackList('doNotTrackSubnets', flows.do_not_track_subnets || []);
        updateDoNotTrackValidationUi();

        document.getElementById('addDoNotTrackSubnet').addEventListener('click', () => {
            addSubnet('doNotTrackSubnets', 'newDoNotTrackSubnet');
            updateDoNotTrackValidationUi();
        });
        document.getElementById('removeDoNotTrackSubnet').addEventListener('click', () => {
            removeSubnet('doNotTrackSubnets');
            updateDoNotTrackValidationUi();
        });

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
