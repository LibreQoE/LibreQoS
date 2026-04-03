import {
    loadConfig,
    createShapedDevice,
    deleteShapedDevice,
    loadNetworkJson,
    loadShapedDevicesPage,
    renderConfigMenu,
    topologyEditorsLockMessage,
    topologyEditorsLocked,
    updateShapedDevice,
    validNodeList,
} from "./config/config_helper";
import { parseIpInput } from "./config/shaped_device_wire.mjs";

let current_rows = [];
let network_json = null;
let total_rows = 0;
let total_circuits = 0;
let page = 0;
let page_size = 25;
let search_term = "";
let edit_device_id = null;
let creating_new = false;
let search_timer = null;
let modal_busy = false;
let topology_editor_locked = false;
let topology_editor_lock_message = "";

function setModalReadOnly(readOnly) {
    $("#sdEditForm input, #sdEditForm textarea, #sdEditForm select").prop("disabled", readOnly);
    $("#sdModalSave").prop("disabled", readOnly || modal_busy);
}

function applyEditorLockState() {
    $("#btnAddDevice").prop("disabled", topology_editor_locked);
    $("#sdTableContainer").toggleClass("opacity-75", topology_editor_locked);

    const banner = $("#devicesEditorLock");
    if (topology_editor_locked && topology_editor_lock_message) {
        banner.removeClass("d-none").text(topology_editor_lock_message);
    } else {
        banner.addClass("d-none").text("");
    }

    setModalReadOnly(topology_editor_locked);
}

function actionButtonAttrs() {
    return topology_editor_locked ? " disabled aria-disabled='true'" : "";
}

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
    return Math.max(1, Math.ceil(total_rows / page_size));
}

function currentQuery() {
    return {
        page,
        page_size,
        search: search_term.trim().length > 0 ? search_term.trim() : undefined,
    };
}

function renderLoading() {
    $("#sdTableContainer").html(
        "<div class='text-center py-5'><div class='spinner-border' role='status'><span class='visually-hidden'>Loading...</span></div></div>",
    );
}

function requestPage() {
    renderLoading();
    $("#btnAddDevice").prop("disabled", true);
    $("#btnSaveDevices").prop("disabled", true);
    loadShapedDevicesPage(
        currentQuery(),
        (data) => {
            current_rows = Array.isArray(data?.rows) ? data.rows : [];
            total_rows = Number.isFinite(Number(data?.total_rows))
                ? Math.max(0, Math.trunc(Number(data.total_rows)))
                : current_rows.length;
            total_circuits = Number.isFinite(Number(data?.total_circuits))
                ? Math.max(0, Math.trunc(Number(data.total_circuits)))
                : 0;

            const totalPagesForResult = Math.max(1, Math.ceil(total_rows / page_size));
            if (page >= totalPagesForResult && total_rows > 0) {
                page = totalPagesForResult - 1;
                requestPage();
                return;
            }

            render();
            $("#btnAddDevice").prop("disabled", topology_editor_locked);
            $("#btnSaveDevices").prop("disabled", false);
            applyEditorLockState();
        },
        () => {
            $("#sdTableContainer").html(
                "<div class='alert alert-danger mb-0'>Failed to load shaped devices.</div>",
            );
            $("#btnAddDevice").prop("disabled", topology_editor_locked);
            $("#btnSaveDevices").prop("disabled", false);
        },
    );
}

function setPage(newPage) {
    const total = totalPages();
    page = Math.min(Math.max(newPage, 0), total - 1);
    requestPage();
}

function setPageSize(newSize) {
    page_size = newSize;
    page = 0;
    requestPage();
}

function renderSummary() {
    const start = total_rows === 0 ? 0 : page * page_size + 1;
    const end = total_rows === 0 ? 0 : page * page_size + current_rows.length;
    let text = "";
    if (total_rows === 0) {
        text = "No devices to display";
    } else if (search_term.trim().length > 0) {
        text = `Showing ${start}-${end} of ${total_rows} matching devices`;
    } else {
        text = `Showing ${start}-${end} of ${total_rows} devices across ${total_circuits} circuits`;
    }
    $("#sdSummary").text(text);
}

function renderTable() {
    const container = $("#sdTableContainer");
    if (current_rows.length === 0) {
        container.html("<div class='alert alert-info mb-0'>No devices match the current search.</div>");
        return;
    }

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

    current_rows.forEach((row) => {
        const comment = row.comment ? String(row.comment) : "";
        const deviceId = escapeHtml(row.device_id || "");
        const actionAttrs = actionButtonAttrs();

        html += "<tr data-device-id='" + deviceId + "'>";
        html +=
            "<td class='text-nowrap'>" +
            "<button class='btn btn-sm btn-outline-primary sd-edit me-1' type='button' data-device-id='" +
            deviceId +
            "' title='Edit device' aria-label='Edit device'" + actionAttrs + "><i class='fa fa-edit' aria-hidden='true'></i></button>" +
            "<button class='btn btn-sm btn-outline-danger sd-delete' type='button' data-device-id='" +
            deviceId +
            "' title='Delete device' aria-label='Delete device'" + actionAttrs + "><i class='fa fa-trash' aria-hidden='true'></i></button>" +
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
    });

    html += "</tbody></table>";
    container.html(html);
}

function buildPaginationModel(total, current) {
    const pages = [];
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

    const result = [];
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
    if (total_rows === 0) {
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
    if (sqmDown || sqmUp) {
        device.sqm_override = `${sqmDown}/${sqmUp}`;
    }

    return device;
}

function findCurrentRow(deviceId) {
    return current_rows.find((row) => row.device_id === deviceId);
}

function openEditModal(deviceId) {
    if (topology_editor_locked) return;
    const device = findCurrentRow(deviceId);
    if (!device) return;
    edit_device_id = device.device_id;
    creating_new = false;
    $("#sdEditModalLabel").text("Edit Device");
    populateModal(device);
    showModal();
}

function openNewModal() {
    if (topology_editor_locked) return;
    edit_device_id = null;
    creating_new = true;
    $("#sdEditModalLabel").text("Add Device");
    populateModal(defaultDevice());
    showModal();
}

function checkIpv4(ip) {
    const ipv4Pattern = /^(\d{1,3}\.){3}\d{1,3}(\/\d{1,2})?$/;
    if (!ipv4Pattern.test(ip)) return false;
    const parts = ip.split("/");
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

function validateModalDevice(device) {
    const errors = [];
    const nodes = network_json ? validNodeList(network_json) : [];

    if (!device.circuit_id) errors.push("Circuit ID is required");
    if (!device.circuit_name) errors.push("Circuit Name is required");
    if (!device.device_id) errors.push("Device ID is required");
    if (!device.device_name) errors.push("Device Name is required");

    const parentNode = device.parent_node || "";
    if (nodes.length === 0) {
        if (parentNode.length > 0) {
            errors.push("You have a flat network, so you can't specify a parent node.");
        }
    } else if (parentNode.length > 0 && nodes.indexOf(parentNode) === -1) {
        errors.push("Parent node does not exist.");
    }

    const ipv4List = Array.isArray(device.ipv4) ? device.ipv4 : [];
    const ipv6List = Array.isArray(device.ipv6) ? device.ipv6 : [];
    if (ipv4List.length === 0 && ipv6List.length === 0) {
        errors.push("You must specify either an IPv4 or IPv6 (or both) address.");
    }

    ipv4List.forEach((tuple) => {
        const formatted = formatIpTuple(tuple, 32);
        if (!formatted || !checkIpv4(formatted)) {
            errors.push(`${formatted || "IPv4 entry"} is not a valid IPv4 address.`);
        }
    });

    ipv6List.forEach((tuple) => {
        const formatted = formatIpTuple(tuple, 128);
        if (!formatted || !checkIpv6(formatted)) {
            errors.push(`${formatted || "IPv6 entry"} is not a valid IPv6 address.`);
        }
    });

    if (!Number.isFinite(device.download_min_mbps) || device.download_min_mbps < 0.1) {
        errors.push("Download min must be 0.1 or more.");
    }
    if (!Number.isFinite(device.upload_min_mbps) || device.upload_min_mbps < 0.1) {
        errors.push("Upload min must be 0.1 or more.");
    }
    if (!Number.isFinite(device.download_max_mbps) || device.download_max_mbps < 0.2) {
        errors.push("Download max must be 0.2 or more.");
    }
    if (!Number.isFinite(device.upload_max_mbps) || device.upload_max_mbps < 0.2) {
        errors.push("Upload max must be 0.2 or more.");
    }

    const sqm = device.sqm_override ? parseSqmOverride(device.sqm_override) : { down: "", up: "" };
    const validSqm = (token) => token === "" || token === "cake" || token === "fq_codel" || token === "none";
    if (!validSqm(sqm.down) || !validSqm(sqm.up)) {
        errors.push("SQM overrides must be blank, cake, fq_codel, or none.");
    }

    return { valid: errors.length === 0, errors };
}

function setModalBusy(busy) {
    modal_busy = busy;
    $("#sdModalSave").prop("disabled", busy || topology_editor_locked);
}

function saveModalChanges() {
    if (topology_editor_locked) return;
    if (modal_busy) return;
    const device = collectModalDevice();
    const validation = validateModalDevice(device);
    if (!validation.valid) {
        alert(validation.errors.join("\n"));
        return;
    }

    setModalBusy(true);
    const onComplete = (msg) => {
        setModalBusy(false);
        if (msg && msg.ok) {
            hideModal();
            if (creating_new) {
                page = 0;
            }
            requestPage();
            return;
        }
        alert((msg && msg.message) ? msg.message : "Unable to save shaped device.");
    };

    if (creating_new) {
        createShapedDevice(device, onComplete, () => {
            setModalBusy(false);
            alert("Unable to create shaped device.");
        });
        return;
    }

    updateShapedDevice(edit_device_id, device, onComplete, () => {
        setModalBusy(false);
        alert("Unable to update shaped device.");
    });
}

function deleteDevice(deviceId) {
    if (topology_editor_locked) return;
    const device = findCurrentRow(deviceId);
    const label = device?.device_name || deviceId || "this device";
    if (!confirm(`Delete ${label}?`)) return;
    deleteShapedDevice(
        deviceId,
        (msg) => {
            if (msg && msg.ok) {
                requestPage();
                return;
            }
            alert((msg && msg.message) ? msg.message : "Unable to delete shaped device.");
        },
        () => {
            alert("Unable to delete shaped device.");
        },
    );
}

function start() {
    renderConfigMenu("devices");

    $("#btnAddDevice").prop("disabled", true);
    $("#btnSaveDevices").prop("disabled", true);
    const initialPageSize = parseInt($("#sdPageSize").val(), 10);
    if (!Number.isNaN(initialPageSize)) {
        page_size = initialPageSize;
    }

    $("#sdSearch").on("input", (event) => {
        search_term = event.target.value;
        if (search_timer) {
            clearTimeout(search_timer);
        }
        search_timer = setTimeout(() => {
            page = 0;
            requestPage();
        }, 200);
    });

    $("#sdPageSize").on("change", (event) => {
        const value = parseInt(event.target.value, 10);
        if (!Number.isNaN(value)) setPageSize(value);
    });

    $("#sdPagination, #sdPaginationTop").on("click", "a.page-link", (event) => {
        event.preventDefault();
        const target = parseInt($(event.currentTarget).data("page"), 10);
        if (!Number.isNaN(target)) setPage(target);
    });

    $("#sdTableContainer").on("click", ".sd-edit", (event) => {
        event.preventDefault();
        const deviceId = $(event.currentTarget).data("device-id");
        if (typeof deviceId === "string" && deviceId.length > 0) {
            openEditModal(deviceId);
        }
    });

    $("#sdTableContainer").on("click", ".sd-delete", (event) => {
        event.preventDefault();
        const deviceId = $(event.currentTarget).data("device-id");
        if (typeof deviceId === "string" && deviceId.length > 0) {
            deleteDevice(deviceId);
        }
    });

    $("#btnAddDevice").on("click", (event) => {
        event.preventDefault();
        openNewModal();
    });

    $("#sdModalSave").on("click", (event) => {
        event.preventDefault();
        saveModalChanges();
    });

    $("#btnSaveDevices").on("click", (event) => {
        event.preventDefault();
        requestPage();
    });

    loadConfig(
        (msg) => {
            const config = msg?.data || window.config || {};
            topology_editor_locked = topologyEditorsLocked(config);
            topology_editor_lock_message = topologyEditorsLockMessage(config);
            applyEditorLockState();

            loadNetworkJson(
                (njs) => {
                    network_json = njs;
                    requestPage();
                    applyEditorLockState();
                },
                () => {
                    $("#sdTableContainer").html(
                        "<div class='alert alert-danger mb-0'>Failed to load network configuration.</div>",
                    );
                },
            );
        },
        () => {
            $("#sdTableContainer").html(
                "<div class='alert alert-danger mb-0'>Failed to load configuration.</div>",
            );
        },
    );
}

$(document).ready(start);
