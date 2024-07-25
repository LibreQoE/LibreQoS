import {BaseDashlet} from "./base_dashlet";
import {RttHistogram} from "../graphs/rtt_histo";
import {clearDashDiv, theading, TopNTableFromMsgData, topNTableHeader, topNTableRow} from "../helpers/builders";
import {
    scaleNumber,
    rttCircleSpan,
    formatThroughput,
    formatRtt,
    formatRetransmit,
    lerpGreenToRedViaOrange, lerpColor
} from "../helpers/scaling";
import {redactCell} from "../helpers/redact";
import {DashboardGraph} from "../graphs/dashboard_graph";

class Top10DownloadSankey extends DashboardGraph {
    constructor(id) {
        super(id);
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
    }

    update(data, links) {
        this.option.series[0].data = data;
        this.option.series[0].links = links;
        this.chart.hideLoading();
        this.chart.setOption(this.option);
    }
}

export class Top10DownloadersVisual extends BaseDashlet {
    constructor(slot) {
        super(slot);
    }

    title() {
        return "Top 10 Downloaders (Visual)";
    }

    tooltip() {
        return "<h5>Top 10 Downloaders</h5><p>The top-10 users by bits-per-second. The ribbon goes red as they approach capacity. The name border indicates round-trip time, while the main label color indicates TCP retransmits. The legend is current download speed, followed by number of retransmits.</p>";
    }

    subscribeTo() {
        return [ "TopDownloads" ];
    }

    buildContainer() {
        let base = super.buildContainer();
        base.appendChild(this.graphDiv());
        return base;
    }

    canBeSlowedDown() {
        return true;
    }

    setup() {
        super.setup();
        this.graph = new Top10DownloadSankey(this.graphDivId());
    }

    onMessage(msg) {
        if (msg.event === "TopDownloads") {
            //console.log(msg);

            let nodes = [];
            let links = [];

            nodes.push({
                name: "Root",
                label: "Root",
            });

            msg.data.forEach((r) => {
                let label = {
                    fontSize: 9,
                    color: "#999"
                };

                let name = r.ip_address+ " (" + scaleNumber(r.bits_per_second.down, 0) + ", " + r.tcp_retransmits.down + "/" + r.tcp_retransmits.up + ")";
                let bytes = r.bits_per_second.down / 8;
                let bytesAsMegabits = bytes / 1000000;
                let maxBytes = r.plan.down / 8;
                let percent = Math.min(100, (bytesAsMegabits / maxBytes) * 100);
                let capacityColor = lerpGreenToRedViaOrange(100 - percent, 100);

                let rttColor = lerpGreenToRedViaOrange(200 - r.median_tcp_rtt, 200);

                let percentRxmit = Math.min(100, r.tcp_retransmits.down + r.tcp_retransmits.up) / 100;
                let rxmitColor = lerpColor([0, 255, 0], [255, 0, 0], percentRxmit);

                nodes.push({
                    name: name,
                    label: label,
                    itemStyle: {
                        color: rxmitColor,
                        borderWidth: 4,
                        borderColor: rttColor,
                    }
                });

                links.push({
                    source: "Root",
                    target: name,
                    value: r.bits_per_second.down,
                    lineStyle: {
                        color: capacityColor
                    }
                });
            });

            this.graph.update(nodes, links);
        }
    }
}