import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";

const HELPER_LINKS = [
    { label: "Top 10 Worst Performing Sites", icon: "fa-temperature-high", href: "executive_worst_sites.html" },
    { label: "Top 10 Most Over-Subscribed Sites", icon: "fa-chart-line", href: "executive_oversubscribed_sites.html" },
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
        const wrap = document.createElement("div");
        wrap.classList.add("d-flex", "flex-column", "gap-3");

        this._contentId = `${this.id}_helpers`;
        const helperSection = document.createElement("div");
        helperSection.id = this._contentId;
        wrap.appendChild(helperSection);

        container.appendChild(wrap);
        return container;
    }

    setup() {
        this.render();
    }

    render() {
        const target = document.getElementById(this._contentId);
        if (!target) return;
        const buttons = HELPER_LINKS.map(link => `
            <a class="btn btn-outline-primary exec-helper-button" href="${link.href}" title="Open ${link.label}">
                <i class="fas ${link.icon} me-2"></i>${link.label}
            </a>
        `).join("");

        target.innerHTML = `
            <div class="card shadow-sm border-0">
                <div class="card-body py-3">
                    <div class="d-flex align-items-center justify-content-between flex-wrap gap-2 mb-2">
                        <div class="exec-section-title mb-0 text-secondary"><i class="fas fa-external-link-alt me-2 text-primary"></i>Helper Views</div>
                        <span class="badge bg-light text-secondary border">Navigation</span>
                    </div>
                    <div class="d-flex flex-wrap gap-2">${buttons}</div>
                </div>
            </div>
        `;
    }
}
