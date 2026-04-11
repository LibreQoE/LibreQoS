import { get_ws_client } from "../pubsub/ws";

const wsClient = get_ws_client();
const secretBindings = [];

function sendWsRequest(responseEvent, request, onComplete, onError) {
    let done = false;
    const responseHandler = (msg) => {
        if (done) return;
        done = true;
        wsClient.off(responseEvent, responseHandler);
        wsClient.off("Error", errorHandler);
        onComplete(msg);
    };
    const errorHandler = (msg) => {
        if (done) return;
        done = true;
        wsClient.off(responseEvent, responseHandler);
        wsClient.off("Error", errorHandler);
        if (onError) {
            onError(msg);
        }
    };
    wsClient.on(responseEvent, responseHandler);
    wsClient.on("Error", errorHandler);
    wsClient.send(request);
}

function ensureOptionalConfigSections(config) {
    if (!config || typeof config !== "object") {
        return config;
    }

    if (!config.splynx_integration || typeof config.splynx_integration !== "object") {
        config.splynx_integration = {};
    }
    if (!config.topology || typeof config.topology !== "object") {
        config.topology = {};
    }

    if (!config.sonar_integration || typeof config.sonar_integration !== "object") {
        config.sonar_integration = {};
    }

    const sonar = config.sonar_integration;
    if (typeof sonar.enable_sonar !== "boolean") sonar.enable_sonar = false;
    if (typeof sonar.sonar_api_url !== "string") sonar.sonar_api_url = "";
    if (typeof sonar.sonar_api_key !== "string") sonar.sonar_api_key = "";
    if (typeof sonar.snmp_community !== "string") sonar.snmp_community = "public";
    if (!Array.isArray(sonar.airmax_model_ids)) sonar.airmax_model_ids = [];
    if (!Array.isArray(sonar.ltu_model_ids)) sonar.ltu_model_ids = [];
    if (!Array.isArray(sonar.active_status_ids)) sonar.active_status_ids = [];
    if (!Array.isArray(sonar.recurring_excluded_service_names)) {
        sonar.recurring_excluded_service_names = [];
    }
    if (!Array.isArray(sonar.recurring_service_rates)) {
        sonar.recurring_service_rates = [];
    }

    const splynx = config.splynx_integration;
    if (typeof splynx.enable_splynx !== "boolean") splynx.enable_splynx = false;
    if (typeof splynx.api_key !== "string") splynx.api_key = "";
    if (typeof splynx.api_secret !== "string") splynx.api_secret = "";
    if (typeof splynx.url !== "string") splynx.url = "";
    if (typeof splynx.strategy !== "string" || splynx.strategy.length === 0) {
        splynx.strategy = "ap_site";
    }

    const topology = config.topology;
    const normalizeMode = (mode) => {
        if (typeof mode !== "string") {
            return "";
        }
        const lowered = mode.trim().toLowerCase();
        if (lowered === "full2") {
            return "full";
        }
        return ["flat", "ap_only", "ap_site", "full"].includes(lowered) ? lowered : "";
    };
    if (typeof topology.compile_mode !== "string" || topology.compile_mode.length === 0) {
        topology.compile_mode = normalizeMode(topology.compile_mode)
            || normalizeMode(config.uisp_integration?.strategy)
            || normalizeMode(splynx.strategy)
            || (config.uisp_integration?.enable_uisp ? "full" : "")
            || (splynx.enable_splynx ? "ap_site" : "")
            || "ap_site";
    } else {
        topology.compile_mode = normalizeMode(topology.compile_mode) || "ap_site";
    }

    return config;
}

const TOPOLOGY_SOURCE_INTEGRATIONS = [
    { name: "UISP", enabled: (config) => !!config?.uisp_integration?.enable_uisp },
    { name: "Splynx", enabled: (config) => !!config?.splynx_integration?.enable_splynx },
    { name: "Powercode", enabled: (config) => !!config?.powercode_integration?.enable_powercode },
    { name: "Sonar", enabled: (config) => !!config?.sonar_integration?.enable_sonar },
    { name: "Netzur", enabled: (config) => !!config?.netzur_integration?.enable_netzur },
    { name: "VISP", enabled: (config) => !!config?.visp_integration?.enable_visp },
    { name: "WispGate", enabled: (config) => !!config?.wispgate_integration?.enable_wispgate },
];

export function activeTopologySourceIntegrations(config) {
    return TOPOLOGY_SOURCE_INTEGRATIONS.filter((entry) => entry.enabled(config)).map(
        (entry) => entry.name,
    );
}

export function topologyEditorsLocked(config) {
    return activeTopologySourceIntegrations(config).length > 0;
}

export function topologyEditorsLockMessage(config) {
    const active = activeTopologySourceIntegrations(config);
    if (active.length === 0) {
        return "";
    }
    return `Editing is disabled because these integrations are the source of truth: ${active.join(", ")}.`;
}

export function loadConfig(onComplete, onError) {
    sendWsRequest(
        "GetConfig",
        { GetConfig: {} },
        (msg) => {
            const payload = msg && msg.data ? msg.data : {};
            window.config = ensureOptionalConfigSections(payload.config || {});
            window.configSecretState = payload.secret_state || {};
            window.configSecretClears = {};
            if (onComplete) onComplete(msg);
        },
        onError,
    );
}

export function saveConfig(onComplete, onError) {
    const clearSecrets = window.configSecretClears || {};
    sendWsRequest(
        "UpdateConfigResult",
        {
            UpdateConfig: {
                config: window.config,
                clear_secrets: clearSecrets,
            },
        },
        (msg) => {
            if (msg && msg.ok) {
                secretBindings.forEach((binding) => {
                    const sectionState = secretFieldMap(window.configSecretState || {}, binding.section);
                    const inputValue = binding.input.value.trim();
                    sectionState[binding.field] = clearSecrets?.[binding.section]?.[binding.field]
                        ? false
                        : (inputValue.length > 0 || sectionState[binding.field]);
                    if (window.config?.[binding.section] && typeof window.config[binding.section] === "object") {
                        window.config[binding.section][binding.field] = "";
                    }
                    binding.input.value = "";
                    setSecretClear(binding.section, binding.field, false);
                    binding.updateStatus();
                });
            }
            if (onComplete) onComplete(msg);
        },
        (msg) => {
            if (onError) {
                onError(msg);
            } else {
                alert("That didn't work");
            }
        },
    );
}

function secretFieldMap(container, section) {
    if (!container[section] || typeof container[section] !== "object") {
        container[section] = {};
    }
    return container[section];
}

export function secretConfigured(section, field) {
    return !!window.configSecretState?.[section]?.[field];
}

export function secretClearMarked(section, field) {
    return !!window.configSecretClears?.[section]?.[field];
}

export function setSecretClear(section, field, clear) {
    if (!window.configSecretClears || typeof window.configSecretClears !== "object") {
        window.configSecretClears = {};
    }
    const sectionMap = secretFieldMap(window.configSecretClears, section);
    if (clear) {
        sectionMap[field] = true;
    } else {
        delete sectionMap[field];
        if (Object.keys(sectionMap).length === 0) {
            delete window.configSecretClears[section];
        }
    }
}

export function secretWillExistAfterSave(section, field, inputId) {
    const input = document.getElementById(inputId);
    const hasReplacement = !!(input && input.value.trim().length > 0);
    if (hasReplacement) {
        return true;
    }
    if (secretClearMarked(section, field)) {
        return false;
    }
    return secretConfigured(section, field);
}

export function bindSecretField({
    section,
    field,
    inputId,
    statusId,
    clearButtonId,
    configuredMessage = "Stored on server. Leave blank to keep the current value.",
    emptyMessage = "No value stored.",
}) {
    const input = document.getElementById(inputId);
    const status = document.getElementById(statusId);
    const clearButton = clearButtonId ? document.getElementById(clearButtonId) : null;
    if (!input || !status) {
        return;
    }

    const updateStatus = () => {
        const hasReplacement = input.value.trim().length > 0;
        const configured = secretConfigured(section, field);
        const clearing = secretClearMarked(section, field);

        if (hasReplacement) {
            status.textContent = "A new value will replace the stored secret when you save.";
        } else if (clearing) {
            status.textContent = "The stored secret will be cleared when you save.";
        } else if (configured) {
            status.textContent = configuredMessage;
        } else {
            status.textContent = emptyMessage;
        }

        if (clearButton) {
            clearButton.disabled = !configured && !hasReplacement && !clearing;
        }
    };

    input.addEventListener("input", () => {
        if (input.value.trim().length > 0) {
            setSecretClear(section, field, false);
        }
        updateStatus();
    });

    if (clearButton) {
        clearButton.addEventListener("click", () => {
            input.value = "";
            setSecretClear(section, field, secretConfigured(section, field));
            updateStatus();
        });
    }

    secretBindings.push({
        section,
        field,
        input,
        updateStatus,
    });
    updateStatus();
}

export function saveNetworkAndDevices(network_json, shaped_devices, onComplete, onError) {
    if (!network_json || typeof network_json !== "object") {
        alert("Invalid network configuration");
        return;
    }
    if (!Array.isArray(shaped_devices)) {
        alert("Invalid shaped devices configuration");
        return;
    }

    const validationErrors = [];
    const validNodes = validNodeList(network_json);
    shaped_devices.forEach((device, index) => {
        if (!device.circuit_id || device.circuit_id.trim() === "") {
            validationErrors.push(`Device ${index + 1}: Circuit ID is required`);
        }
        if (!device.device_id || device.device_id.trim() === "") {
            validationErrors.push(`Device ${index + 1}: Device ID is required`);
        }
        if (
            device.parent_node &&
            validNodes.length > 0 &&
            !validNodes.includes(device.parent_node)
        ) {
            validationErrors.push(
                `Device ${index + 1}: Parent node '${device.parent_node}' does not exist`,
            );
        }

        const dmin = parseFloat(device.download_min_mbps);
        const umin = parseFloat(device.upload_min_mbps);
        const dmax = parseFloat(device.download_max_mbps);
        const umax = parseFloat(device.upload_max_mbps);
        if (
            Number.isNaN(dmin) ||
            Number.isNaN(umin) ||
            Number.isNaN(dmax) ||
            Number.isNaN(umax)
        ) {
            validationErrors.push(
                `Device ${index + 1}: One or more bandwidth fields are not valid numbers`,
            );
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

    sendWsRequest(
        "UpdateNetworkAndDevicesResult",
        { UpdateNetworkAndDevices: { network_json, shaped_devices } },
        (msg) => {
            if (onComplete) onComplete(!!msg.ok, msg.message);
        },
        (msg) => {
            const errorMsg = msg && msg.message ? msg.message : "Request failed";
            if (onComplete) onComplete(false, errorMsg);
            if (onError) {
                onError(msg);
            } else {
                alert("Error saving configuration: " + errorMsg);
            }
        },
    );
}

export function saveNetworkJsonOnly(network_json, onComplete, onError) {
    if (!network_json || typeof network_json !== "object") {
        alert("Invalid network configuration");
        return;
    }

    sendWsRequest(
        "UpdateNetworkJsonOnlyResult",
        { UpdateNetworkJsonOnly: { network_json } },
        (msg) => {
            if (onComplete) onComplete(!!msg.ok, msg.message);
        },
        (msg) => {
            const errorMsg = (msg && msg.message) ? msg.message : "Request failed";
            if (onComplete) onComplete(false, errorMsg);
            if (onError) {
                onError(msg);
            } else {
                alert("Error saving network configuration: " + errorMsg);
            }
        },
    );
}

export function adminCheck(onComplete, onError) {
    sendWsRequest(
        "AdminCheck",
        { AdminCheck: {} },
        (msg) => {
            if (onComplete) onComplete(!!msg.ok);
        },
        onError,
    );
}

export function listNics(onComplete, onError) {
    sendWsRequest(
        "ListNics",
        { ListNics: {} },
        (msg) => {
            if (onComplete) onComplete(msg.data || []);
        },
        onError,
    );
}

export function loadQooProfiles(onComplete, onError) {
    sendWsRequest(
        "QooProfiles",
        { QooProfiles: {} },
        (msg) => {
            if (onComplete) onComplete(msg.data || null);
        },
        onError,
    );
}

export function loadNetworkJson(onComplete, onError) {
    sendWsRequest(
        "NetworkJson",
        { NetworkJson: {} },
        (msg) => {
            if (onComplete) onComplete(msg.data);
        },
        onError,
    );
}

export function loadAllShapedDevices(onComplete, onError) {
    const pageSize = 250;
    const rows = [];

    const loadPage = (page) => {
        sendWsRequest(
            "ShapedDevicesPage",
            { ShapedDevicesPage: { query: { page, page_size: pageSize } } },
            (msg) => {
                const data = msg && msg.data ? msg.data : {};
                const pageRows = Array.isArray(data.rows) ? data.rows : [];
                rows.push(...pageRows);
                const totalRows = Number.isFinite(Number(data.total_rows))
                    ? Math.max(0, Math.trunc(Number(data.total_rows)))
                    : rows.length;
                if (rows.length >= totalRows || pageRows.length < pageSize) {
                    if (onComplete) onComplete(rows);
                    return;
                }
                loadPage(page + 1);
            },
            onError,
        );
    };

    loadPage(0);
}

export function loadShapedDevicesPage(query, onComplete, onError) {
    sendWsRequest(
        "ShapedDevicesPage",
        { ShapedDevicesPage: { query } },
        (msg) => {
            if (onComplete) onComplete(msg.data || null);
        },
        onError,
    );
}

export function getShapedDevice(deviceId, onComplete, onError) {
    sendWsRequest(
        "GetShapedDeviceResult",
        { GetShapedDevice: { device_id: deviceId } },
        (msg) => {
            if (onComplete) onComplete(msg);
        },
        onError,
    );
}

export function createShapedDevice(device, onComplete, onError) {
    sendWsRequest(
        "CreateShapedDeviceResult",
        { CreateShapedDevice: { device } },
        (msg) => {
            if (onComplete) onComplete(msg);
        },
        onError,
    );
}

export function updateShapedDevice(originalDeviceId, device, onComplete, onError) {
    sendWsRequest(
        "UpdateShapedDeviceResult",
        { UpdateShapedDevice: { original_device_id: originalDeviceId, device } },
        (msg) => {
            if (onComplete) onComplete(msg);
        },
        onError,
    );
}

export function deleteShapedDevice(deviceId, onComplete, onError) {
    sendWsRequest(
        "DeleteShapedDeviceResult",
        { DeleteShapedDevice: { device_id: deviceId } },
        (msg) => {
            if (onComplete) onComplete(msg);
        },
        onError,
    );
}

export function loadCircuitDirectoryPage(query, onComplete, onError) {
    sendWsRequest(
        "CircuitDirectoryPage",
        { CircuitDirectoryPage: { query } },
        (msg) => {
            if (onComplete) onComplete(msg.data || null);
        },
        onError,
    );
}

export function loadAllCircuitDirectoryRows(onComplete, onError) {
    const pageSize = 250;
    const rows = [];

    const loadPage = (page) => {
        loadCircuitDirectoryPage(
            { page, page_size: pageSize },
            (data) => {
                const pageRows = Array.isArray(data?.rows) ? data.rows : [];
                rows.push(...pageRows);
                const totalRows = Number.isFinite(Number(data?.total_rows))
                    ? Math.max(0, Math.trunc(Number(data.total_rows)))
                    : rows.length;
                if (rows.length >= totalRows || pageRows.length < pageSize) {
                    if (onComplete) onComplete(rows);
                    return;
                }
                loadPage(page + 1);
            },
            onError,
        );
    };

    loadPage(0);
}

export function loadNodeDirectory(onComplete, onError) {
    sendWsRequest(
        "NodeDirectory",
        { NodeDirectory: {} },
        (msg) => {
            if (onComplete) onComplete(msg.data || []);
        },
        onError,
    );
}

export function loadTreeGuardMetadataSummary(onComplete, onError) {
    sendWsRequest(
        "TreeGuardMetadataSummary",
        { TreeGuardMetadataSummary: {} },
        (msg) => {
            if (onComplete) onComplete(msg.data || null);
        },
        onError,
    );
}

export function getUsers(onComplete, onError) {
    sendWsRequest(
        "GetUsers",
        { GetUsers: {} },
        (msg) => {
            if (onComplete) onComplete(msg.data || []);
        },
        onError,
    );
}

export function addUser(payload, onComplete, onError) {
    sendWsRequest(
        "AddUserResult",
        { AddUser: payload },
        (msg) => {
            if (onComplete) onComplete(msg);
        },
        onError,
    );
}

export function updateUser(payload, onComplete, onError) {
    sendWsRequest(
        "UpdateUserResult",
        { UpdateUser: payload },
        (msg) => {
            if (onComplete) onComplete(msg);
        },
        onError,
    );
}

export function deleteUser(payload, onComplete, onError) {
    sendWsRequest(
        "DeleteUserResult",
        { DeleteUser: payload },
        (msg) => {
            if (onComplete) onComplete(msg);
        },
        onError,
    );
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
        { href: "config_rtt.html", icon: "fa-stopwatch", text: "RTT Thresholds", id: "rtt" },
        { href: "config_tuning.html", icon: "fa-warning", text: "Tuning", id: "tuning" },
        { href: "config_interface.html", icon: "fa-chain", text: "Network Mode", id: "interface" },
        { href: "config_queues.html", icon: "fa-car", text: "Queues", id: "queues" },
        { href: "config_stormguard.html", icon: "fa-bolt", text: "StormGuard", id: "stormguard" },
        { href: "config_treeguard.html", icon: "fa-shield-halved", text: "TreeGuard", id: "treeguard" },
        { href: "config_lts.html", icon: "fa-line-chart", text: "LibreQoS Insight", id: "lts" },
        { href: "config_iprange.html", icon: "fa-address-card", text: "IP Ranges", id: "iprange" },
        { href: "config_flows.html", icon: "fa-arrow-circle-down", text: "Flow Tracking", id: "flows" },
        { href: "config_integration.html", icon: "fa-link", text: "Integration - Common", id: "integration" },
        { href: "config_splynx.html", icon: "fa-link", text: "Splynx", id: "splynx" },
        { href: "config_netzur.html", icon: "fa-link", text: "Netzur", id: "netzur" },
        { href: "config_visp.html", icon: "fa-link", text: "VISP", id: "visp" },
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
                    <li class="config-menu-item">
                        <a href="${item.href}" class="config-menu-link text-decoration-none${item.id === currentPage ? ' active' : ''}">
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
