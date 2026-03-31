import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {mkBadge} from "./bakery_shared";
import {
    requestStormguardConfig,
    stormguardConfigBadgeData,
    subscribeStormguardState,
    updateStormguardDebug,
    updateStormguardStatus,
} from "./stormguard_shared";

function summaryCard(label, icon, toneClass) {
    const col = document.createElement("div");
    col.classList.add("col-6", "col-lg-3");

    const card = document.createElement("div");
    card.classList.add("border", "rounded", "p-3", "h-100");

    const top = document.createElement("div");
    top.classList.add("d-flex", "align-items-center", "justify-content-between", "mb-2");

    const title = document.createElement("div");
    title.classList.add("text-muted", "small", "text-uppercase");
    title.textContent = label;

    const iconEl = document.createElement("i");
    iconEl.className = `fa ${icon} ${toneClass}`;

    const value = document.createElement("div");
    value.classList.add("fs-4", "fw-semibold");
    value.textContent = "—";

    const sub = document.createElement("div");
    sub.classList.add("small", "text-muted");
    sub.textContent = "";

    top.appendChild(title);
    top.appendChild(iconEl);
    card.appendChild(top);
    card.appendChild(value);
    card.appendChild(sub);
    col.appendChild(card);

    return {col, card, value, sub};
}

export class StormguardSummaryDashlet extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 12;
        this.unsubscribe = null;
    }

    title() {
        return "StormGuard Summary";
    }

    tooltip() {
        return "<h5>StormGuard Summary</h5><p>Shows whether StormGuard is enabled, how many watched sites are active, and where recent adjustments or cooldowns are concentrated.</p>";
    }

    subscribeTo() {
        return ["StormguardStatus", "StormguardDebug"];
    }

    buildContainer() {
        const base = super.buildContainer();
        const wrap = document.createElement("div");
        wrap.classList.add("p-2");

        const header = document.createElement("div");
        header.classList.add("d-flex", "justify-content-between", "align-items-center", "flex-wrap", "gap-2", "mb-3");

        this.statusWrap = document.createElement("div");
        this.statusWrap.classList.add("d-flex", "align-items-center", "gap-2", "flex-wrap");

        const links = document.createElement("div");
        links.classList.add("d-flex", "gap-2");

        const configLink = document.createElement("a");
        configLink.href = "config_stormguard.html";
        configLink.className = "btn btn-sm btn-outline-primary";
        configLink.innerHTML = "<i class='fa fa-gear me-1'></i> Config";

        const debugLink = document.createElement("a");
        debugLink.href = "stormguard_debug.html";
        debugLink.className = "btn btn-sm btn-outline-secondary";
        debugLink.innerHTML = "<i class='fa fa-bug me-1'></i> Debug";

        links.appendChild(configLink);
        links.appendChild(debugLink);
        header.appendChild(this.statusWrap);
        header.appendChild(links);
        wrap.appendChild(header);

        const row = document.createElement("div");
        row.classList.add("row", "g-3");

        this.watchedCard = summaryCard("Watched Sites", "fa-sitemap", "text-primary");
        this.activeCard = summaryCard("Active Sites", "fa-wave-square", "text-success");
        this.cooldownCard = summaryCard("Cooling Down", "fa-hourglass-half", "text-warning");
        this.changedCard = summaryCard("Changed Recently", "fa-bolt", "text-danger");

        [
            this.watchedCard,
            this.activeCard,
            this.cooldownCard,
            this.changedCard,
        ].forEach((card) => row.appendChild(card.col));

        this.emptyEl = document.createElement("div");
        this.emptyEl.classList.add("small", "text-muted", "mt-3");

        wrap.appendChild(row);
        wrap.appendChild(this.emptyEl);
        base.appendChild(wrap);
        return base;
    }

    setup() {
        requestStormguardConfig();
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
        const configData = stormguardConfigBadgeData(snapshot.config);
        this.statusWrap.replaceChildren(
            mkBadge(configData.enabledLabel, configData.enabledClass),
            mkBadge(configData.dryRunLabel, "bg-light text-secondary border"),
            mkBadge(`Strategy: ${configData.strategy}`, "bg-info-subtle text-info border border-info-subtle"),
        );

        this.watchedCard.value.textContent = snapshot.watchedSiteCount.toString();
        this.watchedCard.sub.textContent = "Sites currently emitting StormGuard data";

        this.activeCard.value.textContent = snapshot.sites.filter((site) => site.download || site.upload).length.toString();
        this.activeCard.sub.textContent = "Sites with live debug metrics";

        this.cooldownCard.value.textContent = snapshot.cooldownSiteCount.toString();
        this.cooldownCard.sub.textContent = snapshot.cooldownSiteCount > 0
            ? "StormGuard is pacing follow-up actions"
            : "No sites are waiting out cooldowns";

        this.changedCard.value.textContent = snapshot.recentChangeCount.toString();
        this.changedCard.sub.textContent = snapshot.recentChangeCount > 0
            ? "Sites changed in the last 5 minutes"
            : "No recent StormGuard adjustments";

        if (snapshot.empty) {
            if (snapshot.config && snapshot.config.enabled) {
                this.emptyEl.textContent = "StormGuard is enabled, but no watched sites are currently publishing live data.";
            } else {
                this.emptyEl.textContent = "StormGuard is disabled or idle. Use the config page to enable it and choose watched sites.";
            }
        } else {
            this.emptyEl.textContent = snapshot.singleSite
                ? "Single-site deployment detected. The detail panel expands that site’s current state and recent decisions."
                : "Select a site from the list to inspect why StormGuard is holding, cooling down, or changing rates.";
        }
    }
}
