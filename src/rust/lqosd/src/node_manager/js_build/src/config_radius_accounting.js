import { loadConfig, renderConfigMenu, saveConfig } from "./config/config_helper";
import { t } from "./i18n/index.js";

const DEFAULT_TTL_SECONDS = 900;
const DEFAULT_STALE_GRACE_SECONDS = 120;

function parsePositiveInt(value) {
    const num = parseInt(String(value ?? "").trim(), 10);
    if (!Number.isFinite(num) || num <= 0) return null;
    return num;
}

function parsePositiveFloat(value) {
    const num = parseFloat(String(value ?? "").trim());
    if (!Number.isFinite(num) || num <= 0) return null;
    return num;
}

function optionalText(value) {
    const trimmed = String(value ?? "").trim();
    return trimmed.length > 0 ? trimmed : null;
}

function ensureRadiusConfig(config) {
    if (!config || typeof config !== "object") return;

    if (!config.radius_accounting || typeof config.radius_accounting !== "object") {
        config.radius_accounting = {};
    }

    const radius = config.radius_accounting;
    if (typeof radius.enabled !== "boolean") radius.enabled = false;
    if (!Number.isFinite(Number(radius.default_ttl_seconds))) {
        radius.default_ttl_seconds = DEFAULT_TTL_SECONDS;
    }
    if (!Number.isFinite(Number(radius.stale_grace_seconds))) {
        radius.stale_grace_seconds = DEFAULT_STALE_GRACE_SECONDS;
    }
    if (!radius.dynamic_circuit_application || typeof radius.dynamic_circuit_application !== "object") {
        radius.dynamic_circuit_application = {};
    }
    const application = radius.dynamic_circuit_application;
    if (typeof application.enabled !== "boolean") application.enabled = false;
    if (typeof application.match_shaped_devices_by_mac !== "boolean") {
        application.match_shaped_devices_by_mac = false;
    }
    if (typeof application.match_shaped_devices_by_username !== "boolean") {
        application.match_shaped_devices_by_username = false;
    }
    if (!Array.isArray(radius.clients)) radius.clients = [];
}

function sourceListToText(source) {
    if (Array.isArray(source)) {
        return source.map((entry) => String(entry ?? "").trim()).filter(Boolean).join("\n");
    }
    return String(source ?? "").trim();
}

function textToSourceList(value) {
    return String(value ?? "")
        .split(/\r?\n|,/)
        .map((entry) => entry.trim())
        .filter(Boolean);
}

function isValidIPv4(ip) {
    if (!/^(\d{1,3}\.){3}\d{1,3}$/.test(ip)) return false;
    const parts = ip.split(".").map((part) => parseInt(part, 10));
    return parts.length === 4 && !parts.some((part) => Number.isNaN(part) || part < 0 || part > 255);
}

function isValidIPv6(ip) {
    return ip.includes(":") && /^[0-9a-fA-F:.]+$/.test(ip);
}

function isValidIpAddress(ip) {
    return isValidIPv4(ip) || isValidIPv6(ip);
}

function isValidIpOrCidr(value) {
    const text = String(value ?? "").trim();
    const [ip, mask, extra] = text.split("/");
    if (!ip || extra !== undefined) return false;
    if (!isValidIpAddress(ip)) return false;
    if (mask === undefined) return true;

    const maskNum = parseInt(mask, 10);
    if (!Number.isInteger(maskNum)) return false;
    return ip.includes(":") ? maskNum >= 0 && maskNum <= 128 : maskNum >= 0 && maskNum <= 32;
}

function getDefaultClient() {
    return {
        name: "",
        source: [],
        secret_file: "",
    };
}

function announceClientChange(message) {
    const status = document.getElementById("radiusClientsStatus");
    if (status) status.textContent = message;
}

function renderClientsTable(focusClientIndex = null) {
    const tbody = document.getElementById("radiusClientsBody");
    if (!tbody) return;
    tbody.innerHTML = "";

    const clients = window.config?.radius_accounting?.clients;
    if (!Array.isArray(clients) || clients.length === 0) {
        const empty = document.createElement("tr");
        const cell = document.createElement("td");
        cell.colSpan = 4;
        cell.className = "text-muted";
        cell.textContent = t("config.radius_accounting.clients.empty");
        empty.appendChild(cell);
        tbody.appendChild(empty);
        return;
    }

    clients.forEach((client, index) => {
        const tr = document.createElement("tr");

        const addCell = (label, child) => {
            const td = document.createElement("td");
            td.dataset.label = label;
            td.appendChild(child);
            tr.appendChild(td);
        };

        const name = document.createElement("input");
        name.type = "text";
        name.className = "form-control form-control-sm";
        name.placeholder = t("config.radius_accounting.clients.name_placeholder");
        name.value = String(client?.name ?? "");
        name.setAttribute("aria-label", t("config.radius_accounting.clients.name_aria", {index: index + 1}));
        name.addEventListener("input", (ev) => {
            window.config.radius_accounting.clients[index].name = ev.target.value;
        });
        addCell(t("config.radius_accounting.clients.name"), name);

        const source = document.createElement("textarea");
        source.className = "form-control form-control-sm";
        source.rows = 2;
        source.placeholder = "192.0.2.10\n192.0.2.0/24";
        source.value = sourceListToText(client?.source);
        source.setAttribute("aria-label", t("config.radius_accounting.clients.source_aria", {index: index + 1}));
        source.addEventListener("input", (ev) => {
            window.config.radius_accounting.clients[index].source = textToSourceList(ev.target.value);
        });
        addCell(t("config.radius_accounting.clients.source"), source);

        const secretFile = document.createElement("input");
        secretFile.type = "text";
        secretFile.className = "form-control form-control-sm";
        secretFile.placeholder = "/etc/libreqos/radius/client.secret";
        secretFile.value = String(client?.secret_file ?? "");
        secretFile.setAttribute("aria-label", t("config.radius_accounting.clients.secret_file_aria", {index: index + 1}));
        secretFile.addEventListener("input", (ev) => {
            window.config.radius_accounting.clients[index].secret_file = ev.target.value;
        });
        addCell(t("config.radius_accounting.clients.secret_file"), secretFile);

        const removeTd = document.createElement("td");
        removeTd.dataset.label = t("common.table.actions");
        const removeBtn = document.createElement("button");
        removeBtn.type = "button";
        removeBtn.className = "btn btn-sm btn-outline-danger";
        removeBtn.textContent = t("config.radius_accounting.clients.remove");
        removeBtn.setAttribute("aria-label", t("config.radius_accounting.clients.remove_aria", {index: index + 1}));
        removeBtn.addEventListener("click", () => {
            window.config.radius_accounting.clients.splice(index, 1);
            renderClientsTable(Math.min(index, window.config.radius_accounting.clients.length - 1));
            announceClientChange(t("config.radius_accounting.clients.removed", {index: index + 1}));
        });
        removeTd.appendChild(removeBtn);
        tr.appendChild(removeTd);

        tbody.appendChild(tr);

        if (index === focusClientIndex) {
            name.focus();
        }
    });
}

function setFallbackSpeedInputsEnabled(enabled) {
    [
        "radiusDownloadMinMbps",
        "radiusUploadMinMbps",
        "radiusDownloadMaxMbps",
        "radiusUploadMaxMbps",
    ].forEach((id) => {
        const input = document.getElementById(id);
        if (input) input.disabled = !enabled;
    });
}

function updateConfigFromUi() {
    const radius = window.config.radius_accounting;
    radius.enabled = !!document.getElementById("radiusEnabled")?.checked;
    radius.listen = optionalText(document.getElementById("radiusListen")?.value);
    radius.default_ttl_seconds = parsePositiveInt(
        document.getElementById("radiusTtlSeconds")?.value,
    ) ?? DEFAULT_TTL_SECONDS;
    radius.stale_grace_seconds = parsePositiveInt(
        document.getElementById("radiusStaleGraceSeconds")?.value,
    ) ?? DEFAULT_STALE_GRACE_SECONDS;

    const application = radius.dynamic_circuit_application;
    application.enabled = !!document.getElementById("radiusDynamicApplicationEnabled")?.checked;
    application.match_shaped_devices_by_mac = !!document.getElementById("radiusMatchShapedDevicesByMac")?.checked;
    application.match_shaped_devices_by_username = !!document.getElementById("radiusMatchShapedDevicesByUsername")?.checked;
    application.fallback_parent_node = optionalText(document.getElementById("radiusFallbackParentNode")?.value);
    application.fallback_parent_node_id = optionalText(document.getElementById("radiusFallbackParentNodeId")?.value);
    application.fallback_anchor_node_id = optionalText(document.getElementById("radiusFallbackAnchorNodeId")?.value);

    if (document.getElementById("radiusFallbackSpeedEnabled")?.checked) {
        radius.fallback_speed_profile = {
            download_min_mbps: parsePositiveFloat(document.getElementById("radiusDownloadMinMbps")?.value),
            upload_min_mbps: parsePositiveFloat(document.getElementById("radiusUploadMinMbps")?.value),
            download_max_mbps: parsePositiveFloat(document.getElementById("radiusDownloadMaxMbps")?.value),
            upload_max_mbps: parsePositiveFloat(document.getElementById("radiusUploadMaxMbps")?.value),
        };
    } else {
        radius.fallback_speed_profile = null;
    }

    if (Array.isArray(radius.clients)) {
        radius.clients = radius.clients.map((client) => ({
            name: String(client?.name ?? "").trim(),
            source: textToSourceList(sourceListToText(client?.source)),
            secret_file: String(client?.secret_file ?? "").trim(),
        }));
    }
}

function validateListenAddress(value) {
    const text = String(value ?? "").trim();
    if (!text) return false;

    const ipv6Match = text.match(/^\[[^\]]+\]:(\d+)$/);
    if (ipv6Match) {
        const ip = text.slice(1, text.indexOf("]"));
        if (!isValidIPv6(ip)) return false;
        const port = parseInt(ipv6Match[1], 10);
        return port > 0 && port <= 65535;
    }

    const portStart = text.lastIndexOf(":");
    if (portStart <= 0) return false;
    const ip = text.slice(0, portStart);
    if (!isValidIPv4(ip)) return false;
    const port = parseInt(text.slice(portStart + 1), 10);
    return Number.isInteger(port) && port > 0 && port <= 65535;
}

function validateConfig() {
    const errors = [];
    const radius = window.config?.radius_accounting;

    if (!radius || typeof radius !== "object") {
        errors.push(t("config.radius_accounting.validation.section_missing"));
    } else {
        if (radius.enabled && !validateListenAddress(radius.listen)) {
            errors.push(t("config.radius_accounting.validation.listen_required"));
        }

        if (parsePositiveInt(radius.default_ttl_seconds) === null) {
            errors.push(t("config.radius_accounting.validation.ttl"));
        }
        if (parsePositiveInt(radius.stale_grace_seconds) === null) {
            errors.push(t("config.radius_accounting.validation.stale_grace"));
        }

        const application = radius.dynamic_circuit_application || {};
        if (!application.fallback_parent_node
            && (application.fallback_parent_node_id || application.fallback_anchor_node_id)) {
            errors.push(t("config.radius_accounting.validation.fallback_parent"));
        }

        if (radius.fallback_speed_profile) {
            const profile = radius.fallback_speed_profile;
            const dmin = parsePositiveFloat(profile.download_min_mbps);
            const umin = parsePositiveFloat(profile.upload_min_mbps);
            const dmax = parsePositiveFloat(profile.download_max_mbps);
            const umax = parsePositiveFloat(profile.upload_max_mbps);

            if (dmin === null || umin === null || dmax === null || umax === null) {
                errors.push(t("config.radius_accounting.validation.speed_positive"));
            } else {
                if (dmin > dmax) errors.push(t("config.radius_accounting.validation.speed_download_order"));
                if (umin > umax) errors.push(t("config.radius_accounting.validation.speed_upload_order"));
            }
        }

        const clients = Array.isArray(radius.clients) ? radius.clients : [];
        if (radius.enabled && clients.length === 0) {
            errors.push(t("config.radius_accounting.validation.client_required"));
        }

        clients.forEach((client, index) => {
            const label = client?.name
                ? t("config.radius_accounting.validation.client_named", {name: client.name})
                : t("config.radius_accounting.validation.client_numbered", {index: index + 1});
            const sources = Array.isArray(client?.source) ? client.source : [];
            if (sources.length === 0) {
                errors.push(t("config.radius_accounting.validation.source_required", {label}));
            }
            sources.forEach((source) => {
                if (!isValidIpOrCidr(source)) {
                    errors.push(t("config.radius_accounting.validation.source_invalid", {label, source}));
                }
            });
            if (!String(client?.secret_file ?? "").trim()) {
                errors.push(t("config.radius_accounting.validation.secret_required", {label}));
            }
        });
    }

    if (errors.length === 0) return true;

    alert(`${t("config.radius_accounting.validation.header")}\n${errors.join("\n")}`);
    return false;
}

function loadUiFromConfig() {
    const radius = window.config.radius_accounting;
    const application = radius.dynamic_circuit_application;
    const profile = radius.fallback_speed_profile;

    document.getElementById("radiusEnabled").checked = !!radius.enabled;
    document.getElementById("radiusListen").value = radius.listen ?? "";
    document.getElementById("radiusTtlSeconds").value = radius.default_ttl_seconds ?? DEFAULT_TTL_SECONDS;
    document.getElementById("radiusStaleGraceSeconds").value = radius.stale_grace_seconds ?? DEFAULT_STALE_GRACE_SECONDS;
    document.getElementById("radiusDynamicApplicationEnabled").checked = !!application.enabled;
    document.getElementById("radiusMatchShapedDevicesByMac").checked = !!application.match_shaped_devices_by_mac;
    document.getElementById("radiusMatchShapedDevicesByUsername").checked = !!application.match_shaped_devices_by_username;
    document.getElementById("radiusFallbackParentNode").value = application.fallback_parent_node ?? "";
    document.getElementById("radiusFallbackParentNodeId").value = application.fallback_parent_node_id ?? "";
    document.getElementById("radiusFallbackAnchorNodeId").value = application.fallback_anchor_node_id ?? "";

    document.getElementById("radiusFallbackSpeedEnabled").checked = !!profile;
    document.getElementById("radiusDownloadMinMbps").value = profile?.download_min_mbps ?? "";
    document.getElementById("radiusUploadMinMbps").value = profile?.upload_min_mbps ?? "";
    document.getElementById("radiusDownloadMaxMbps").value = profile?.download_max_mbps ?? "";
    document.getElementById("radiusUploadMaxMbps").value = profile?.upload_max_mbps ?? "";
    setFallbackSpeedInputsEnabled(!!profile);

    renderClientsTable();
}

renderConfigMenu("radius_accounting");

loadConfig(() => {
    ensureRadiusConfig(window.config);
    loadUiFromConfig();

    document.getElementById("radiusFallbackSpeedEnabled").addEventListener("change", (ev) => {
        setFallbackSpeedInputsEnabled(!!ev.target.checked);
    });

    document.getElementById("addRadiusClient").addEventListener("click", () => {
        window.config.radius_accounting.clients.push(getDefaultClient());
        const newIndex = window.config.radius_accounting.clients.length - 1;
        renderClientsTable(newIndex);
        announceClientChange(t("config.radius_accounting.clients.added", {index: newIndex + 1}));
    });

    document.getElementById("saveButton").addEventListener("click", () => {
        updateConfigFromUi();
        if (!validateConfig()) return;
        saveConfig(() => {
            alert(t("config.radius_accounting.saved"));
        });
    });
});
