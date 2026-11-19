import {t} from "../i18n/index.js";
import {ThroughputBpsDash} from "./throughput_bps_dash";
import {ThroughputPpsDash} from "./throughput_pps_dash";
import {ShapedUnshapedDash} from "./shaped_unshaped_dash";
import {TrackedFlowsCount} from "./tracked_flow_count_dash";
import {ThroughputRingDash} from "./throughput_ring_dash";
import {LiveTrafficPulseDash} from "./live_traffic_pulse_dash";
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
import {Top10UploadersVisual} from "./top10_uploads_graphic";
import {Top10Uploaders} from "./top10_uploaders";
import {StormguardSummaryDashlet} from "./stormguard_summary";
import {StormguardSiteListDashlet} from "./stormguard_site_list";
import {StormguardSiteDetailDashlet} from "./stormguard_site_detail";
import {StormguardRecentActivityDashlet} from "./stormguard_recent_activity";
// New Traffic Overview dashlets
import {ShaperTopAsnDownload} from "./top_asn_download";
import {ShaperTopAsnUpload} from "./top_asn_upload";
import {ShaperChildrenDown} from "./children_sankey_down";
import {ShaperChildrenUp} from "./children_sankey_up";
import {ShaperWorldMapDown} from "./world_map_down";
import {ShaperWorldMapUp} from "./world_map_up";
import {ExecutiveSnapshotDashlet} from "./executive_snapshot";
import {QueueDynamicsDashlet} from "./queue_dynamics_dash";
import {ExecutiveHelpersDashlet} from "./executive_helpers";
import {
    ExecutiveDownloadHeatmapDashlet,
    ExecutiveGlobalHeatmapDashlet,
    ExecutiveRetransmitsHeatmapDashlet,
    ExecutiveRttHeatmapDashlet,
    ExecutiveUploadHeatmapDashlet
} from "./executive_heatmap_panels";

export const DashletMenu = [
    { name: () => t("dashlet.name.throughput_bits_per_second"), category: () => t("dashlet.category.throughput"), tag: "throughputBps", size: 3 },
    { name: () => t("dashlet.name.throughput_packets_per_second"), category: () => t("dashlet.category.throughput"), tag: "throughputPps", size: 3 },
    { name: () => t("dashlet.name.mapped_unmapped_traffic"), category: () => t("dashlet.category.shaped"), tag: "shapedUnshaped", size: 3 },
    { name: () => t("dashlet.name.tracked_flows_counter"), category: () => t("dashlet.category.flows"), tag: "trackedFlowsCount", size: 3 },
    { name: () => t("dashlet.name.last_5_minutes_throughput"), category: () => t("dashlet.category.throughput"), tag: "throughputRing", size: 6 },
    { name: () => t("dashlet.name.live_traffic_pulse"), category: () => t("dashlet.category.executive"), tag: "throughputRing30", size: 12 },
    { name: () => t("dashlet.name.round_trip_time_histogram"), category: () => t("dashlet.category.rtt"), tag: "rttHistogram", size: 6 },
    { name: () => t("dashlet.name.top_10_downloaders"), category: () => t("dashlet.category.top_10"), tag: "top10downloaders", size: 6 },
    { name: () => t("dashlet.name.top_10_downloaders_visual"), category: () => t("dashlet.category.top_10"), tag: "top10downloadersV", size: 6 },
    { name: () => t("dashlet.name.top_10_uploaders"), category: () => t("dashlet.category.top_10"), tag: "top10uploaders", size: 6 },
    { name: () => t("dashlet.name.top_10_uploaders_visual"), category: () => t("dashlet.category.top_10"), tag: "top10uploadersV", size: 6 },
    { name: () => t("dashlet.name.worst_10_round_trip_time"), category: () => t("dashlet.category.top_10"), tag: "worst10downloaders", size: 6 },
    { name: () => t("dashlet.name.worst_10_round_trip_time_visual"), category: () => t("dashlet.category.top_10"), tag: "worst10downloadersV", size: 6 },
    { name: () => t("dashlet.name.worst_10_retransmits"), category: () => t("dashlet.category.top_10"), tag: "worst10retransmits", size: 6 },
    { name: () => t("dashlet.name.worst_10_retransmits_visual"), category: () => t("dashlet.category.top_10"), tag: "worst10retransmitsV", size: 6 },
    { name: () => t("dashlet.name.top_10_flows_total_bytes"), category: () => t("dashlet.category.flows"), tag: "top10flowsBytes", size: 6 },
    { name: () => t("dashlet.name.top_10_flows_rate"), category: () => t("dashlet.category.flows"), tag: "top10flowsRate", size: 6 },
    { name: () => t("dashlet.name.top_10_endpoints_by_country"), category: () => t("dashlet.category.flows"), tag: "top10endpointsCountry", size: 6 },
    { name: () => t("dashlet.name.flow_duration"), category: () => t("dashlet.category.flows"), tag: "flowDuration", size: 6 },
    { name: () => t("dashlet.name.ether_protocols"), category: () => t("dashlet.category.flows"), tag: "etherProtocols", size: 6 },
    { name: () => t("dashlet.name.ip_protocols"), category: () => t("dashlet.category.flows"), tag: "ipProtocols", size: 6 },
    { name: () => t("dashlet.name.cpu_utilization"), category: () => t("dashlet.category.shaper"), tag: "cpu", size: 3 },
    { name: () => t("dashlet.name.ram_utilization"), category: () => t("dashlet.category.shaper"), tag: "ram", size: 3 },
    { name: () => t("dashlet.name.network_tree_summary"), category: () => t("dashlet.category.tree"), tag: "treeSummary", size: 6 },
    { name: () => t("dashlet.name.combined_top_10_box"), category: () => t("dashlet.category.top_10"), tag: "combinedTop10", size: 6 },
    { name: () => t("dashlet.name.total_cake_stats"), category: () => t("dashlet.category.cake"), tag: "totalCakeStats", size: 3 },
    { name: () => t("dashlet.name.total_tcp_retransmits"), category: () => t("dashlet.category.retransmits"), tag: "totalRetransmits", size: 3 },
    { name: () => t("dashlet.name.network_tree_sankey"), category: () => t("dashlet.category.tree"), tag: "networkTreeSankey", size: 6 },
    { name: () => t("dashlet.name.round_trip_time_histogram_3d"), category: () => t("dashlet.category.rtt"), tag: "rttHistogram3D", size: 12 },
    { name: () => t("dashlet.name.insight_shaper_status"), category: () => t("dashlet.category.insight"), tag: "ltsShaperStatus", size: 3 },
    { name: () => t("dashlet.name.insight_last_24_hours"), category: () => t("dashlet.category.insight"), tag: "ltsLast24", size: 3 },
    { name: () => t("dashlet.name.stormguard_bandwidth_adjustments"), category: () => t("dashlet.category.queue_management"), tag: "stormguardStatus", size: 6 },
    { name: () => t("dashlet.name.stormguard_summary"), category: () => t("dashlet.category.queue_management"), tag: "stormguardSummary", size: 12 },
    { name: () => t("dashlet.name.stormguard_sites"), category: () => t("dashlet.category.queue_management"), tag: "stormguardSiteList", size: 4 },
    { name: () => t("dashlet.name.stormguard_site_detail"), category: () => t("dashlet.category.queue_management"), tag: "stormguardSiteDetail", size: 8 },
    { name: () => t("dashlet.name.stormguard_recent_activity"), category: () => t("dashlet.category.queue_management"), tag: "stormguardRecentActivity", size: 12 },
    // Traffic Overview (Insight-like)
    { name: () => t("dashlet.name.shaper_top_asn_download"), category: () => t("dashlet.category.traffic"), tag: "shaperTopAsnDown", size: 6 },
    { name: () => t("dashlet.name.shaper_top_asn_upload"), category: () => t("dashlet.category.traffic"), tag: "shaperTopAsnUp", size: 6 },
    { name: () => t("dashlet.name.shaper_children_download"), category: () => t("dashlet.category.traffic"), tag: "shaperChildrenDown", size: 6 },
    { name: () => t("dashlet.name.shaper_children_upload"), category: () => t("dashlet.category.traffic"), tag: "shaperChildrenUp", size: 6 },
    { name: () => t("dashlet.name.shaper_world_map_download"), category: () => t("dashlet.category.traffic"), tag: "shaperWorldMapDown", size: 6 },
    { name: () => t("dashlet.name.shaper_world_map_upload"), category: () => t("dashlet.category.traffic"), tag: "shaperWorldMapUp", size: 6 },
    { name: () => t("dashlet.name.network_snapshot"), category: () => t("dashlet.category.executive"), tag: "executiveSnapshot", size: 12 },
    { name: () => t("dashlet.name.queue_dynamics"), category: () => t("dashlet.category.executive"), tag: "queueDynamics", size: 12 },
    { name: () => t("dashlet.name.executive_helper_links"), category: () => t("dashlet.category.executive"), tag: "executiveHelpers", size: 12 },
    { name: () => t("dashlet.name.global_heatmap"), category: () => t("dashlet.category.executive"), tag: "executiveGlobalHeatmap", size: 12 },
    { name: () => t("dashlet.name.median_rtt_heatmap"), category: () => t("dashlet.category.executive"), tag: "executiveHeatmapRtt", size: 6 },
    { name: () => t("dashlet.name.tcp_retransmits_heatmap"), category: () => t("dashlet.category.executive"), tag: "executiveHeatmapRetrans", size: 6 },
    { name: () => t("dashlet.name.circuit_utilization_heatmap"), category: () => t("dashlet.category.executive"), tag: "executiveHeatmapDownload", size: 6 },
    { name: () => t("dashlet.name.site_ap_utilization_heatmap"), category: () => t("dashlet.category.executive"), tag: "executiveHeatmapUpload", size: 6 },
];

export function widgetFactory(widgetName, count) {
    let widget = null;
    switch (widgetName) {
        case "throughputBps":   widget = new ThroughputBpsDash(count); break;
        case "throughputPps":   widget = new ThroughputPpsDash(count); break;
        case "shapedUnshaped":  widget = new ShapedUnshapedDash(count); break;
        case "trackedFlowsCount": widget = new TrackedFlowsCount(count); break;
        case "throughputRing":  widget = new ThroughputRingDash(count); break;
        case "throughputRing30": widget = new LiveTrafficPulseDash(count); break;
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
        case "stormguardSummary": widget = new StormguardSummaryDashlet(count); break;
        case "stormguardSiteList": widget = new StormguardSiteListDashlet(count); break;
        case "stormguardSiteDetail": widget = new StormguardSiteDetailDashlet(count); break;
        case "stormguardRecentActivity": widget = new StormguardRecentActivityDashlet(count); break;
        // Traffic Overview
        case "shaperTopAsnDown"  : widget = new ShaperTopAsnDownload(count); break;
        case "shaperTopAsnUp"    : widget = new ShaperTopAsnUpload(count); break;
        case "shaperChildrenDown": widget = new ShaperChildrenDown(count); break;
        case "shaperChildrenUp"  : widget = new ShaperChildrenUp(count); break;
        case "shaperWorldMapDown": widget = new ShaperWorldMapDown(count); break;
        case "shaperWorldMapUp"  : widget = new ShaperWorldMapUp(count); break;
        case "executiveSnapshot": widget = new ExecutiveSnapshotDashlet(count); break;
        case "queueDynamics": widget = new QueueDynamicsDashlet(count); break;
        case "executiveHelpers": widget = new ExecutiveHelpersDashlet(count); break;
        case "executiveGlobalHeatmap": widget = new ExecutiveGlobalHeatmapDashlet(count); break;
        case "executiveHeatmapRtt": widget = new ExecutiveRttHeatmapDashlet(count); break;
        case "executiveHeatmapRetrans": widget = new ExecutiveRetransmitsHeatmapDashlet(count); break;
        case "executiveHeatmapDownload": widget = new ExecutiveDownloadHeatmapDashlet(count); break;
        case "executiveHeatmapUpload": widget = new ExecutiveUploadHeatmapDashlet(count); break;
        default: {
            // translatable: false — "I don't know how to construct a widget of type [" is a developer console diagnostic, never rendered
            console.log("I don't know how to construct a widget of type [" + widgetName + "]");
            return null;
        }
    }
    return widget;
}
