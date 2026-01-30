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
import {TopTreeSankey} from "./top_tree_sankey";
import {Top10DownloadersVisual} from "./top10_downloads_graphic";
import {Worst10DownloadersVisual} from "./worst10_downloaders_graphic";
import {Worst10RetransmitsVisual} from "./worst10_retransmits_graphic";
import {FlowDurationDash} from "./flow_durations_dash";
import {LtsShaperStatus} from "./ltsShaperStatus";
import {LtsLast24Hours} from "./ltsLast24Hours";
import {TcpRetransmitsDash} from "./total_retransmits";
import {StormguardStatusDashlet} from "./stormguard_status";
import {BakeryStatusDashlet} from "./bakery_status";
import {Top10UploadersVisual} from "./top10_uploads_graphic";
import {Top10Uploaders} from "./top10_uploaders";
// New Traffic Overview dashlets
import {ShaperTopAsnDownload} from "./top_asn_download";
import {ShaperTopAsnUpload} from "./top_asn_upload";
import {ShaperChildrenDown} from "./children_sankey_down";
import {ShaperChildrenUp} from "./children_sankey_up";
import {ShaperWorldMapDown} from "./world_map_down";
import {ShaperWorldMapUp} from "./world_map_up";
import {ExecutiveSnapshotDashlet} from "./executive_snapshot";
import {ExecutiveHelpersDashlet} from "./executive_helpers";
import {
    ExecutiveDownloadHeatmapDashlet,
    ExecutiveGlobalHeatmapDashlet,
    ExecutiveRetransmitsHeatmapDashlet,
    ExecutiveRttHeatmapDashlet,
    ExecutiveUploadHeatmapDashlet
} from "./executive_heatmap_panels";

export const DashletMenu = [
    { name: "Throughput Bits/Second", tag: "throughputBps", size: 3, category: "Throughput" },
    { name: "Throughput Packets/Second", tag: "throughputPps", size: 3, category: "Throughput" },
    { name: "Mapped/Unmapped Traffic", tag: "shapedUnshaped", size: 3, category: "Shaped" },
    { name: "Tracked Flows Counter", tag: "trackedFlowsCount", size: 3, category: "Flows" },
    { name: "Last 5 Minutes Throughput", tag: "throughputRing", size: 6, category: "Throughput" },
    { name: "Round-Trip Time Histogram", tag: "rttHistogram", size: 6, category: "RTT" },
    { name: "Top 10 Downloaders", tag: "top10downloaders", size: 6, category: "Top 10" },
    { name: "Top 10 Downloaders (Visual)", tag: "top10downloadersV", size: 6, category: "Top 10" },
    { name: "Top 10 Uploaders", tag: "top10uploaders", size: 6, category: "Top 10" },
    { name: "Top 10 Uploaders (Visual)", tag: "top10uploadersV", size: 6, category: "Top 10" },
    { name: "Worst 10 Round-Trip Time", tag: "worst10downloaders", size: 6, category: "Top 10" },
    { name: "Worst 10 Round-Trip Time (Visual)", tag: "worst10downloadersV", size: 6, category: "Top 10" },
    { name: "Worst 10 Retransmits", tag: "worst10retransmits", size: 6, category: "Top 10" },
    { name: "Worst 10 Retransmits (Visual)", tag: "worst10retransmitsV", size: 6, category: "Top 10" },
    { name: "Top 10 Flows (total bytes)", tag: "top10flowsBytes", size: 6, category: "Flows" },
    { name: "Top 10 Flows (rate)", tag: "top10flowsRate", size: 6, category: "Flows" },
    { name: "Top 10 Endpoints by Country", tag: "top10endpointsCountry", size: 6, category: "Flows" },
    { name: "Flow Duration", tag: "flowDuration", size: 6, category: "Flows" },
    { name: "Ether Protocols", tag: "etherProtocols", size: 6, category: "Flows" },
    { name: "IP Protocols", tag: "ipProtocols", size: 6, category: "Flows" },
    { name: "CPU Utilization", tag: "cpu", size: 3, category: "Shaper" },
    { name: "RAM Utilization", tag: "ram", size: 3, category: "Shaper" },
    { name: "Network Tree Summary", tag: "treeSummary", size: 6, category: "Tree" },
    { name: "Combined Top 10 Box", tag: "combinedTop10", size: 6, category: "Top 10" },
    { name: "Total Cake Stats", tag: "totalCakeStats", size: 3, category: "CAKE" },
    { name: "Total TCP Retransmits", tag: "totalRetransmits", size: 3, category: "Retransmits" },
    { name: "Network Tree Sankey", tag: "networkTreeSankey", size: 6, category: "Tree" },
    { name: "Round-Trip Time Histogram 3D", tag: "rttHistogram3D", size: 12, category: "RTT" },
    { name: "(Insight) Shaper Status", tag: "ltsShaperStatus", size: 3, category: "Insight" },
    { name: "(Insight) Last 24 Hours", tag: "ltsLast24", size: 3, category: "Insight" },
    { name: "Stormguard Bandwidth Adjustments", tag: "stormguardStatus", size: 6, category: "Queue Management" },
    { name: "Bakery Circuit Activity", tag: "bakeryStatus", size: 6, category: "Queue Management" },
    // Traffic Overview (Insight-like)
    { name: "Shaper Top ASN (Download)", tag: "shaperTopAsnDown", size: 6, category: "Traffic" },
    { name: "Shaper Top ASN (Upload)", tag: "shaperTopAsnUp", size: 6, category: "Traffic" },
    { name: "Shaper Children (Download)", tag: "shaperChildrenDown", size: 6, category: "Traffic" },
    { name: "Shaper Children (Upload)", tag: "shaperChildrenUp", size: 6, category: "Traffic" },
    { name: "Shaper World Map (Download)", tag: "shaperWorldMapDown", size: 6, category: "Traffic" },
    { name: "Shaper World Map (Upload)", tag: "shaperWorldMapUp", size: 6, category: "Traffic" },
    { name: "Network Snapshot", tag: "executiveSnapshot", size: 12, category: "Executive" },
    { name: "Executive Helper Links", tag: "executiveHelpers", size: 12, category: "Executive" },
    { name: "Global Heatmap", tag: "executiveGlobalHeatmap", size: 12, category: "Executive" },
    { name: "Median RTT Heatmap", tag: "executiveHeatmapRtt", size: 6, category: "Executive" },
    { name: "TCP Retransmits Heatmap", tag: "executiveHeatmapRetrans", size: 6, category: "Executive" },
    { name: "Utilization Heatmap", tag: "executiveHeatmapDownload", size: 6, category: "Executive" },
    { name: "QoO Heatmap", tag: "executiveHeatmapUpload", size: 6, category: "Executive" },
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
        case "top10uploaders":widget = new Top10Uploaders(count); break;
        case "top10uploadersV":widget = new Top10UploadersVisual(count); break;
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
        case "totalRetransmits" : widget = new TcpRetransmitsDash(count); break;
        case "networkTreeSankey": widget = new TopTreeSankey(count); break;
        case "ltsShaperStatus"  : widget = new LtsShaperStatus(count); break;
        case "ltsLast24"        : widget = new LtsLast24Hours(count); break;
        case "stormguardStatus" : widget = new StormguardStatusDashlet(count); break;
        case "bakeryStatus"     : widget = new BakeryStatusDashlet(count); break;
        // Traffic Overview
        case "shaperTopAsnDown"  : widget = new ShaperTopAsnDownload(count); break;
        case "shaperTopAsnUp"    : widget = new ShaperTopAsnUpload(count); break;
        case "shaperChildrenDown": widget = new ShaperChildrenDown(count); break;
        case "shaperChildrenUp"  : widget = new ShaperChildrenUp(count); break;
        case "shaperWorldMapDown": widget = new ShaperWorldMapDown(count); break;
        case "shaperWorldMapUp"  : widget = new ShaperWorldMapUp(count); break;
        case "executiveSnapshot": widget = new ExecutiveSnapshotDashlet(count); break;
        case "executiveHelpers": widget = new ExecutiveHelpersDashlet(count); break;
        case "executiveGlobalHeatmap": widget = new ExecutiveGlobalHeatmapDashlet(count); break;
        case "executiveHeatmapRtt": widget = new ExecutiveRttHeatmapDashlet(count); break;
        case "executiveHeatmapRetrans": widget = new ExecutiveRetransmitsHeatmapDashlet(count); break;
        case "executiveHeatmapDownload": widget = new ExecutiveDownloadHeatmapDashlet(count); break;
        case "executiveHeatmapUpload": widget = new ExecutiveUploadHeatmapDashlet(count); break;
        default: {
            console.log("I don't know how to construct a widget of type [" + widgetName + "]");
            return null;
        }
    }
    return widget;
}
