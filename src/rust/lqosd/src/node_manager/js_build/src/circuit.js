// Obtain URL parameters
import {DirectChannel} from "./pubsub/direct_channels";
import {clearDiv, formatLastSeen, simpleRow, simpleRowHtml, theading} from "./helpers/builders";
import {formatRetransmit, formatRtt, formatThroughput, lerpGreenToRedViaOrange} from "./helpers/scaling";
import {BitsPerSecondGauge} from "./graphs/bits_gauge";
import {CircuitTotalGraph} from "./graphs/circuit_throughput_graph";
import {CircuitRetransmitGraph} from "./graphs/circuit_retransmit_graph";
import {scaleNanos, scaleNumber} from "./helpers/scaling";
import {DevicePingHistogram} from "./graphs/device_ping_graph";
import {FlowsSankey} from "./graphs/flow_sankey";
import {subscribeWS} from "./pubsub/ws";
import {CakeBacklog} from "./graphs/cake_backlog";
import {CakeDelays} from "./graphs/cake_delays";
import {CakeQueueLength} from "./graphs/cake_queue_length";
import {CakeTraffic} from "./graphs/cake_traffic";
import {CakeMarks} from "./graphs/cake_marks";
import {CakeDrops} from "./graphs/cake_drops";

const params = new Proxy(new URLSearchParams(window.location.search), {
    get: (searchParams, prop) => searchParams.get(prop),
});

let circuit_id = decodeURI(params.id);
let plan = null;
let channelLink = null;
let pinger = null;
let flowChannel = null;
let speedometer = null;
let totalThroughput = null;
let totalRetransmits = null;
let deviceGraphs = {};
let devicePings = [];
let flowSankey = null;
let funnelGraphs = {};
let funnelParents = [];

function connectPrivateChannel() {
    channelLink = new DirectChannel({
        CircuitWatcher: {
            circuit: circuit_id
        }
    }, (msg) => {
        if (msg.devices !== null) {
            //console.log(msg.devices);
            fillLiveDevices(msg.devices);
            updateSpeedometer(msg.devices);
        }
    });
}

function connectPingers(circuits) {
    let ipList = [];
    circuits.forEach((circuit) => {
        circuit.ipv4.forEach((ip) => {
            ipList.push([ip[0], circuit.device_id]);
        });
        circuit.ipv6.forEach((ip) => {
            ipList.push([ip[0], circuit.device_id]);
        });
    });

    pinger = new DirectChannel({
        PingMonitor: {
            ips: ipList
        }
    },(msg) => {
        //console.log(msg);
        if (msg.ip != null && msg.ip !== "test") {
            // Stats Updates
            if (devicePings[msg.ip] === undefined) {
                devicePings[msg.ip] = {
                    count: 0,
                    timeout: 0,
                    success: 0,
                    times: [],
                }
            }

            devicePings[msg.ip].count++;
            if (msg.result === "NoResponse") {
                devicePings[msg.ip].timeout++;
            } else {
                devicePings[msg.ip].success++;
                devicePings[msg.ip].times.push(msg.result.Ping.time_nanos);
                if (devicePings[msg.ip].times.length > 300) {
                    devicePings[msg.ip].times.shift();
                }
                let graphId = "pingGraph_" + msg.result.Ping.label;
                let graph = deviceGraphs[graphId];
                if (graph !== undefined) {
                    graph.update(msg.result.Ping.time_nanos);
                }
            }

            // Visual Updates
            let target = document.getElementById("ip_" + msg.ip);
            if (target != null) {
                let myPing = devicePings[msg.ip];
                if (myPing.count === myPing.timeout) {
                    target.innerHTML = "<i class='fa fa-ban text-danger'></i>";
                } else {
                    let loss = ((myPing.timeout / myPing.count) * 100);
                    let lossStr = loss.toFixed(1);
                    let avg = 0;
                    myPing.times.forEach((time) => {
                        avg += time;
                    });
                    avg = avg / myPing.times.length;
                    let lossColor = "text-success";
                    if (loss > 0 && loss < 10) {
                        lossColor = "text-warning";
                    } else if (loss >= 10) {
                        lossColor = "text-danger";
                    }
                    let pingRamp = Math.min(avg / 200, 1);
                    let pingColor = lerpGreenToRedViaOrange(pingRamp, 1);
                    target.innerHTML = "<i class='fa fa-check text-success'></i> <span class='tiny'><span class='" + lossColor + "'>" + lossStr + "%</span> / <span style='color: " + pingColor + "'>" + scaleNanos(avg) + "</span></span>";
                }
            }
        }
    });
}

function connectFlowChannel() {
    flowChannel = new DirectChannel({
        FlowsByCircuit: {
            circuit: circuit_id
        }
    }, (msg) => {
        //console.log(msg);
        let activeFlows = flowSankey.update(msg);
        flowSankey.chart.resize();
        $("#activeFlowCount").text(activeFlows);
        updateTrafficTab(msg);
    });
}

function updateTrafficTab(msg) {
    let target = document.getElementById("allTraffic");

    let table = document.createElement("table");
    table.classList.add("table", "table-sm", "table-striped");
    let thead = document.createElement("thead");
    thead.appendChild(theading("Protocol"));
    thead.appendChild(theading("Current Rate", 2));
    thead.appendChild(theading("Total Bytes", 2));
    thead.appendChild(theading("Total Packets", 2));
    thead.appendChild(theading("TCP Retransmits", 2));
    thead.appendChild(theading("RTT", 2));
    thead.appendChild(theading("ASN"));
    thead.appendChild(theading("Country"));
    thead.appendChild(theading("Remote IP"));
    table.appendChild(thead);
    let tbody = document.createElement("tbody");
    const one_second_in_nanos = 1000000000; // For display filtering

    // Sort msg.flows by flows[0].rate_estimate_bps.down + flows[0].rate_estimate_bps.up descending
    msg.flows.sort((a, b) => {
        let aRate = a[1].rate_estimate_bps.down + a[1].rate_estimate_bps.up;
        let bRate = b[1].rate_estimate_bps.down + b[1].rate_estimate_bps.up;
        return bRate - aRate;
    });

    msg.flows.forEach((flow) => {
        if (flow[0].last_seen_nanos > one_second_in_nanos) return;
        let row = document.createElement("tr");
        row.classList.add("small");
        row.appendChild(simpleRow(flow[0].protocol_name));
        row.appendChild(simpleRowHtml(formatThroughput(flow[1].rate_estimate_bps.down * 8, plan.down)));
        row.appendChild(simpleRowHtml(formatThroughput(flow[1].rate_estimate_bps.up * 8, plan.up)));
        row.appendChild(simpleRow(scaleNumber(flow[1].bytes_sent.down)));
        row.appendChild(simpleRow(scaleNumber(flow[1].bytes_sent.up)));
        row.appendChild(simpleRow(scaleNumber(flow[1].packets_sent.down)));
        row.appendChild(simpleRow(scaleNumber(flow[1].packets_sent.up)));
        row.appendChild(simpleRowHtml(formatRetransmit(flow[1].tcp_retransmits.down)));
        row.appendChild(simpleRowHtml(formatRetransmit(flow[1].tcp_retransmits.up)));
        row.appendChild(simpleRow(scaleNanos(flow[1].rtt[0].nanoseconds)));
        row.appendChild(simpleRow(scaleNanos(flow[1].rtt[1].nanoseconds)));
        row.appendChild(simpleRow(flow[0].asn_name));
        row.appendChild(simpleRow(flow[0].asn_country));
        row.appendChild(simpleRow(flow[0].remote_ip));

        tbody.appendChild(row);
    });

    table.appendChild(tbody);

    clearDiv(target);
    target.appendChild(table);
}

function updateSpeedometer(devices) {
    let totalDown = 0;
    let totalUp = 0;
    let planDown = 0;
    let planUp = 0;
    let retransmitsDown = 0;
    let retransmitsUp = 0;
    devices.forEach((device) => {
        totalDown += device.bytes_per_second.down;
        totalUp += device.bytes_per_second.up;
        planDown = Math.max(planDown, device.plan.down);
        planUp = Math.max(planUp, device.plan.up);
        retransmitsDown += device.tcp_retransmits.down;
        retransmitsUp += device.tcp_retransmits.up;

        let throughputGraph = deviceGraphs["throughputGraph_" + device.device_id];
        if (throughputGraph !== undefined) {
            throughputGraph.update(device.bytes_per_second.down * 8, device.bytes_per_second.up * 8);
        }

        let retransmitGraph = deviceGraphs["tcpRetransmitsGraph_" + device.device_id];
        if (retransmitGraph !== undefined) {
            retransmitGraph.update(device.tcp_retransmits.down, device.tcp_retransmits.up);
        }
    });
    speedometer.update(totalDown * 8, totalUp * 8, planDown, planUp);
    totalThroughput.update(totalDown * 8, totalUp * 8);
    totalRetransmits.update(retransmitsDown, retransmitsUp);
}

function fillLiveDevices(devices) {
    devices.forEach((device) => {
        let last_seen = document.getElementById("last_seen_" + device.device_id);
        let throughputDown = document.getElementById("throughputDown_" + device.device_id);
        let throughputUp = document.getElementById("throughputUp_" + device.device_id);
        let rttDown = document.getElementById("rttDown_" + device.device_id);
        let rttUp = document.getElementById("rttUp_" + device.device_id);
        let tcp_retransmitsDown = document.getElementById("tcp_retransmitsDown_" + device.device_id);
        let tcp_retransmitsUp = document.getElementById("tcp_retransmitsUp_" + device.device_id);

        if (last_seen !== null) {
            last_seen.innerHTML = formatLastSeen(device.last_seen_nanos);
        }

        if (throughputDown !== null) {
            throughputDown.innerHTML = formatThroughput(device.bytes_per_second.down * 8, device.plan.down);
        }

        if (throughputUp !== null) {
            throughputUp.innerHTML = formatThroughput(device.bytes_per_second.up * 8, device.plan.up);
        }

        if (rttDown !== null) {
            if (device.median_latency !== null) {
                rttDown.innerHTML = formatRtt(device.median_latency);
            }
        }

        if (rttUp !== null) {
            if (device.median_latency !== null && device.median_latency.up !== null) {
                rttUp.innerHTML = formatRtt(device.median_latency.up);
            }
        }

        if (tcp_retransmitsDown !== null) {
            tcp_retransmitsDown.innerHTML = formatRetransmit(device.tcp_retransmits.down);
        }

        if (tcp_retransmitsUp !== null) {
            tcp_retransmitsUp.innerHTML = formatRetransmit(device.tcp_retransmits.up);
        }
    });
}

function initialDevices(circuits) {
    let target = document.getElementById("devices")
    clearDiv(target);

    circuits.forEach((circuit) => {
        let d = document.createElement("div");
        d.classList.add("col-3");

        // Device Information Section

        let name = document.createElement("h5");
        name.innerHTML = "<i class='fa fa-computer'></i> " + circuit.device_name;
        d.appendChild(name);

        let infoTable = document.createElement("table");
        infoTable.classList.add("table", "table-sm", "table-striped");
        let tbody = document.createElement("tbody");

        // MAC Row
        let tr = document.createElement("tr");
        let td = document.createElement("td");
        td.innerHTML = "<b>MAC Address</b>";
        tr.appendChild(td);
        td = document.createElement("td");
        td.colSpan = 2;
        td.innerHTML = circuit.mac;
        tr.appendChild(td);
        tbody.appendChild(tr);

        // Comment Row
        let tr2 = document.createElement("tr");
        td = document.createElement("td");
        td.innerHTML = "<b>Comment</b>";
        tr2.appendChild(td);
        td = document.createElement("td");
        td.colSpan = 2;
        td.innerHTML = circuit.comment;
        tr2.appendChild(td);
        tbody.appendChild(tr2);

        // IPv4 Row
        let tr3 = document.createElement("tr");
        td = document.createElement("td");
        td.innerHTML = "<b>IPv4 Address(es)</b>";
        tr3.appendChild(td);
        td = document.createElement("td");
        td.colSpan = 2;
        let ipv4Table = document.createElement("table");
        ipv4Table.classList.add("table", "table-sm");
        let ipv4Body = document.createElement("tbody");
        circuit.ipv4.forEach((ip) => {
            let tr = document.createElement("tr");
            let label = document.createElement("td");
            label.innerHTML = ip[0] + "/" + ip[1];
            label.classList.add("small");
            tr.appendChild(label);
            let value = document.createElement("td");
            value.id = "ip_" + ip[0];
            value.innerText = "-";
            tr.appendChild(value);
            ipv4Body.appendChild(tr);
        });
        if (circuit.ipv4.length === 0) {
            let tr = document.createElement("tr");
            let label = document.createElement("td");
            label.innerHTML = "No IPv4 addresses assigned";
            tr.appendChild(label);
            ipv4Body.appendChild(tr);
        }
        ipv4Table.appendChild(ipv4Body);
        td.appendChild(ipv4Table);

        tr3.appendChild(td);
        tbody.appendChild(tr3);

        // IPv6 Row
        let tr4 = document.createElement("tr");
        td = document.createElement("td");
        td.innerHTML = "<b>IPv6 Address(es)</b>";
        tr4.appendChild(td);
        td = document.createElement("td");
        td.colSpan = 2;

        let ipv6 = document.createElement("table");
        ipv6.classList.add("table", "table-sm");
        let ipv6Body = document.createElement("tbody");
        circuit.ipv6.forEach((ip) => {
            let tr = document.createElement("tr");
            let label = document.createElement("td");
            label.innerHTML = ip[0] + "/" + ip[1];
            label.classList.add("small");
            tr.appendChild(label);
            let value = document.createElement("td");
            value.id = "ip_" + ip[0];
            value.innerText = "-";
            tr.appendChild(value);
            ipv6Body.appendChild(tr);
        });
        if (circuit.ipv6.length === 0) {
            let tr = document.createElement("tr");
            let label = document.createElement("td");
            label.innerHTML = "No IPv6 addresses assigned";
            tr.appendChild(label);
            ipv6Body.appendChild(tr);
        }
        ipv6.appendChild(ipv6Body);
        td.appendChild(ipv6);

        /*let ipv6 = "";
        circuit.ipv6.forEach((ip) => {
            ipv6 += ip[0] + "/" + ip[1] + "<br>";
        });
        if (ipv6 === "") ipv6 = "No IPv6 addresses assigned";
        td.innerHTML = ipv6;*/
        tr4.appendChild(td);
        tbody.appendChild(tr4);

        // Placeholder for Last Seen
        let tr8 = document.createElement("tr");
        td = document.createElement("td");
        td.innerHTML = "<b>Last Seen</b>";
        tr8.appendChild(td);
        td = document.createElement("td");
        td.colSpan = 2;
        td.id = "last_seen_" + circuit.device_id;
        td.innerHTML = "<i class='fa fa-spinner fa-spin'></i> Loading...";
        tr8.appendChild(td);
        tbody.appendChild(tr8);

        // Placeholder for throughput
        let tr5 = document.createElement("tr");
        td = document.createElement("td");
        td.innerHTML = "<b>Throughput</b>";
        tr5.appendChild(td);
        td = document.createElement("td");
        td.id = "throughputDown_" + circuit.device_id;
        td.innerHTML = "<i class='fa fa-spinner fa-spin'></i> Loading...";
        tr5.appendChild(td);
        td = document.createElement("td");
        td.id = "throughputUp_" + circuit.device_id;
        td.innerHTML = "<i class='fa fa-spinner fa-spin'></i> Loading...";
        tr5.appendChild(td);
        tbody.appendChild(tr5);

        // Placeholder for RTT
        let tr6 = document.createElement("tr");
        td = document.createElement("td");
        td.innerHTML = "<b>RTT</b>";
        tr6.appendChild(td);
        td = document.createElement("td");
        td.colSpan = 2;
        td.id = "rttDown_" + circuit.device_id;
        td.innerHTML = "Sampling...";
        tr6.appendChild(td);
        tbody.appendChild(tr6);

        // Placeholder for TCP Retransmits
        let tr7 = document.createElement("tr");
        td = document.createElement("td");
        td.innerHTML = "<b>TCP Re-Xmits</b>";
        tr7.appendChild(td);
        td = document.createElement("td");
        td.id = "tcp_retransmitsDown_" + circuit.device_id;
        td.innerHTML = "<i class='fa fa-spinner fa-spin'></i> Loading...";
        tr7.appendChild(td);
        td = document.createElement("td");
        td.id = "tcp_retransmitsUp_" + circuit.device_id;
        td.innerHTML = "<i class='fa fa-spinner fa-spin'></i> Loading...";
        tr7.appendChild(td);
        tbody.appendChild(tr7);

        infoTable.appendChild(tbody);
        d.appendChild(infoTable);
        target.appendChild(d);

        // Graph for Throughput
        let throughputGraph = document.createElement("div");
        throughputGraph.classList.add("col-3")
        throughputGraph.id = "throughputGraph_" + circuit.device_id;
        throughputGraph.style.height = "250px";
        throughputGraph.innerHTML = "<i class='fa fa-spinner fa-spin'></i> Loading...";
        target.appendChild(throughputGraph);
        deviceGraphs[throughputGraph.id] = new CircuitTotalGraph(throughputGraph.id, circuit.device_name + " Throughput");

        // Graph for TCP Retransmits
        let tcpRetransmitsGraph = document.createElement("div");
        tcpRetransmitsGraph.classList.add("col-3")
        tcpRetransmitsGraph.id = "tcpRetransmitsGraph_" + circuit.device_id;
        tcpRetransmitsGraph.style.height = "250px";
        tcpRetransmitsGraph.innerHTML = "<i class='fa fa-spinner fa-spin'></i> Loading...";
        target.appendChild(tcpRetransmitsGraph);
        deviceGraphs[tcpRetransmitsGraph.id] = new CircuitRetransmitGraph(tcpRetransmitsGraph.id, circuit.device_name + " Retransmits");

        // Ping Graph Section
        let pingGraph = document.createElement("div");
        pingGraph.classList.add("col-3");
        pingGraph.id = "pingGraph_" + circuit.device_id;
        pingGraph.style.height = "250px";
        pingGraph.innerHTML = "<i class='fa fa-spinner fa-spin'></i> Loading...";
        target.appendChild(pingGraph);
        deviceGraphs[pingGraph.id] = new DevicePingHistogram(pingGraph.id);

    });
}

function initialFunnel(parentNode) {
    let target = document.getElementById("theFunnel");
    $.get("/local-api/networkTree", (data) => {
        let immediateParent = null;
        data.forEach((node) => {
            if (node[1].name === parentNode) {
                immediateParent = node[1];
            }
        });

        if (immediateParent === null) {
            clearDiv(target);
            target.appendChild(document.createTextNode("No parent node found"));
            return;
        }

        let parentDiv = document.createElement("div");
        immediateParent.parents.reverse().forEach((parent) => {
            //console.log(data[parent]);
            let row = document.createElement("div");
            row.classList.add("row");

            let col = document.createElement("div");
            col.classList.add("col-12");
            let heading = document.createElement("h5");
            heading.innerHTML = "<i class='fa fa-sitemap'></i> " + data[parent][1].name;
            col.appendChild(heading);
            row.appendChild(col);
            parentDiv.appendChild(row);

            // Row for graphs
            row = document.createElement("div");
            row.classList.add("row");

            let col_tp = document.createElement("div");
            col_tp.classList.add("col-4");
            col_tp.id = "funnel_tp_" + parent;
            col_tp.style.height = "250px";
            row.appendChild(col_tp);

            let col_rxmit = document.createElement("div");
            col_rxmit.classList.add("col-4");
            col_rxmit.id = "funnel_rxmit_" + parent;
            row.appendChild(col_rxmit);

            let col_rtt = document.createElement("div");
            col_rtt.classList.add("col-4");
            col_rtt.id = "funnel_rtt_" + parent;
            row.appendChild(col_rtt);

            parentDiv.appendChild(row);
        });
        clearDiv(target);
        target.appendChild(parentDiv);
        // Ugly hack to defer until the DOM is updated
        requestAnimationFrame(() => {setTimeout(() => {
            immediateParent.parents.reverse().forEach((parent) => {
                let tpGraph = new CircuitTotalGraph("funnel_tp_" + parent, data[parent][1].name + " Throughput");
                let rxmitGraph = new CircuitRetransmitGraph("funnel_rxmit_" + parent, data[parent][1].name + " Retransmits");
                let rttGraph = new DevicePingHistogram("funnel_rtt_" + parent);
                funnelGraphs[parent] = {
                    tp: tpGraph,
                    rxmit: rxmitGraph,
                    rtt: rttGraph,
                };
            });
            funnelParents = immediateParent.parents;
            subscribeWS(["NetworkTree"], onTreeEvent);
        }, 0)});
    });
}

function onTreeEvent(msg) {
    //console.log(msg);
    funnelParents.forEach((parent) => {
        if (msg.event !== "NetworkTree") return;
        let myMessage = msg.data[parent][1];
        if (myMessage === undefined) return;
        let tpGraph = funnelGraphs[parent].tp;
        let rxmitGraph = funnelGraphs[parent].rxmit;
        let rttGraph = funnelGraphs[parent].rtt;

        tpGraph.update(myMessage.current_throughput[0] * 8, myMessage.current_throughput[0] *8);
        rxmitGraph.update(myMessage.current_retransmits[0], myMessage.current_retransmits[1]);
        myMessage.rtts.forEach((rtt) => {
            rttGraph.updateMs(rtt);
        });
        tpGraph.chart.resize();
        rxmitGraph.chart.resize();
        rttGraph.chart.resize();
    });
}

function subscribeToCake() {
    let backlogGraph = new CakeBacklog("cakeBacklog");
    let delaysGraph = new CakeDelays("cakeDelays");
    let queueLength = new CakeQueueLength("cakeQueueLength");
    let traffic = new CakeTraffic("cakeTraffic");
    let marks = new CakeMarks("cakeMarks");
    let drops = new CakeDrops("cakeDrops");
    channelLink = new DirectChannel({
        CakeWatcher: {
            circuit: circuit_id
        }
    }, (msg) => {
        //console.log(msg);

        // Cake Memory Usage
        $("#cakeQueueMemory").text(scaleNumber(msg.current_download.memory_used) + " / " + scaleNumber(msg.current_upload.memory_used));
        backlogGraph.update(msg);
        backlogGraph.chart.resize();
        delaysGraph.update(msg);
        delaysGraph.chart.resize();
        queueLength.update(msg);
        queueLength.chart.resize();
        traffic.update(msg);
        traffic.chart.resize();
        marks.update(msg);
        marks.chart.resize();
        drops.update(msg);
        drops.chart.resize();
    });
}

function loadInitial() {
    $.ajax({
        type: "POST",
        url: "/local-api/circuitById",
        data: JSON.stringify({ id: circuit_id }),
        contentType: 'application/json',
        success: (circuits) => {
            //console.log(circuits);
            let circuit = circuits[0];
            $("#circuitName").text(circuit.circuit_name);
            $("#parentNode").text(circuit.parent_node);
            $("#bwMax").text(circuit.download_max_mbps + " / " + circuit.upload_max_mbps + " Mbps");
            $("#bwMin").text(circuit.download_min_mbps + " / " + circuit.upload_min_mbps + " Mbps");
            plan = {
                down: circuit.download_max_mbps,
                up: circuit.upload_max_mbps,
            };
            initialDevices(circuits);
            speedometer = new BitsPerSecondGauge("bitsGauge");
            totalThroughput = new CircuitTotalGraph("throughputGraph", "Total Circuit Throughput");
            totalRetransmits = new CircuitRetransmitGraph("rxmitGraph", "Total Circuit Retransmits");
            flowSankey = new FlowsSankey("flowSankey");

            connectPrivateChannel();
            connectPingers(circuits);
            connectFlowChannel();
            initialFunnel(circuit.parent_node);
            subscribeToCake();
        },
        error: () => {
            alert("Circuit with id " + circuit_id + " not found");
        }
    })
}

loadInitial();