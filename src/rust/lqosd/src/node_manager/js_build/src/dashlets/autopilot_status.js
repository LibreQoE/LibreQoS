import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";

export class AutopilotStatusDashlet extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 6;
    }

    title() {
        return "Autopilot Status";
    }

    tooltip() {
        return "<h5>Autopilot Status</h5><p>Shows Autopilot enablement, dry-run state, CPU pressure, managed allowlists, current virtualization/SQM states, and warnings.</p>";
    }

    subscribeTo() {
        return ["AutopilotStatus"];
    }

    buildContainer() {
        let base = super.buildContainer();

        const wrap = document.createElement("div");
        wrap.classList.add("p-2");

        const table = document.createElement("table");
        table.classList.add("table", "table-sm", "mb-2");
        const tbody = document.createElement("tbody");

        const mkRow = (label, valueEl) => {
            const tr = document.createElement("tr");
            const tdL = document.createElement("td");
            tdL.classList.add("text-muted");
            tdL.style.width = "45%";
            tdL.textContent = label;
            const tdV = document.createElement("td");
            tdV.appendChild(valueEl);
            tr.appendChild(tdL);
            tr.appendChild(tdV);
            return tr;
        };

        this.enabledEl = document.createElement("span");
        this.dryRunEl = document.createElement("span");
        this.cpuEl = document.createElement("span");
        this.nodesEl = document.createElement("span");
        this.circuitsEl = document.createElement("span");
        this.virtualizedEl = document.createElement("span");
        this.fqCodelEl = document.createElement("span");
        this.lastActionEl = document.createElement("span");

        tbody.appendChild(mkRow("Enabled", this.enabledEl));
        tbody.appendChild(mkRow("Dry Run", this.dryRunEl));
        tbody.appendChild(mkRow("CPU Max", this.cpuEl));
        tbody.appendChild(mkRow("Managed Nodes", this.nodesEl));
        tbody.appendChild(mkRow("Managed Circuits", this.circuitsEl));
        tbody.appendChild(mkRow("Virtualized Nodes", this.virtualizedEl));
        tbody.appendChild(mkRow("fq_codel Circuits", this.fqCodelEl));
        tbody.appendChild(mkRow("Last Action", this.lastActionEl));

        table.appendChild(tbody);
        wrap.appendChild(table);

        const warningsHeader = document.createElement("div");
        warningsHeader.classList.add("text-muted", "small", "mb-1");
        warningsHeader.textContent = "Warnings";
        wrap.appendChild(warningsHeader);

        this.warningsEl = document.createElement("div");
        this.warningsEl.classList.add("small");
        wrap.appendChild(this.warningsEl);

        base.appendChild(wrap);
        return base;
    }

    onMessage(msg) {
        if (msg.event !== "AutopilotStatus") {
            return;
        }

        const d = msg.data || {};
        this.enabledEl.textContent = d.enabled ? "Yes" : "No";
        this.dryRunEl.textContent = d.dry_run ? "Yes" : "No";

        if (d.cpu_max_pct === null || d.cpu_max_pct === undefined) {
            this.cpuEl.textContent = "N/A";
        } else {
            this.cpuEl.textContent = `${d.cpu_max_pct}%`;
        }

        this.nodesEl.textContent = (d.managed_nodes ?? 0).toString();
        this.circuitsEl.textContent = (d.managed_circuits ?? 0).toString();
        this.virtualizedEl.textContent = (d.virtualized_nodes ?? 0).toString();
        this.fqCodelEl.textContent = (d.fq_codel_circuits ?? 0).toString();
        this.lastActionEl.textContent = d.last_action_summary ?? "—";

        const warnings = Array.isArray(d.warnings) ? d.warnings : [];
        if (warnings.length === 0) {
            this.warningsEl.textContent = "—";
            return;
        }

        const ul = document.createElement("ul");
        ul.classList.add("mb-0");
        warnings.slice(0, 8).forEach((w) => {
            const li = document.createElement("li");
            li.textContent = w;
            ul.appendChild(li);
        });
        this.warningsEl.innerHTML = "";
        this.warningsEl.appendChild(ul);
    }
}
