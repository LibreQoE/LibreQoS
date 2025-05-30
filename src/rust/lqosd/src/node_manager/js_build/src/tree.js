import {clearDiv, clientTableHeader, formatLastSeen, simpleRow, simpleRowHtml, theading} from "./helpers/builders";
import {
    formatCakeStat, formatCakeStatPercent,
    formatRetransmit, formatRetransmitRaw,
    formatRtt,
    formatThroughput,
} from "./helpers/scaling";
import {scaleNumber} from "./lq_js_common/helpers/scaling";
import {subscribeWS} from "./pubsub/ws";

var tree = null;
var parent = 0;
var upParent = 0;
var maxDepth = 1;
var subscribed = false;

// This runs first and builds the initial structure on the page
function getInitialTree() {
    $.get("/local-api/networkTree", (data) => {
        //console.log(data);
        tree = data;

        let treeTable = document.createElement("table");
        treeTable.classList.add("table", "table-striped", "table-bordered");
        let thead = document.createElement("thead");
        thead.appendChild(theading("Name"));
        thead.appendChild(theading("Limit"));
        thead.appendChild(theading("⬇️"));
        thead.appendChild(theading("⬆️"));
        thead.appendChild(theading("RTT", 2, "<h5>TCP Round-Trip Time</h5><p>Current median TCP round-trip time. Time taken for a full send-acknowledge round trip. Low numbers generally equate to a smoother user experience.</p>", "tts_retransmits"));
        thead.appendChild(theading("Retr", 2, "<h5>TCP Retransmits</h5><p>Number of TCP retransmits in the last second.</p>", "tts_retransmits"));
        thead.appendChild(theading("Marks", 2, "<h5>Cake Marks</h5><p>Number of times the Cake traffic manager has applied ECN marks to avoid congestion.</p>", "tts_marks"));
        thead.appendChild(theading("Drops", 2, "<h5>Cake Drops</h5><p>Number of times the Cake traffic manager has dropped packets to avoid congestion.</p>", "tts_drops"));

        treeTable.appendChild(thead);
        let tbody = document.createElement("tbody");

        for (let i=0; i<tree.length; i++) {
            let nodeId = tree[i][0];
            let node = tree[i][1];

            if (nodeId === parent) {
                fillHeader(node)
            }

            if (node.immediate_parent !== null && node.immediate_parent === parent) {
                let row = buildRow(i);
                tbody.appendChild(row);
                if (maxDepth > 1) {
                    iterateChildren(i, tbody, 1);
                }
            }
        }

        if (parent !== 0) {
            let row = document.createElement("tr");
            let col = document.createElement("td");
            col.colSpan = 12;
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

        if (!subscribed) {
            subscribeWS(["NetworkTree", "NetworkTreeClients"], onMessage);
            subscribed = true;
        }
    });
}

function fillHeader(node) {
    //console.log("Header");
    $("#nodeName").text(node.name);
    let limitD = "";
    if (node.max_throughput[0] === 0) {
        limitD = "Unlimited";
    } else {
        limitD = scaleNumber(node.max_throughput[0] * 1000 * 1000, 1);
    }
    let limitU = "";
    if (node.max_throughput[1] === 0) {
        limitU = "Unlimited";
    } else {
        limitU = scaleNumber(node.max_throughput[1] * 1000 * 1000, 1);
    }
    $("#parentLimitsD").text(limitD);
    $("#parentLimitsU").text(limitU);
    $("#parentTpD").html(formatThroughput(node.current_throughput[0] * 8, node.max_throughput[0]));
    $("#parentTpU").html(formatThroughput(node.current_throughput[1] * 8, node.max_throughput[1]));
    //console.log(node);
    $("#parentRttD").html(formatRtt(node.rtts[0]));
    $("#parentRttU").html(formatRtt(node.rtts[1]));
    let retr = 0;
    if (node.current_tcp_packets[0] > 0) {
        retr = node.current_retransmits[0] / node.current_tcp_packets[0];
    }
    $("#parentRxmitD").html(formatRetransmit(retr));
    retr = 0;
    if (node.current_tcp_packets[1] > 0) {
        retr = node.current_retransmits[1] / node.current_tcp_packets[1];
    }
    $("#parentRxmitU").html(formatRetransmit(retr));
}

function iterateChildren(idx, tBody, depth) {
    for (let i=0; i<tree.length; i++) {
        let node = tree[i][1];
        if (node.immediate_parent !== null && node.immediate_parent === tree[idx][0]) {
            let row = buildRow(i, depth);
            tBody.appendChild(row);
            if (depth < maxDepth-1) {
                iterateChildren(i, tBody, depth + 1);
            }
        }
    }
}

function buildRow(i, depth=0) {
    let node = tree[i][1];
    let nodeId = tree[i][0];
    let row = document.createElement("tr");
    row.classList.add("small");
    let col = document.createElement("td");
    col.style.textOverflow = "ellipsis";
    let nodeName = "";
    if (depth > 0) {
        nodeName += "└";
    }
    for (let j=1; j<depth; j++) {
        nodeName += "─";
    }
    if (depth > 0) nodeName += " ";
    nodeName += "<a href='/tree.html?parent=" + nodeId + "&upParent=" + parent + "' class='redactable'>";
    nodeName += node.name;
    nodeName += "</a>";
    if (node.type !== null) {
        nodeName += " (" + node.type + ")";
    }
    col.innerHTML = nodeName;
    col.classList.add("small");
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "limit-" + nodeId;
    col.classList.add("small");
    col.style.width = "8%";
    let limit = "";
    if (node.max_throughput[0] === 0) {
        limit = "Unlimited";
    } else {
        limit = scaleNumber(node.max_throughput[0] * 1000 * 1000, 1);
    }
    limit += " / ";
    if (node.max_throughput[1] === 0) {
        limit += "Unlimited";
    } else {
        limit += scaleNumber(node.max_throughput[1] * 1000 * 1000, 1);
    }
    col.textContent = limit;
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "down-" + nodeId;
    col.classList.add("small");
    col.style.width = "6%";
    col.innerHTML = formatThroughput(node.current_throughput[0] * 8, node.max_throughput[0]);
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "up-" + nodeId;
    col.classList.add("small");
    col.style.width = "6%";
    col.innerHTML = formatThroughput(node.current_throughput[1] * 8, node.max_throughput[1]);
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
    msg.data.forEach((n) => {
        let nodeId = n[0];
        let node = n[1];

        if (nodeId === parent) {
            fillHeader(node);
        }

        let col = document.getElementById("down-" + nodeId);
        if (col !== null) {
            col.innerHTML = formatThroughput(node.current_throughput[0] * 8, node.max_throughput[0]);
        }
        col = document.getElementById("up-" + nodeId);
        if (col !== null) {
            col.innerHTML = formatThroughput(node.current_throughput[1] * 8, node.max_throughput[1]);
        }
        col = document.getElementById("rtt-down-" + nodeId);
        if (col !== null) {
            col.innerHTML = formatRtt(node.rtts[0]);
        }
        col = document.getElementById("rtt-up-" + nodeId);
        if (col !== null) {
            col.innerHTML = formatRtt(node.rtts[1]);
        }
        col = document.getElementById("re-xmit-down-" + nodeId);
        if (col !== null) {
            if (node.current_retransmits[0] !== undefined) {
                let retr = 0;
                if (node.current_tcp_packets[0] > 0) {
                    retr = node.current_retransmits[0] / node.current_tcp_packets[0];
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
                if (node.current_tcp_packets[1] > 0) {
                    retr = node.current_retransmits[1] / node.current_tcp_packets[1];
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
            tr.appendChild(simpleRowHtml(formatThroughput(device.bytes_per_second.down * 8, device.plan.down)));
            tr.appendChild(simpleRowHtml(formatThroughput(device.bytes_per_second.up * 8, device.plan.up)));
            if (device.median_latency !== null) {
                tr.appendChild(simpleRowHtml(formatRtt(device.median_latency.down)));
                tr.appendChild(simpleRowHtml(formatRtt(device.median_latency.up)));
            } else {
                tr.appendChild(simpleRow("-"));
                tr.appendChild(simpleRow("-"));
            }
            let retr = 0;
            if (device.tcp_packets.down > 0) {
                retr = device.tcp_retransmits.down / device.tcp_packets.down;
            }
            tr.appendChild(simpleRowHtml(formatRetransmit(retr)));
            retr = 0;
            if (device.tcp_packets.up > 0) {
                retr = device.tcp_retransmits.up / device.tcp_packets.up;
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

if (localStorage.getItem("treeMaxDepth") !== null) {
    maxDepth = parseInt(localStorage.getItem("treeMaxDepth"));
    $("#maxDepth").val(maxDepth);
}

$("#maxDepth").on("change", function() {
    maxDepth = parseInt($(this).val());
    localStorage.setItem("treeMaxDepth", maxDepth);
    getInitialTree();
});

getInitialTree();
