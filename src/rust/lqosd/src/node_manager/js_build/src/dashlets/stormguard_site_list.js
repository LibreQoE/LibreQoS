import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {mkBadge} from "./bakery_shared";
import {t} from "../i18n/index.js";
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
        return t("dashlet.name.stormguard_sites");
    }

    tooltip() {
        return `<h5>${this.title()}</h5><p>${t("dashlet.stormguard_sites.tooltip")}</p>`;
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
                    <th>${t("topology.node_kind.site")}</th>
                    <th>${t("common.table.rates")}</th>
                    <th>${t("topology_probes.status")}</th>
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
            this.caption.textContent = t("dashlet.stormguard_sites.empty");
            this.tbody.innerHTML = `<tr><td colspan="3" class="text-muted">${t("dashlet.stormguard_sites.no_sites")}</td></tr>`;
            return;
        }

        this.caption.textContent = snapshot.singleSite
            ? t("dashlet.stormguard_sites.single")
            : t("dashlet.stormguard_sites.sorted");

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
            action.textContent = site.lastActionLabel || t("dashlet.stormguard_sites.no_action");
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
