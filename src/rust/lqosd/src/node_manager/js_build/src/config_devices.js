import {
    loadAllShapedDevices,
    loadNetworkJson,
    renderConfigMenu,
    saveNetworkAndDevices,
    validNodeList,
} from "./config/config_helper";

let shaped_devices = null;
let network_json = null;

function rowPrefix(rowId, boxId) {
    return "sdr_" + rowId + "_" + boxId;
}

function makeSheetBox(rowId, boxId, value, small=false) {
    let html = "";
    if (!small) {
        html = "<td style='padding: 0px'><input id='" + rowPrefix(rowId, boxId) + "' type=\"text\" value=\"" + value + "\"></input></td>"
    } else {
        html = "<td style='padding: 0px'><input id='" + rowPrefix(rowId, boxId) + "' type=\"text\" value=\"" + value + "\" style='font-size: 8pt;'></input></td>"
    }
    return html;
}

function makeSheetNumberBox(rowId, boxId, value) {
    let html = "<td style='padding: 0px'><input id='" + rowPrefix(rowId, boxId) + "' type=\"number\" value=\"" + value + "\" style='width: 100px; font-size: 8pt;' step=\"0.1\"></input></td>"
    return html;
}

function separatedIpArray(rowId, boxId, value) {
    let html = "<td style='padding: 0px'>";
    let val = "";
    for (let i = 0; i < value.length; i++) {
        val += value[i][0];
        val += "/";
        val += value[i][1];
        val += ", ";
    }
    if (val.length > 0) {
        val = val.substring(0, val.length-2);
    }
    html += "<input id='" + rowPrefix(rowId, boxId) + "' type='text' style='font-size: 8pt; width: 100px;' value='" + val + "'></input>";
    html += "</td>";
    return html;
}

function nodeDropDown(rowId, boxId, selectedNode) {
    let html = "<td style='padding: 0px'>";
    html += "<select id='" + rowPrefix(rowId, boxId) + "' style='font-size: 8pt; width: 150px;'>";

    function iterate(data, level) {
        let html = "";
        for (const [key, value] of Object.entries(data)) {
            html += "<option value='" + key + "'";
            if (key === selectedNode) html += " selected";
            html += ">";
            for (let i=0; i<level; i++) html += "-";
            html += key;
            html += "</option>";

            if (value.children != null)
                html += iterate(value.children, level+1);
        }
        return html;
    }
    html += iterate(network_json, 0);

    html += "</select>";
    html += "</td>";
    return html;
}

function newSdRow() {
    shaped_devices.unshift({
        circuit_id: "new_circuit",
        circuit_name: "new circuit",
        device_id: "new_device",
        device_name: "new device",
        mac: "",
        ipv4: "",
        ipv6: "",
        download_min_mbps: 100,
        upload_min_mbps: 100,
        download_max_mbps: 100,
        upload_max_mbps: 100,
        comment: "",
        sqm_override: "",
    });
    shapedDevices();
}

function deleteSdRow(id) {
    shaped_devices.splice(id, 1);
    shapedDevices();
}

function shapedDevices() {
    console.log(shaped_devices);
    let html = "<div class='alert alert-info' style='padding: 6px; margin-bottom: 8px; font-size: 10pt;'>"
        + "SQM overrides can be set per direction. Leave a side blank to use the global default; set to 'none' to disable that side."
        + "</div>";
    html += "<table style='height: 500px; overflow: scroll; border-collapse: collapse; width: 100%; padding: 0px'>";
    html += "<thead style='position: sticky; top: 0; height: 50px; background: navy; color: white;'>";
    html += "<tr style='font-size: 9pt;'><th>Circuit ID</th><th>Circuit Name</th><th>Device ID</th><th>Device Name</th><th>Parent Node</th><th>MAC</th><th>IPv4</th><th>IPv6</th><th>Download Min</th><th>Upload Min</th><th>Download Max</th><th>Upload Max</th><th>Comment</th><th>SQM Down</th><th>SQM Up</th><th></th></th></tr>";
    html += "</thead>";
    for (var i=0; i<shaped_devices.length; i++) {
        let row = shaped_devices[i];
        html += "<tr>";
        html += makeSheetBox(i, "circuit_id", row.circuit_id, true);
        html += makeSheetBox(i, "circuit_name", row.circuit_name, true);
        html += makeSheetBox(i, "device_id", row.device_id, true);
        html += makeSheetBox(i, "device_name", row.device_name, true);
        html += nodeDropDown(i, "parent_node", row.parent_node, true);
        html += makeSheetBox(i, "mac", row.mac, true);
        html += separatedIpArray(i, "ipv4", row.ipv4);
        html += separatedIpArray(i, "ipv6", row.ipv6);
        html += makeSheetNumberBox(i, "download_min_mbps", row.download_min_mbps);
        html += makeSheetNumberBox(i, "upload_min_mbps", row.upload_min_mbps);
        html += makeSheetNumberBox(i, "download_max_mbps", row.download_max_mbps);
        html += makeSheetNumberBox(i, "upload_max_mbps", row.upload_max_mbps);
        html += makeSheetBox(i, "comment", row.comment, true);
        // SQM override dropdowns (optional, per direction)
        const overrideRaw = (row.sqm_override || "").toLowerCase();
        let downSel = "", upSel = "";
        if (overrideRaw.indexOf('/') !== -1) {
            const parts = overrideRaw.split('/')
            downSel = (parts[0] || "").trim();
            upSel = (parts[1] || "").trim();
        } else if (overrideRaw.length > 0) {
            downSel = overrideRaw;
            upSel = overrideRaw;
        }
        const opts = ["", "cake", "fq_codel", "none"]; // empty means default
        const labels = {"": "(default)", "cake": "cake", "fq_codel": "fq_codel", "none": "none"};
        // Down
        let sqmDownHtml = "<td style='padding: 0px'>";
        sqmDownHtml += "<select title='Download SQM override (blank=cfg default, none=disable)' id='" + rowPrefix(i, "sqm_override_down") + "' style='font-size: 8pt; width: 120px;'>";
        for (let k = 0; k < opts.length; k++) {
            const v = opts[k];
            sqmDownHtml += "<option value='" + v + "'" + (downSel === v ? " selected" : "") + ">" + labels[v] + "</option>";
        }
        sqmDownHtml += "</select></td>";
        html += sqmDownHtml;
        // Up
        let sqmUpHtml = "<td style='padding: 0px'>";
        sqmUpHtml += "<select title='Upload SQM override (blank=cfg default, none=disable)' id='" + rowPrefix(i, "sqm_override_up") + "' style='font-size: 8pt; width: 120px;'>";
        for (let k = 0; k < opts.length; k++) {
            const v = opts[k];
            sqmUpHtml += "<option value='" + v + "'" + (upSel === v ? " selected" : "") + ">" + labels[v] + "</option>";
        }
        sqmUpHtml += "</select></td>";
        html += sqmUpHtml;
        html += "<td><button class='btn btn-sm btn-secondary' type='button' onclick='window.deleteSdRow(" + i + ")'><i class='fa fa-trash'></i></button></td>";

        html += "</tr>";
    }
    html += "</tbody></table>";
    $("#shapedDeviceTable").html(html);
}

function start() {
    // Render the configuration menu
    renderConfigMenu('devices');
    // Load shaped devices data
    loadNetworkJson((njs) => {
        network_json = njs;
        loadAllShapedDevices((data) => {
            shaped_devices = data;
            shapedDevices();
        }, () => {
            alert("Failed to load shaped devices");
        });
    }, () => {
        alert("Failed to load network configuration");
    });

    // Setup button handlers
    $("#btnNewDevice").on('click', newSdRow);
    $("#btnSaveDevices").on('click', () => {
        // Validate before saving
        const validation = validateSd();
        if (!validation.valid) {
            alert("Cannot save - please fix validation errors first");
            return;
        }

        // Update shaped devices from UI
        for (let i=0; i<shaped_devices.length; i++) {
            let row = shaped_devices[i];
            row.circuit_id = $("#" + rowPrefix(i, "circuit_id")).val();
            row.circuit_name = $("#" + rowPrefix(i, "circuit_name")).val();
            row.device_id = $("#" + rowPrefix(i, "device_id")).val();
            row.device_name = $("#" + rowPrefix(i, "device_name")).val();
            row.parent_node = $("#" + rowPrefix(i, "parent_node")).val();
            row.mac = $("#" + rowPrefix(i, "mac")).val();
            row.ipv4 = ipAddressesToTuple($("#" + rowPrefix(i, "ipv4")).val());
            row.ipv6 = ipAddressesToTuple($("#" + rowPrefix(i, "ipv6")).val());
            row.download_min_mbps = parseFloat($("#" + rowPrefix(i, "download_min_mbps")).val());
            row.upload_min_mbps = parseFloat($("#" + rowPrefix(i, "upload_min_mbps")).val());
            row.download_max_mbps = parseFloat($("#" + rowPrefix(i, "download_max_mbps")).val());
            row.upload_max_mbps = parseFloat($("#" + rowPrefix(i, "upload_max_mbps")).val());
            row.comment = $("#" + rowPrefix(i, "comment")).val();
            const sqmDown = $("#" + rowPrefix(i, "sqm_override_down")).val().trim().toLowerCase();
            const sqmUp = $("#" + rowPrefix(i, "sqm_override_up")).val().trim().toLowerCase();
            // Compose normalized token string: trimmed, lowercase
            if (!sqmDown && !sqmUp) {
                delete row.sqm_override; // default behavior
            } else {
                // Allow partial overrides; keep slash even if one side empty
                row.sqm_override = `${sqmDown}/${sqmUp}`;
            }
        }

        saveNetworkAndDevices(network_json, shaped_devices, (success, message) => {
            if (success) {
                alert("Configuration saved successfully!");
            } else {
                alert("Failed to save configuration: " + message);
            }
        });
    });
    // Render the configuration menu and expose needed globals
    renderConfigMenu('devices');
    window.deleteSdRow = deleteSdRow;
}

function validateSd() {
    let valid = true;
    let errors = [];
    $(".invalid").removeClass("invalid");
    let validNodes = validNodeList(network_json);

    for (let i=0; i<shaped_devices.length; i++) {
        // Check that circuit ID is good
        let controlId = "#" + rowPrefix(i, "circuit_id");
        let circuit_id = $(controlId).val();
        if (circuit_id.length === 0) {
            valid = false;
            errors.push("Circuits must have a Circuit ID");
            $(controlId).addClass("invalid");
        }

        // Check that the Circuit Name is good
        controlId = "#" + rowPrefix(i, "circuit_name");
        let circuit_name = $(controlId).val();
        if (circuit_name.length === 0) {
            valid = false;
            errors.push("Circuits must have a Circuit Name");
            $(controlId).addClass("invalid");
        }

        // Check that the Device ID is good
        controlId = "#" + rowPrefix(i, "device_id");
        let device_id = $(controlId).val();
        if (device_id.length === 0) {
            valid = false;
            errors.push("Circuits must have a Device ID");
            $(controlId).addClass("invalid");
        }
        for (let j=0; j<shaped_devices.length; j++) {
            if (i !== j) {
                if (shaped_devices[j].device_id === device_id) {
                    valid = false;
                    errors.push("Devices with duplicate ID [" + device_id + "] detected");
                    $(controlId).addClass("invalid");
                    $("#" + rowPrefix(j, "device_id")).addClass("invalid");
                }
            }
        }

        // Check that the Device Name is good
        controlId = "#" + rowPrefix(i, "device_name");
        let device_name = $(controlId).val();
        if (device_name.length === 0) {
            valid = false;
            errors.push("Circuits must have a Device Name");
            $(controlId).addClass("invalid");
        }

        // Check the parent node
        controlId = "#" + rowPrefix(i, "parent_node");
        let parent_node = $(controlId).val();
        if (parent_node == null) parent_node = "";
        if (validNodes.length === 0) {
            // Flat
            if (parent_node.length > 0) {
                valid = false;
                errors.push("You have a flat network, so you can't specify a parent node.");
                $(controlId).addClass("invalid");
            }
        } else {
            // Hierarchy - so we need to know if it exists
            if (validNodes.indexOf(parent_node) === -1) {
                valid = false;
                errors.push("Parent node: " + parent_node + " does not exist");
                $(controlId).addClass("invalid");
            }
        }

        // We can ignore the MAC address

        // IPv4
        controlId = "#" + rowPrefix(i, "ipv4");
        let ipv4 = $(controlId).val();
        if (ipv4.length > 0) {
            // We have IP addresses
            if (ipv4.indexOf(',') !== -1) {
                // We have multiple addresses
                let ips = ipv4.replace(' ', '').split(',');
                for (let j=0; j<ips.length; j++) {
                    if (!checkIpv4(ips[j].trim())) {
                        valid = false;
                        errors.push(ips[j] + "is not a valid IPv4 address");
                        $(controlId).addClass("invalid");
                    }
                    let dupes = checkIpv4Duplicate(ips[j], i);
                    if (dupes > 0 && dupes !== i) {
                        valid = false;
                        errors.push(ips[j] + " is a duplicate IP");
                        $(controlId).addClass("invalid");
                        $("#" + rowPrefix(dupes, "ipv4")).addClass("invalid");
                    }
                }
            } else {
                // Just the one
                if (!checkIpv4(ipv4)) {
                    valid = false;
                    errors.push(ipv4 + "is not a valid IPv4 address");
                    $(controlId).addClass("invalid");
                }
                let dupes = checkIpv4Duplicate(ipv4, i);
                if (dupes > 0) {
                    valid = false;
                    errors.push(ipv4 + " is a duplicate IP");
                    $(controlId).addClass("invalid");
                    $("#" + rowPrefix(dupes, "ipv4")).addClass("invalid");
                }
            }
        }

        // IPv6
        controlId = "#" + rowPrefix(i, "ipv6");
        let ipv6 = $(controlId).val();
        if (ipv6.length > 0) {
            // We have IP addresses
            if (ipv6.indexOf(',') !== -1) {
                // We have multiple addresses
                let ips = ipv6.replace(' ', '').split(',');
                for (let j=0; j<ips.length; j++) {
                    if (!checkIpv6(ips[j].trim())) {
                        valid = false;
                        errors.push(ips[j] + "is not a valid IPv6 address");
                        $(controlId).addClass("invalid");
                    }
                    let dupes = checkIpv6Duplicate(ips[j], i);
                    if (dupes > 0 && dupes !== i) {
                        valid = false;
                        errors.push(ips[j] + " is a duplicate IP");
                        $(controlId).addClass("invalid");
                        $("#" + rowPrefix(dupes, "ipv6")).addClass("invalid");
                    }
                }
            } else {
                // Just the one
                if (!checkIpv6(ipv6)) {
                    valid = false;
                    errors.push(ipv6 + "is not a valid IPv6 address");
                    $(controlId).addClass("invalid");
                }
                let dupes = checkIpv6Duplicate(ipv6, i);
                if (dupes > 0 && dupes !== i) {
                    valid = false;
                    errors.push(ipv6 + " is a duplicate IP");
                    $(controlId).addClass("invalid");
                    $("#" + rowPrefix(dupes, "ipv6")).addClass("invalid");
                }
            }
        }

        // Combined - must be an address between them
        if (ipv4.length === 0 && ipv6.length === 0) {
            valid = false;
            errors.push("You must specify either an IPv4 or IPv6 (or both) address");
            $(controlId).addClass("invalid");
            $("#" + rowPrefix(i, "ipv4")).addClass("invalid");
        }

        // Download Min
        controlId = "#" + rowPrefix(i, "download_min_mbps");
        let download_min = $(controlId).val();
        download_min = parseFloat(download_min);
        if (isNaN(download_min)) {
            valid = false;
            errors.push("Download min is not a valid number");
            $(controlId).addClass("invalid");
        } else if (download_min < 0.1) {
            valid = false;
            errors.push("Download min must be 0.1 or more");
            $(controlId).addClass("invalid");
        }

        // Upload Min
        controlId = "#" + rowPrefix(i, "upload_min_mbps");
        let upload_min = $(controlId).val();
        upload_min = parseFloat(upload_min);
        if (isNaN(upload_min)) {
            valid = false;
            errors.push("Upload min is not a valid number");
            $(controlId).addClass("invalid");
        } else if (upload_min < 0.1) {
            valid = false;
            errors.push("Upload min must be 0.1 or more");
            $(controlId).addClass("invalid");
        }

        // Download Max
        controlId = "#" + rowPrefix(i, "download_max_mbps");
        let download_max = $(controlId).val();
        download_max = parseFloat(download_max);
        if (isNaN(download_max)) {
            valid = false;
            errors.push("Download Max is not a valid number");
            $(controlId).addClass("invalid");
        } else if (download_max < 0.2) {
            valid = false;
            errors.push("Download Max must be 0.2 or more");
            $(controlId).addClass("invalid");
        }

        // Upload Max
        controlId = "#" + rowPrefix(i, "upload_max_mbps");
        let upload_max = $(controlId).val();
        upload_max = parseFloat(upload_max);
        if (isNaN(upload_max)) {
            valid = false;
            errors.push("Upload Max is not a valid number");
            $(controlId).addClass("invalid");
        } else if (upload_max < 0.2) {
            valid = false;
            errors.push("Upload Max must be 0.2 or more");
            $(controlId).addClass("invalid");
        }
    }

    if (!valid) {
        let errorMessage = "Invalid ShapedDevices Entries:\n";
        for (let i=0; i<errors.length; i++) {
            errorMessage += errors[i] + "\n";
        }
        alert(errorMessage);
    }

    return {
        valid: valid,
        errors: errors
    };
}

function checkIpv4(ip) {
    const ipv4Pattern =
        /^(\d{1,3}\.){3}\d{1,3}$/;

    if (ip.indexOf('/') === -1) {
        return ipv4Pattern.test(ip);
    } else {
        let parts = ip.split('/');
        return ipv4Pattern.test(parts[0]);
    }
}

function checkIpv6(ip) {
    // Check if the input is a valid IPv6 address with prefix
    const regex = /^(([0-9a-fA-F]{1,4}:){7,7}[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,7}:|([0-9a-fA-F]{1,4}:){1,6}:[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,5}(:[0-9a-fA-F]{1,4}){1,2}|([0-9a-fA-F]{1,4}:){1,4}(:[0-9a-fA-F]{1,4}){1,3}|([0-9a-fA-F]{1,4}:){1,3}(:[0-9a-fA-F]{1,4}){1,4}|([0-9a-fA-F]{1,4}:){1,2}(:[0-9a-fA-F]{1,4}){1,5}|[0-9a-fA-F]{1,4}:((:[0-9a-fA-F]{1,4}){1,6})|:((:[0-9a-fA-F]{1,4}){1,7}|:)|fe80:(:[0-9a-fA-F]{0,4}){0,4}%[0-9a-zA-Z]{1,}|::(ffff(:0{1,4}){0,1}:){0,1}((25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])\.){3,3}(25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])|([0-9a-fA-F]{1,4}:){1,4}:((25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])\.){3,3}(25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9]))(\/([0-9]{1,3}))?$/;
    return regex.test(ip);
}

function checkIpv4Duplicate(ip, index) {
    ip = ip.trim();
    for (let i=0; i < shaped_devices.length; i++) {
        if (i !== index) {
            let sd = shaped_devices[i];
            for (let j=0; j<sd.ipv4.length; j++) {
                let formatted = "";
                if (ip.indexOf('/') > 0) {
                    formatted = sd.ipv4[j][0] + "/" + sd.ipv4[j][1];
                } else {
                    formatted = sd.ipv4[j][0];
                }
                if (formatted === ip) {
                    return index;
                }
            }
        }
    }
    return -1;
}

function checkIpv6Duplicate(ip, index) {
    ip = ip.trim();
    for (let i=0; i < shaped_devices.length; i++) {
        if (i !== index) {
            let sd = shaped_devices[i];
            for (let j=0; j<sd.ipv6.length; j++) {
                let formatted = "";
                if (ip.indexOf('/') > 0) {
                    formatted = sd.ipv6[j][0] + "/" + sd.ipv6[j][1];
                } else {
                    formatted = sd.ipv6[j][0];
                }
                if (formatted === ip) {
                    return index;
                }
            }
        }
    }
    return -1;
}

// Local helper copied from configuration.js to avoid cross-bundle dependency
function ipAddressesToTuple(ip) {
    if (!ip || ip.length === 0) return [];
    let ips = ip.replace(' ', '').split(',');
    for (let i = 0; i < ips.length; i++) {
        let this_ip = ips[i].trim();
        let parts = this_ip.split('/');
        ips[i] = [ parts[0], parseInt(parts[1]) ];
    }
    return ips;
}

$(document).ready(start);
