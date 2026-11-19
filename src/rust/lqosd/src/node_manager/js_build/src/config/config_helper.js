import { get_ws_client } from "../pubsub/ws";
import {t} from "../i18n/index.js";
import {
    TOPOLOGY_SOURCES,
    activeTopologySourceIntegrations,
    buildTopologyConfigUpdate,
    topologyEditorsLocked,
    topologyEditorsLockMessage,
    topologySource,
    topologySourceDefinition,
    topologySourceLabel,
} from "./topology_source";


const wsClient = get_ws_client();
const secretBindings = [];
const CONFIG_SAVE_TIMEOUT_MS = 10000;

export function sendWsRequest(responseEvent, request, onComplete, onError, options = {}) {
    let done = false;
    let timeoutId = null;
    const finish = (callback, msg) => {
        if (done) return;
        done = true;
        if (timeoutId !== null) {
            clearTimeout(timeoutId);
            timeoutId = null;
        }
        wsClient.off(responseEvent, responseHandler);
        // translatable: false — "Error" is the websocket event name, not a label
        wsClient.off("Error", errorHandler);
        if (callback) {
            callback(msg);
        }
    };
    const responseHandler = (msg) => {
        finish(onComplete, msg);
    };
    const errorHandler = (msg) => {
        finish(onError, msg);
    };
    wsClient.on(responseEvent, responseHandler);
    wsClient.on("Error", errorHandler);
    if (Number.isFinite(options.timeoutMs) && options.timeoutMs > 0) {
        timeoutId = setTimeout(() => {
            finish(onError, {
                message: options.timeoutMessage
                    || t("config.common.timeout.waiting", {event: responseEvent}),
            });
        }, options.timeoutMs);
    }
    wsClient.send(request);
}

function ensureOptionalConfigSections(config) {
    if (!config || typeof config !== "object") {
        return config;
    }

    if (!config.splynx_integration || typeof config.splynx_integration !== "object") {
        config.splynx_integration = {};
    }
    if (!config.integration_common || typeof config.integration_common !== "object") {
        config.integration_common = {};
    }
    if (!config.topology || typeof config.topology !== "object") {
        config.topology = {};
    }
    if (!config.ssl || typeof config.ssl !== "object") {
        config.ssl = {};
    }
    if (!config.local_api || typeof config.local_api !== "object") {
        config.local_api = {};
    }
    if (!config.uisp_integration || typeof config.uisp_integration !== "object") {
        config.uisp_integration = {};
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
    const ssl = config.ssl;
    if (typeof ssl.enabled !== "boolean") ssl.enabled = false;
    if (typeof ssl.managed_by_libreqos !== "boolean") ssl.managed_by_libreqos = false;
    if (typeof ssl.external_hostname !== "string" && ssl.external_hostname !== null) {
        ssl.external_hostname = null;
    }

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
    const rawTopologyMode = typeof topology.compile_mode === "string"
        ? topology.compile_mode.trim()
        : "";
    if (rawTopologyMode.length === 0) {
        topology.compile_mode = normalizeMode(config.uisp_integration?.strategy)
            || normalizeMode(splynx.strategy)
            || (config.uisp_integration?.enable_uisp ? "full" : "")
            || (splynx.enable_splynx ? "ap_site" : "")
            || "full";
    } else {
        // Keep an unsupported value visible so the selector UI can require an
        // explicit valid choice instead of silently resetting it.
        topology.compile_mode = normalizeMode(rawTopologyMode) || rawTopologyMode;
    }
    return config;
}

export {
    TOPOLOGY_SOURCES,
    activeTopologySourceIntegrations,
    buildTopologyConfigUpdate,
    topologyEditorsLocked,
    topologyEditorsLockMessage,
    topologySource,
    topologySourceDefinition,
    topologySourceLabel,
};

/**
 * Make the per-integration enable_* checkbox a read-only display of the
 * selector. Missing sections are left absent (the flag is false); this helper
 * never writes window.config.
 */
export function applyTopologySourceCheckboxState(config) {
    const source = topologySource(config);
    for (const definition of TOPOLOGY_SOURCES) {
        if (!definition.integration || !definition.domId) continue;
        const input = document.getElementById(definition.domId);
        if (!input) continue;
        input.disabled = true;
        input.checked = source === definition.value;
        const wrapper = input.closest(".form-check");
        if (!wrapper) continue;
        let note = wrapper.querySelector(".topology-source-note");
        if (!note) {
            note = document.createElement("div");
            note.className = "form-text topology-source-note";
            wrapper.appendChild(note);
        }
        note.textContent = source === definition.value
            ? t("config.common.topology_source.selected")
            : t("config.common.topology_source.not_active");
    }
}

export function loadConfig(onComplete, onError) {
    sendWsRequest(
        "GetConfig",
        { GetConfig: {} },
        (msg) => {
            const payload = msg && msg.data ? msg.data : {};
            window.config = ensureOptionalConfigSections(payload.config || {});
            window.configView = payload;
            window.configSecretState = payload.secret_state || {};
            window.configSecretClears = {};
            if (onComplete) onComplete(msg);
            setTimeout(() => applyTopologySourceCheckboxState(window.config), 0);
        },
        onError,
    );
}

export function saveConfig(onComplete, onError) {
    const clearSecrets = window.configSecretClears || {};
    const reportError = (msg) => {
        const errorMsg = (msg && msg.message) ? msg.message : t("config.common.request_failed");
        const detail = (msg && typeof msg === "object")
            ? { ...msg, message: errorMsg }
            : { message: errorMsg };
        if (onError) {
            onError(detail);
        } else {
            alert(t("config.common.error.saving", {detail: errorMsg}));
        }
    };
    sendWsRequest(
        "UpdateConfigResult",
        {
            UpdateConfig: {
                config: window.config,
                clear_secrets: clearSecrets,
            },
        },
        (msg) => {
            if (!msg || !msg.ok) {
                reportError(msg);
                return;
            }
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
            if (onComplete) onComplete(msg);
        },
        reportError,
        {
            timeoutMs: CONFIG_SAVE_TIMEOUT_MS,
            timeoutMessage: t("config.common.timeout.save"),
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
    configuredMessage = t("config.common.secret.stored"),
    emptyMessage = t("config.common.secret.empty"),
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
            status.textContent = t("config.common.secret.replacing");
        } else if (clearing) {
            status.textContent = t("config.common.secret.clearing");
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
        alert(t("config.common.validation.invalid_network"));
        return;
    }
    if (!Array.isArray(shaped_devices)) {
        alert(t("config.common.validation.invalid_devices"));
        return;
    }

    const validationErrors = [];
    const validNodes = validNodeList(network_json);
    shaped_devices.forEach((device, index) => {
        if (!device.circuit_id || device.circuit_id.trim() === "") {
            validationErrors.push(t("config.common.validation.device_circuit", {index: index + 1}));
        }
        if (!device.device_id || device.device_id.trim() === "") {
            validationErrors.push(t("config.common.validation.device_id", {index: index + 1}));
        }
        if (
            device.parent_node &&
            validNodes.length > 0 &&
            !validNodes.includes(device.parent_node)
        ) {
            validationErrors.push(
                t("config.common.validation.device_parent", {
                    index: index + 1,
                    node: device.parent_node,
                }),
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
                t("config.common.validation.device_bandwidth", {index: index + 1}),
            );
        } else {
            if (dmin < 0.1 || umin < 0.1) {
                validationErrors.push(t("config.common.validation.device_min_rates", {index: index + 1}));
            }
            if (dmax < 0.2 || umax < 0.2) {
                validationErrors.push(t("config.common.validation.device_max_rates", {index: index + 1}));
            }
        }
    });

    if (validationErrors.length > 0) {
        alert(`${t("config.common.validation.header")}\n${validationErrors.join("\n")}`);
        return;
    }

    sendWsRequest(
        "UpdateNetworkAndDevicesResult",
        { UpdateNetworkAndDevices: { network_json, shaped_devices } },
        (msg) => {
            if (onComplete) onComplete(!!msg.ok, msg.message);
        },
        (msg) => {
            const errorMsg = msg && msg.message ? msg.message : t("config.common.request_failed");
            if (onComplete) onComplete(false, errorMsg);
            if (onError) {
                onError(msg);
            } else {
                alert(t("config.common.error.saving", {detail: errorMsg}));
            }
        },
    );
}

export function saveNetworkJsonOnly(network_json, onComplete, onError) {
    if (!network_json || typeof network_json !== "object") {
        alert(t("config.common.validation.invalid_network"));
        return;
    }

    sendWsRequest(
        "UpdateNetworkJsonOnlyResult",
        { UpdateNetworkJsonOnly: { network_json } },
        (msg) => {
            if (onComplete) onComplete(!!msg.ok, msg.message);
        },
        (msg) => {
            const errorMsg = (msg && msg.message) ? msg.message : t("config.common.request_failed");
            if (onComplete) onComplete(false, errorMsg);
            if (onError) {
                onError(msg);
            } else {
                alert(t("config.common.error.saving_network", {detail: errorMsg}));
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
    // Every config page renders this menu, so its labels live in the shared
    // `config.common` namespace instead of one page's. Product names stay
    // literal: they read the same in both locales.
    const menuItems = [
        { href: "config_general.html", icon: "fa-server", text: t("config.common.menu.general"), id: "general" },
        { href: "config_ssl.html", icon: "fa-lock", text: t("config.common.menu.ssl"), id: "ssl" },
        { href: "config_rtt.html", icon: "fa-stopwatch", text: t("config.common.menu.rtt"), id: "rtt" },
        { href: "config_tuning.html", icon: "fa-warning", text: t("config.common.menu.tuning"), id: "tuning" },
        { href: "config_interface.html", icon: "fa-chain", text: t("config.common.menu.interface"), id: "interface" },
        { href: "config_queues.html", icon: "fa-car", text: t("config.common.menu.queues"), id: "queues" },
        { href: "config_dataplane.html", icon: "fa-tachometer", text: t("config.common.menu.dataplane"), id: "dataplane" },
        { href: "config_stormguard.html", icon: "fa-bolt", text: "StormGuard", id: "stormguard" },
        { href: "config_lts.html", icon: "fa-line-chart", text: t("config.common.menu.lts"), id: "lts" },
        { href: "config_iprange.html", icon: "fa-address-card", text: t("config.common.menu.iprange"), id: "iprange" },
        { href: "config_flows.html", icon: "fa-arrow-circle-down", text: t("config.common.menu.flows"), id: "flows" },
        { href: "config_dynamic_circuits.html", icon: "fa-layer-group", text: t("config.common.menu.dynamic_circuits"), id: "dynamic_circuits" },
        { href: "config_radius_accounting.html", icon: "fa-satellite-dish", text: t("config.common.menu.radius_accounting"), id: "radius_accounting" },
        { href: "config_integration.html", icon: "fa-link", text: t("config.common.menu.integration"), id: "integration" },
        { href: "config_splynx.html", icon: "fa-link", text: "Splynx", id: "splynx" },
        { href: "config_netzur.html", icon: "fa-link", text: "Netzur", id: "netzur" },
        { href: "config_visp.html", icon: "fa-link", text: "VISP", id: "visp" },
        { href: "config_uisp.html", icon: "fa-link", text: "UISP", id: "uisp" },
        { href: "config_powercode.html", icon: "fa-link", text: "Powercode", id: "powercode" },
        { href: "config_sonar.html", icon: "fa-link", text: "Sonar", id: "sonar" },
        { href: "config_wispgate.html", icon: "fa-link", text: "WispGate", id: "wispgate" },
        { href: "config_network.html", icon: "fa-map", text: t("config.common.menu.network"), id: "network" },
        { href: "config_devices.html", icon: "fa-table", text: t("config.common.menu.devices"), id: "devices" },
        { href: "config_users.html", icon: "fa-users", text: t("config.common.menu.users"), id: "users" }
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
