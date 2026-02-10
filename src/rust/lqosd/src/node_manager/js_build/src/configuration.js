// Mostly ported from the original node manager
import { get_ws_client } from "./pubsub/ws";

let nics = null;
let lqosd_config = null;
let shaped_devices = null;
let network_json = null;
let qoo_profiles = null;

const wsClient = get_ws_client();

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

const bindings = [
    // General
    { field: "bindVersion", path: ".version", data: "string", editable: false },
    { field: "bindPath", path: ".lqos_directory", data: "string", editable: true, required: true },
    { field: "bindNodeId", path: ".node_id", data: "string", editable: true, required: true },
    { field: "bindNodeName", path: ".node_name", data: "string", editable: true, required: true },
    { field: "bindQooProfile", path: ".qoo_profile_id", data: "select-nullable", editable: true },
    { field: "bindPacketCaptureTime", path: ".packet_capture_time", data: "integer", editable: true, min: 1, max: 300 },
    { field: "bindQueueCheckPeriodMs", path: ".queue_check_period_ms", data: "integer", editable: true, min: 100, max: 100000 },

    // Tuning
    { field: "bindStopIrqBalance", path: ".tuning.stop_irq_balance", data: "bool", editable: true },
    { field: "bindNetdevBudgetUs", path: ".tuning.netdev_budget_usecs", data: "integer", editable: true },
    { field: "bindNetdevBudgetPackets", path: ".tuning.netdev_budget_packets", data: "integer", editable: true },
    { field: "bindRxUs", path: ".tuning.rx_usecs", data: "integer", editable: true },
    { field: "bindTxUs", path: ".tuning.tx_usecs", data: "integer", editable: true },
    { field: "bindDisableRxVlan", path: ".tuning.disable_rxvlan", data: "bool", editable: true },
    { field: "bindDisableTxVlan", path: ".tuning.disable_txvlan", data: "bool", editable: true },
    { field: "bindDisableOffload", path: ".tuning.disable_offload", data: "array_of_strings", editable: true },

    // Bridge/Stick
    { conditional: true, condition: "if_exists", path: ".bridge", showDiv: "bridgeMode", hideDiv: "OnAStickMode" },
    { field: "bindUseXdpBridge", path: ".bridge.use_xdp_bridge", data: "bool", editable: true },
    { field: "bindBridgeToInternet", path: ".bridge.to_internet", data: "interface", editable: true },
    { field: "bindBridgeToNetwork", path: ".bridge.to_network", data: "interface", editable: true },
    { end_conditional: true },
    { conditional: true, condition: "if_exists", path: ".single_interface", showDiv: "OnAStickMode", hideDiv: "bridgeMode" },
    { field: "bindSingleInterfaceNic", path: ".single_interface.interface", data: "interface", editable: true },
    { field: "bindSingleInterfaceInternetVlan", path: ".single_interface.internet_vlan", data: "integer", editable: true },
    { field: "bindSingleInterfaceNetworklan", path: ".single_interface.network_vlan", data: "integer", editable: true },
    { end_conditional: true },

    // Queues
    { field: "bindSqm", path: ".queues.default_sqm", data: "select-premade", editable: true },
    { field: "bindMonitorMode", path: ".queues.monitor_only", data: "bool", editable: true },
    { field: "bindUplinkMbps", path: ".queues.uplink_bandwidth_mbps", data: "integer", editable: true },
    { field: "bindDownlinkMbps", path: ".queues.downlink_bandwidth_mbps", data: "integer", editable: true },
    { field: "bindGeneratedDownlinkMbps", path: ".queues.generated_pn_download_mbps", data: "integer", editable: true },
    { field: "bindGeneratedUplinkMbps", path: ".queues.generated_pn_upload_mbps", data: "integer", editable: true },
    { field: "bindDryRun", path: ".queues.dry_run", data: "bool", editable: true },
    { field: "bindSudo", path: ".queues.sudo", data: "bool", editable: true },
    { field: "bindBinpack", path: ".queues.use_binpacking", data: "bool", editable: true },
    { field: "bindQueuesAvailableOverride", path: ".queues.override_available_queues", data: "integer", editable: true },

    // LTS
    { field: "bindEnableLTS", path: ".long_term_stats.gather_stats", data: "bool", editable: true },
    { field: "bindLtsCollation", path: ".long_term_stats.collation_period_seconds", data: "integer", editable: true },
    { field: "bindLtsLicense", path: ".long_term_stats.license_key", data: "string", editable: true },
    { field: "bindLtsUispInterval", path: ".long_term_stats.uisp_reporting_interval_seconds", data: "integer", editable: true },

    // IP Ranges
    { field: "bindIgnoreRanges", path: ".ip_ranges.ignore_subnets", data: "ip_array", editable: true },
    { field: "bindAllowRanges", path: ".ip_ranges.allow_subnets", data: "ip_array", editable: true },

    // Flow tracking
    { field: "bindFlowsTimeout", path: ".flows.flow_timeout_seconds", data: "integer", editable: true },
    { field: "bindEnableNetflow", path: ".flows.netflow_enabled", data: "bool", editable: true },
    { field: "bindFlowsPort", path: ".flows.netflow_port", data: "integer", editable: true },
    { field: "bindFlowsTarget", path: ".flows.netflow_ip", data: "string", editable: true },
    { field: "bindFlowsVersion", path: ".flows.netflow_ip", data: "select-premade", editable: true },

    // Integration Common
    { field: "bindCircuitNameAsAddress", path: ".integration_common.circuit_name_as_address", data: "bool", editable: true },
    { field: "bindOverwriteNetJson", path: ".integration_common.always_overwrite_network_json", data: "bool", editable: true },
    { field: "bindIntegrationMikrotik", path: ".integration_common.use_mikrotik_ipv6", data: "bool", editable: true },
    { field: "bindQueueRefreshInterval", path: ".integration_common.queue_refresh_interval_mins", data: "integer", editable: true },

    // Splynx
    { field: "bindSplynxEnable", path: ".splynx_integration.enable_splynx", data: "bool", editable: true },
    { field: "bindSplynxApiKey", path: ".splynx_integration.api_key", data: "string", editable: true },
    { field: "bindSplynxApiSecret", path: ".splynx_integration.api_secret", data: "string", editable: true },
    { field: "bindSplynxApiUrl", path: ".splynx_integration.url", data: "string", editable: true },

    // Netzur
    { field: "bindNetzurEnable", path: ".netzur_integration.enable_netzur", data: "bool", editable: true },
    { field: "bindNetzurApiKey", path: ".netzur_integration.api_key", data: "string", editable: true },
    { field: "bindNetzurApiUrl", path: ".netzur_integration.api_url", data: "string", editable: true },
    { field: "bindNetzurTimeout", path: ".netzur_integration.timeout_secs", data: "integer", editable: true },

    // UISP
    { field: "bindUispEnable", path: ".uisp_integration.enable_uisp", data: "bool", editable: true },
    { field: "bindUispToken", path: ".uisp_integration.token", data: "string", editable: true },
    { field: "bindUispUrl", path: ".uisp_integration.url", data: "string", editable: true },
    { field: "bindUispSite", path: ".uisp_integration.site", data: "string", editable: true },
    { field: "bindUispStrategy", path: ".uisp_integration.strategy", data: "select-premade", editable: true },
    { field: "bindUispSuspended", path: ".uisp_integration.suspended_strategy", data: "select-premade", editable: true },
    { field: "bindUispAirmaxCapacity", path: ".uisp_integration.airmax_capacity", data: "float", editable: true },
    { field: "bindUispLtuCapacity", path: ".uisp_integration.ltu_capacity", data: "float", editable: true },
    { field: "bindUispIpv6Mikrotik", path: ".uisp_integration.ipv6_with_mikrotik", data: "bool", editable: true },
    { field: "bindUispOverheadFactor", path: ".uisp_integration.bandwidth_overhead_factor", data: "float", editable: true },
    { field: "bindUispCommitMultiplier", path: ".uisp_integration.commit_bandwidth_multiplier", data: "float", editable: true },
    { field: "bindUispExcludeSites", path: ".uisp_integration.exclude_sites", data: "array_of_strings", editable: true },
    { field: "bindUispExceptionCpes", path: ".uisp_integration.exception_cpes", data: "array_of_strings", editable: true },
    { field: "bindUispUsePtmpAsParent", path: ".uisp_integration.use_ptmp_as_parent", data: "bool", editable: true },
    { field: "bindUispIgnoreCalculatedCapacity", path:".uisp_integration.ignore_calculated_capacity", data: "bool", editable: true },

    // Powercode
    { field: "bindPowercodeEnable", path: ".powercode_integration.enable_powercode", data: "bool", editable: true },
    { field: "bindPowercodeKey", path: ".powercode_integration.powercode_api_key", data: "string", editable: true },
    { field: "bindPowercodeUrl", path: ".powercode_integration.powercode_api_url", data: "string", editable: true },

    // Sonar
    { field: "bindSonarEnable", path: ".sonar_integration.enable_sonar", data: "bool", editable: true },
    { field: "bindSonarApiKey", path: ".sonar_integration.sonar_api_key", data: "string", editable: true },
    { field: "bindSonarApiUrl", path: ".sonar_integration.sonar_api_url", data: "string", editable: true },
    { field: "bindSonarSnmp", path: ".sonar_integration.snmp_community", data: "string", editable: true },
    { field: "bindSonarAirmax", path: ".sonar_integration.airmax_model_ids", data: "array_of_strings", editable: true },
    { field: "bindSonarLtu", path: ".sonar_integration.ltu_model_ids", data: "array_of_strings", editable: true },
    { field: "bindSonarActive", path: ".sonar_integration.active_status_ids", data: "array_of_strings", editable: true },

    // Influx
    { field: "bindInfluxEnable", path: ".influxdb.enable_influxdb", data: "bool", editable: true },
    { field: "bindInfluxUrl", path: ".influxdb.url", data: "string", editable: true },
    { field: "bindInfluxOrg", path: ".influxdb.org", data: "string", editable: true },
    { field: "bindInfluxBucket", path: ".influxdb.bucket", data: "string", editable: true },
    { field: "bindInfluxToken", path: ".influxdb.token", data: "string", editable: true },

];

function getConfigPath(path) {
    if (!path || lqosd_config == null) {
        return { found: false, value: undefined };
    }
    let trimmed = path.startsWith(".") ? path.slice(1) : path;
    if (trimmed.length === 0) {
        return { found: true, value: lqosd_config };
    }
    let current = lqosd_config;
    let parts = trimmed.split(".");
    for (let i = 0; i < parts.length; i++) {
        let part = parts[i];
        if (current == null || !Object.prototype.hasOwnProperty.call(current, part)) {
            return { found: false, value: undefined };
        }
        current = current[part];
    }
    return { found: true, value: current };
}

function doBindings() {
    let active = true;

    for (var i=0; i<bindings.length; ++i) {
        let entry = bindings[i];

        if (entry.conditional != null) {
            console.log("Conditional encountered");
            if (entry.condition === "if_exists") {
                let result = getConfigPath(entry.path);
                if (result.found && result.value != null) {
                    console.log("Conditional fired");
                    active = true;
                    if (entry.showDiv != null) {
                        $("#" + entry.showDiv).show();
                    }
                    if (entry.hideDiv != null) {
                        $("#" + entry.hideDiv).hide();
                    }
                } else {
                    console.log("Conditional did not fire")
                    active = false;
                }
            }
        } else if (entry.end_conditional != null) {
            console.log("Conditional ended");
            active = true;
        } else {
            if (!active) {
                console.log("Skipping " + entry.path);
                continue;
            }
            let value = getConfigPath(entry.path).value;
            let controlId = "#" + entry.field;
            if (entry.data === "bool") {
                $(controlId).prop('checked', value);
            } else if (entry.data === "select-nullable") {
                $(controlId).val(value === null || value === undefined ? "" : value);
            } else if (entry.data === "selector-premade") {
                $(controlId + " select").val(value);
            } else if (entry.data === "array_of_strings") {
                let expanded = "";
                let values = Array.isArray(value) ? value : [];
                for (let j = 0; j < values.length; j++) {
                    expanded += values[j] + " ";
                }
                expanded = expanded.trimEnd();
                $(controlId).val(expanded);
            } else if (entry.data === "ip_array") {
                let expanded = "";
                let values = Array.isArray(value) ? value : [];
                for (let j = 0; j < values.length; j++) {
                    expanded += values[j] + "\n";
                }
                expanded = expanded.trimEnd();
                $(controlId).val(expanded);
            } else if (entry.data === "interface") {
                console.log("NIC: " + entry.field);
                fillNicList(entry.field, value);
            } else {
                $(controlId).val(value);
            }
            $(controlId).prop('readonly', !entry.editable);
        }
    }
}

function setBridgeMode() {
    $("#bridgeMode").show();
    $("#OnAStickMode").hide();
    lqosd_config.single_interface = null;
    lqosd_config.bridge = {
        use_xdp_bridge: true,
        to_internet: "",
        to_network: "",
    };
    $("#bindUseXdpBridge").prop('checked', true);
    fillNicList("bindBridgeToInternet", nics[0][0]);
    fillNicList("bindBridgeToNetwork", nics[0][0]);
}

function setStickMode() {
    $("#bridgeMode").hide();
    $("#OnAStickMode").show();
    lqosd_config.bridge = null;
    lqosd_config.single_interface = {
        interface: "",
        internet_vlan: 2,
        network_vlan: 3,
    };
    fillNicList("bindSingleInterfaceNic", nics[0][0]);
    $("#bindSingleInterfaceInternetVlan").val('2');
    $("#bindSingleInterfaceNetworklan").val('3');
}

function detectChanges() {
    let changes = [];
    for (let i=0; i<bindings.length; ++i) {
        let entry = bindings[i];
        let controlId = entry.field;
        if (entry.path === ".bridge" || entry.path === ".single_interface") continue;
        if (entry.path != null) {
            let currentValue = $("#" + controlId).val();
            // TODO: Bridge/Stick special case
            if (entry.data === "bool") {
                currentValue = $("#" + controlId).is(':checked');
            } else if (entry.data === "array_of_strings") {
                currentValue = currentValue.replace('\n', '').split(' ');
            } else if (entry.data === "ip_array") {
                currentValue = currentValue.split('\n');
            }
            let result = getConfigPath(entry.path);
            if (!result.found) {
                continue;
            }
            let storedValue = result.value;
            if (entry.data === "string" || entry.data === "integer") {
                if (storedValue === null && currentValue === "")
                    currentValue = null;
            } else if (entry.data === "select-nullable") {
                if (storedValue === null && currentValue === "")
                    currentValue = null;
            }
            //console.log(entry.path, currentValue, storedValue);
            if (String(currentValue) !== String(storedValue)) {
                console.log("Change detected!");
                console.log(entry.path, " has changed. ", storedValue, '=>', currentValue);
                changes.push(i);
            }
        }
    }
    return changes;
}

function validateConfig() {
    console.log("Starting validator");
    $(".invalid").removeClass("invalid");
    let changes = detectChanges();
    if (changes.length === 0) {
        console.log("Nothing changed!");
        alert("No configuration changes were made.");
        return {
            valid: false,
            changes: [],
        };
    }

    let valid = true;
    let errors = [];
    for (let i=0; i<changes.length; i++) {
        let target = bindings[changes[i]];
        if (target.data === "string") {
            let newValue = $("#" + target.field).val();
            if (target.required != null && target.required === true) {
                if (newValue.length === 0) {
                    valid = false;
                    errors.push(target.path + " is required.");
                    $("#" + target.field).addClass("invalid");
                }
            }
        } else if (target.data === "integer") {
            let newValue = $("#" + target.field).val();
            newValue = parseInt(newValue);
            if (isNaN(newValue)) {
                valid = false;
                errors.push(target.path + " must be an integer number.");
                $("#" + target.field).addClass("invalid");
            } else {
                if (target.min != null) {
                    if (newValue < target.min) {
                        valid = false;
                        errors.push(target.path + " must be between " + target.min + " and " + target.max + ".");
                        $("#" + target.field).addClass("invalid");
                    }
                }
                if (target.max != null) {
                    if (newValue > target.max) {
                        valid = false;
                        errors.push(target.path + " must be between " + target.min + " and " + target.max + ".");
                        $("#" + target.field).addClass("invalid");
                    }
                }
            }
        } else if (target.data === "float") {
            let newValue = $("#" + target.field).val();
            newValue = parseFloat(newValue);
            if (isNaN(newValue)) {
                valid = false;
                errors.push(target.path + " must be a decimal number.");
                $("#" + target.field).addClass("invalid");
            } else {
                if (target.min != null) {
                    if (newValue < target.min) {
                        valid = false;
                        errors.push(target.path + " must be between " + target.min + " and " + target.max + ".");
                        $("#" + target.field).addClass("invalid");
                    }
                }
                if (target.max != null) {
                    if (newValue > target.max) {
                        valid = false;
                        errors.push(target.path + " must be between " + target.min + " and " + target.max + ".");
                        $("#" + target.field).addClass("invalid");
                    }
                }
            }
        }
    }

    if (!valid) {
        let errorMessage = "";
        for (let i=0; i<errors.length; i++) {
            errorMessage += errors[i] + "\n";
        }
        alert("Validation errors\n" + errorMessage);
    }

    console.log("Ending Validator");
    return {
        valid: valid,
        changes: changes,
    };
}

function getFinalValue(target) {
    let selector = "#" + target.field;
    switch (target.data) {
        case "bool": return $(selector).is(":checked");
        case "string": return $(selector).val();
        case "integer": return parseInt($(selector).val());
        case "float": return parseFloat($(selector).val());
        case "array_of_strings": return $(selector).val().split(' ');
        case "interface": return $(selector).val();
        case "select-premade": return $(selector).val();
        case "select-nullable": {
            const v = $(selector).val();
            return v === "" ? null : v;
        }
        case "ip_array": return $(selector).val().split('\n');
        default: console.log("Not handled: " + target);
    }
}

function updateSavedConfig(changes) {
    for (let i=0; i<changes.length; i++) {
        let target = bindings[changes[i]];

        let parts = target.path.split(".");
        if (parts.length === 2) {
            // It's a top-level entry so we have to write to the master variable
            lqosd_config[parts[1]] = getFinalValue(target);
        }

        let configTarget = lqosd_config;
        for (let j=1; j<parts.length-1; j++) {
            configTarget = configTarget[parts[j]];

            // Note: we're doing a stupid dance here because of JS's pass-by-value for
            // a field value, when any sane language would just let me use a reference.
            // Stopping at the pre-value level and then referencing its' child forces
            // JS to pass by reference and do what we want!
            if (j === parts.length-2) {
                configTarget[parts[j+1]] = getFinalValue(target);
            }
        }
    }
}

function saveConfig() {
    let validationResult = validateConfig();
    if (!validationResult.valid) return;

    updateSavedConfig(validationResult.changes);
    sendWsRequest(
        "UpdateConfigResult",
        { UpdateConfig: { config: lqosd_config } },
        (msg) => {
            if (msg && msg.ok) {
                alert("Configuration saved");
            } else {
                const message = msg && msg.message ? msg.message : "Error";
                alert("Configuration not saved: " + message);
            }
        },
        () => {
            alert("Configuration not saved: Error");
        },
    );
}

function iterateNetJson(level, depth) {
    let html = "<div style='margin-left: " + depth * 30 + "px; margin-top: 4px;'>";
    for (const [key, value] of Object.entries(level)) {
        const isVirtual = value && value.virtual === true;
        html += "<div>";
        html += "<strong>" + key + "</strong>";
        if (depth > 0) {
            html += "  <button class='btn btn-sm btn-outline-secondary' onclick='window.promoteNode(\"" + key + "\") '><i class='fa fa-arrow-left'></i> Promote</button>";
        }
        html += "  <button class='btn btn-sm btn-outline-secondary' onclick='window.renameNode(\"" + key + "\") '><i class='fa fa-pencil'></i> Rename</button>";
        html += "  <button class='btn btn-sm btn-outline-warning' onclick='window.deleteNode(\"" + key + "\") '><i class='fa fa-trash'></i> Delete</button>";
        html += "<br />";
        html += "Download: " + value.downloadBandwidthMbps + " Mbps <button type='button' class='btn btn-sm btn-outline-secondary' onclick='window.nodeSpeedChange(\"" + key + "\", \"d\")'><i class='fa fa-pencil'></i></button><br />";
        html += "Upload: " + value.uploadBandwidthMbps + " Mbps <button type='button' class='btn btn-sm btn-outline-secondary' onclick='window.nodeSpeedChange(\"" + key + "\", \"u\")'><i class='fa fa-pencil'></i></button><br />";
        html += "Virtual: " + (isVirtual ? "<span class='badge bg-secondary'><i class='fa fa-ghost'></i> Yes</span>" : "<span class='badge bg-light text-dark'>No</span>") +
            " <button type='button' class='btn btn-sm btn-outline-secondary' onclick='window.toggleVirtualNode(\"" + key + "\")'><i class='fa " + (isVirtual ? "fa-toggle-on" : "fa-toggle-off") + "'></i> Toggle</button><br />";
        let num_children = 0;
        for (let i=0; i<shaped_devices.length; i++) {
            if (shaped_devices[i].parent_node === key) {
                num_children++;
            }
        }
        html += "<em>Associated Devices: " + num_children + "</em><br />";
        //console.log(`${key}: ${value}`);
        //console.log("Children", value.children);
        if (value.children != null) {
            html += iterateNetJson(value.children, depth + 1);
        }
        html += "</div>";
    }
    html += "</div>";
    return html;
}

function RenderNetworkJson() {
    let html = "";
    html += iterateNetJson(network_json, 0);
    $("#netjson").html(html);
}

function flattenNetwork() {
    if (confirm("Are you sure you wish to flatten your network? All topology will be removed, giving a flat network. All Shaped Devices will be reparented to the single node.")) {
        network_json = {};
        RenderNetworkJson();
        for (let i=0; i<shaped_devices.length; i++) {
            shaped_devices[i].parent_node = "";
        }
        shapedDevices();
    }
}

function addNetworkNode() {
    let newName = $("#njsNewNodeName").val();
    let newDown = parseInt($("#njsNewNodeDown").val());
    let newUp = parseInt($("#njsNewNodeUp").val());
    if (newName.length > 0 && newDown > 1 && newUp > 1) {
        network_json[newName] = {
            downloadBandwidthMbps: newDown,
            uploadBandwidthMbps: newUp,
            virtual: false,
            children: {}
        }
    }
    RenderNetworkJson();
}

function promoteNode(nodeId) {
    console.log("Promoting ", nodeId);
    let previousParent = null;

    function iterate(tree, depth) {
        for (const [key, value] of Object.entries(tree)) {
            console.log(key);
            if (key === nodeId) {
                console.log(key);
                let tmp = value;
                delete tree[nodeId];
                previousParent[nodeId] = tmp;
            }

            if (value.children != null) {
                previousParent = tree;
                iterate(value.children, depth+1);
            }
        }
    }

    iterate(network_json);
    RenderNetworkJson();
}

function nodeSpeedChange(nodeId, direction) {
    let newVal = prompt("New download value in Mbps");
    newVal = parseInt(newVal);
    if (isNaN(newVal)) {
        alert("That's not an integer!");
        return;
    }
    if (newVal < 1) {
        alert("New value must be greater than 1");
        return;
    }

    // Find and set
    function iterate(tree, depth) {
        for (const [key, value] of Object.entries(tree)) {
            console.log(key);
            if (key === nodeId) {
                switch (direction) {
                    case 'd' : value.downloadBandwidthMbps = newVal; break;
                    case 'u' : value.uploadBandwidthMbps = newVal; break;
                    default: console.log("Oopsie - unknown direction");
                }
            }

            if (value.children != null) {
                iterate(value.children, depth+1);
            }
        }
    }

    iterate(network_json);
    RenderNetworkJson();
}

function toggleVirtualNode(nodeId) {
    function iterate(tree) {
        for (const [key, value] of Object.entries(tree)) {
            if (key === nodeId) {
                value.virtual = !(value && value.virtual === true);
            }
            if (value.children != null) {
                iterate(value.children);
            }
        }
    }

    iterate(network_json);
    RenderNetworkJson();
}

function deleteNode(nodeId) {
    if (!confirm("Are you sure you want to delete " + nodeId + "? All child nodes will also be deleted.")) {
        return;
    }
    let deleteList = [ nodeId ];
    let deleteParent = "";

    // Find the node to delete
    function iterate(tree, depth, parent) {
        for (const [key, value] of Object.entries(tree)) {
            if (key === nodeId) {
                // Find nodes that will go away
                if (value.children != null) {
                    iterateTargets(value.children, depth+1);
                }
                deleteParent = parent;
                delete tree[key];
            }

            if (value.children != null) {
                iterate(value.children, depth+1, key);
            }
        }
    }

    function iterateTargets(tree, depth) {
        for (const [key, value] of Object.entries(tree)) {
            deleteList.push(key);

            if (value.children != null) {
                iterateTargets(value.children, depth+1);
            }
        }
    }

    // Find the nodes to delete and erase them
    iterate(network_json, "");

    // Now we have a list in deleteList of all the nodes that were deleted
    // We need to go through ShapedDevices and re-parent devices
    console.log(deleteParent);
    if (deleteParent == null) {
        // We deleted something at the top of the tree, so there's no
        // natural parent! So we'll set them to be at the root. That's
        // only really the right answer if the user went "flat" - but there's
        // no way to know. So they'll have to fix some validation themselves.
        for (let i=0; i<shaped_devices.length; i++) {
            let sd = shaped_devices[i];
            if (deleteList.indexOf(sd.parent_node) > -1) {
                sd.parent_node = "";
            }
        }
        alert("Because there was no obvious parent, you may have to fix some parenting in your Shaped Devices list.");
    } else {
        // Move everything up the tree
        for (let i=0; i<shaped_devices.length; i++) {
            let sd = shaped_devices[i];
            if (deleteList.indexOf(sd.parent_node) > -1) {
                sd.parent_node = deleteParent;
            }
        }
    }

    // Update the display
    RenderNetworkJson();
    shapedDevices();
}

function renameNode(nodeId) {
    let newName = prompt("New node name?");

    function iterate(tree, depth) {
        for (const [key, value] of Object.entries(tree)) {
            console.log(key);
            if (key === nodeId) {
                console.log(key);
                let tmp = value;
                delete tree[nodeId];
                tree[newName] = tmp;
            }

            if (value.children != null) {
                iterate(value.children, depth+1);
            }
        }
    }

    iterate(network_json);

    for (let i=0; i<shaped_devices.length; i++) {
        let sd = shaped_devices[i];
        if (sd.parent_node === nodeId) sd.parent_node = newName;
    }

    RenderNetworkJson();
    shapedDevices();
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
    let html = "<td style='padding: 0px'><input id='" + rowPrefix(rowId, boxId) + "' type=\"number\" step=\"any\" value=\"" + value + "\" style='width: 100px; font-size: 8pt;'></input></td>"
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

function validNodeList() {
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

function validateSd() {
    let valid = true;
    let errors = [];
    $(".invalid").removeClass("invalid");
    let validNodes = validNodeList();

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

function ipAddressesToTuple(ip) {
    if (ip.length === 0) return [];
    let ips = ip.replace(' ', '').split(',');
    for (let i=0; i<ips.length; i++) {
        let this_ip = ips[i].trim();
        let parts = this_ip.split('/');
        ips[i] = [ parts[0], parseInt(parts[1]) ];
    }
    return ips;
}

function saveNetAndDevices() {
    let isValid = validateSd().valid;
    if (!isValid) {
        alert("Validation errors in ShapedDevices. Not Saving");
        return;
    }

    // Update the Shaped Devices to match the onscreen list
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
    }

    // Submit both for saving
    console.log(network_json);
    let submission = {
        shaped_devices: shaped_devices,
        network_json: network_json,
    }
    sendWsRequest(
        "UpdateNetworkAndDevicesResult",
        { UpdateNetworkAndDevices: submission },
        (msg) => {
            if (msg && msg.ok) {
                alert("Configuration saved");
            } else {
                const message = msg && msg.message ? msg.message : "Error";
                alert("Configuration not saved: " + message);
            }
        },
        () => {
            alert("Configuration not saved: Error");
        },
    );
}

function shapedDevices() {
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

function userManager() {
    let html = "<p>For now, please use <em>bin/lqusers</em> to manage users.</p>";
    $("#userManager").html(html);
}

function fillNicList(id, selected) {
    let select = $("#" + id);
    let html = "";
    for (i=0; i<nics.length; i++) {
        html += "<option value=\"";
        html += nics[i][0] + "\"";
        if (nics[i][0] === selected) {
            html += " selected";
        }
        html += ">" + nics[i][0] + " - " + nics[i][1] + " - " + nics[i][2] + "</option>";
    }
    select.html(html);
}

function fillQooProfileList(selectedId) {
    const select = $("#bindQooProfile");
    if (!qoo_profiles || !qoo_profiles.profiles) {
        select.html("<option value=''>Default</option>");
        select.val(selectedId === null || selectedId === undefined ? "" : selectedId);
        return;
    }

    const defaultId = qoo_profiles.default_profile_id || "";
    const defaultProfile = qoo_profiles.profiles.find(p => p.id === defaultId);
    const defaultLabel = defaultProfile ? defaultProfile.name : (defaultId || "Web browsing");

    let html = `<option value=''> (default) ${defaultLabel} </option>`;
    for (let i = 0; i < qoo_profiles.profiles.length; i++) {
        const p = qoo_profiles.profiles[i];
        html += "<option value='";
        html += p.id;
        html += "'>";
        html += p.name;
        html += " (";
        html += p.id;
        html += ")";
        html += "</option>";
    }
    select.html(html);
    select.val(selectedId === null || selectedId === undefined ? "" : selectedId);
}

function buildNICList(id, selected, disabled=false) {
    let html = "<select id='" + id + "'";
    if (disabled) html += " disabled='true' ";
    html += ">";
    for (i=0; i<nics.length; i++) {
        html += "<option value=\"";
        html += nics[i][0] + "\"";
        if (nics[i][0] == selected) {
            html += " selected";
        }
        html += ">" + nics[i][0] + " - " + nics[i][1] + " - " + nics[i][2] + "</option>";
    }
    html += "</select>";
    return html;
}

function display() {
    let colorPreference = window.localStorage.getItem("colorPreference");
    if (colorPreference == null) {
        window.localStorage.setItem("colorPreference", 0);
        colorPreference = 0;
    }
    $("#colorMode option[id='" + colorPreference + "']").attr("selected", true);
    let redact = window.localStorage.getItem("redact");
    if (redact == null) {
        window.localStorage.setItem("redact", false);
        redact = false;
    }
    if (redact == "false") redact = false;
    $("#redact").prop('checked', redact);
    $("#applyDisplay").on('click', () => {
        let colorPreference = $("#colorMode").find('option:selected').attr('id');
        window.localStorage.setItem("colorPreference", colorPreference);
        let redact = $("#redact").prop('checked');
        window.localStorage.setItem("redact", redact);
    });
}

function start() {
    // Bindings
    $("#btnFlattenNetwork").on('click', flattenNetwork);
    $("#btnAddNetworkNode").on('click', addNetworkNode);
    window.setBridgeMode = setBridgeMode;
    window.setStickMode = setStickMode;
    window.newSdRow = newSdRow;
    window.promoteNode = promoteNode;
    window.renameNode = renameNode;
    window.deleteNode = deleteNode;
    window.nodeSpeedChange = nodeSpeedChange;
    window.toggleVirtualNode = toggleVirtualNode;
    window.deleteSdRow = deleteSdRow;

    // Old
    display();
    sendWsRequest(
        "AdminCheck",
        { AdminCheck: {} },
        (msg) => {
            const is_admin = msg && msg.ok;
            if (!is_admin) {
                $("#controls").html("<p class='alert alert-danger' role='alert'>You have to be an administrative user to change configuration.");
                $("#userManager").html("<p class='alert alert-danger' role='alert'>Only administrators can see/change user information.");
            } else {
                // Handle Saving ispConfig.py
                $("#btnSaveIspConfig").on('click', (data) => {
                    saveConfig();
                });
                $("#btnSaveNetDevices").on('click', (data) => {
                    saveNetAndDevices();
                });
            }

            sendWsRequest(
                "GetConfig",
                { GetConfig: {} },
                (cfgMsg) => {
                    lqosd_config = cfgMsg.data;
                    console.log("Bindings Done");
                    sendWsRequest(
                        "ListNics",
                        { ListNics: {} },
                        (nicsMsg) => {
                            nics = nicsMsg.data;
                            sendWsRequest(
                                "QooProfiles",
                                { QooProfiles: {} },
                                (profilesMsg) => {
                                    qoo_profiles = profilesMsg.data;
                                    fillQooProfileList(lqosd_config?.qoo_profile_id);
                                    console.log(lqosd_config);
                                    doBindings();

                                    // User management
                                    if (is_admin) {
                                        userManager();
                                    }

                                    sendWsRequest(
                                        "NetworkJson",
                                        { NetworkJson: {} },
                                        (netMsg) => {
                                            network_json = netMsg.data;
                                            sendWsRequest(
                                                "AllShapedDevices",
                                                { AllShapedDevices: {} },
                                                (sdMsg) => {
                                                    shaped_devices = sdMsg.data;
                                                    shapedDevices();
                                                    RenderNetworkJson();
                                                },
                                                () => {
                                                    alert("Unable to load shaped devices");
                                                },
                                            );
                                        },
                                        () => {
                                            alert("Unable to load network.json");
                                        },
                                    );
                                },
                                () => {
                                    // Profiles are optional for now; still allow config to load.
                                    fillQooProfileList(lqosd_config?.qoo_profile_id);
                                    console.log(lqosd_config);
                                    doBindings();
                                },
                            );
                        },
                        () => {
                            alert("Unable to list NICs");
                        },
                    );
                },
                () => {
                    alert("Unable to load configuration");
                },
            );
        },
        () => {
            alert("Unable to confirm admin status");
        },
    );
}

$(document).ready(start);
