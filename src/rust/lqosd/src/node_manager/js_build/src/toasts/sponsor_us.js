import { get_ws_client } from "../pubsub/ws";

const sponsorBtn = "<a href=\"https://github.com/sponsors/LibreQoE/\" target='_blank' class='text-primary-emphasis'><i class=\"fa fa-heart\"></i> Sponsor Us on GitHub</a>";
const sponsorMessages = [
    "LibreQoS includes shaping and core controls. Insight adds historical dashboards and alerts so you can spot issues before tickets arrive. Start a free 30-day trial.",
    "Need proof before or after changes? Insight keeps long-term latency, retransmit, and flow history in one place. Try it free.",
    "Heatmaps in Insight make congestion trends obvious across sites and APs. Find busy hours fast. Start your free trial.",
    "Managing multiple shapers? Insight gives you a single dashboard view across locations. Start free for 30 days.",
    "Insight AI reports summarize what changed and where to look first, so troubleshooting takes minutes instead of hours.",
    "When customers say internet is slow, Insight helps you verify latency, retransmits, and utilization quickly. Try it free.",
    "LibreQoS handles shaping. Insight adds visibility, trends, and alerts to run operations proactively. Start a free trial.",
    "See circuit and site behavior over time, not just right now. Insight gives you the historical context to make better decisions.",
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
