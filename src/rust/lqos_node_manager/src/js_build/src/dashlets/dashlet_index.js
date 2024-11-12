import {ThroughputBpsDash} from "./throughput_bps_dash";
import {ThroughputPpsDash} from "./throughput_pps_dash";
import {ShapedUnshapedDash} from "./shaped_unshaped_dash";
import {TrackedFlowsCount} from "./tracked_flow_count_dash";
import {ThroughputRingDash} from "./throughput_ring_dash";
import {RttHistoDash} from "./rtt_histo_dash";
import {Top10Downloaders} from "./top10_downloaders";
import {Worst10Downloaders} from "./worst10_downloaders";
import {Top10FlowsBytes} from "./top10flows_bytes";
import {Top10FlowsRate} from "./top10flows_rate";
import {Top10EndpointsByCountry} from "./endpoints_by_country";
import {EtherProtocols} from "./ether_protocols";
import {IpProtocols} from "./ip_protocols";
import {Worst10Retransmits} from "./worst10_retransmits";
import {CpuDash} from "./cpu_dash";
import {RamDash} from "./ram_dash";
import {TopTreeSummary} from "./top_tree_summary";
import {CombinedTopDashlet} from "./combined_top_dash";
import {RttHisto3dDash} from "./rtt_histo3d_dash";
import {QueueStatsTotalDash} from "./queue_stats_total";
import {TreeCapacityDash} from "./tree_capacity_dash";
import {CircuitCapacityDash} from "./circuit_capacity_dash";
import {TopTreeSankey} from "./top_tree_sankey";
import {Top10DownloadersVisual} from "./top10_downloads_graphic";
import {Worst10DownloadersVisual} from "./worst10_downloaders_graphic";
import {Worst10RetransmitsVisual} from "./worst10_retransmits_graphic";
import {FlowDurationDash} from "./flow_durations_dash";

export const DashletMenu = [
    { name: "Throughput Bits/Second", tag: "throughputBps", size: 3 },
    { name: "Throughput Packets/Second", tag: "throughputPps", size: 3 },
    { name: "Shaped/Unshaped Pie", tag: "shapedUnshaped", size: 3 },
    { name: "Tracked Flows Counter", tag: "trackedFlowsCount", size: 3 },
    { name: "Last 5 Minutes Throughput", tag: "throughputRing", size: 6 },
    { name: "Round-Trip Time Histogram", tag: "rttHistogram", size: 6 },
    { name: "Top 10 Downloaders", tag: "top10downloaders", size: 6 },
    { name: "Top 10 Downloaders (Visual)", tag: "top10downloadersV", size: 6 },
    { name: "Worst 10 Round-Trip Time", tag: "worst10downloaders", size: 6 },
    { name: "Worst 10 Round-Trip Time (Visual)", tag: "worst10downloadersV", size: 6 },
    { name: "Worst 10 Retransmits", tag: "worst10retransmits", size: 6 },
    { name: "Worst 10 Retransmits (Visual)", tag: "worst10retransmitsV", size: 6 },
    { name: "Top 10 Flows (total bytes)", tag: "top10flowsBytes", size: 6 },
    { name: "Top 10 Flows (rate)", tag: "top10flowsRate", size: 6 },
    { name: "Top 10 Endpoints by Country", tag: "top10endpointsCountry", size: 6 },
    { name: "Flow Duration", tag: "flowDuration", size: 6 },
    { name: "Ether Protocols", tag: "etherProtocols", size: 6 },
    { name: "IP Protocols", tag: "ipProtocols", size: 6 },
    { name: "CPU Utilization", tag: "cpu", size: 3 },
    { name: "RAM Utilization", tag: "ram", size: 3 },
    { name: "Network Tree Summary", tag: "treeSummary", size: 6 },
    { name: "Combined Top 10 Box", tag: "combinedTop10", size: 6 },
    { name: "Total Cake Stats", tag: "totalCakeStats", size: 3 },
    { name: "Circuits At Capacity", tag: "circuitCapacity", size: 6 },
    { name: "Tree Nodes At Capacity", tag: "treeCapacity", size: 6 },
    { name: "Network Tree Sankey", tag: "networkTreeSankey", size: 6 },
    { name: "Round-Trip Time Histogram 3D", tag: "rttHistogram3D", size: 12 },
];

export function widgetFactory(widgetName, count) {
    let widget = null;
    switch (widgetName) {
        case "throughputBps":   widget = new ThroughputBpsDash(count); break;
        case "throughputPps":   widget = new ThroughputPpsDash(count); break;
        case "shapedUnshaped":  widget = new ShapedUnshapedDash(count); break;
        case "trackedFlowsCount": widget = new TrackedFlowsCount(count); break;
        case "throughputRing":  widget = new ThroughputRingDash(count); break;
        case "rttHistogram":    widget = new RttHistoDash(count); break;
        case "rttHistogram3D":    widget = new RttHisto3dDash(count); break;
        case "top10downloaders":widget = new Top10Downloaders(count); break;
        case "top10downloadersV":widget = new Top10DownloadersVisual(count); break;
        case "worst10downloaders":widget = new Worst10Downloaders(count); break;
        case "worst10downloadersV":widget = new Worst10DownloadersVisual(count); break;
        case "worst10retransmits":widget = new Worst10Retransmits(count); break;
        case "worst10retransmitsV":widget = new Worst10RetransmitsVisual(count); break;
        case "top10flowsBytes"  : widget = new Top10FlowsBytes(count); break;
        case "top10flowsRate"   : widget = new Top10FlowsRate(count); break;
        case "top10endpointsCountry"   : widget = new Top10EndpointsByCountry(count); break;
        case "etherProtocols"   : widget = new EtherProtocols(count); break;
        case "ipProtocols"      : widget = new IpProtocols(count); break;
        case "flowDuration"     : widget = new FlowDurationDash(count); break;
        case "cpu"              : widget = new CpuDash(count); break;
        case "ram"              : widget = new RamDash(count); break;
        case "treeSummary"      : widget = new TopTreeSummary(count); break;
        case "combinedTop10"    : widget = new CombinedTopDashlet(count); break;
        case "totalCakeStats"   : widget = new QueueStatsTotalDash(count); break;
        case "circuitCapacity"  : widget = new CircuitCapacityDash(count); break;
        case "treeCapacity"     : widget = new TreeCapacityDash(count); break;
        case "networkTreeSankey": widget = new TopTreeSankey(count); break;
        default: {
            console.log("I don't know how to construct a widget of type [" + widgetName + "]");
            return null;
        }
    }
    return widget;
}