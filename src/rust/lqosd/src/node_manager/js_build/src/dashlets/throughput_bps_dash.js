import {BaseDashlet} from "./base_dashlet";
import {scaleNumber} from "../lq_js_common/helpers/scaling";

export class ThroughputBpsDash extends BaseDashlet{
    title() {
        return "Throughput Bits/Second";
    }

    tooltip() {
        return "<h5>Throughput Bits/Second</h5><p>Shows the current throughput in bits per second. Traffic is divided between upload (from the ISP) and download (to the ISP) traffic.</p>";
    }

    subscribeTo() {
        return [ "Throughput" ];
    }

    buildContainer() {
        let base = super.buildContainer();
        base.style.height = "270px";
        base.style.overflow = "auto";
        return base;
    }

    setup() {
        super.setup();
        this.medians = null;
        this.tickCount = 0;
        this.busy = false;
        this.upRing = [];
        this.dlRing = [];

        let target = document.getElementById(this.id);

        // Create row
        const row = document.createElement("div");
        row.classList.add("row");
        row.style.height = "100%";

        // ---------------------
        // LEFT COLUMN
        // ---------------------
        const colLeft = document.createElement("div");
        colLeft.classList.add("col-auto", "text-center");

        // Recent
        const recentWrapper = document.createElement("div");
        recentWrapper.classList.add("mb-3");

        const recentDlHeader = document.createElement("div");
        recentDlHeader.classList.add("stat-header");
        recentDlHeader.textContent = "Download:";

        const recentDlValue = document.createElement("div");
        recentDlValue.classList.add("stat-value-big");
        recentDlValue.textContent = "-";
        recentDlValue.id = this.id + "_dl_bps";

        const recentUp = document.createElement("div");
        recentUp.classList.add("stat-header");
        recentUp.textContent = "Upload:";

        const recentUpValue = document.createElement("div");
        recentUpValue.classList.add("stat-value-big");
        recentUpValue.textContent = "-";
        recentUpValue.id = this.id + "_up_bps";

        recentWrapper.appendChild(recentDlHeader);
        recentWrapper.appendChild(recentDlValue);
        recentWrapper.appendChild(recentUp);
        recentWrapper.appendChild(recentUpValue);

        // Current
        const currentWrapper = document.createElement("div");

        const currentHeader = document.createElement("div");
        currentHeader.classList.add("stat-header");
        currentHeader.textContent = "Current:";

        const currentDlValue = document.createElement("div");
        currentDlValue.classList.add("fw-bold", "text-secondary");
        currentDlValue.textContent = "-";
        currentDlValue.id = this.id + "_cdl_bps";

        const currentUlValue = document.createElement("div");
        currentUlValue.classList.add("fw-bold", "text-secondary");
        currentUlValue.textContent = "-";
        currentUlValue.id = this.id + "_cul_bps";

        currentWrapper.appendChild(currentHeader);
        currentWrapper.appendChild(currentDlValue);
        currentWrapper.appendChild(currentUlValue);

        colLeft.appendChild(recentWrapper);
        colLeft.appendChild(currentWrapper);

        // ---------------------
        // DIVIDER COLUMN
        // ---------------------
        const colDivider = document.createElement("div");
        colDivider.classList.add("col-auto", "px-3");

        const divider = document.createElement("div");
        divider.classList.add("vertical-divider", "h-100");

        colDivider.appendChild(divider);

        // ---------------------
        // RIGHT COLUMN
        // ---------------------
        const colRight = document.createElement("div");
        colRight.classList.add("col-auto");

        if (!window.hasLts) {
            // No LTS for you
            const yestWrapper = document.createElement("div");
            yestWrapper.classList.add("mb-3");

            const yestHeader = document.createElement("div");
            yestHeader.classList.add("stat-header");

            const yestValue = document.createElement("span");
            yestValue.classList.add("fw-bold", "text-secondary");
            yestValue.innerHTML = "<i class=\"fa fa-fw fa-centerline fa-line-chart nav-icon\"></i> Requires Insight";

            yestWrapper.appendChild(yestHeader);
            yestWrapper.appendChild(yestValue);
            colRight.appendChild(yestWrapper);
        } else {

            // Yesterday
            const yestWrapper = document.createElement("div");
            yestWrapper.classList.add("mb-3");

            const yestHeader = document.createElement("div");
            yestHeader.classList.add("stat-header");
            yestHeader.textContent = "This Time Yesterday:";

            const yestValueDl = document.createElement("div");
            yestValueDl.classList.add("fw-bold", "text-secondary");

            const yestValueDlInner = document.createElement("span");
            yestValueDlInner.textContent = "-";
            yestValueDlInner.id = this.id + "_yest_dl_bps";

            const YestSpanDl = document.createElement("span");
            YestSpanDl.classList.add("small", "ms-2");
            YestSpanDl.textContent = "-";
            YestSpanDl.id = this.id + "_yest_dl_bps_span";

            const yestValueUl = document.createElement("div");
            yestValueUl.classList.add("fw-bold", "text-secondary");

            const yestValueUlInner = document.createElement("span");
            yestValueUlInner.textContent = "-";
            yestValueUlInner.id = this.id + "_yest_ul_bps";

            const YestSpanUl = document.createElement("span");
            YestSpanUl.classList.add("small", "ms-2");
            YestSpanUl.textContent = "-";
            YestSpanUl.id = this.id + "_yest_ul_bps_span";

            yestValueDl.appendChild(yestValueDlInner);
            yestValueDl.appendChild(YestSpanDl);
            yestValueUl.appendChild(yestValueUlInner);
            yestValueUl.appendChild(YestSpanUl);
            yestWrapper.appendChild(yestHeader);
            yestWrapper.appendChild(yestValueDl);
            yestWrapper.appendChild(yestValueUl);

            // Last Week
            const lastWeekWrapper = document.createElement("div");

            const lastWeekHeader = document.createElement("div");
            lastWeekHeader.classList.add("stat-header");
            lastWeekHeader.textContent = "This Time Last Week:";

            const lastWeekValueDl = document.createElement("div");
            lastWeekValueDl.classList.add("fw-bold", "text-secondary");

            const lastWeekValueDlInner = document.createElement("span");
            lastWeekValueDlInner.textContent = "-";
            lastWeekValueDlInner.id = this.id + "_last_dl_bps";

            const lastWeekSpanDl = document.createElement("span");
            lastWeekSpanDl.classList.add("small", "ms-2");
            lastWeekSpanDl.textContent = "-";
            lastWeekSpanDl.id = this.id + "_last_dl_bps_span";

            const lastWeekValueUl = document.createElement("div");
            lastWeekValueUl.classList.add("fw-bold", "text-secondary");

            const lastWeekValueUlInner = document.createElement("span");
            lastWeekValueUlInner.textContent = "-";
            lastWeekValueUlInner.id = this.id + "_last_ul_bps";

            const lastWeekSpanUl = document.createElement("span");
            lastWeekSpanUl.classList.add("small", "ms-2");
            lastWeekSpanUl.textContent = "-";
            lastWeekSpanUl.id = this.id + "_last_ul_bps_span";

            lastWeekValueDl.appendChild(lastWeekValueDlInner);
            lastWeekValueDl.appendChild(lastWeekSpanDl);
            lastWeekValueUl.appendChild(lastWeekValueUlInner);
            lastWeekValueUl.appendChild(lastWeekSpanUl);
            lastWeekWrapper.appendChild(lastWeekHeader);
            lastWeekWrapper.appendChild(lastWeekValueDl);
            lastWeekWrapper.appendChild(lastWeekValueUl);

            colRight.appendChild(yestWrapper);
            colRight.appendChild(lastWeekWrapper);
        }

        // ---------------------
        // ASSEMBLE
        // ---------------------
        row.appendChild(colLeft);
        row.appendChild(colDivider);
        row.appendChild(colRight);

        // Add it all
        target.appendChild(row);
    }

    onMessage(msg) {
        const RingSize = 10;
        if (msg.event === "Throughput") {
            this.tickCount++;
            if (this.busy === false && (this.medians === null || this.tickCount > 300)) {
                this.tickCount = 0;
                this.busy = true;
                $.get("/local-api/ltsRecentMedian", (m) => {
                    this.medians = m[0];
                    this.medians.yesterday[0] = this.medians.yesterday[0] * 8;
                    this.medians.yesterday[1] = this.medians.yesterday[1] * 8;
                    this.medians.last_week[0] = this.medians.last_week[0] * 8;
                    this.medians.last_week[1] = this.medians.last_week[1] * 8;
                });
            }

            this.upRing.push(msg.data.bps.up);
            this.dlRing.push(msg.data.bps.down);
            if (this.upRing.length > RingSize) {
                this.upRing.shift();
            }
            if (this.dlRing.length > RingSize) {
                this.dlRing.shift();
            }

            // Get the mean from upRing
            let upMedian = 0;
            if (this.upRing.length > 0) {
                upMedian = this.upRing.reduce((a, b) => a + b) / this.upRing.length;
            }

            // Get the median from dlRing
            let dlMedian = 0;
            if (this.dlRing.length > 0) {
                dlMedian = this.dlRing.reduce((a, b) => a + b) / this.dlRing.length;
            }

            // Big numbers are smoothed medians
            let dl = document.getElementById(this.id + "_dl_bps");
            dl.textContent = scaleNumber(dlMedian, 0);
            let ul = document.getElementById(this.id + "_up_bps");
            ul.textContent = scaleNumber(upMedian, 0);

            // Small numbers are current (jittery)
            let cdl = document.getElementById(this.id + "_cdl_bps");
            cdl.textContent = scaleNumber(msg.data.bps.down, 0);
            let cul = document.getElementById(this.id + "_cul_bps");
            cul.textContent = scaleNumber(msg.data.bps.up, 0);

            // Update the yesterday values
            if (this.medians !== null) {
                document.getElementById(this.id + "_yest_dl_bps").textContent = scaleNumber(this.medians.yesterday[0], 0);
                document.getElementById(this.id + "_yest_ul_bps").textContent = scaleNumber(this.medians.yesterday[1], 0);

                let [yest_dl_color, yest_dl_icon, yest_dl_percent] = this.priorComparision(dlMedian, this.medians.yesterday[0]);
                if (yest_dl_percent === null) {
                    document.getElementById(this.id + "_yest_dl_bps_span").innerHTML = "";
                } else {
                    document.getElementById(this.id + "_yest_dl_bps_span").innerHTML = `<i class="fa ${yest_dl_icon} ${yest_dl_color}"></i> ${yest_dl_percent.toFixed(0)}%`;
                }

                let [yest_ul_color, yest_ul_icon, yest_ul_percent] = this.priorComparision(upMedian, this.medians.yesterday[1]);
                if (yest_ul_percent === null) {
                    document.getElementById(this.id + "_yest_ul_bps_span").innerHTML = "";
                } else {
                    document.getElementById(this.id + "_yest_ul_bps_span").innerHTML = `<i class="fa ${yest_ul_icon} ${yest_ul_color}"></i> ${yest_ul_percent.toFixed(0)}%`;
                }
            }

            // Update the last week values
            if (this.medians !== null) {
                document.getElementById(this.id + "_last_dl_bps").textContent = scaleNumber(this.medians.last_week[0], 0);
                document.getElementById(this.id + "_last_ul_bps").textContent = scaleNumber(this.medians.last_week[1], 0);

                let [last_dl_color, last_dl_icon, last_dl_percent] = this.priorComparision(dlMedian, this.medians.last_week[0]);
                if (last_dl_percent === null) {
                    document.getElementById(this.id + "_last_dl_bps_span").textContent = "";
                } else {
                    document.getElementById(this.id + "_last_dl_bps_span").innerHTML = `<i class="fa ${last_dl_icon} ${last_dl_color}"></i> ${last_dl_percent.toFixed(0)}%`;
                }

                let [last_ul_color, last_ul_icon, last_ul_percent] = this.priorComparision(upMedian, this.medians.last_week[1]);
                if (last_ul_percent === null) {
                    document.getElementById(this.id + "_last_ul_bps_span").textContent = "";
                } else {
                    document.getElementById(this.id + "_last_ul_bps_span").innerHTML = `<i class="fa ${last_ul_icon} ${last_ul_color}"></i> ${last_ul_percent.toFixed(0)}%`;
                }
            }
        }
    }

    priorComparision(current, previous) {
        if (previous === 0) return ["", "", null];
        let color = "text-success";
        let icon = "fa-arrow-up";
        let diff = current - previous;
        if (diff < 0) {
            color = "text-danger";
            icon = "fa-arrow-down";
        }
        let percent = (diff / previous) * 100;
        return [color, icon, percent];
    }
}