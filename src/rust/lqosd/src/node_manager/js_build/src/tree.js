import {clearDiv, clientTableHeader, formatLastSeen, simpleRow, simpleRowHtml, theading} from "./helpers/builders";
import {
    formatCakeStat, formatCakeStatPercent,
    formatRetransmit, formatRetransmitRaw,
    formatRtt,
    formatThroughput,
} from "./helpers/scaling";
import {colorByQoqScore} from "./helpers/color_scales";
import {scaleNumber, toNumber} from "./lq_js_common/helpers/scaling";
import {get_ws_client, subscribeWS} from "./pubsub/ws";

var tree = null;
var parent = 0;
var upParent = 0;
var subscribed = false;
var expandedNodes = new Set();
var childrenByParentId = new Map();
const wsClient = get_ws_client();
const QOO_TOOLTIP_HTML = "<h5>Quality of Outcome (QoO)</h5>" +
    "<p>Quality of Outcome (QoO) is IETF IPPM “Internet Quality” (draft-ietf-ippm-qoo).<br>" +
    "https://datatracker.ietf.org/doc/draft-ietf-ippm-qoo/<br>" +
    "LibreQoS implements a latency and loss-based model to estimate quality of outcome.</p>";

const listenOnce = (eventName, handler) => {
    const wrapped = (msg) => {
        wsClient.off(eventName, wrapped);
        handler(msg);
    };
    wsClient.on(eventName, wrapped);
};

function initTooltipsWithin(rootEl) {
    if (!rootEl) return;
    if (typeof bootstrap === "undefined" || !bootstrap.Tooltip) return;
    const elements = rootEl.querySelectorAll('[data-bs-toggle="tooltip"]');
    elements.forEach((element) => {
        if (bootstrap.Tooltip.getOrCreateInstance) {
            bootstrap.Tooltip.getOrCreateInstance(element);
        } else {
            new bootstrap.Tooltip(element);
        }
    });
}

function formatQooScore(score0to100, fallback = "-") {
    if (score0to100 === null || score0to100 === undefined) {
        return fallback;
    }
    const numeric = Number(score0to100);
    if (!Number.isFinite(numeric) || numeric === 255) {
        return fallback;
    }
    const clamped = Math.min(100, Math.max(0, Math.round(numeric)));
    const color = colorByQoqScore(clamped);
    return "<span class='muted' style='color: " + color + "'>■</span>" + clamped;
}

function buildChildrenMap() {
    childrenByParentId = new Map();
    for (let i=0; i<tree.length; i++) {
        let node = tree[i][1];
        if (node.immediate_parent !== null) {
            if (!childrenByParentId.has(node.immediate_parent)) {
                childrenByParentId.set(node.immediate_parent, []);
            }
            childrenByParentId.get(node.immediate_parent).push(i);
        }
    }
}

function hasChildren(nodeId) {
    let children = childrenByParentId.get(nodeId);
    return children !== undefined && children.length > 0;
}

function toggleNode(nodeId) {
    if (!hasChildren(nodeId)) {
        return;
    }
    if (expandedNodes.has(nodeId)) {
        expandedNodes.delete(nodeId);
    } else {
        expandedNodes.add(nodeId);
    }
    renderTree();
}

function renderTree() {
    let treeTable = document.createElement("table");
    treeTable.classList.add("table", "table-striped", "table-bordered");
    let thead = document.createElement("thead");
    thead.appendChild(theading("Name"));
    thead.appendChild(theading("Limit"));
    thead.appendChild(theading("⬇️"));
    thead.appendChild(theading("⬆️"));
    thead.appendChild(theading("RTT", 2, "<h5>TCP Round-Trip Time</h5><p>Current median TCP round-trip time. Time taken for a full send-acknowledge round trip. Low numbers generally equate to a smoother user experience.</p>", "tts_retransmits"));
    thead.appendChild(theading("QoO", 2, QOO_TOOLTIP_HTML, "tts_qoo"));
    thead.appendChild(theading("Retr", 2, "<h5>TCP Retransmits</h5><p>Number of TCP retransmits in the last second.</p>", "tts_retransmits"));
    thead.appendChild(theading("Marks", 2, "<h5>Cake Marks</h5><p>Number of times the Cake traffic manager has applied ECN marks to avoid congestion.</p>", "tts_marks"));
    thead.appendChild(theading("Drops", 2, "<h5>Cake Drops</h5><p>Number of times the Cake traffic manager has dropped packets to avoid congestion.</p>", "tts_drops"));

    treeTable.appendChild(thead);
    let tbody = document.createElement("tbody");

    let topChildren = childrenByParentId.get(parent) || [];
    topChildren.forEach((childIdx) => {
        let row = buildRow(childIdx);
        tbody.appendChild(row);
        let childId = tree[childIdx][0];
        if (expandedNodes.has(childId)) {
            iterateChildren(childIdx, tbody, 1);
        }
    });

    if (parent !== 0) {
        let row = document.createElement("tr");
        let col = document.createElement("td");
        col.colSpan = 14;
        col.classList.add("small", "text-center");
        if (upParent === 0) {
            upParent = tree[parent][1].immediate_parent;
        }
        col.innerHTML = "<a href='tree.html?parent=" + upParent + "' class='redactable'><i class='fa fa-chevron-up'></i> Up One Level - " + tree[upParent][1].name + "</a>";
        row.appendChild(col);
        thead.appendChild(row);
    }

    treeTable.appendChild(tbody);

    // Clear and apply
    let target = document.getElementById("tree");
    clearDiv(target)
    target.appendChild(treeTable);
    initTooltipsWithin(treeTable);
}

// This runs first and builds the initial structure on the page
function getInitialTree() {
    listenOnce("NetworkTree", (msg) => {
        const data = msg && msg.data ? msg.data : [];
        tree = data;
        buildChildrenMap();
        if (tree[parent] !== undefined) {
            fillHeader(tree[parent][1]);
        }
        renderTree();

        if (!subscribed) {
            subscribeWS(["NetworkTree", "NetworkTreeClients"], onMessage);
            subscribed = true;
        }
    });
    wsClient.send({ NetworkTree: {} });
}

function fillHeader(node) {
    //console.log("Header");
    $("#nodeName").text(node.name);
    let limitD = "";
    if (node.max_throughput[0] === 0) {
        limitD = "Unlimited";
    } else {
        limitD = scaleNumber(toNumber(node.max_throughput[0], 0) * 1000 * 1000, 1);
    }
    let limitU = "";
    if (node.max_throughput[1] === 0) {
        limitU = "Unlimited";
    } else {
        limitU = scaleNumber(toNumber(node.max_throughput[1], 0) * 1000 * 1000, 1);
    }
    $("#parentLimitsD").text(limitD);
    $("#parentLimitsU").text(limitU);
    $("#parentTpD").html(formatThroughput(toNumber(node.current_throughput[0], 0) * 8, node.max_throughput[0]));
    $("#parentTpU").html(formatThroughput(toNumber(node.current_throughput[1], 0) * 8, node.max_throughput[1]));
    //console.log(node);
    $("#parentRttD").html(formatRtt(node.rtts[0]));
    $("#parentRttU").html(formatRtt(node.rtts[1]));
    $("#parentQooD").html(formatQooScore(node.qoo ? node.qoo[0] : null));
    $("#parentQooU").html(formatQooScore(node.qoo ? node.qoo[1] : null));
    let retr = 0;
    const packetsDown = toNumber(node.current_tcp_packets[0], 0);
    if (packetsDown > 0) {
        retr = toNumber(node.current_retransmits[0], 0) / packetsDown;
    }
    $("#parentRxmitD").html(formatRetransmit(retr));
    retr = 0;
    const packetsUp = toNumber(node.current_tcp_packets[1], 0);
    if (packetsUp > 0) {
        retr = toNumber(node.current_retransmits[1], 0) / packetsUp;
    }
    $("#parentRxmitU").html(formatRetransmit(retr));
}

function iterateChildren(idx, tBody, depth) {
    let nodeId = tree[idx][0];
    let children = childrenByParentId.get(nodeId) || [];
    children.forEach((childIdx) => {
        let row = buildRow(childIdx, depth);
        tBody.appendChild(row);
        let childId = tree[childIdx][0];
        if (expandedNodes.has(childId)) {
            iterateChildren(childIdx, tBody, depth + 1);
        }
    });
}

function buildRow(i, depth=0) {
    let node = tree[i][1];
    let nodeId = tree[i][0];
    let row = document.createElement("tr");
    row.classList.add("small");
    let col = document.createElement("td");
    col.style.textOverflow = "ellipsis";
    col.classList.add("small");
    if (depth > 0) {
        col.style.paddingLeft = (depth * 1.25) + "rem";
    }
    let nameWrap = document.createElement("div");
    nameWrap.classList.add("d-flex", "align-items-center", "gap-1");
    if (hasChildren(nodeId)) {
        let toggle = document.createElement("button");
        toggle.type = "button";
        toggle.classList.add("btn", "btn-link", "btn-sm", "p-0", "text-decoration-none");
        toggle.style.lineHeight = "1";
        let icon = document.createElement("i");
        icon.classList.add("fa", "fa-fw", expandedNodes.has(nodeId) ? "fa-minus" : "fa-plus");
        toggle.appendChild(icon);
        toggle.title = expandedNodes.has(nodeId) ? "Collapse" : "Expand";
        toggle.setAttribute("aria-label", toggle.title);
        toggle.addEventListener("click", (event) => {
            event.preventDefault();
            event.stopPropagation();
            toggleNode(nodeId);
        });
        nameWrap.appendChild(toggle);
    } else {
        let spacer = document.createElement("i");
        spacer.classList.add("fa", "fa-fw", "fa-plus");
        spacer.style.visibility = "hidden";
        nameWrap.appendChild(spacer);
    }
    if (node.virtual === true) {
        let virtualIcon = document.createElement("i");
        virtualIcon.classList.add("fa", "fa-fw", "fa-ghost", "text-secondary");
        virtualIcon.setAttribute("data-bs-toggle", "tooltip");
        virtualIcon.setAttribute("data-bs-placement", "top");
        virtualIcon.setAttribute("title", "Virtual node (logical only; not shaped in HTB).");
        nameWrap.appendChild(virtualIcon);
    }
    let link = document.createElement("a");
    link.href = "/tree.html?parent=" + nodeId + "&upParent=" + parent;
    link.classList.add("redactable");
    link.textContent = node.name;
    nameWrap.appendChild(link);
    if (node.type !== null) {
        let typeText = document.createElement("span");
        typeText.textContent = " (" + node.type + ")";
        nameWrap.appendChild(typeText);
    }
    col.appendChild(nameWrap);
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "limit-" + nodeId;
    col.classList.add("small");
    col.style.width = "8%";
    let limit = "";
    if (node.max_throughput[0] === 0) {
        limit = "Unlimited";
    } else {
        limit = scaleNumber(toNumber(node.max_throughput[0], 0) * 1000 * 1000, 1);
    }
    limit += " / ";
    if (node.max_throughput[1] === 0) {
        limit += "Unlimited";
    } else {
        limit += scaleNumber(toNumber(node.max_throughput[1], 0) * 1000 * 1000, 1);
    }
    col.textContent = limit;
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "down-" + nodeId;
    col.classList.add("small");
    col.style.width = "6%";
    col.innerHTML = formatThroughput(toNumber(node.current_throughput[0], 0) * 8, node.max_throughput[0]);
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "up-" + nodeId;
    col.classList.add("small");
    col.style.width = "6%";
    col.innerHTML = formatThroughput(toNumber(node.current_throughput[1], 0) * 8, node.max_throughput[1]);
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "rtt-down-" + nodeId;
    col.style.width = "6%";
    col.innerHTML = formatRtt(node.rtts[0]);
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "rtt-up-" + nodeId;
    col.style.width = "6%";
    col.innerHTML = formatRtt(node.rtts[1]);
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "qoo-down-" + nodeId;
    col.style.width = "6%";
    col.innerHTML = formatQooScore(node.qoo ? node.qoo[0] : null);
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "qoo-up-" + nodeId;
    col.style.width = "6%";
    col.innerHTML = formatQooScore(node.qoo ? node.qoo[1] : null);
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "re-xmit-down-" + nodeId;
    col.style.width = "6%";
    if (node.current_retransmits[0] !== undefined) {
        col.innerHTML = formatRetransmitRaw(node.current_retransmits[0]);
    } else {
        col.textContent = "-";
    }
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "re-xmit-up-" + nodeId;
    col.style.width = "6%";
    if (node.current_retransmits[1] !== undefined) {
        col.innerHTML = formatRetransmitRaw(node.current_retransmits[1]);
    } else {
        col.textContent = "-";
    }
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "ecn-down-" + nodeId;
    col.style.width = "6%";
    if (node.current_marks[0] !== undefined) {
        col.innerHTML = formatCakeStatPercent(node.current_marks[0], node.current_packets[0]);
    } else {
        col.textContent = "-";
    }
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "ecn-up-" + nodeId;
    col.style.width = "6%";
    if (node.current_marks[1] !== undefined) {
        col.innerHTML = formatCakeStatPercent(node.current_marks[1], node.current_packets[1]);
    } else {
        col.textContent = "-";
    }
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "drops-down-" + nodeId;
    col.style.width = "6%";
    if (node.current_drops[0] !== undefined) {
        col.innerHTML = formatCakeStatPercent(node.current_drops[0], node.current_packets[0]);
    } else {
        col.textContent = "-";
    }
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "drops-up-" + nodeId;
    //col.style.width = "6%";
    if (node.current_drops[1] !== undefined) {
        col.innerHTML = formatCakeStat(node.current_drops[1], node.current_packets[1]);
    } else {
        col.textContent = "-";
    }
    row.appendChild(col);

    return row;
}

function treeUpdate(msg) {
    //console.log(msg);
    let needsRebuild = false;
    msg.data.forEach((n) => {
        let nodeId = n[0];
        let node = n[1];

        if (tree[nodeId] === undefined) {
            tree[nodeId] = [nodeId, node];
            needsRebuild = true;
        } else {
            if (tree[nodeId][1].immediate_parent !== node.immediate_parent) {
                needsRebuild = true;
            }
            tree[nodeId][1] = node;
        }

        if (nodeId === parent) {
            fillHeader(node);
        }

        let col = document.getElementById("down-" + nodeId);
        if (col !== null) {
            col.innerHTML = formatThroughput(toNumber(node.current_throughput[0], 0) * 8, node.max_throughput[0]);
        }
        col = document.getElementById("up-" + nodeId);
        if (col !== null) {
            col.innerHTML = formatThroughput(toNumber(node.current_throughput[1], 0) * 8, node.max_throughput[1]);
        }
        col = document.getElementById("rtt-down-" + nodeId);
        if (col !== null) {
            col.innerHTML = formatRtt(node.rtts[0]);
        }
        col = document.getElementById("rtt-up-" + nodeId);
        if (col !== null) {
            col.innerHTML = formatRtt(node.rtts[1]);
        }
        col = document.getElementById("qoo-down-" + nodeId);
        if (col !== null) {
            col.innerHTML = formatQooScore(node.qoo ? node.qoo[0] : null);
        }
        col = document.getElementById("qoo-up-" + nodeId);
        if (col !== null) {
            col.innerHTML = formatQooScore(node.qoo ? node.qoo[1] : null);
        }
        col = document.getElementById("re-xmit-down-" + nodeId);
        if (col !== null) {
            if (node.current_retransmits[0] !== undefined) {
                let retr = 0;
                const packetsDown = toNumber(node.current_tcp_packets[0], 0);
                if (packetsDown > 0) {
                    retr = toNumber(node.current_retransmits[0], 0) / packetsDown;
                }
                col.innerHTML = formatRetransmit(retr);
            } else {
                col.textContent = "-";
            }
        }
        col = document.getElementById("re-xmit-up-" + nodeId);
        if (col !== null) {
            if (node.current_retransmits[1] !== undefined) {
                let retr = 0;
                const packetsUp = toNumber(node.current_tcp_packets[1], 0);
                if (packetsUp > 0) {
                    retr = toNumber(node.current_retransmits[1], 0) / packetsUp;
                }
                col.innerHTML = formatRetransmit(retr);
            } else {
                col.textContent = "-";
            }
        }
        col = document.getElementById("ecn-down-" + nodeId);
        if (col !== null) {
            if (node.current_marks[0] !== undefined) {
                col.innerHTML = formatCakeStatPercent(node.current_marks[0], node.current_packets[0]);
            } else {
                col.textContent = "-";
            }
        }
        col = document.getElementById("ecn-up-" + nodeId);
        if (col !== null) {
            if (node.current_marks[1] !== undefined) {
                col.innerHTML = formatCakeStatPercent(node.current_marks[1], node.current_packets[1]);
            } else {
                col.textContent = "-";
            }
        }
        col = document.getElementById("drops-down-" + nodeId);
        if (col !== null) {
            if (node.current_drops[0] !== undefined) {
                col.innerHTML = formatCakeStatPercent(node.current_drops[0], node.current_packets[0]);
            } else {
                col.textContent = "-";
            }
        }
        col = document.getElementById("drops-up-" + nodeId);
        if (col !== null) {
            if (node.current_drops[1] !== undefined) {
                col.innerHTML = formatCakeStatPercent(node.current_drops[1], node.current_packets[1]);
            } else {
                col.textContent = "-";
            }
        }
    });
    if (needsRebuild) {
        buildChildrenMap();
        renderTree();
    }
}

function clientsUpdate(msg) {
    let myName = tree[parent][1].name;

    let target = document.getElementById("clients");
    let table = document.createElement("table");
    table.classList.add("table", "table-striped", "table-bordered");
    table.appendChild(clientTableHeader());
    let tbody = document.createElement("tbody");
    clearDiv(target);

    msg.data.forEach((device) => {
        if (device.parent_node === myName) {
            let tr = document.createElement("tr");
            tr.classList.add("small");
            let linkTd = document.createElement("td");
            let circuitLink = document.createElement("a");
            circuitLink.href = "/circuit.html?id=" + device.circuit_id;
            circuitLink.innerText = device.circuit_name;
            linkTd.appendChild(circuitLink);
            tr.appendChild(linkTd);
            tr.appendChild(simpleRow(device.device_name, true));
            tr.appendChild(simpleRow(device.plan.down + " / " + device.plan.up));
            tr.appendChild(simpleRow(device.parent_node));
            tr.appendChild(simpleRow(device.ip));
            tr.appendChild(simpleRow(formatLastSeen(device.last_seen_nanos)));
            tr.appendChild(simpleRowHtml(formatThroughput(toNumber(device.bytes_per_second.down, 0) * 8, device.plan.down)));
            tr.appendChild(simpleRowHtml(formatThroughput(toNumber(device.bytes_per_second.up, 0) * 8, device.plan.up)));
            if (device.median_latency !== null) {
                tr.appendChild(simpleRowHtml(formatRtt(device.median_latency.down)));
                tr.appendChild(simpleRowHtml(formatRtt(device.median_latency.up)));
            } else {
                tr.appendChild(simpleRow("-"));
                tr.appendChild(simpleRow("-"));
            }
            let retr = 0;
            const devicePacketsDown = toNumber(device.tcp_packets.down, 0);
            if (devicePacketsDown > 0) {
                retr = toNumber(device.tcp_retransmits.down, 0) / devicePacketsDown;
            }
            tr.appendChild(simpleRowHtml(formatRetransmit(retr)));
            retr = 0;
            const devicePacketsUp = toNumber(device.tcp_packets.up, 0);
            if (devicePacketsUp > 0) {
                retr = toNumber(device.tcp_retransmits.up, 0) / devicePacketsUp;
            }
            tr.appendChild(simpleRowHtml(formatRetransmit(retr)));

            // Add it
            tbody.appendChild(tr);
        }
    });
    table.appendChild(tbody);
    target.appendChild(table);
}

function onMessage(msg) {
    if (msg.event === "NetworkTree") {
        treeUpdate(msg);
    } else if (msg.event === "NetworkTreeClients") {
        clientsUpdate(msg);
    }
}

const params = new Proxy(new URLSearchParams(window.location.search), {
    get: (searchParams, prop) => searchParams.get(prop),
});

if (params.parent !== null) {
    parent = parseInt(params.parent);
} else {
    parent = 0;
}

if (params.upParent !== null) {
    upParent = parseInt(params.upParent);
}

getInitialTree();
