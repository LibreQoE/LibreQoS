let shaped_devices = null;
let network_json = null;

function start() {
    // Load shaped devices data
    $.get("/local-api/allShapedDevices", (data) => {
        shaped_devices = data;
        
        // Load network data
        $.get("/local-api/networkJson", (njs) => {
            network_json = njs;
            shapedDevices();
        });
    });

    // Setup button handlers
    $("#btnNewDevice").on('click', newSdRow);
    window.deleteSdRow = deleteSdRow;
}

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
    let html = "<td style='padding: 0px'><input id='" + rowPrefix(rowId, boxId) + "' type=\"number\" value=\"" + value + "\" style='width: 100px; font-size: 8pt;'></input></td>"
    return html;
}

function separatedIpArray(rowId, boxId, value) {
    let html = "<td style='padding: 0px'>";
    let val = "";
    for (i = 0; i<value.length; i++) {
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
    });
    shapedDevices();
}

function deleteSdRow(id) {
    shaped_devices.splice(id, 1);
    shapedDevices();
}

function shapedDevices() {
    // Initialize shaped_devices if null
    if (!shaped_devices) {
        shaped_devices = [];
    }
    
    let html = "<table style='height: 500px; overflow: scroll; border-collapse: collapse; width: 100%; padding: 0px'>";
    html += "<thead style='position: sticky; top: 0; height: 50px; background: navy; color: white;'>";
    html += "<tr style='font-size: 9pt;'><th>Circuit ID</th><th>Circuit Name</th><th>Device ID</th><th>Device Name</th><th>Parent Node</th><th>MAC</th><th>IPv4</th><th>IPv6</th><th>Download Min</th><th>Upload Min</th><th>Download Max</th><th>Upload Max</th><th>Comment</th><th></th></th></tr>";
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
        html += "<td><button class='btn btn-sm btn-secondary' type='button' onclick='window.deleteSdRow(" + i + ")'><i class='fa fa-trash'></i></button></td>";

        html += "</tr>";
    }
    html += "</tbody></table>";
    $("#shapedDeviceTable").html(html);
}

function start() {
    // Load shaped devices data
    $.get("/local-api/networkJson", (njs) => {
        network_json = njs;
        $.get("/local-api/allShapedDevices", (data) => {
            shaped_devices = data;
            shapedDevices();
        });
    });

    // Setup button handlers
    $("#btnNewDevice").on('click', newSdRow);
    window.deleteSdRow = deleteSdRow;
}

$(document).ready(start);
