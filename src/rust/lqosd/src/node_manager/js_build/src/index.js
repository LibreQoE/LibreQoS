import {Dashboard} from "./lq_js_common/dashboard/dashboard";
import {sponsorTag} from "./toasts/sponsor_us";
import {globalWarningToasts} from "./toasts/global_warnings";
import {showTimeControls} from "./components/timescale";
import {DashletMenu, widgetFactory} from "./dashlets/dashlet_index";

const defaultLayout = {
    version: 3,
    activeTab: 0,
    tabs: [
        {
            id: "executive-summary",
            name: "Executive Summary",
            dashlets: [
                { tag: "executiveSnapshot", size: 12 },
                { tag: "executiveGlobalHeatmap", size: 12 },
                { tag: "executiveHeatmapDownload", size: 6 },
                { tag: "executiveHeatmapUpload", size: 6 },
                { tag: "executiveHeatmapRtt", size: 6 },
                { tag: "executiveHeatmapRetrans", size: 6 },
                { tag: "executiveHelpers", size: 12 }
            ]
        },
        {
            id: "overview",
            name: "Overview",
            dashlets: [
                { tag: "throughputRing", size: 12 },
                { tag: "shaperTopAsnDown", size: 6 },
                { tag: "shaperTopAsnUp", size: 6 },
                { tag: "shaperChildrenDown", size: 6 },
                { tag: "shaperChildrenUp", size: 6 },
                { tag: "shaperWorldMapDown", size: 6 },
                { tag: "shaperWorldMapUp", size: 6 }
            ]
        },
        {
            id: "system-overview",
            name: "System Overview",
            dashlets: [
                { tag: "throughputBps", size: 2 },
                { tag: "shapedUnshaped", size: 2 },
                { tag: "ram", size: 2 },
                { tag: "cpu", size: 5 },
                { tag: "rttHistogram", size: 3 },
                { tag: "throughputPps", size: 4 },
                { tag: "trackedFlowsCount", size: 4 }
            ]
        },
        {
            id: "network",
            name: "Network",
            dashlets: [
                { tag: "networkTreeSankey", size: 6 },
                { tag: "treeSummary", size: 6 }
            ]
        },
        {
            id: "top-10",
            name: "Top 10",
            dashlets: [
                { tag: "top10downloaders", size: 6 },
                { tag: "worst10downloaders", size: 6 },
                { tag: "worst10retransmits", size: 6 },
                { tag: "top10flowsBytes", size: 6 },
                { tag: "top10flowsRate", size: 6 },
                { tag: "top10endpointsCountry", size: 6 },
                { tag: "combinedTop10", size: 6 }
            ]
        },
        {
            id: "protocols-cake",
            name: "Protocols & Cake",
            dashlets: [
                { tag: "etherProtocols", size: 6 },
                { tag: "ipProtocols", size: 6 },
                { tag: "totalCakeStats", size: 6 }
            ]
        },
        {
            id: "treeguard",
            name: "TreeGuard",
            dashlets: [
                { tag: "treeguardControlLoop", size: 4 },
                { tag: "treeguardStateMix", size: 4 },
                { tag: "treeguardDecisionImpact", size: 4 },
                { tag: "treeguardActivity", size: 12 }
            ]
        },
        {
            id: "stormguard",
            name: "StormGuard",
            dashlets: [
                { tag: "stormguardSummary", size: 12 },
                { tag: "stormguardSiteList", size: 4 },
                { tag: "stormguardSiteDetail", size: 8 },
                { tag: "stormguardRecentActivity", size: 12 }
            ]
        },
        {
            id: "bakery",
            name: "Bakery",
            dashlets: [
                { tag: "bakeryPipeline", size: 3 },
                { tag: "bakeryChangeMix", size: 3 },
                { tag: "bakeryStatus", size: 3 },
                { tag: "bakeryCapacity", size: 3 },
                { tag: "bakeryActivity", size: 12 }
            ]
        }
    ]
};

window.timeGraphs = [];
showTimeControls("timescale");
sponsorTag("toasts");
globalWarningToasts();
const dashboard = new Dashboard("dashboard", "mainDashboard", defaultLayout, widgetFactory, DashletMenu, false, "");
dashboard.build();
