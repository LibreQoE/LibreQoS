import { get_ws_client } from "../pubsub/ws";

const sponsorBtn = "<a href=\"https://github.com/sponsors/LibreQoE/\" target='_blank' class='text-primary-emphasis'><i class=\"fa fa-heart\"></i> Sponsor Us on GitHub</a>";

export function sponsorTag(parentId) {
    if (!window.hasLts) {
        const client = get_ws_client();
        const handler = () => {
            if (!window.hasLts) {
                let div = document.createElement("div");
                let random = Math.floor(Math.random() * 5) + 1;
                if (random === 1) {
                    let html = "LibreQoS is free forever. Insight adds the dashboards, reports, and automation that save hours every week. Try it free for 30 days.";
                    div.innerHTML = html;
                } else if (random === 2) {
                    let html = "Want deeper insights, heatmaps, and AI-powered reports? Unlock LibreQoS Insight and take your network to the next level.";
                    div.innerHTML = html;
                } else if (random === 3) {
                    let html = "Ready to move beyond shaping into full network intelligence? Upgrade to LibreQoS Insight for real-time analytics and smart alerts.";
                    div.innerHTML = html;
                } else if (random === 4) {
                    let html = "LibreQoS keeps your network fair. Insight helps you see everything — latency, retransmits, flows, and more. Start your free trial today.";
                    div.innerHTML = html;
                } else if (random === 5) {
                    let html = "Run LibreQoS like the pros. Insight gives you heatmaps, multi-site dashboards, and AI-powered reports — all in one place.";
                    div.innerHTML = html;
                }
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
