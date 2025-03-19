import {clearDiv} from "../helpers/builders";
import {createAndShowModal} from "../lq_js_common/helpers/alert_modal";

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
            button.innerText = period;
            if (period === this.activePeriod) {
                button.classList.add("btn-primary");
            } else {
                button.classList.add("btn-outline-primary");
            }
            button.classList.add("btn", "btn-sm", "me-1");
            button.onclick = () => {
                if (period !== "Live" && !window.hasLts) {
                    createAndShowModal('Extended Time Periods Require Insight', 'Displaying extended time periods requires an Insight subscription or free trial. Click the "Insight" button in the menu to learn more. Invest in your network --- sign up for Insight today!');
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
