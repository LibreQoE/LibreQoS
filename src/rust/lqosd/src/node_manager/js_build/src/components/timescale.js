import {clearDiv} from "../helpers/builders";

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
                        graph.onTimeChange();
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
