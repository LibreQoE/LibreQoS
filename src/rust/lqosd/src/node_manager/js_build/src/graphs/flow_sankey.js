import {DashboardGraph} from "./dashboard_graph";
import {toNumber} from "../lq_js_common/helpers/scaling";
import {isRedacted} from "../helpers/redact";

const SANKEY_RECENT_FLOW_WINDOW_NANOS = 10_000_000_000;
const SANKEY_TOP_FLOW_LIMIT = 20;

function totalFlowRate(flow) {
    return (
        toNumber(flow?.[1]?.rate_estimate_bps?.down, 0) +
        toNumber(flow?.[1]?.rate_estimate_bps?.up, 0)
    );
}

function renderableSankeyFlows(flowMsg) {
    const flows = Array.isArray(flowMsg?.flows) ? flowMsg.flows : [];
    return flows
        .filter((flow) => toNumber(flow?.[0]?.last_seen_nanos, 0) <= SANKEY_RECENT_FLOW_WINDOW_NANOS)
        .sort((a, b) => totalFlowRate(b) - totalFlowRate(a))
        .slice(0, SANKEY_TOP_FLOW_LIMIT);
}

export function getRenderableSankeyFlowCount(flowMsg) {
    return renderableSankeyFlows(flowMsg).length;
}

export class FlowsSankey extends DashboardGraph {
    constructor(id) {
        super(id);
        this.option = {
            title: { text: 'Flows Sankey' },
            series: [
                {
                    type: 'sankey',
                    data: [],
                    links: []
                }
            ]
        };
        this.option && this.chart.setOption(this.option);
        this.chart.hideLoading();
    }

    update(flows) {
        // Store keyed objects to accumulate traffic for each column.
        let localDevices = {};
        let asns = {};
        let protocols = {};
        let remoteDevices = {};

        // Iterate over each flow and accumulate traffic.
        let flowCount = 0;

        renderableSankeyFlows(flows).forEach((flow) => {
            flowCount++;
            let localDevice = flow[0].device_name;
            let proto = flow[0].protocol_name;
            let asn = "ASN: " + flow[2].asn_id;
            if (flow[0].asn_name !== "") asn += " " + flow[0].asn_name;
            let remoteDevice = flow[0].remote_ip;
        
            // Ensure all members are present. The arrays hold links to subsequent
            // columns.
            if (localDevices[localDevice] === undefined) {
                localDevices[localDevice] = {};
            }
            if (asns[asn] === undefined) {
                asns[asn] = {};
            }
            if (protocols[proto] === undefined) {
                protocols[proto] = {};
            }
            if (remoteDevices[remoteDevice] === undefined) {
                remoteDevices[remoteDevice] = 0;
            }

            // Accumulate traffic.
            let currentRate =
                toNumber(flow[1].rate_estimate_bps.down, 0) +
                toNumber(flow[1].rate_estimate_bps.up, 0);
            if (localDevices[localDevice][asn] === undefined) {
                localDevices[localDevice][asn] = currentRate;
            } else {
                localDevices[localDevice][asn] += currentRate;
            }
            if (asns[asn][proto] === undefined) {
                asns[asn][proto] = currentRate;
            } else {
                asns[asn][proto] += currentRate;
            }
            if (protocols[proto][remoteDevice] === undefined) {
                protocols[proto][remoteDevice] = currentRate;
            } else {
                protocols[proto][remoteDevice] += currentRate;
            }
        });

        const redact = isRedacted();
        const localLabel = redact ? { color: 'magenta', fontFamily: "Illegible" } : { color: 'magenta' };

        // Accumulate the graph information.
        let data = [];
        let links = [];

        // For each key/value pair in the localDevices object, create a node.
        for (let localDevice in localDevices) {
            data.push({
                name: localDevice,
                label: localLabel
            });
            for (let asn in localDevices[localDevice]) {
                links.push({source: localDevice, target: asn, value: localDevices[localDevice][asn]});
            }
        }

        // For each key/value pair in the protocols object, create a node.
        for (let asn in asns) {
            data.push({
                name: asn,
                label: {
                    color: 'red'
                }
            });
            for (let proto in asns[asn]) {
                links.push({source: asn, target: proto, value: asns[asn][proto]});
            }
        }

        // For each key/value pair in the asns object, create a node.
        for (let proto in protocols) {
            data.push({
                name: proto,
                label: {
                    color: 'green'
                }
            });
            for (let remoteDevice in protocols[proto]) {
                links.push({source: proto, target: remoteDevice, value: protocols[proto][remoteDevice]});
            }
        }

        // For each key/value pair in the remoteDevices object, create a node.
        for (let remoteDevice in remoteDevices) {
            data.push({
                name: remoteDevice,
                label: {
                    color: 'orange'
                }
            });
        }

        // Apply it
        this.option.series[0].data = data;
        this.option.series[0].links = links;
        // console.log(data);
        // console.log(links);
        this.chart.hideLoading();
        this.chart.setOption(this.option);
        return flowCount;
    }
}
