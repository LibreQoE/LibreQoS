import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";

export class DashletBaseInsight extends BaseDashlet {
    onTimeChange() {
        if (window.timePeriods.activePeriod === "Live") {
            document.getElementById(this.id).classList.remove("insight-box");
        } else {
            let e = document.getElementById(this.id);
            e.classList.add("insight-box");
        }
    }
}