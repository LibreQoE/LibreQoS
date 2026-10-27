import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";

const HELPER_LINKS = [
    { label: "Top 10 Worst Performing Sites", icon: "fa-temperature-high", href: "executive_worst_sites.html" },
    { label: "Most Over-Subscribed Sites", fullLabel: "Top 10 Most Over-Subscribed Sites", icon: "fa-chart-line", href: "executive_oversubscribed_sites.html" },
    { label: "Sites Due for Upgrade", icon: "fa-arrow-up-right-dots", href: "executive_sites_due_upgrade.html" },
    { label: "Circuits Due for Upgrade", icon: "fa-wave-square", href: "executive_circuits_due_upgrade.html" },
    { label: "Top 10 ASNs by Traffic Volume", icon: "fa-globe-americas", href: "executive_top_asns.html" },
];

export class ExecutiveHelpersDashlet extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 12;
    }

    canBeSlowedDown() { return true; }
    title() { return "Helper Views"; }
    tooltip() { return "Quick navigation to common executive helper pages."; }
    subscribeTo() { return []; }

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
            <a class="btn btn-outline-primary exec-helper-button" href="${link.href}" title="Open ${link.fullLabel || link.label}">
                <i class="fas ${link.icon}"></i>
                <span class="exec-helper-label">${link.label}</span>
                <i class="fas fa-chevron-right exec-helper-chevron"></i>
            </a>
        `).join("");

        target.innerHTML = buttons;
    }
}
