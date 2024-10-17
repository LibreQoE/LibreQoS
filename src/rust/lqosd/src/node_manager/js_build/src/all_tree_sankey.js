import { DashboardGraph } from "./graphs/dashboard_graph";
import {lerpGreenToRedViaOrange} from "./helpers/scaling";
import {isRedacted} from "./helpers/redact";
import {GenericRingBuffer} from "./helpers/ringbuffer";
import {trimStringWithElipsis} from "./helpers/strings_help";

class AllTreeSankeyGraph extends GenericRingBuffer {
    constructor() {
        super(10);
    }

    onTick(graph) {
        let self = this;
        $.get("/local-api/networkTree", (data) => {
            // Maintain a 10-second ringbuffer of recent data
            this.push(data);

            let redact = isRedacted();
            let nodes = [];
            let links = [];

            // Build the basic tree from the current head, to ensure
            // that we're displaying the most recent nodes.
            let head = self.getHead();
            if (head === undefined) { return }
            let startDepth = head[rootId][1].parents.length - 1;
            for (let i=0; i<head.length; i++) {
                let depth = head[i][1].parents.length - startDepth;
                if (depth > maxDepth) {
                    continue;
                }
                // If head[i][1].parents does not contain rootId, skip
                if (rootId !== 0 && !head[i][1].parents.includes(rootId)) {
                    continue;
                }
                let name = head[i][1].name;
                let bytes = head[i][1].current_throughput[0];
                let bytesAsMegabits = bytes / 1000000;
                let maxBytes = head[i][1].max_throughput[0] / 8;
                let percent = Math.min(100, (bytesAsMegabits / maxBytes) * 100);
                let capacityColor = lerpGreenToRedViaOrange(100 - percent, 100);

                let color = lerpGreenToRedViaOrange(200 - lastRtt[name], 200);

                let label = {
                    fontSize: 10,
                    color: "#999",
                    formatter: (params) => {
                        // Trim to 10 chars with elipsis
                        return trimStringWithElipsis(params.name, 14);
                    }
                };
                if (redact) label.backgroundColor = label.color;

                nodes.push({
                    name: head[i][1].name,
                    label: label,
                    itemStyle: {
                        color: color
                    },
                    n: 1,
                });

                if (i > 0) {
                    let immediateParent = head[i][1].immediate_parent;
                    links.push({
                        source: head[immediateParent][1].name,
                        target: head[i][1].name,
                        value: head[i][1].current_throughput[0],
                        lineStyle: {
                            color: capacityColor,
                        },
                        maxBytes: maxBytes,
                        n: 1,
                    });
                }
            }

            // Now we iterate over the entire ringbuffer to accumulate data over the period
            // of the ringbuffer.
            self.iterate((data) => {
                for (let i=0; i<data.length; i++) {
                    // Search for links that match so we can update the value
                    if (i > 0) {
                        let immediateParent = data[i][1].immediate_parent;
                        let link = links.find((link) => { return link.source === data[immediateParent][1].name && link.target === data[i][1].name; });
                        if (link !== undefined) {
                            link.value += data[i][1].current_throughput[0];
                            link.n++;
                        }
                    }
                }
            });

            // Now go through the links and average the values, recalculating the color
            for (let i=0; i<links.length; i++) {
                links[i].value /= links[i].n;
                let percent = Math.min(100, (links[i].value / links[i].maxBytes) * 100);
                links[i].lineStyle.color = lerpGreenToRedViaOrange(100 - percent, 100);
            }

            // Update the graph
            graph.update(nodes, links);
        });
    }
}

var allNodes = [];
let rootId = 0;
let lastRtt = {};
let paused = false;

function idOfNode(name) {
    for (let i=0; i<allNodes.length; i++) {
        if (allNodes[i][1].name === name) {
            return i;
        }
    }
    return 0;
}

class AllTreeSankey extends DashboardGraph {
    constructor(id) {
        super(id);
        this.model = new AllTreeSankeyGraph();
        this.option = {
            series: [
                {
                    nodeAlign: 'left',
                    type: 'sankey',
                    data: [],
                    links: []
                }
            ]
        };
        this.option && this.chart.setOption(this.option);
        this.chart.showLoading();
        this.chart.on('click', (params) => {
            console.log(params.name);
            console.log(this.nodeMap);
            let name = params.name;
            // If it contains a >, it's a link
            if (name.indexOf(" > ") === -1) {
                rootId = idOfNode(name);
            } else {
                rootId = idOfNode(params.data.source);
            }
        });
        $("#btnRoot").click(() => { rootId = 0; });
    }

    update(data, links) {
        this.option.series[0].data = data;
        this.option.series[0].links = links;
        this.chart.hideLoading();
        this.chart.setOption(this.option);
    }
}

function start() {
    graph.model.onTick(graph);
    setTimeout(start, 1000);
}

function getMaxDepth() {
    let maxDepth = 10;
    let storedDepth = localStorage.getItem("atsDepth");
    if (storedDepth !== null) {
        maxDepth = parseInt(storedDepth);
    } else {
        localStorage.setItem("atsDepth", maxDepth.toString());
    }
    return maxDepth;
}

function bindMaxDepth() {
    let d = document.getElementById("maxDepth");
    d.value = maxDepth;
    d.onclick = () => {
        maxDepth = parseInt(d.value);
        localStorage.setItem("atsDepth", maxDepth.toString());
    };
}

let maxDepth = getMaxDepth();
bindMaxDepth();
let graph = new AllTreeSankey("sankey");

$("#btnPause").click(() => {
    paused = !paused;
    if (paused) {
        $("#btnPause").html("<i class='fa fa-play'></i> Resume");
    } else {
        $("#btnPause").html("<i class='fa fa-pause'></i>Pause");
    }
});

start();