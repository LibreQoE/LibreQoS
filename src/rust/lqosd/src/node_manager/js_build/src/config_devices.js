import {
    loadAllShapedDevices,
    loadNetworkJson,
    renderConfigMenu,
    saveNetworkAndDevices,
    validNodeList,
} from "./config/config_helper";

let shaped_devices = [];
let network_json = null;
let filtered_indices = [];
let page = 0;
let page_size = 25;
let search_term = "";
let invalid_indices = new Set();
let edit_index = null;
let creating_new = false;
let search_timer = null;

function defaultDevice() {
    return {
        circuit_id: "new_circuit",
        circuit_name: "new circuit",
        device_id: "new_device",
        device_name: "new device",
        parent_node: "",
        mac: "",
        ipv4: [],
        ipv6: [],
        download_min_mbps: 100,
        upload_min_mbps: 100,
        download_max_mbps: 100,
        upload_max_mbps: 100,
        comment: "",
        sqm_override: "",
    };
}

function escapeHtml(value) {
    const text = value === null || value === undefined ? "" : String(value);
    return text
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;")
        .replace(/\"/g, "&quot;")
        .replace(/'/g, "&#39;");
}

function formatNumber(value) {
    const numeric = parseFloat(value);
    if (!Number.isFinite(numeric)) return "-";
    let formatted = numeric.toFixed(3);
    formatted = formatted.replace(/\.?0+$/, "");
    return formatted;
}

function formatIdCell(value, maxLength = 5) {
    const raw = value === null || value === undefined ? "" : String(value);
    const full = escapeHtml(raw);
    if (!raw) {
        return "<span class='text-body-secondary'>-</span>";
    }
    let display = raw;
    if (raw.length > maxLength) {
        display = raw.slice(0, maxLength) + "...";
    }
    return (
        "<span class='font-monospace' title='" +
        full +
        "'>" +
        escapeHtml(display) +
        "</span>"
    );
}

function formatRateHtml(down, up) {
    return (
        "<div class='small'>" +
        "<div><span class='text-body-secondary'><i class='fa fa-arrow-down' aria-hidden='true'></i></span> " +
        escapeHtml(formatNumber(down)) +
        "</div>" +
        "<div><span class='text-body-secondary'><i class='fa fa-arrow-up' aria-hidden='true'></i></span> " +
        escapeHtml(formatNumber(up)) +
        "</div>" +
        "</div>"
    );
}

function formatIpAddr(address, family) {
    if (address === null || address === undefined) return "";
    if (Array.isArray(address)) {
        if (family === 4 && address.length === 4) {
            return address.map((part) => String(part)).join(".");
        }
        if (family === 6) {
            if (address.length === 16) {
                const groups = [];
                for (let i = 0; i < 16; i += 2) {
                    const high = Number(address[i]) || 0;
                    const low = Number(address[i + 1]) || 0;
                    const value = ((high << 8) | low) >>> 0;
                    groups.push(value.toString(16));
                }
                return groups.join(":");
            }
            if (address.length === 8) {
                return address.map((part) => Number(part).toString(16)).join(":");
            }
            return address.map((part) => String(part)).join(":");
        }
        return address.map((part) => String(part)).join(".");
    }
    return String(address);
}

function formatIpTuple(tuple, defaultPrefix) {
    if (!tuple || tuple.length === 0) return "";
    const addr = tuple[0] ? formatIpAddr(tuple[0], defaultPrefix === 128 ? 6 : 4) : "";
    const prefix = Number.isFinite(tuple[1]) ? tuple[1] : defaultPrefix;
    if (!addr) return "";
    return addr + "/" + prefix;
}

function formatIpList(list, defaultPrefix) {
    if (!Array.isArray(list) || list.length === 0) return "";
    return list
        .map((tuple) => formatIpTuple(tuple, defaultPrefix))
        .filter((val) => val.length > 0)
        .map((val) => escapeHtml(val))
        .join("<br>");
}

function formatIpColumn(device) {
    const v4 = formatIpList(device.ipv4, 32);
    const v6 = formatIpList(device.ipv6, 128);
    if (!v4 && !v6) {
        return "<span class='text-body-secondary'>-</span>";
    }

    let html = "<div class='small redactable'>";
    if (v4) {
        html += "<div><span class='text-body-secondary'>v4</span><br>" + v4 + "</div>";
    }
    if (v6) {
        html += "<div class='mt-1'><span class='text-body-secondary'>v6</span><br>" + v6 + "</div>";
    }
    html += "</div>";
    return html;
}

function ipListToText(list, defaultPrefix) {
    if (!Array.isArray(list) || list.length === 0) return "";
    return list
        .map((tuple) => formatIpTuple(tuple, defaultPrefix))
        .filter((val) => val.length > 0)
        .join("\n");
}

function parseIpInput(text, family) {
    if (!text) return [];
    const defaultPrefix = family === 6 ? 128 : 32;
    const tokens = text.split(/[\n,]+/);
    const result = [];
    tokens.forEach((token) => {
        const trimmed = token.trim();
        if (!trimmed) return;
        const parts = trimmed.split("/");
        const addr = parts[0].trim();
        if (!addr) return;
        let prefix = defaultPrefix;
        if (parts.length > 1 && parts[1].trim().length > 0) {
            const parsed = parseInt(parts[1].trim(), 10);
            if (!Number.isNaN(parsed)) prefix = parsed;
        }
        result.push([addr, prefix]);
    });
    return result;
}

function parseSqmOverride(raw) {
    const normalized = (raw || "").toLowerCase();
    let down = "";
    let up = "";
    if (normalized.includes("/")) {
        const parts = normalized.split("/");
        down = (parts[0] || "").trim();
        up = (parts[1] || "").trim();
    } else if (normalized.length > 0) {
        down = normalized.trim();
        up = normalized.trim();
    }
    return { down, up };
}

function formatSqmDisplay(raw) {
    const parsed = parseSqmOverride(raw);
    const down = parsed.down || "default";
    const up = parsed.up || "default";
    return (
        "<div class='small'>" +
        "<div><span class='text-body-secondary'><i class='fa fa-arrow-down' aria-hidden='true'></i></span> " +
        escapeHtml(down) +
        "</div>" +
        "<div><span class='text-body-secondary'><i class='fa fa-arrow-up' aria-hidden='true'></i></span> " +
        escapeHtml(up) +
        "</div>" +
        "</div>"
    );
}

function buildParentNodeOptions(selectedNode) {
    let html = "<option value=''>-- none --</option>";
    if (!network_json || typeof network_json !== "object") return html;

    function iterate(data, level) {
        let local = "";
        for (const [key, value] of Object.entries(data)) {
            const indent = "-".repeat(level);
            local += "<option value='" + escapeHtml(key) + "'";
            if (key === selectedNode) local += " selected";
            local += ">" + indent + escapeHtml(key) + "</option>";
            if (value && value.children != null) {
                local += iterate(value.children, level + 1);
            }
        }
        return local;
    }

    html += iterate(network_json, 0);
    return html;
}

function totalPages() {
    return Math.max(1, Math.ceil(filtered_indices.length / page_size));
}

function applySearch(resetPage = true) {
    const term = (search_term || "").toLowerCase().trim();
    filtered_indices = [];

    if (!term) {
        for (let i = 0; i < shaped_devices.length; i++) {
            filtered_indices.push(i);
        }
    } else {
        for (let i = 0; i < shaped_devices.length; i++) {
            const device = shaped_devices[i] || {};
            const parts = [
                device.circuit_id,
                device.circuit_name,
                device.device_id,
                device.device_name,
                device.parent_node,
                device.mac,
            ];

            if (Array.isArray(device.ipv4)) {
                device.ipv4.forEach((tuple) => parts.push(formatIpTuple(tuple, 32)));
            }
            if (Array.isArray(device.ipv6)) {
                device.ipv6.forEach((tuple) => parts.push(formatIpTuple(tuple, 128)));
            }

            const haystack = parts
                .filter((val) => val !== undefined && val !== null)
                .join(" ")
                .toLowerCase();

            if (haystack.includes(term)) {
                filtered_indices.push(i);
            }
        }
    }

    if (resetPage) {
        page = 0;
    } else {
        page = Math.min(page, totalPages() - 1);
    }

    render();
}

function setPage(newPage) {
    const total = totalPages();
    page = Math.min(Math.max(newPage, 0), total - 1);
    render();
}

function setPageSize(newSize) {
    page_size = newSize;
    page = 0;
    render();
}

function renderSummary() {
    const total = shaped_devices.length;
    const filtered = filtered_indices.length;
    const start = filtered === 0 ? 0 : page * page_size + 1;
    const end = filtered === 0 ? 0 : Math.min(page * page_size + page_size, filtered);
    let text = "";
    if (filtered === 0) {
        text = "No devices to display";
    } else if (filtered === total) {
        text = `Showing ${start}-${end} of ${total} devices`;
    } else {
        text = `Showing ${start}-${end} of ${filtered} devices (filtered from ${total})`;
    }
    $("#sdSummary").text(text);
}

function renderTable() {
    const container = $("#sdTableContainer");
    if (filtered_indices.length === 0) {
        container.html("<div class='alert alert-info mb-0'>No devices match the current search.</div>");
        return;
    }

    const start = page * page_size;
    const end = Math.min(start + page_size, filtered_indices.length);

    let html =
        "<table id='shapedDeviceTable' class='table table-striped table-hover table-sm align-middle small mb-0'>";
    html += "<thead class='table-dark sticky-top' style='top: 0; z-index: 2;'>";
    html += "<tr>";
    html += "<th class='text-nowrap'>Actions</th>";
    html += "<th class='text-nowrap'>Circuit ID</th>";
    html += "<th>Circuit Name</th>";
    html += "<th class='text-nowrap'>Device ID</th>";
    html += "<th>Device Name</th>";
    html += "<th>Parent Node</th>";
    html += "<th class='text-nowrap'>MAC</th>";
    html += "<th>IP Addresses</th>";
    html += "<th class='text-nowrap'>Min Mbps</th>";
    html += "<th class='text-nowrap'>Max Mbps</th>";
    html += "<th class='text-nowrap'>SQM</th>";
    html += "<th>Comment</th>";
    html += "</tr>";
    html += "</thead><tbody>";

    for (let i = start; i < end; i++) {
        const idx = filtered_indices[i];
        const row = shaped_devices[idx] || {};
        const invalidClass = invalid_indices.has(idx) ? " table-danger" : "";
        const comment = row.comment ? String(row.comment) : "";

        html += "<tr data-index='" + idx + "' class='" + invalidClass + "'>";
        html +=
            "<td class='text-nowrap'>" +
            "<button class='btn btn-sm btn-outline-primary sd-edit me-1' type='button' data-index='" +
            idx +
            "' title='Edit device' aria-label='Edit device'><i class='fa fa-edit' aria-hidden='true'></i></button>" +
            "<button class='btn btn-sm btn-outline-danger sd-delete' type='button' data-index='" +
            idx +
            "' title='Delete device' aria-label='Delete device'><i class='fa fa-trash' aria-hidden='true'></i></button>" +
            "</td>";
        html += "<td class='text-nowrap'>" + formatIdCell(row.circuit_id, 5) + "</td>";
        html +=
            "<td><span class='redactable'>" +
            escapeHtml(row.circuit_name || "") +
            "</span></td>";
        html += "<td class='text-nowrap'>" + formatIdCell(row.device_id, 5) + "</td>";
        html +=
            "<td><span class='redactable'>" +
            escapeHtml(row.device_name || "") +
            "</span></td>";
        html += "<td>" + escapeHtml(row.parent_node || "") + "</td>";
        html +=
            "<td class='text-nowrap font-monospace'><span class='redactable'>" +
            escapeHtml(row.mac || "") +
            "</span></td>";
        html += "<td>" + formatIpColumn(row) + "</td>";
        html += "<td>" + formatRateHtml(row.download_min_mbps, row.upload_min_mbps) + "</td>";
        html += "<td>" + formatRateHtml(row.download_max_mbps, row.upload_max_mbps) + "</td>";
        html += "<td>" + formatSqmDisplay(row.sqm_override || "") + "</td>";
        html += "<td style='max-width: 220px;'>";
        if (comment.length > 0) {
            html +=
                "<span class='d-block text-truncate' style='max-width: 220px;' title='" +
                escapeHtml(comment) +
                "'>" +
                escapeHtml(comment) +
                "</span>";
        } else {
            html += "<span class='text-body-secondary'>-</span>";
        }
        html += "</td>";
        html += "</tr>";
    }

    html += "</tbody></table>";
    container.html(html);
}

function buildPaginationModel(total, current) {
    let pages = [];
    const add = (value) => {
        if (value < 0 || value >= total) return;
        if (!pages.includes(value)) pages.push(value);
    };
    add(0);
    add(total - 1);
    add(current - 2);
    add(current - 1);
    add(current);
    add(current + 1);
    add(current + 2);
    pages.sort((a, b) => a - b);

    let result = [];
    let last = null;
    pages.forEach((p) => {
        if (last !== null && p - last > 1) {
            result.push(null);
        }
        result.push(p);
        last = p;
    });
    return result;
}

function renderPagination() {
    const total = totalPages();
    const listBottom = $("#sdPagination");
    const listTop = $("#sdPaginationTop");
    if (filtered_indices.length === 0) {
        listBottom.html("");
        listTop.html("");
        return;
    }

    const prevDisabled = page === 0 ? " disabled" : "";
    const nextDisabled = page >= total - 1 ? " disabled" : "";
    let html = "";

    html +=
        "<li class='page-item" +
        prevDisabled +
        "'><a class='page-link' href='#' data-page='" +
        (page - 1) +
        "'>Prev</a></li>";

    const model = buildPaginationModel(total, page);
    model.forEach((p) => {
        if (p === null) {
            html += "<li class='page-item disabled'><span class='page-link'>…</span></li>";
        } else {
            const active = p === page ? " active" : "";
            html +=
                "<li class='page-item" +
                active +
                "'><a class='page-link' href='#' data-page='" +
                p +
                "'>" +
                (p + 1) +
                "</a></li>";
        }
    });

    html +=
        "<li class='page-item" +
        nextDisabled +
        "'><a class='page-link' href='#' data-page='" +
        (page + 1) +
        "'>Next</a></li>";

    listBottom.html(html);
    listTop.html(html);
}

function render() {
    renderSummary();
    renderTable();
    renderPagination();
}

function showModal() {
    const modalEl = document.getElementById("sdEditModal");
    if (!modalEl || typeof bootstrap === "undefined") return;
    const modal = bootstrap.Modal.getOrCreateInstance(modalEl);
    modal.show();
}

function hideModal() {
    const modalEl = document.getElementById("sdEditModal");
    if (!modalEl || typeof bootstrap === "undefined") return;
    const modal = bootstrap.Modal.getOrCreateInstance(modalEl);
    modal.hide();
}

function populateModal(device) {
    $("#sdModalCircuitId").val(device.circuit_id || "");
    $("#sdModalCircuitName").val(device.circuit_name || "");
    $("#sdModalDeviceId").val(device.device_id || "");
    $("#sdModalDeviceName").val(device.device_name || "");
    $("#sdModalMac").val(device.mac || "");
    $("#sdModalIpv4").val(ipListToText(device.ipv4, 32));
    $("#sdModalIpv6").val(ipListToText(device.ipv6, 128));
    $("#sdModalDownloadMin").val(device.download_min_mbps);
    $("#sdModalUploadMin").val(device.upload_min_mbps);
    $("#sdModalDownloadMax").val(device.download_max_mbps);
    $("#sdModalUploadMax").val(device.upload_max_mbps);
    $("#sdModalComment").val(device.comment || "");

    const parentSelect = $("#sdModalParentNode");
    parentSelect.html(buildParentNodeOptions(device.parent_node || ""));
    const nodes = network_json ? validNodeList(network_json) : [];
    parentSelect.prop("disabled", nodes.length === 0);

    const sqm = parseSqmOverride(device.sqm_override || "");
    $("#sdModalSqmDown").val(sqm.down);
    $("#sdModalSqmUp").val(sqm.up);
}

function collectModalDevice() {
    const device = {
        circuit_id: $("#sdModalCircuitId").val().trim(),
        circuit_name: $("#sdModalCircuitName").val().trim(),
        device_id: $("#sdModalDeviceId").val().trim(),
        device_name: $("#sdModalDeviceName").val().trim(),
        parent_node: $("#sdModalParentNode").val() || "",
        mac: $("#sdModalMac").val().trim(),
        ipv4: parseIpInput($("#sdModalIpv4").val(), 4),
        ipv6: parseIpInput($("#sdModalIpv6").val(), 6),
        download_min_mbps: parseFloat($("#sdModalDownloadMin").val()),
        upload_min_mbps: parseFloat($("#sdModalUploadMin").val()),
        download_max_mbps: parseFloat($("#sdModalDownloadMax").val()),
        upload_max_mbps: parseFloat($("#sdModalUploadMax").val()),
        comment: $("#sdModalComment").val().trim(),
    };

    const sqmDown = $("#sdModalSqmDown").val().trim().toLowerCase();
    const sqmUp = $("#sdModalSqmUp").val().trim().toLowerCase();
    if (!sqmDown && !sqmUp) {
        delete device.sqm_override;
    } else {
        device.sqm_override = `${sqmDown}/${sqmUp}`;
    }

    return device;
}

function openEditModal(index) {
    const device = shaped_devices[index];
    if (!device) return;
    edit_index = index;
    creating_new = false;
    $("#sdEditModalLabel").text("Edit Device");
    populateModal(device);
    showModal();
}

function openNewModal() {
    edit_index = null;
    creating_new = true;
    $("#sdEditModalLabel").text("Add Device");
    populateModal(defaultDevice());
    showModal();
}

function saveModalChanges() {
    const device = collectModalDevice();
    if (creating_new) {
        shaped_devices.unshift(device);
    } else if (edit_index !== null) {
        shaped_devices[edit_index] = device;
    }
    invalid_indices = new Set();
    hideModal();
    applySearch(creating_new);
}

function deleteDevice(index) {
    const device = shaped_devices[index];
    if (!device) return;
    const label = device.device_name || device.device_id || "this device";
    if (!confirm(`Delete ${label}?`)) return;
    shaped_devices.splice(index, 1);
    invalid_indices = new Set();
    applySearch(false);
}

function checkIpv4(ip) {
    const ipv4Pattern = /^(\d{1,3}\.){3}\d{1,3}$/;
    const parts = ip.split("/");
    if (!ipv4Pattern.test(parts[0])) return false;
    if (parts.length > 1 && parts[1].length > 0) {
        const prefix = parseInt(parts[1], 10);
        if (Number.isNaN(prefix) || prefix < 0 || prefix > 32) return false;
    }
    return true;
}

function checkIpv6(ip) {
    const regex = /^(([0-9a-fA-F]{1,4}:){7,7}[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,7}:|([0-9a-fA-F]{1,4}:){1,6}:[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,5}(:[0-9a-fA-F]{1,4}){1,2}|([0-9a-fA-F]{1,4}:){1,4}(:[0-9a-fA-F]{1,4}){1,3}|([0-9a-fA-F]{1,4}:){1,3}(:[0-9a-fA-F]{1,4}){1,4}|([0-9a-fA-F]{1,4}:){1,2}(:[0-9a-fA-F]{1,4}){1,5}|[0-9a-fA-F]{1,4}:((:[0-9a-fA-F]{1,4}){1,6})|:((:[0-9a-fA-F]{1,4}){1,7}|:)|fe80:(:[0-9a-fA-F]{0,4}){0,4}%[0-9a-zA-Z]{1,}|::(ffff(:0{1,4}){0,1}:){0,1}((25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])\.){3,3}(25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])|([0-9a-fA-F]{1,4}:){1,4}:((25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])\.){3,3}(25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9]))(\/(\d{1,3}))?$/;
    return regex.test(ip);
}

function normalizeIp(tuple, family) {
    const defaultPrefix = family === 6 ? 128 : 32;
    if (!tuple || tuple.length === 0) return "";
    const addr = tuple[0] ? formatIpAddr(tuple[0], family).trim() : "";
    if (!addr) return "";
    const prefix = Number.isFinite(tuple[1]) ? tuple[1] : defaultPrefix;
    const normalizedAddr = family === 6 ? addr.toLowerCase() : addr;
    return normalizedAddr + "/" + prefix;
}

function validateDevices() {
    let valid = true;
    let errors = [];
    invalid_indices = new Set();

    const nodes = network_json ? validNodeList(network_json) : [];
    const deviceIds = new Map();
    const ipv4s = new Map();
    const ipv6s = new Map();

    const markInvalid = (index, message) => {
        valid = false;
        errors.push(message);
        invalid_indices.add(index);
    };

    shaped_devices.forEach((device, index) => {
        if (!device.circuit_id || device.circuit_id.trim().length === 0) {
            markInvalid(index, "Circuits must have a Circuit ID");
        }
        if (!device.circuit_name || device.circuit_name.trim().length === 0) {
            markInvalid(index, "Circuits must have a Circuit Name");
        }
        if (!device.device_id || device.device_id.trim().length === 0) {
            markInvalid(index, "Circuits must have a Device ID");
        }
        if (!device.device_name || device.device_name.trim().length === 0) {
            markInvalid(index, "Circuits must have a Device Name");
        }

        if (device.device_id) {
            const existing = deviceIds.get(device.device_id);
            if (existing !== undefined && existing !== index) {
                markInvalid(index, `Devices with duplicate ID [${device.device_id}] detected`);
                invalid_indices.add(existing);
            } else {
                deviceIds.set(device.device_id, index);
            }
        }

        const parentNode = device.parent_node || "";
        if (nodes.length === 0) {
            if (parentNode.length > 0) {
                markInvalid(index, "You have a flat network, so you can't specify a parent node.");
            }
        } else if (parentNode.length > 0 && nodes.indexOf(parentNode) === -1) {
            markInvalid(index, "Parent node: " + parentNode + " does not exist");
        }

        const ipv4List = Array.isArray(device.ipv4) ? device.ipv4 : [];
        const ipv6List = Array.isArray(device.ipv6) ? device.ipv6 : [];
        if (ipv4List.length === 0 && ipv6List.length === 0) {
            markInvalid(index, "You must specify either an IPv4 or IPv6 (or both) address");
        }

        ipv4List.forEach((tuple) => {
            const formatted = normalizeIp(tuple, 4);
            if (!formatted) return;
            if (!checkIpv4(formatted)) {
                markInvalid(index, formatted + " is not a valid IPv4 address");
            }
            const dupe = ipv4s.get(formatted);
            if (dupe !== undefined && dupe !== index) {
                markInvalid(index, formatted + " is a duplicate IP");
                invalid_indices.add(dupe);
            } else {
                ipv4s.set(formatted, index);
            }
        });

        ipv6List.forEach((tuple) => {
            const formatted = normalizeIp(tuple, 6);
            if (!formatted) return;
            if (!checkIpv6(formatted)) {
                markInvalid(index, formatted + " is not a valid IPv6 address");
            }
            const dupe = ipv6s.get(formatted);
            if (dupe !== undefined && dupe !== index) {
                markInvalid(index, formatted + " is a duplicate IP");
                invalid_indices.add(dupe);
            } else {
                ipv6s.set(formatted, index);
            }
        });

        const downloadMin = parseFloat(device.download_min_mbps);
        const uploadMin = parseFloat(device.upload_min_mbps);
        const downloadMax = parseFloat(device.download_max_mbps);
        const uploadMax = parseFloat(device.upload_max_mbps);

        if (Number.isNaN(downloadMin)) {
            markInvalid(index, "Download min is not a valid number");
        } else if (downloadMin < 0.1) {
            markInvalid(index, "Download min must be 0.1 or more");
        }

        if (Number.isNaN(uploadMin)) {
            markInvalid(index, "Upload min is not a valid number");
        } else if (uploadMin < 0.1) {
            markInvalid(index, "Upload min must be 0.1 or more");
        }

        if (Number.isNaN(downloadMax)) {
            markInvalid(index, "Download max is not a valid number");
        } else if (downloadMax < 0.2) {
            markInvalid(index, "Download max must be 0.2 or more");
        }

        if (Number.isNaN(uploadMax)) {
            markInvalid(index, "Upload max is not a valid number");
        } else if (uploadMax < 0.2) {
            markInvalid(index, "Upload max must be 0.2 or more");
        }
    });

    if (!valid) {
        let errorMessage = "Invalid ShapedDevices Entries:\n";
        errors.forEach((message) => {
            errorMessage += message + "\n";
        });
        alert(errorMessage);
    }

    render();
    return { valid, errors };
}

function start() {
    renderConfigMenu("devices");

    $("#btnSaveDevices").prop("disabled", true);
    $("#btnAddDevice").prop("disabled", true);
    const initialPageSize = parseInt($("#sdPageSize").val(), 10);
    if (!Number.isNaN(initialPageSize)) {
        page_size = initialPageSize;
    }

    $("#sdSearch").on("input", (event) => {
        search_term = event.target.value;
        if (search_timer) {
            clearTimeout(search_timer);
        }
        search_timer = setTimeout(() => applySearch(true), 200);
    });

    $("#sdPageSize").on("change", (event) => {
        const value = parseInt(event.target.value, 10);
        if (!Number.isNaN(value)) setPageSize(value);
    });

    $("#sdPagination").on("click", "a.page-link", (event) => {
        event.preventDefault();
        const target = parseInt($(event.currentTarget).data("page"), 10);
        if (!Number.isNaN(target)) setPage(target);
    });

    $("#sdTableContainer").on("click", ".sd-edit", (event) => {
        event.preventDefault();
        const idx = parseInt($(event.currentTarget).data("index"), 10);
        if (!Number.isNaN(idx)) openEditModal(idx);
    });

    $("#sdTableContainer").on("click", ".sd-delete", (event) => {
        event.preventDefault();
        const idx = parseInt($(event.currentTarget).data("index"), 10);
        if (!Number.isNaN(idx)) deleteDevice(idx);
    });

    $("#btnAddDevice").on("click", (event) => {
        event.preventDefault();
        openNewModal();
    });

    $("#sdModalSave").on("click", (event) => {
        event.preventDefault();
        saveModalChanges();
    });

    $("#btnSaveDevices").on("click", () => {
        const validation = validateDevices();
        if (!validation.valid) {
            return;
        }
        saveNetworkAndDevices(network_json, shaped_devices, (success, message) => {
            if (success) {
                alert("Configuration saved successfully!");
            } else {
                alert("Failed to save configuration: " + message);
            }
        });
    });

    loadNetworkJson(
        (njs) => {
            network_json = njs;
            loadAllShapedDevices(
                (data) => {
                    shaped_devices = Array.isArray(data) ? data : [];
                    invalid_indices = new Set();
                    applySearch(true);
                    $("#btnSaveDevices").prop("disabled", false);
                    $("#btnAddDevice").prop("disabled", false);
                },
                () => {
                    $("#sdTableContainer").html(
                        "<div class='alert alert-danger mb-0'>Failed to load shaped devices.</div>",
                    );
                },
            );
        },
        () => {
            $("#sdTableContainer").html(
                "<div class='alert alert-danger mb-0'>Failed to load network configuration.</div>",
            );
        },
    );
}

$(document).ready(start);
