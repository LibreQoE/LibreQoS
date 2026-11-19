import {
    activeTopologySourceIntegrations,
    loadAllShapedDevices,
    loadConfig,
    loadNetworkJson,
    renderConfigMenu,
    saveNetworkAndDevices,
    saveNetworkJsonOnly,
    topologyEditorsLockMessage,
    topologyEditorsLocked,
} from "./config/config_helper";
import {renderMarkupText} from "./config/markup_text.mjs";
import {t, tAttr, tHtml} from "./i18n/index.js";
import {escapeAttr} from "./i18n/escape.mjs";

let network_json = null;
let shaped_devices = null;
let shaped_devices_dirty = false;
const MIN_NODE_MBPS = 0.1;
let topology_editor_locked = false;
let topology_editor_lock_message = "";

function editorButtonAttrs() {
    // translatable: false — "disabled aria-disabled='true'" is an HTML attribute fragment, not a label
    return topology_editor_locked ? " disabled aria-disabled='true'" : "";
}

function applyEditorLockState() {
    $("#btnSaveNetwork").prop("disabled", topology_editor_locked);
    $("#netjson").toggleClass("opacity-75", topology_editor_locked);

    const banner = $("#networkEditorLock");
    if (topology_editor_locked && topology_editor_lock_message) {
        banner.removeClass("d-none").text(topology_editor_lock_message);
    } else {
        banner.addClass("d-none").text("");
    }
}

function renderNetworkNode(level, depth) {
    let html = `<div class="card lqos-network-node-card mb-3" style="margin-left: ${depth * 30}px;">`;
    html += `<div class="card-body">`;
    
    for (const [key, value] of Object.entries(level)) {
        const actionAttrs = editorButtonAttrs();
        // Node header with actions
        html += `<div class="d-flex flex-wrap justify-content-between align-items-center gap-2 mb-2">`;
        html += `<h5 class="card-title mb-0">${key}</h5>`;
        html += `<div class="d-flex flex-wrap gap-2">`;
        if (depth > 0) {
            html += `<button class="btn btn-sm btn-outline-primary" onclick="promoteNode('${key}')" title="${tAttr("config.network.node.promote")} ${escapeAttr(key)}" aria-label="${tAttr("config.network.node.promote")} ${escapeAttr(key)}"${actionAttrs}>
                        <i class="fas fa-arrow-up"></i> ${t("config.network.node.promote")}
                     </button>`;
        }
        html += `<button class="btn btn-sm btn-outline-primary" onclick="renameNode('${key}')" title="${tAttr("config.network.node.rename")} ${escapeAttr(key)}" aria-label="${tAttr("config.network.node.rename")} ${escapeAttr(key)}"${actionAttrs}>
                    <i class="fas fa-pencil-alt"></i> ${t("config.network.node.rename")}
                 </button>`;
        html += `<button class="btn btn-sm btn-outline-danger" onclick="deleteNode('${key}')" title="${tAttr("common.action.delete")} ${escapeAttr(key)}" aria-label="${tAttr("common.action.delete")} ${escapeAttr(key)}"${actionAttrs}>
                    <i class="fas fa-trash-alt"></i> ${t("common.action.delete")}
                 </button>`;
        html += `</div></div>`;

        // Node details
        html += `<div class="mb-3 d-flex flex-wrap align-items-center gap-2">`;
        html += `<span class="badge bg-primary-subtle text-primary border border-primary-subtle">${t("config.network.node.download")} ${value.downloadBandwidthMbps} Mbps</span>`;
        html += `<button class="btn btn-sm btn-outline-primary" onclick="nodeSpeedChange('${key}', 'd')" title="${tAttr("config.network.node.edit_download", {node: key})}" aria-label="${tAttr("config.network.node.edit_download", {node: key})}"${actionAttrs}>
                    <i class="fas fa-pencil-alt" aria-hidden="true"></i>
                 </button>`;
        html += `<span class="badge bg-success-subtle text-success border border-success-subtle">${t("config.network.node.upload")} ${value.uploadBandwidthMbps} Mbps</span>`;
        html += `<button class="btn btn-sm btn-outline-primary" onclick="nodeSpeedChange('${key}', 'u')" title="${tAttr("config.network.node.edit_upload", {node: key})}" aria-label="${tAttr("config.network.node.edit_upload", {node: key})}"${actionAttrs}>
                    <i class="fas fa-pencil-alt" aria-hidden="true"></i>
                 </button>`;
        html += `</div>`;

        // Child nodes
        if (value.children) {
            html += renderNetworkNode(value.children, depth + 1);
        }
    }
    
    html += `</div></div>`;
    return html;
}

function renderNetwork() {
    if (!network_json || Object.keys(network_json).length === 0) {
        $("#netjson").html(`<div class="lqos-config-note">${t("config.network.empty")}</div>`);
        return;
    }
    $("#netjson").html(renderNetworkNode(network_json, 0));
}

function promoteNode(nodeId) {
    if (topology_editor_locked) return;
    // translatable: false — "Promoting " is a console diagnostic, never rendered
    console.log("Promoting ", nodeId);
    let previousParent = null;

    function iterate(tree, depth) {
        for (const [key, value] of Object.entries(tree)) {
            if (key === nodeId) {
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
    renderNetwork();
}

function nodeSpeedChange(nodeId, direction) {
    if (topology_editor_locked) return;
    let newVal = prompt(direction === "d"
        ? t("config.network.prompt.download")
        : t("config.network.prompt.upload"));
    newVal = parseFloat(newVal);
    if (isNaN(newVal)) {
        alert(t("config.network.validation.number"));
        return;
    }
    if (newVal < MIN_NODE_MBPS) {
        alert(t("config.network.validation.minimum", {value: MIN_NODE_MBPS}));
        return;
    }

    function iterate(tree) {
        for (const [key, value] of Object.entries(tree)) {
            if (key === nodeId) {
                if (direction === 'd') {
                    value.downloadBandwidthMbps = newVal;
                } else {
                    value.uploadBandwidthMbps = newVal;
                }
            }

            if (value.children != null) {
                iterate(value.children);
            }
        }
    }

    iterate(network_json);
    renderNetwork();
}

function deleteNode(nodeId) {
    if (topology_editor_locked) return;
    if (!confirm(t("config.network.confirm_delete", {node: nodeId}))) {
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
                shaped_devices_dirty = true;
            }
        }
        alert(t("config.network.no_parent_warning"));
    } else {
        // Move everything up the tree
        for (let i=0; i<shaped_devices.length; i++) {
            let sd = shaped_devices[i];
            if (deleteList.indexOf(sd.parent_node) > -1) {
                sd.parent_node = deleteParent;
                shaped_devices_dirty = true;
            }
        }
    }

    // Update the display
    renderNetwork();
    shapedDevices();
}

function renameNode(nodeId) {
    if (topology_editor_locked) return;
    let newName = prompt(t("config.network.prompt.rename"));
    if (!newName || newName.trim() === "") {
        alert(t("config.network.validation.name"));
        return;
    }

    // Check if the new name already exists
    function checkExists(tree) {
        for (const [key, _] of Object.entries(tree)) {
            if (key === newName) {
                return true;
            }
            if (tree[key].children) {
                if (checkExists(tree[key].children)) {
                    return true;
                }
            }
        }
        return false;
    }

    if (checkExists(network_json)) {
        alert(t("config.network.validation.name_exists"));
        return;
    }

    function iterate(tree, depth) {
        for (const [key, value] of Object.entries(tree)) {
            if (key === nodeId) {
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

    // Update shaped devices
    for (let i=0; i<shaped_devices.length; i++) {
        let sd = shaped_devices[i];
        if (sd.parent_node === nodeId) {
            sd.parent_node = newName;
            shaped_devices_dirty = true;
        }
    }

    renderNetwork();
    shapedDevices();
}

function start() {
    // Render the configuration menu
    renderConfigMenu('network');
    // The page subtitle wraps `network.json` in `<code>`, which the DOM applier
    // cannot keep: the element is empty in the page and rendered here.
    renderMarkupText({networkLayoutSubtitle: tHtml("config.network.subtitle")});
    
    // Add links
    window.promoteNode = promoteNode;
    window.renameNode = renameNode;
    window.deleteNode = deleteNode;
    window.nodeSpeedChange = nodeSpeedChange;

    // Add save button handler
    // Add network save button handler
    $("#btnSaveNetwork").on('click', () => {
        if (topology_editor_locked) {
            alert(topology_editor_lock_message);
            return;
        }
        // Validate network structure
        if (!network_json || Object.keys(network_json).length === 0) {
            alert(t("config.network.validation.empty"));
            return;
        }

        const onComplete = (success, message) => {
            if (success) {
                shaped_devices_dirty = false;
                alert(message);
            } else {
                alert(t("config.network.error.save", {message}));
            }
        };

        if (shaped_devices_dirty) {
            saveNetworkAndDevices(network_json, shaped_devices, onComplete);
            return;
        }

        saveNetworkJsonOnly(network_json, onComplete);
    });

    loadConfig(
        (msg) => {
            const config = window.config || msg?.data?.config || {};
            topology_editor_locked = topologyEditorsLocked(config);
            topology_editor_lock_message = topologyEditorsLockMessage(config);
            const activeIntegrations = activeTopologySourceIntegrations(config);
            if (!topology_editor_lock_message && activeIntegrations.length > 0) {
                topology_editor_lock_message = t("config.network.locked_integrations", {
                    integrations: activeIntegrations.join(", "),
                });
            }
            applyEditorLockState();

            const finishLoad = () => {
                loadNetworkJson((njs) => {
                    network_json = njs;
                    renderNetwork();
                    applyEditorLockState();
                }, () => {
                    alert(t("config.network.error.load_network"));
                });
            };

            if (topology_editor_locked) {
                shaped_devices = [];
                finishLoad();
                return;
            }

            loadAllShapedDevices((data) => {
                shaped_devices = data;
                finishLoad();
            }, () => {
                alert(t("config.network.error.load_devices"));
            });
        },
        () => {
            alert(t("config.network.error.load_config"));
        },
    );
}

$(document).ready(start);
