import {clearDiv, theading} from "./helpers/builders";
import {formatRtt, formatThroughput, lerpGreenToRedViaOrange, scaleNumber} from "./helpers/scaling";
import {subscribeWS} from "./pubsub/ws";

var tree = null;
var parent = 0;

// This runs first and builds the initial structure on the page
function getInitialTree() {
    $.get("/local-api/networkTree/0", (data) => {
        //console.log(data);
        tree = data;

        let treeTable = document.createElement("table");
        treeTable.classList.add("table", "table-striped", "table-bordered");
        let thead = document.createElement("thead");
        thead.appendChild(theading("Name"));
        thead.appendChild(theading("Limit"));
        thead.appendChild(theading("⬇️"));
        thead.appendChild(theading("⬆️"));
        thead.appendChild(theading("RTT ⬇️"));
        thead.appendChild(theading("RTT ⬆️"));
        thead.appendChild(theading("Re-xmit ⬇️"));
        thead.appendChild(theading("Re-xmit ⬆️"));
        thead.appendChild(theading("ECN ⬇️"));
        thead.appendChild(theading("ECN ⬆️"));
        thead.appendChild(theading("Drops ⬇️"));
        thead.appendChild(theading("Drops ⬆️"));

        treeTable.appendChild(thead);
        let tbody = document.createElement("tbody");
        for (let i=0; i<tree.length; i++) {
            let nodeId = tree[i][0];
            let node = tree[i][1];

            if (nodeId === parent) {
                $("#nodeName").text(node.name);
                let limit = "";
                if (node.max_throughput[0] === 0) {
                    limit = "Unlimited";
                } else {
                    limit = scaleNumber(node.max_throughput[0] * 1000 * 1000, 0);
                }
                limit += " / ";
                if (node.max_throughput[1] === 0) {
                    limit += "Unlimited";
                } else {
                    limit += scaleNumber(node.max_throughput[1] * 1000 * 1000, 0);
                }
                $("#parentLimits").text(limit);
                $("#parentTpD").html(formatThroughput(node.current_throughput[0] * 8, node.max_throughput[0]));
                $("#parentTpU").html(formatThroughput(node.current_throughput[1] * 8, node.max_throughput[1]));
                console.log(node);
                $("#parentRttD").html(formatRtt(node.rtts[0]));
                $("#parentRttU").html(formatRtt(node.rtts[1]));
            }

            if (node.immediate_parent !== null && node.immediate_parent === parent) {
                let row = buildRow(i);
                tbody.appendChild(row);
                iterateChildren(i, tbody, 1);
            }
        }
        treeTable.appendChild(tbody);

        // Clear and apply
        let target = document.getElementById("tree");
        clearDiv(target)
        target.appendChild(treeTable);

        subscribeWS(["NetworkTree"], onMessage);
    });
}

function iterateChildren(idx, tBody, depth) {
    for (let i=0; i<tree.length; i++) {
        let node = tree[i][1];
        if (node.immediate_parent !== null && node.immediate_parent === tree[idx][0]) {
            let row = buildRow(i, depth);
            tBody.appendChild(row);
            iterateChildren(i, tBody, depth+1);
        }
    }
}

function buildRow(i, depth=0) {
    let node = tree[i][1];
    let nodeId = tree[i][0];
    let row = document.createElement("tr");
    row.classList.add("small");
    let col = document.createElement("td");
    let nodeName = "";
    if (depth > 0) {
        nodeName += "└";
    }
    for (let j=1; j<depth; j++) {
        nodeName += "─";
    }
    if (depth > 0) nodeName += " ";
    nodeName += "<a href='/tree.html?parent=" + nodeId + "'>";
    nodeName += node.name;
    nodeName += "</a>";
    if (node.type !== null) {
        nodeName += " (" + node.type + ")";
    }
    col.innerHTML = nodeName;
    col.classList.add("small", "redactable");
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "limit-" + nodeId;
    col.classList.add("small");
    let limit = "";
    if (node.max_throughput[0] === 0) {
        limit = "Unlimited";
    } else {
        limit = scaleNumber(node.max_throughput[0] * 1000 * 1000, 0);
    }
    limit += " / ";
    if (node.max_throughput[1] === 0) {
        limit += "Unlimited";
    } else {
        limit += scaleNumber(node.max_throughput[0] * 1000 * 1000, 0);
    }
    col.textContent = limit;
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "down-" + nodeId;
    col.classList.add("small");
    col.innerHTML = formatThroughput(node.current_throughput[0] * 8, node.max_throughput[0]);
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "up-" + nodeId;
    col.classList.add("small");
    col.innerHTML = formatThroughput(node.current_throughput[1] * 8, node.max_throughput[1]);
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "rtt-down-" + nodeId;
    col.innerHTML = formatRtt(node.rtts[0]);
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "rtt-up-" + nodeId;
    col.innerHTML = formatRtt(node.rtts[1]);
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "re-xmit-down-" + nodeId;
    if (node.current_retransmits[0] !== undefined) {
        col.textContent = node.current_retransmits[0];
    } else {
        col.textContent = "-";
    }
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "re-xmit-up-" + nodeId;
    if (node.current_retransmits[1] !== undefined) {
        col.textContent = node.current_retransmits[1];
    } else {
        col.textContent = "-";
    }
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "ecn-down-" + nodeId;
    if (node.current_marks[0] !== undefined) {
        col.textContent = node.current_marks[0];
    } else {
        col.textContent = "-";
    }
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "ecn-up-" + nodeId;
    if (node.current_marks[1] !== undefined) {
        col.textContent = node.current_marks[1];
    } else {
        col.textContent = "-";
    }
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "drops-down-" + nodeId;
    if (node.current_drops[0] !== undefined) {
        col.textContent = node.current_drops[0];
    } else {
        col.textContent = "-";
    }
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "drops-up-" + nodeId;
    if (node.current_drops[1] !== undefined) {
        col.textContent = node.current_drops[1];
    } else {
        col.textContent = "-";
    }
    row.appendChild(col);

    return row;
}

function onMessage(msg) {
    if (msg.event === "NetworkTree") {
        //console.log(msg);
        msg.data.forEach((n) => {
            let nodeId = n[0];
            let node = n[1];

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
                    col.textContent = node.current_retransmits[0];
                } else {
                    col.textContent = "-";
                }
            }
            col = document.getElementById("re-xmit-up-" + nodeId);
            if (col !== null) {
                if (node.current_retransmits[1] !== undefined) {
                    col.textContent = node.current_retransmits[1];
                } else {
                    col.textContent = "-";
                }
            }
            col = document.getElementById("ecn-down-" + nodeId);
            if (col !== null) {
                if (node.current_marks[0] !== undefined) {
                    col.textContent = node.current_marks[0];
                } else {
                    col.textContent = "-";
                }
            }
            col = document.getElementById("ecn-up-" + nodeId);
            if (col !== null) {
                if (node.current_marks[1] !== undefined) {
                    col.textContent = node.current_marks[1];
                } else {
                    col.textContent = "-";
                }
            }
            col = document.getElementById("drops-down-" + nodeId);
            if (col !== null) {
                if (node.current_drops[0] !== undefined) {
                    col.textContent = node.current_drops[0];
                } else {
                    col.textContent = "-";
                }
            }
            col = document.getElementById("drops-up-" + nodeId);
            if (col !== null) {
                if (node.current_drops[1] !== undefined) {
                    col.textContent = node.current_drops[1];
                } else {
                    col.textContent = "-";
                }
            }
        });
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

getInitialTree();
