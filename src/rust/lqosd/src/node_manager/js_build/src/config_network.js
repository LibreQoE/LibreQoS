let network_json = null;

function renderNetworkNode(level, depth) {
    let html = `<div class="card mb-3" style="margin-left: ${depth * 30}px;">`;
    html += `<div class="card-body">`;
    
    for (const [key, value] of Object.entries(level)) {
        // Node header with actions
        html += `<div class="d-flex justify-content-between align-items-center mb-2">`;
        html += `<h5 class="card-title mb-0">${key}</h5>`;
        html += `<div>`;
        if (depth > 0) {
            html += `<button class="btn btn-sm btn-outline-secondary me-1" onclick="promoteNode('${key}')">
                        <i class="fas fa-arrow-up"></i> Promote
                     </button>`;
        }
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

function deleteNode(nodeId) {
    if (!confirm(`Are you sure you want to delete ${nodeId} and all its children?`)) {
        return;
    }

    function iterate(tree) {
        for (const [key, value] of Object.entries(tree)) {
            if (key === nodeId) {
                delete tree[key];
                return;
            }

            if (value.children != null) {
                iterate(value.children);
            }
        }
    }

    iterate(network_json);
    renderNetwork();
}

function renameNode(nodeId) {
    let newName = prompt("Enter new node name:");
    if (!newName || newName.trim() === "") {
        alert("Please enter a valid name");
        return;
    }

    function iterate(tree) {
        for (const [key, value] of Object.entries(tree)) {
            if (key === nodeId) {
                tree[newName] = value;
                delete tree[nodeId];
                return;
            }

            if (value.children != null) {
                iterate(value.children);
            }
        }
    }

    iterate(network_json);
    renderNetwork();
}

function start() {
    // Add save button handler
    $("#btnSaveNetwork").on('click', () => {
        alert("Save functionality coming soon!");
    });

    // Load network data
    $.get("/local-api/networkJson", (njs) => {
        network_json = njs;
        renderNetwork();
    });
}

$(document).ready(start);
