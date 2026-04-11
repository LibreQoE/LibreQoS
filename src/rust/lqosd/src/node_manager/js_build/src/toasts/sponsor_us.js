import { get_ws_client } from "../pubsub/ws";

const sponsorBtn = "<a href=\"https://github.com/sponsors/LibreQoE/\" target='_blank' class='text-primary-emphasis'><i class=\"fa fa-heart\"></i> Sponsor Us on GitHub</a>";
const sponsorMessages = [
    "LibreQoS handles shaping. Insight adds history, dashboards, and alerts so you catch issues before tickets arrive.",
    "Need proof before or after changes? Insight keeps latency, retransmit, and flow history in one place.",
    "Insight heatmaps make congestion trends obvious across sites and APs. Find busy hours fast.",
    "Managing multiple shapers? Insight gives you one dashboard view across locations.",
    "Insight AI reports summarize what changed and where to look first, cutting troubleshooting time.",
    "When customers say internet is slow, Insight helps you verify latency, retransmits, and utilization fast.",
    "LibreQoS handles shaping. Insight adds visibility, trends, and alerts so you can run ops proactively.",
    "See circuit and site behavior over time, not just right now. Insight gives you historical context.",
];

export function sponsorTag(parentId) {
    if (!window.hasLts) {
        const client = get_ws_client();
        const handler = () => {
            if (!window.hasLts) {
                let div = document.createElement("div");
                let random = Math.floor(Math.random() * sponsorMessages.length);
                div.innerHTML = sponsorMessages[random];
                div.classList.add("alert", "alert-warning", "toasty");
                let parent = document.getElementById(parentId);
                parent.appendChild(div);
            }
            client.off("DeviceCount", handler);
        };
        client.on("DeviceCount", handler);
        client.send({ DeviceCount: {} });
    }
}
