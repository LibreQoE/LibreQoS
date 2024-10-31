import {DashboardGraph} from "./dashboard_graph";
import {scaleNumber} from "../helpers/scaling";

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
        let protocols = {};
        let asns = {};
        let remoteDevices = {};

        const one_second_in_nanos = 1000000000;

        // Iterate over each flow and accumulate traffic.
        let flowCount = 0;
        flows.flows.forEach((flow) => {
            if (flow[0].last_seen_nanos > one_second_in_nanos) return;
            flowCount++;
            let localDevice = flow[0].device_name;
            let proto = flow[0].protocol_name;
            let asn = "ASN: " + flow[2].asn_id;
            if (flow[0].asn_name !== "") asn += " " + flow[0].asn_name;
            let remoteDevice = flow[0].remote_ip;

            // Ensure all members are present. The arrays hold links to subsequent
            // columns.
            if (localDevices[localDevice] === undefined) {
                localDevices[localDevice] = {}
            }
            if (protocols[proto] === undefined) {
                protocols[proto] = {};
            }
            if (asns[asn] === undefined) {
                asns[asn] = {};
            }
            if (remoteDevices[remoteDevice] === undefined) {
                remoteDevices[remoteDevice] = 0;
            }

            // Accumulate traffic.
            let currentRate = flow[1].rate_estimate_bps.down + flow[1].rate_estimate_bps.up;
            if (localDevices[localDevice][proto] === undefined) {
                localDevices[localDevice][proto] = currentRate;
            } else {
                localDevices[localDevice][proto] += currentRate;
            }
            if (protocols[proto][asn] === undefined) {
                protocols[proto][asn] = currentRate;
            } else {
                protocols[proto][asn] += currentRate;
            }
            if (asns[asn][remoteDevice] === undefined) {
                asns[asn][remoteDevice] = currentRate;
            } else {
                asns[asn][remoteDevice] += currentRate;
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
            for (let proto in localDevices[localDevice]) {
                links.push({source: localDevice, target: proto, value: localDevices[localDevice][proto]});
            }
        }

        // For each key/value pair in the protocols object, create a node.
        for (let proto in protocols) {
            data.push({
                name: proto,
                label: {
                    color: 'green'
                }
            });
            for (let asn in protocols[proto]) {
                links.push({source: proto, target: asn, value: protocols[proto][asn]});
            }
        }

        // For each key/value pair in the asns object, create a node.
        for (let asn in asns) {
            data.push({
                name: asn,
                label: {
                    color: 'red'
                }
            });
            for (let remoteDevice in asns[asn]) {
                links.push({source: asn, target: remoteDevice, value: asns[asn][remoteDevice]});
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