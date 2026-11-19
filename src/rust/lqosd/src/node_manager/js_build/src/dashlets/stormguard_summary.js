import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {t} from "../i18n/index.js";
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
        return t("dashlet.name.stormguard_summary");
    }

    tooltip() {
        return t("dashlet.stormguard.summary.tooltip");
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
        configLink.innerHTML = `<i class='fa fa-gear me-1'></i> ${t("dashlet.stormguard.summary.config_link")}`;

        const debugLink = document.createElement("a");
        debugLink.href = "stormguard_debug.html";
        debugLink.className = "btn btn-sm btn-outline-secondary";
        debugLink.innerHTML = `<i class='fa fa-bug me-1'></i> ${t("dashlet.stormguard.summary.debug_link")}`;

        links.appendChild(configLink);
        links.appendChild(debugLink);
        header.appendChild(this.statusWrap);
        header.appendChild(links);
        wrap.appendChild(header);

        const row = document.createElement("div");
        row.classList.add("row", "g-3");

        this.watchedCard = summaryCard(t("dashlet.stormguard.card.watched_sites"), "fa-sitemap", "text-primary");
        this.activeCard = summaryCard(t("dashlet.stormguard.card.active_sites"), "fa-wave-square", "text-success");
        this.cooldownCard = summaryCard(t("dashlet.stormguard.state.cooling"), "fa-hourglass-half", "text-warning");
        this.changedCard = summaryCard(t("dashlet.stormguard.card.changed_recently"), "fa-bolt", "text-danger");

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
            mkBadge(t("dashlet.stormguard.badge.strategy", {strategy: configData.strategy}), "bg-info-subtle text-info border border-info-subtle"),
        );

        this.watchedCard.value.textContent = snapshot.watchedSiteCount.toString();
        this.watchedCard.sub.textContent = t("dashlet.stormguard.card.watched_hint");

        this.activeCard.value.textContent = snapshot.sites.filter((site) => site.download || site.upload).length.toString();
        this.activeCard.sub.textContent = t("dashlet.stormguard.card.active_hint");

        this.cooldownCard.value.textContent = snapshot.cooldownSiteCount.toString();
        this.cooldownCard.sub.textContent = snapshot.cooldownSiteCount > 0
            ? t("dashlet.stormguard.card.cooling_hint")
            : t("dashlet.stormguard.card.no_cooling_hint");

        this.changedCard.value.textContent = snapshot.recentChangeCount.toString();
        this.changedCard.sub.textContent = snapshot.recentChangeCount > 0
            ? t("dashlet.stormguard.card.changed_hint")
            : t("dashlet.stormguard.card.no_changes_hint");

        if (snapshot.empty) {
            if (snapshot.config && snapshot.config.enabled) {
                this.emptyEl.textContent = t("dashlet.stormguard.summary.empty_enabled");
            } else {
                this.emptyEl.textContent = t("dashlet.stormguard.summary.empty_disabled");
            }
        } else {
            this.emptyEl.textContent = snapshot.singleSite
                ? t("dashlet.stormguard.summary.single_site")
                : t("dashlet.stormguard.summary.choose_site");
        }
    }
}
