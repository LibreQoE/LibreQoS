import { get_ws_client } from "../pubsub/ws";
import { t, tn } from "../i18n/index.js";

// Each row is a getter so the banner renders in the locale the page is serving,
// not the one that happened to be active when this module loaded.
const sponsorMessages = [
    () => t("sponsor.message.shaping_history"),
    () => t("sponsor.message.proof"),
    () => t("sponsor.message.heatmaps"),
    () => t("sponsor.message.multiple_shapers"),
    () => t("sponsor.message.ai_reports"),
    () => t("sponsor.message.slow_internet"),
    () => t("sponsor.message.visibility"),
    () => t("sponsor.message.historical_context"),
];

function getSponsorState(mappedCircuits) {
    if (window.mappedCircuitLimit !== 1000) {
        return null;
    }

    if (mappedCircuits > 1000) {
        // The count in the message is the excess the cap suppresses, not the
        // configured total; `mapped_circuits` counts configured circuits.
        return {
            message: tn("sponsor.limit.exceeded", Math.max(0, mappedCircuits - 1000)),
            alertClass: "alert-danger",
        };
    }

    if (mappedCircuits >= 800) {
        return {
            message: t("sponsor.limit.approaching"),
            alertClass: "alert-warning",
        };
    }

    return {
        message: sponsorMessages[Math.floor(Math.random() * sponsorMessages.length)](),
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
