import {clearDiv} from "../helpers/builders";
import {createAndShowModal} from "../lq_js_common/helpers/alert_modal";
import {t} from "../i18n/index.js";

// translatable: false — "Live" is the time-period token the controls compare against and the button ids spell, not a label

class TimeControls {
    constructor(parentId) {
        this.parentId = parentId;
        const periods = ["Live", "1h", "6h", "12h", "24h", "7d"];
        this.activePeriod = periods[0];
        let parent = document.getElementById(parentId);
        clearDiv(parent);
        periods.forEach((period) => {
            let button = document.createElement("button");
            button.id = "tp_" + period;
            button.innerText = period === "Live" ? t("common.time.live") : period;
            if (period === "Live") {
                const dot = document.createElement("span");
                dot.className = "time-live-dot";
                dot.setAttribute("aria-hidden", "true");
                button.prepend(dot);
            }
            if (period === this.activePeriod) {
                button.classList.add("btn-primary");
            } else {
                button.classList.add("btn-outline-primary");
            }
            button.classList.add("btn", "btn-sm");
            button.onclick = () => {
                if (period !== "Live" && !window.hasLts) {
                    createAndShowModal(
                        t("common.time.extended_history_title"),
                        t("common.time.extended_history_body"),
                    );
                    return;
                }

                this.activePeriod = period;
                periods.forEach((p) => {
                    let b = document.getElementById("tp_" + p);
                    if (p === period) {
                        b.classList.remove("btn-outline-primary");
                        b.classList.add("btn-primary");
                    } else {
                        b.classList.remove("btn-primary");
                        b.classList.add("btn-outline-primary");
                    }
                });
                if (window.timeGraphs !== undefined) {
                    window.timeGraphs = window.timeGraphs.filter((graph) => graph && !graph.disposed);
                    window.timeGraphs.forEach((graph) => {
                        if (graph !== null) graph.onTimeChange();
                    });
                }
            };
            parent.appendChild(button);
        });
    }
}

export function showTimeControls(parentId) {
    window.timePeriods = new TimeControls(parentId);
}
