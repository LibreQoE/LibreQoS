import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";

function clamp(n, min, max) {
    return Math.min(Math.max(n, min), max);
}

function splitOnce(s, sep) {
    const str = (s ?? "").toString();
    const idx = str.indexOf(sep);
    if (idx === -1) return [str, ""];
    return [str.slice(0, idx), str.slice(idx + sep.length)];
}

function parseDirectionalSqmToken(token) {
    const t = (token ?? "").toString().trim();
    if (!t) return { down: "", up: "" };
    if (!t.includes("/")) return { down: t, up: t };
    const [down, up] = splitOnce(t, "/");
    return { down: (down ?? "").toString().trim(), up: (up ?? "").toString().trim() };
}

function formatSqmLabel(token) {
    const { down, up } = parseDirectionalSqmToken(token);
    if (!down && !up) return "";
    if (down === up) return down;
    return `DL ${down}, UL ${up}`;
}

function mkIcon(iconClass, extraClasses = []) {
    const icon = document.createElement("i");
    icon.classList.add("fa", "fa-fw", iconClass);
    extraClasses.forEach((c) => icon.classList.add(c));
    return icon;
}

function setBgClass(el, cls) {
    [
        "bg-primary",
        "bg-secondary",
        "bg-success",
        "bg-info",
        "bg-warning",
        "bg-danger",
        "bg-light",
        "bg-dark",
    ].forEach((c) => el.classList.remove(c));
    if (cls) el.classList.add(cls);
}

function mkProgressMetric() {
    const container = document.createElement("div");
    container.classList.add("d-flex", "align-items-center", "gap-2");

    const progress = document.createElement("div");
    progress.classList.add("progress", "flex-grow-1");
    progress.style.height = "0.6rem";

    const bar = document.createElement("div");
    bar.classList.add("progress-bar");
    bar.setAttribute("role", "progressbar");
    bar.style.width = "0%";
    progress.appendChild(bar);

    const text = document.createElement("span");
    text.classList.add("small");
    text.textContent = "—";

    container.appendChild(progress);
    container.appendChild(text);

    return { container, progress, bar, text };
}

function updateProgressMetric(metric, { value, max, text, bgClass, title }) {
    const safeMax = Number.isFinite(max) && max > 0 ? max : 1;
    const safeValue = Number.isFinite(value) ? value : 0;
    const pct = clamp((safeValue / safeMax) * 100.0, 0, 100);

    metric.bar.style.width = `${pct.toFixed(1)}%`;
    metric.bar.setAttribute("aria-valuemin", "0");
    metric.bar.setAttribute("aria-valuemax", safeMax.toString());
    metric.bar.setAttribute("aria-valuenow", safeValue.toString());
    setBgClass(metric.bar, bgClass);

    metric.text.textContent = text ?? "—";
    metric.container.title = title ?? "";
}

function renderLastAction(summaryRaw) {
    const raw = (summaryRaw ?? "").toString().trim();

    const wrap = document.createElement("div");
    wrap.classList.add("d-flex", "align-items-start", "gap-2");
    wrap.style.whiteSpace = "normal";
    wrap.style.wordBreak = "break-word";

    if (!raw) {
        wrap.appendChild(mkIcon("fa-minus-circle", ["text-muted"]));
        const span = document.createElement("span");
        span.textContent = "—";
        wrap.appendChild(span);
        return wrap;
    }

    wrap.title = raw;

    let iconClass = "fa-info-circle";
    let iconExtra = ["text-muted"];
    let label = raw;

    let m = raw.match(/^Reloaded LibreQoS:\s*(.+)$/);
    if (m) {
        iconClass = "fa-refresh";
        iconExtra = ["text-success"];
        label = `Reloaded LibreQoS: ${m[1]}`;
    }

    m = raw.match(/^(Virtualized|Unvirtualized) node '(.+)'$/);
    if (m) {
        iconClass = m[1] === "Virtualized" ? "fa-compress" : "fa-expand";
        iconExtra = [];
        label = `${m[1]} node: ${m[2]}`;
    }

    m = raw.match(/^Queued (virtualization|restore) for node '(.+)'$/);
    if (m) {
        iconClass = "fa-hourglass-half";
        iconExtra = ["text-primary"];
        label = `Queued ${m[1]}: ${m[2]}`;
    }

    m = raw.match(/^Would set SQM override for circuit '(.+)' -> (.+)$/);
    if (m) {
        iconClass = "fa-eye";
        iconExtra = ["text-muted"];
        const tokenLabel = formatSqmLabel(m[2]);
        label = tokenLabel
            ? `Dry-run SQM override: ${m[1]} → ${tokenLabel}`
            : `Dry-run SQM override: ${m[1]}`;
    }

    m = raw.match(/^SQM override for circuit '(.+)' -> (.+)$/);
    if (m) {
        const token = (m[2] ?? "").toString();
        const tokenLower = token.toLowerCase();
        if (tokenLower.includes("cake")) {
            iconClass = "fa-birthday-cake";
            iconExtra = [];
        } else if (tokenLower.includes("fq_codel")) {
            iconClass = "fa-tachometer";
            iconExtra = [];
        } else {
            iconClass = "fa-sliders";
            iconExtra = [];
        }
        const tokenLabel = formatSqmLabel(token);
        label = tokenLabel ? `SQM override: ${m[1]} → ${tokenLabel}` : `SQM override: ${m[1]}`;
    }

    wrap.appendChild(mkIcon(iconClass, iconExtra));
    const span = document.createElement("span");
    span.textContent = label;
    wrap.appendChild(span);
    return wrap;
}

export class TreeGuardStatusDashlet extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 6;
        this.lastStatusData = null;
    }

    title() {
        return "TreeGuard Status";
    }

    tooltip() {
        return "<h5>TreeGuard Status</h5><p>Shows TreeGuard enablement, dry-run state, CPU pressure, managed allowlists, current virtualization/SQM states, and warnings.</p>";
    }

    subscribeTo() {
        return ["TreeGuardStatus"];
    }

    buildContainer() {
        const base = super.buildContainer();
        const wrap = document.createElement("div");
        wrap.classList.add("p-2");

        const tableWrap = document.createElement("div");
        tableWrap.classList.add("lqos-table-wrap");

        const table = document.createElement("table");
        table.classList.add("lqos-table", "lqos-table-compact", "mb-0");
        const tbody = document.createElement("tbody");

        const mkRow = (label, valueEl) => {
            const tr = document.createElement("tr");
            const tdL = document.createElement("td");
            tdL.classList.add("table-label-cell");
            tdL.style.width = "45%";
            tdL.textContent = label;
            const tdV = document.createElement("td");
            tdV.classList.add("table-value-cell");
            tdV.appendChild(valueEl);
            tr.appendChild(tdL);
            tr.appendChild(tdV);
            return tr;
        };

        this.enabledEl = document.createElement("span");
        this.dryRunEl = document.createElement("span");
        this.cpuMetric = mkProgressMetric();
        this.nodesMetric = mkProgressMetric();
        this.circuitsMetric = mkProgressMetric();
        this.virtualizedMetric = mkProgressMetric();
        this.fqCodelMetric = mkProgressMetric();
        this.lastActionEl = document.createElement("div");
        this.lastActionEl.classList.add("small");

        tbody.appendChild(mkRow("Enabled", this.enabledEl));
        tbody.appendChild(mkRow("Dry Run", this.dryRunEl));
        tbody.appendChild(mkRow("CPU Max", this.cpuMetric.container));
        tbody.appendChild(mkRow("Managed Nodes", this.nodesMetric.container));
        tbody.appendChild(mkRow("Managed Circuits", this.circuitsMetric.container));
        tbody.appendChild(mkRow("Virtualized Nodes", this.virtualizedMetric.container));
        tbody.appendChild(mkRow("fq_codel Circuits", this.fqCodelMetric.container));
        tbody.appendChild(mkRow("Last Action", this.lastActionEl));

        table.appendChild(tbody);
        tableWrap.appendChild(table);
        wrap.appendChild(tableWrap);

        const warningsHeader = document.createElement("div");
        warningsHeader.classList.add("text-muted", "small", "mt-3", "mb-1");
        warningsHeader.textContent = "Warnings";
        wrap.appendChild(warningsHeader);

        this.warningsEl = document.createElement("div");
        this.warningsEl.classList.add("small");
        wrap.appendChild(this.warningsEl);

        base.appendChild(wrap);
        return base;
    }

    onMessage(msg) {
        if (msg.event !== "TreeGuardStatus") {
            return;
        }

        const data = msg.data || {};
        this.lastStatusData = data;
        this.renderStatus(data);
    }

    renderStatus(data) {
        this.enabledEl.textContent = data.enabled ? "Yes" : "No";
        this.dryRunEl.textContent = data.dry_run ? "Yes" : "No";

        const cpuRaw = data.cpu_max_pct;
        if (cpuRaw === null || cpuRaw === undefined) {
            updateProgressMetric(this.cpuMetric, {
                value: 0,
                max: 100,
                text: "N/A",
                bgClass: "bg-secondary",
                title: "CPU usage unavailable",
            });
        } else {
            const cpu = clamp(Number(cpuRaw), 0, 100);
            const bg = cpu >= 90 ? "bg-danger" : cpu >= 70 ? "bg-warning" : "bg-success";
            updateProgressMetric(this.cpuMetric, {
                value: cpu,
                max: 100,
                text: `${cpu.toFixed(0)}%`,
                bgClass: bg,
                title: `CPU max: ${cpu.toFixed(0)}%`,
            });
        }

        const managedNodes = Math.max(0, Math.trunc(Number(data.managed_nodes ?? 0) || 0));
        const managedCircuits = Math.max(0, Math.trunc(Number(data.managed_circuits ?? 0) || 0));
        const virtualizedNodesExact = Math.max(0, Math.trunc(Number(data.virtualized_nodes ?? 0) || 0));
        const fqCodelCircuitsExact = Math.max(0, Math.trunc(Number(data.fq_codel_circuits ?? 0) || 0));
        const totalNodes = Math.max(0, Math.trunc(Number(data.total_nodes ?? 0) || 0));
        const nodesMax = totalNodes > 0 ? totalNodes : Math.max(1, managedNodes);
        const nodesPct = nodesMax ? clamp((managedNodes / nodesMax) * 100, 0, 100) : 0;
        updateProgressMetric(this.nodesMetric, {
            value: managedNodes,
            max: nodesMax,
            text: totalNodes > 0 ? `${managedNodes} / ${totalNodes}` : managedNodes.toString(),
            bgClass: "bg-info",
            title: totalNodes > 0 ? `${managedNodes} / ${totalNodes} (${nodesPct.toFixed(0)}%)` : `${managedNodes}`,
        });

        const totalCircuits = Math.max(0, Math.trunc(Number(data.total_circuits ?? 0) || 0));
        const circuitsMax = totalCircuits > 0 ? totalCircuits : Math.max(1, managedCircuits);
        const circuitsPct = circuitsMax ? clamp((managedCircuits / circuitsMax) * 100, 0, 100) : 0;
        updateProgressMetric(this.circuitsMetric, {
            value: managedCircuits,
            max: circuitsMax,
            text: totalCircuits > 0 ? `${managedCircuits} / ${totalCircuits}` : managedCircuits.toString(),
            bgClass: "bg-info",
            title: totalCircuits > 0
                ? `${managedCircuits} / ${totalCircuits} (${circuitsPct.toFixed(0)}%)`
                : `${managedCircuits}`,
        });

        const virtualizedNodes = virtualizedNodesExact;
        const virtMax = Math.max(1, managedNodes);
        const virtPct = managedNodes > 0 ? clamp((virtualizedNodes / virtMax) * 100, 0, 100) : 0;
        updateProgressMetric(this.virtualizedMetric, {
            value: virtualizedNodes,
            max: virtMax,
            text: managedNodes > 0 ? `${virtualizedNodes} / ${managedNodes}` : virtualizedNodes.toString(),
            bgClass: "bg-primary",
            title: managedNodes > 0
                ? `${virtualizedNodes} / ${managedNodes} (${virtPct.toFixed(0)}%)`
                : `${virtualizedNodes}`,
        });

        const fqCodelCircuits = fqCodelCircuitsExact;
        const fqMax = Math.max(1, managedCircuits);
        const fqPct = managedCircuits > 0 ? clamp((fqCodelCircuits / fqMax) * 100, 0, 100) : 0;
        updateProgressMetric(this.fqCodelMetric, {
            value: fqCodelCircuits,
            max: fqMax,
            text: managedCircuits > 0 ? `${fqCodelCircuits} / ${managedCircuits}` : fqCodelCircuits.toString(),
            bgClass: "bg-warning",
            title: managedCircuits > 0
                ? `${fqCodelCircuits} / ${managedCircuits} (${fqPct.toFixed(0)}%)`
                : `${fqCodelCircuits}`,
        });

        this.lastActionEl.innerHTML = "";
        this.lastActionEl.appendChild(renderLastAction(data.last_action_summary));

        const warnings = Array.isArray(data.warnings) ? data.warnings : [];
        if (warnings.length === 0) {
            this.warningsEl.textContent = "—";
            return;
        }

        const ul = document.createElement("ul");
        ul.classList.add("mb-0");
        warnings.slice(0, 8).forEach((warning) => {
            const li = document.createElement("li");
            li.textContent = warning;
            ul.appendChild(li);
        });
        this.warningsEl.innerHTML = "";
        this.warningsEl.appendChild(ul);
    }
}
