import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {mkBadge} from "./bakery_shared";
import {
    directionSummary,
    formatStormguardMbps,
    selectStormguardSite,
    subscribeStormguardState,
    updateStormguardDebug,
    updateStormguardStatus,
} from "./stormguard_shared";

function directionBadge(summary) {
    return mkBadge(summary.label, summary.className, summary.reason || "");
}

export class StormguardSiteListDashlet extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 4;
        this.unsubscribe = null;
    }

    title() {
        return "StormGuard Sites";
    }

    tooltip() {
        return "<h5>StormGuard Sites</h5><p>Ranks watched sites so operators can quickly see where StormGuard is acting, cooling down, or simply holding steady.</p>";
    }

    subscribeTo() {
        return ["StormguardStatus", "StormguardDebug"];
    }

    buildContainer() {
        const base = super.buildContainer();
        const wrap = document.createElement("div");
        wrap.classList.add("p-2");

        this.caption = document.createElement("div");
        this.caption.classList.add("text-muted", "small", "mb-2");
        wrap.appendChild(this.caption);

        const tableWrap = document.createElement("div");
        tableWrap.classList.add("table-responsive", "lqos-table-wrap");
        const table = document.createElement("table");
        table.classList.add("lqos-table", "lqos-table-compact", "align-middle", "mb-0");
        table.innerHTML = `
            <thead>
                <tr>
                    <th>Site</th>
                    <th>Rates</th>
                    <th>Status</th>
                </tr>
            </thead>
        `;
        this.tbody = document.createElement("tbody");
        table.appendChild(this.tbody);
        tableWrap.appendChild(table);
        wrap.appendChild(tableWrap);

        base.appendChild(wrap);
        return base;
    }

    setup() {
        this.unsubscribe = subscribeStormguardState((snapshot) => this.renderSnapshot(snapshot));
    }

    onMessage(msg) {
        if (msg.event === "StormguardStatus") {
            updateStormguardStatus(msg.data || []);
        }
        if (msg.event === "StormguardDebug") {
            updateStormguardDebug(msg.data || []);
        }
    }

    renderSnapshot(snapshot) {
        if (snapshot.empty) {
            this.caption.textContent = "No watched sites are currently available.";
            this.tbody.innerHTML = "<tr><td colspan='3' class='text-muted'>StormGuard has no live site data to rank yet.</td></tr>";
            return;
        }

        this.caption.textContent = snapshot.singleSite
            ? "Single watched site. The detail panel on the right is the main view."
            : "Sorted by recent change activity, cooldowns, and attention score.";

        this.tbody.innerHTML = "";
        snapshot.sites.forEach((site) => {
            const tr = document.createElement("tr");
            if (site.site === snapshot.selectedSite) {
                tr.classList.add("table-active");
            }
            tr.style.cursor = "pointer";
            tr.onclick = () => selectStormguardSite(site.site);

            const nameTd = document.createElement("td");
            const name = document.createElement("div");
            name.classList.add("fw-semibold");
            name.textContent = site.site;
            const action = document.createElement("div");
            action.classList.add("small", "text-muted");
            action.textContent = site.lastActionLabel || "No recent action";
            nameTd.appendChild(name);
            nameTd.appendChild(action);

            const rateTd = document.createElement("td");
            rateTd.innerHTML = `
                <div><i class="fa fa-arrow-down text-primary me-1"></i>${formatStormguardMbps(site.currentDownMbps)}</div>
                <div><i class="fa fa-arrow-up text-success me-1"></i>${formatStormguardMbps(site.currentUpMbps)}</div>
            `;

            const statusTd = document.createElement("td");
            const badges = document.createElement("div");
            badges.classList.add("d-flex", "flex-column", "gap-1");
            badges.appendChild(directionBadge(site.downloadSummary || directionSummary(site.download)));
            badges.appendChild(directionBadge(site.uploadSummary || directionSummary(site.upload)));
            statusTd.appendChild(badges);

            tr.appendChild(nameTd);
            tr.appendChild(rateTd);
            tr.appendChild(statusTd);
            this.tbody.appendChild(tr);
        });
    }
}
