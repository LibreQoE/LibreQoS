export function loadConfig(onComplete) {
    $.get("/local-api/getConfig", (data) => {
        window.config = data;
        onComplete();
    });
}

export function saveConfig(onComplete) {
    $.ajax({
        type: "POST",
        url: "/local-api/updateConfig",
        data: JSON.stringify(window.config),
        contentType: 'application/json',
        success: () => {
            onComplete();
        },
        error: () => {
            alert("That didn't work");
        }
    });
}

export function saveNetworkAndDevices(network_json, shaped_devices, onComplete) {
    // Validate network_json structure
    if (!network_json || typeof network_json !== 'object') {
        alert("Invalid network configuration");
        return;
    }

    // Validate shaped_devices structure
    if (!Array.isArray(shaped_devices)) {
        alert("Invalid shaped devices configuration");
        return;
    }

    // Validate individual shaped devices
    const validationErrors = [];
    const validNodes = validNodeList(network_json);
    console.log(validNodes);
    
    shaped_devices.forEach((device, index) => {
        // Required fields
        if (!device.circuit_id || device.circuit_id.trim() === "") {
            validationErrors.push(`Device ${index + 1}: Circuit ID is required`);
        }
        if (!device.device_id || device.device_id.trim() === "") {
            validationErrors.push(`Device ${index + 1}: Device ID is required`);
        }

        // Parent node validation
        if (device.parent_node && validNodes.length > 0 && !validNodes.includes(device.parent_node)) {
            validationErrors.push(`Device ${index + 1}: Parent node '${device.parent_node}' does not exist`);
        }

        // Bandwidth validation (supports fractional Mbps)
        // Minimums must be >= 0.1 Mbps, maximums must be >= 0.2 Mbps
        const dmin = parseFloat(device.download_min_mbps);
        const umin = parseFloat(device.upload_min_mbps);
        const dmax = parseFloat(device.download_max_mbps);
        const umax = parseFloat(device.upload_max_mbps);

        if (Number.isNaN(dmin) || Number.isNaN(umin) || Number.isNaN(dmax) || Number.isNaN(umax)) {
            validationErrors.push(`Device ${index + 1}: One or more bandwidth fields are not valid numbers`);
        } else {
            if (dmin < 0.1 || umin < 0.1) {
                validationErrors.push(`Device ${index + 1}: Min rates must be >= 0.1 Mbps`);
            }
            if (dmax < 0.2 || umax < 0.2) {
                validationErrors.push(`Device ${index + 1}: Max rates must be >= 0.2 Mbps`);
            }
        }
    });

    if (validationErrors.length > 0) {
        alert("Validation errors:\n" + validationErrors.join("\n"));
        return;
    }

    // Prepare data for submission
    const submission = {
        network_json,
        shaped_devices
    };
    console.log(submission);

    // Send to server and handle plain-text "Ok" response
    $.ajax({
        type: "POST",
        url: "/local-api/updateNetworkAndDevices",
        contentType: 'application/json',
        data: JSON.stringify(submission),
        success: (data) => {
            if (onComplete) onComplete(data === "Ok", data);
        },
        error: (xhr) => {
            const errorMsg = xhr.responseText || xhr.statusText || "Request failed";
            console.error("AJAX Error:", {
                status: xhr.status,
                statusText: xhr.statusText,
                response: xhr.responseText
            });
            if (onComplete) onComplete(false, errorMsg);
            alert("Error saving configuration: " + errorMsg);
        }
    });
}

export function validNodeList(network_json) {
    let nodes = [];

    function iterate(data, level) {
        for (const [key, value] of Object.entries(data)) {
            nodes.push(key);
            if (value.children != null)
                iterate(value.children, level+1);
        }
    }

    iterate(network_json, 0);

    return nodes;
}

export function renderConfigMenu(currentPage) {
    const menuItems = [
        { href: "config_general.html", icon: "fa-server", text: "General", id: "general" },
        { href: "config_tuning.html", icon: "fa-warning", text: "Tuning", id: "tuning" },
        { href: "config_interface.html", icon: "fa-chain", text: "Network Mode", id: "interface" },
        { href: "config_queues.html", icon: "fa-car", text: "Queues", id: "queues" },
        { href: "config_stormguard.html", icon: "fa-bolt", text: "StormGuard", id: "stormguard" },
        { href: "config_lts.html", icon: "fa-line-chart", text: "LibreQoS Insight", id: "lts" },
        { href: "config_iprange.html", icon: "fa-address-card", text: "IP Ranges", id: "iprange" },
        { href: "config_flows.html", icon: "fa-arrow-circle-down", text: "Flow Tracking", id: "flows" },
        { href: "config_integration.html", icon: "fa-link", text: "Integration - Common", id: "integration" },
        { href: "config_spylnx.html", icon: "fa-link", text: "Splynx", id: "spylnx" },
        { href: "config_netzur.html", icon: "fa-link", text: "Netzur", id: "netzur" },
        { href: "config_uisp.html", icon: "fa-link", text: "UISP", id: "uisp" },
        { href: "config_powercode.html", icon: "fa-link", text: "Powercode", id: "powercode" },
        { href: "config_sonar.html", icon: "fa-link", text: "Sonar", id: "sonar" },
        { href: "config_wispgate.html", icon: "fa-link", text: "WispGate", id: "wispgate" },
        { href: "config_network.html", icon: "fa-map", text: "Network Layout", id: "network" },
        { href: "config_devices.html", icon: "fa-table", text: "Shaped Devices", id: "devices" },
        { href: "config_users.html", icon: "fa-users", text: "LibreQoS Users", id: "users" }
    ];

    const menuHtml = `
        <div class="row">
            <div class="col-12">
                <ul class="config-menu">
                ${menuItems.map(item => `
                    <li class="config-menu-item${item.id === currentPage ? ' active' : ''}">
                        <a href="${item.href}" class="text-decoration-none">
                            <i class="fa ${item.icon}"></i> ${item.text}
                        </a>
                    </li>
                `).join('')}
                </ul>
                <hr class="mt-3 mb-3" />
            </div>
        </div>
    `;

    // Find the container element and inject the menu
    const container = document.getElementById('configMenuContainer');
    if (container) {
        container.innerHTML = menuHtml;
    } else {
        // If no specific container, inject at the beginning of the body
        document.body.insertAdjacentHTML('afterbegin', menuHtml);
    }
}
