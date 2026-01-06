import {DashboardGraph} from "./dashboard_graph";
import {toNumber} from "../lq_js_common/helpers/scaling";

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

        const ten_second_in_nanos = 10000000000;

        // Iterate over each flow and accumulate traffic.
        let flowCount = 0;
        
        // Sort flows by total rate (down + up) descending, then take top 20
        let sortedTopFlows = flows.flows
            .slice() // copy to avoid mutating original
            .sort((a, b) => {
                const rateA =
                    toNumber(a[1]?.rate_estimate_bps?.down, 0) +
                    toNumber(a[1]?.rate_estimate_bps?.up, 0);
                const rateB =
                    toNumber(b[1]?.rate_estimate_bps?.down, 0) +
                    toNumber(b[1]?.rate_estimate_bps?.up, 0);
                return rateB - rateA;
            })
            .slice(0, 20);
        
        sortedTopFlows.forEach((flow) => {
            if (toNumber(flow[0].last_seen_nanos, 0) > ten_second_in_nanos) return;
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

        // Accumulate the graph information.
        let data = [];
        let links = [];

        // For each key/value pair in the localDevices object, create a node.
        for (let localDevice in localDevices) {
            data.push({
                name: localDevice,
                label: {
                    color: 'magenta'
                }
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
