import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {t, tAttr} from "../i18n/index.js";

// The labels are thunks, not strings: the table is built once at module load,
// and each button must read in the locale the page is served in.
const HELPER_LINKS = [
    { label: () => t("executive.page.title.worst_sites"), icon: "fa-temperature-high", href: "executive_worst_sites.html" },
    { label: () => t("executive.helpers.oversubscribed_short"), fullLabel: () => t("executive.page.title.oversubscribed"), icon: "fa-chart-line", href: "executive_oversubscribed_sites.html" },
    { label: () => t("executive.page.title.sites_due_upgrade"), icon: "fa-arrow-up-right-dots", href: "executive_sites_due_upgrade.html" },
    { label: () => t("executive.page.title.circuits_due_upgrade"), icon: "fa-wave-square", href: "executive_circuits_due_upgrade.html" },
    { label: () => t("executive.page.title.top_asns"), icon: "fa-globe-americas", href: "executive_top_asns.html" },
];

export class ExecutiveHelpersDashlet extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 12;
    }

    canBeSlowedDown() { return true; }
    title() { return t("executive.helpers.title"); }
    tooltip() { return t("executive.helpers.tooltip"); }
    subscribeTo() { return []; }

    // translatable: false — "_helpers" is the element-id suffix this dashlet appends, not a label
    buildContainer() {
        const container = super.buildContainer();
        this._contentId = `${this.id}_helpers`;
        const helperSection = document.createElement("div");
        helperSection.id = this._contentId;
        helperSection.className = "exec-helper-row";
        container.appendChild(helperSection);
        return container;
    }

    setup() {
        this.render();
    }

    render() {
        const target = document.getElementById(this._contentId);
        if (!target) return;
        const buttons = HELPER_LINKS.map(link => `
            <a class="btn btn-outline-primary exec-helper-button" href="${link.href}" title="${tAttr("executive.helpers.open", {label: (link.fullLabel || link.label)()})}">
                <i class="fas ${link.icon}"></i>
                <span class="exec-helper-label">${link.label()}</span>
                <i class="fas fa-chevron-right exec-helper-chevron"></i>
            </a>
        `).join("");

        target.innerHTML = buttons;
    }
}
