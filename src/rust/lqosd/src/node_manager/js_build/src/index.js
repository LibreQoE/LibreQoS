import {Dashboard} from "./lq_js_common/dashboard/dashboard";
import {checkForUpgrades} from "./toasts/version_check";
import {sponsorTag} from "./toasts/sponsor_us";
import {globalWarningToasts} from "./toasts/global_warnings";
import {showTimeControls} from "./components/timescale";
import {DashletMenu, widgetFactory} from "./dashlets/dashlet_index";

const defaultLayout = {
    version: 3,
    activeTab: 0,
    tabs: [
        {
            name: "Executive Summary",
            dashlets: [
                { tag: "executiveSnapshot", size: 12 },
                { tag: "executiveHelpers", size: 12 },
                { tag: "executiveGlobalHeatmap", size: 12 },
                { tag: "executiveHeatmapDownload", size: 6 },
                { tag: "executiveHeatmapUpload", size: 6 },
                { tag: "executiveHeatmapRtt", size: 6 },
                { tag: "executiveHeatmapRetrans", size: 6 }
            ]
        },
        {
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
            name: "System Overview",
            dashlets: [
                { tag: "throughputBps", size: 2 },
                { tag: "shapedUnshaped", size: 2 },
                { tag: "throughputPps", size: 2 },
                { tag: "trackedFlowsCount", size: 2 },
                { tag: "rttHistogram", size: 2 },
                { tag: "cpu", size: 3 },
                { tag: "ram", size: 3 }
            ]
        },
        {
            name: "Network",
            dashlets: [
                { tag: "networkTreeSankey", size: 6 },
                { tag: "treeSummary", size: 6 }
            ]
        },
        {
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
            name: "Protocols & Cake",
            dashlets: [
                { tag: "etherProtocols", size: 6 },
                { tag: "ipProtocols", size: 6 },
                { tag: "totalCakeStats", size: 6 }
            ]
        }
    ]
};

window.timeGraphs = [];
showTimeControls("timescale");
checkForUpgrades();
sponsorTag("toasts");
globalWarningToasts();
const dashboard = new Dashboard("dashboard", "mainDashboard", defaultLayout, widgetFactory, DashletMenu, false, "");
dashboard.build();
