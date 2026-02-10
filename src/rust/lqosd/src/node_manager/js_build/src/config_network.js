import {
    loadAllShapedDevices,
    loadNetworkJson,
    renderConfigMenu,
    saveNetworkAndDevices,
} from "./config/config_helper";

let network_json = null;
let shaped_devices = null;

function renderNetworkNode(level, depth) {
    let html = `<div class="card mb-3" style="margin-left: ${depth * 30}px;">`;
    html += `<div class="card-body">`;
    
    for (const [key, value] of Object.entries(level)) {
        const isVirtual = value && value.virtual === true;
        // Node header with actions
        html += `<div class="d-flex justify-content-between align-items-center mb-2">`;
        html += `<h5 class="card-title mb-0">${key}${isVirtual ? ' <span class="badge bg-secondary ms-2"><i class="fa fa-ghost"></i> Virtual</span>' : ''}</h5>`;
        html += `<div>`;
        if (depth > 0) {
            html += `<button class="btn btn-sm btn-outline-secondary me-1" onclick="promoteNode('${key}')">
                        <i class="fas fa-arrow-up"></i> Promote
                     </button>`;
        }
        html += `<button class="btn btn-sm btn-outline-secondary me-1" onclick="toggleVirtualNode('${key}')">
                    <i class="fas ${isVirtual ? 'fa-toggle-on' : 'fa-toggle-off'}"></i> Virtual
                 </button>`;
        html += `<button class="btn btn-sm btn-outline-secondary me-1" onclick="renameNode('${key}')">
                    <i class="fas fa-pencil-alt"></i> Rename
                 </button>`;
        html += `<button class="btn btn-sm btn-outline-danger" onclick="deleteNode('${key}')">
                    <i class="fas fa-trash-alt"></i> Delete
                 </button>`;
        html += `</div></div>`;

        // Node details
        html += `<div class="mb-3">`;
        html += `<span class="badge bg-primary me-2">Download: ${value.downloadBandwidthMbps} Mbps</span>`;
        html += `<button class="btn btn-sm btn-outline-secondary me-2" onclick="nodeSpeedChange('${key}', 'd')">
                    <i class="fas fa-pencil-alt"></i>
                 </button>`;
        html += `<span class="badge bg-success me-2">Upload: ${value.uploadBandwidthMbps} Mbps</span>`;
        html += `<button class="btn btn-sm btn-outline-secondary" onclick="nodeSpeedChange('${key}', 'u')">
                    <i class="fas fa-pencil-alt"></i>
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
        $("#netjson").html(`<div class="alert alert-info">No network nodes found. Add one to get started!</div>`);
        return;
    }
    $("#netjson").html(renderNetworkNode(network_json, 0));
}

function promoteNode(nodeId) {
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
    let newVal = prompt(`New ${direction === 'd' ? 'download' : 'upload'} value in Mbps`);
    newVal = parseInt(newVal);
    if (isNaN(newVal)) {
        alert("Please enter a valid number");
        return;
    }
    if (newVal < 1) {
        alert("Value must be greater than 1");
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
    renderNetwork();
}

function deleteNode(nodeId) {
    if (!confirm(`Are you sure you want to delete ${nodeId} and all its children?`)) {
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
    renderNetwork();
    shapedDevices();
}

function renameNode(nodeId) {
    let newName = prompt("New node name?");
    if (!newName || newName.trim() === "") {
        alert("Please enter a valid name");
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
        alert("A node with that name already exists");
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
        }
    }

    renderNetwork();
    shapedDevices();
}

function start() {
    // Render the configuration menu
    renderConfigMenu('network');
    
    // Add links
    window.promoteNode = promoteNode;
    window.renameNode = renameNode;
    window.deleteNode = deleteNode;
    window.nodeSpeedChange = nodeSpeedChange;
    window.toggleVirtualNode = toggleVirtualNode;

    // Add save button handler
    // Add network save button handler
    $("#btnSaveNetwork").on('click', () => {
        // Validate network structure
        if (!network_json || Object.keys(network_json).length === 0) {
            alert("Network configuration is empty");
            return;
        }

        // Save with empty shaped_devices since we're only saving network
        saveNetworkAndDevices(network_json, shaped_devices, (success, message) => {
            if (success) {
                alert(message);
            } else {
                alert("Failed to save network configuration: " + message);
            }
        });
    });

    // Load network data
    loadAllShapedDevices((data) => {
        shaped_devices = data;
        loadNetworkJson((njs) => {
            network_json = njs;
            renderNetwork();
        }, () => {
            alert("Failed to load network configuration");
        });
    }, () => {
        alert("Failed to load shaped devices");
    });
}

$(document).ready(start);
