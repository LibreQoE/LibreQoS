import {Dashboard} from "./lq_js_common/dashboard/dashboard";
import {checkForUpgrades} from "./toasts/version_check";
import {sponsorTag} from "./toasts/sponsor_us";
import {globalWarningToasts} from "./toasts/global_warnings";
import {showTimeControls} from "./components/timescale";
import {DashletMenu, widgetFactory} from "./dashlets/dashlet_index";

const defaultLayout = [
    {name: "Throughput Bits/Second", tag: "throughputBps", size:2},
    {name: "Shaped/Unshaped Pie", tag: "shapedUnshaped", size :2},
    {name: "Throughput Packets/Second", tag: "throughputPps", size:2},
    {name: "Tracked Flows Counter", tag: "trackedFlowsCount", size:2},
    {name: "Last 5 Minutes Throughput", tag: "throughputRing" ,size:2},
    {name: "Round-Trip Time Histogram", tag: "rttHistogram", size:2},
    {name: "Network Tree Sankey", tag: "networkTreeSankey",size:6},
    {name: "Network Tree Summary", tag:"treeSummary", size:6},
    {name: "Top 10 Downloaders", tag: "top10downloaders", size:6},
    {name: "Worst 10 Round-Trip Time", tag: "worst10downloaders", size:6},
    {name: "Worst 10 Retransmits", tag: "worst10retransmits", size:6},
    {name: "Top 10 Flows (total bytes)", tag: "top10flowsBytes", size:6},
    {name: "Top 10 Flows (rate)", tag: "top10flowsRate", size:6},
    {name: "Top 10 Endpoints by Country", tag: "top10endpointsCountry", size:6},
    {name: "Ether Protocols",tag: "etherProtocols", size:6},
    {name: "IP Protocols", tag:"ipProtocols", size:6},
    {name: "CPU Utilization",tag:"cpu",size:3},
    {name: "RAM Utilization", tag:"ram", size :3},
    {name: "Combined Top 10 Box", tag: "combinedTop10", size:6},
    {name: "Total Cake Stats", tag:"totalCakeStats", size:6},
    {name: "Circuits At Capacity", tag: "circuitCapacity", size:6},
    {name: "Tree Nodes At Capacity", tag:"treeCapacity", size:6}
];

window.timeGraphs = [];
showTimeControls("timescale");
checkForUpgrades();
sponsorTag("toasts");
globalWarningToasts();
const dashboard = new Dashboard("dashboard", "mainDashboard", defaultLayout, widgetFactory, DashletMenu, false, "");
dashboard.build();
