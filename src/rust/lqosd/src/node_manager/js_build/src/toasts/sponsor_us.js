import { get_ws_client } from "../pubsub/ws";

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

function getSponsorState(mappedCircuits) {
    if (window.mappedCircuitLimit !== 1000) {
        return null;
    }

    if (mappedCircuits > 1000) {
        return {
            message: "This deployment is above the unlicensed 1,000 mapped circuit limit; add a license to remove the cap.",
            alertClass: "alert-danger",
        };
    }

    if (mappedCircuits >= 800) {
        return {
            message: "Unlicensed deployments are limited to 1,000 mapped circuits, and this system is approaching that limit.",
            alertClass: "alert-warning",
        };
    }

    return {
        message: sponsorMessages[Math.floor(Math.random() * sponsorMessages.length)],
        alertClass: "alert-success",
    };
}

export function sponsorTag(parentId) {
    const client = get_ws_client();
    const handler = (msg) => {
        client.off("DeviceCount", handler);

        const parent = document.getElementById(parentId);
        if (!parent) {
            return;
        }

        const mappedCircuits = Number(msg?.data?.mapped_circuits ?? 0);
        const sponsorState = getSponsorState(mappedCircuits);
        if (!sponsorState) {
            return;
        }

        const div = document.createElement("div");
        div.textContent = sponsorState.message;
        div.classList.add("alert", sponsorState.alertClass, "toasty");
        div.setAttribute("role", "alert");
        parent.appendChild(div);
    };

    client.on("DeviceCount", handler);
    client.send({ DeviceCount: {} });
}
