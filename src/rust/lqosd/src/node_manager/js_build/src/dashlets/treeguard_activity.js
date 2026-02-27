import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {get_ws_client} from "../pubsub/ws";

function formatUnixSecondsToLocalTime(unixSeconds) {
    const n = typeof unixSeconds === "number" ? unixSeconds : parseInt(unixSeconds, 10);
    if (!Number.isFinite(n) || n <= 0) {
        return "";
    }
    return new Date(n * 1000).toLocaleTimeString(undefined, {
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
    });
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

function formatSqmLabel(prefix, token) {
    const { down, up } = parseDirectionalSqmToken(token);
    if (!down && !up) return prefix;
    if (down === up) return `${prefix}: ${down}`;
    return `${prefix}: DL ${down}, UL ${up}`;
}

function mkIcon(iconClass, extraClasses = []) {
    const icon = document.createElement("i");
    icon.classList.add("fa", "fa-fw", iconClass);
    extraClasses.forEach((c) => icon.classList.add(c));
    return icon;
}

function renderAction(actionRaw) {
    const raw = (actionRaw ?? "").toString();
    const [verbRaw, payloadRaw] = splitOnce(raw, ":");
    const verb = (verbRaw ?? "").toString().trim();
    const payload = (payloadRaw ?? "").toString().trim();

    const lowerVerb = verb.toLowerCase();

    // Defaults
    let iconClass = "fa-question-circle";
    let iconExtra = ["text-muted"];
    let label = raw;

    const isFailed = lowerVerb.endsWith("_failed") || lowerVerb.includes("failed");

    if (lowerVerb === "virtualize") {
        iconClass = "fa-compress";
        iconExtra = [];
        label = "Virtualize";
    } else if (lowerVerb === "unvirtualize") {
        iconClass = "fa-expand";
        iconExtra = [];
        label = "Unvirtualize";
    } else if (lowerVerb === "dry_run_toggled") {
        iconClass = "fa-toggle-on";
        iconExtra = ["text-muted"];
        label = "Dry-run toggled";
    } else if (lowerVerb === "reload_success") {
        iconClass = "fa-refresh";
        iconExtra = ["text-success"];
        label = "Reload success";
    } else if (lowerVerb === "reload_skipped") {
        iconClass = "fa-refresh";
        iconExtra = ["text-muted"];
        label = "Reload skipped";
    } else if (lowerVerb === "reload_failed") {
        iconClass = "fa-refresh";
        iconExtra = ["text-danger"];
        label = "Reload failed";
    } else if (lowerVerb.startsWith("clear_virtual_override")) {
        iconClass = "fa-eraser";
        iconExtra = ["text-warning"];
        label = "Clear virtual override";
        if (lowerVerb.endsWith("_conflict")) label += " (conflict)";
        if (lowerVerb.endsWith("_failed")) label += " failed";
    } else if (lowerVerb.startsWith("set_virtual_override")) {
        iconClass = "fa-compress";
        iconExtra = isFailed ? ["text-danger"] : [];
        label = isFailed ? "Set virtual override failed" : "Set virtual override";
    } else if (lowerVerb.startsWith("clear_sqm_overrides")) {
        iconClass = "fa-eraser";
        iconExtra = ["text-warning"];
        label = "Clear SQM overrides";
        if (lowerVerb.endsWith("_conflict")) label += " (conflict)";
    } else if (lowerVerb === "set_sqm_override_failed") {
        iconClass = "fa-exclamation-circle";
        iconExtra = ["text-danger"];
        label = "SQM override failed";
    } else if (lowerVerb === "apply_sqm_live_failed") {
        iconClass = "fa-exclamation-circle";
        iconExtra = ["text-danger"];
        label = formatSqmLabel("SQM live apply failed", payload);
    } else if (lowerVerb === "would_set_sqm_override") {
        iconClass = "fa-eye";
        iconExtra = ["text-muted"];
        label = formatSqmLabel("Dry-run SQM override", payload);
    } else if (lowerVerb === "set_sqm_override") {
        if (payload.toLowerCase().includes("cake")) {
            iconClass = "fa-birthday-cake";
        } else if (payload.toLowerCase().includes("fq_codel")) {
            iconClass = "fa-tachometer";
        } else {
            iconClass = "fa-sliders";
        }
        iconExtra = [];
        label = formatSqmLabel("SQM override", payload);
    } else if (lowerVerb === "set_sqm_live") {
        iconClass = "fa-bolt";
        iconExtra = [];
        label = formatSqmLabel("SQM live apply", payload);
    } else if (isFailed) {
        iconClass = "fa-exclamation-circle";
        iconExtra = ["text-danger"];
        label = verb;
    }

    return {
        raw,
        label,
        iconClass,
        iconExtra,
    };
}

export class TreeGuardActivityDashlet extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 12;
        this.circuitNameById = new Map();
        this.nodeIdByName = new Map();
    }

    title() {
        return "TreeGuard Activity";
    }

    tooltip() {
        return "<h5>TreeGuard Activity</h5><p>Recent TreeGuard actions, including dry-run entries and persisted changes.</p>";
    }

    subscribeTo() {
        return ["TreeGuardActivity"];
    }

    setup() {
        const wsClient = get_ws_client();
        const wrapped = (msg) => {
            wsClient.off("AllShapedDevices", wrapped);
            const devices = msg && Array.isArray(msg.data) ? msg.data : [];
            devices.forEach((d) => {
                const id = (d && d.circuit_id ? String(d.circuit_id) : "").trim();
                const name = (d && d.circuit_name ? String(d.circuit_name) : "").trim();
                if (!id || !name) return;
                if (!this.circuitNameById.has(id)) {
                    this.circuitNameById.set(id, name);
                }
            });
        };
        wsClient.on("AllShapedDevices", wrapped);
        wsClient.send({ AllShapedDevices: {} });

        const treeWrapped = (msg) => {
            wsClient.off("NetworkTree", treeWrapped);
            const data = msg && Array.isArray(msg.data) ? msg.data : [];
            data.forEach((entry) => {
                if (!Array.isArray(entry) || entry.length < 2) return;
                const id = entry[0];
                const node = entry[1];
                const name = (node && node.name ? String(node.name) : "").trim();
                if (!name) return;
                if (!this.nodeIdByName.has(name)) {
                    this.nodeIdByName.set(name, id);
                }
            });
        };
        wsClient.on("NetworkTree", treeWrapped);
        wsClient.send({ NetworkTree: {} });
    }

    buildContainer() {
        let base = super.buildContainer();

        const wrap = document.createElement("div");
        wrap.classList.add("p-2");

        const table = document.createElement("table");
        table.classList.add("table", "table-sm", "table-striped", "mb-0", "small");

        const thead = document.createElement("thead");
        thead.classList.add("small");
        const headRow = document.createElement("tr");
        ["Local Time", "Entity", "Action", "Persisted", "Reason"].forEach((h) => {
            const th = document.createElement("th");
            th.textContent = h;
            headRow.appendChild(th);
        });
        thead.appendChild(headRow);

        this.tbody = document.createElement("tbody");

        table.appendChild(thead);
        table.appendChild(this.tbody);
        wrap.appendChild(table);

        base.appendChild(wrap);
        return base;
    }

    onMessage(msg) {
        if (msg.event !== "TreeGuardActivity") {
            return;
        }

        const entries = Array.isArray(msg.data) ? msg.data : [];
        this.tbody.innerHTML = "";

        if (entries.length === 0) {
            const tr = document.createElement("tr");
            const td = document.createElement("td");
            td.colSpan = 5;
            td.classList.add("text-muted");
            td.textContent = "No recent activity";
            tr.appendChild(td);
            this.tbody.appendChild(tr);
            return;
        }

        entries.slice(0, 50).forEach((e) => {
            const tr = document.createElement("tr");
            tr.classList.add("small");

            const tdTime = document.createElement("td");
            tdTime.textContent = formatUnixSecondsToLocalTime(e.time);

            const tdEntity = document.createElement("td");
            const et = (e.entity_type ?? "").toString();
            const eid = (e.entity_id ?? "").toString();
            const entityType = et.toLowerCase().trim();
            const entityId = eid.trim();

            const prefix = document.createElement("span");
            prefix.classList.add("text-muted");
            prefix.textContent = et ? `${et}: ` : "";
            tdEntity.appendChild(prefix);

            const mkLink = (href, text, title = "") => {
                const a = document.createElement("a");
                a.href = href;
                a.textContent = text;
                a.classList.add("redactable");
                if (title) a.title = title;
                return a;
            };

            if (entityType === "circuit" && entityId) {
                const name = this.circuitNameById.get(entityId);
                const display = (name ?? "").toString().trim() || entityId;
                const title = name ? entityId : "";
                tdEntity.appendChild(
                    mkLink(`circuit.html?id=${encodeURIComponent(entityId)}`, display, title),
                );
            } else if (entityType === "node" && entityId) {
                const nodeId = this.nodeIdByName.get(entityId);
                if (nodeId !== undefined && nodeId !== null) {
                    tdEntity.appendChild(
                        mkLink(
                            `tree.html?parent=${encodeURIComponent(String(nodeId))}`,
                            entityId,
                            `Node ID: ${nodeId}`,
                        ),
                    );
                } else {
                    const span = document.createElement("span");
                    span.textContent = entityId;
                    span.classList.add("redactable");
                    tdEntity.appendChild(span);
                }
            } else {
                const span = document.createElement("span");
                span.textContent = entityId || et || "";
                span.classList.add("redactable");
                tdEntity.appendChild(span);
            }

            const tdAction = document.createElement("td");
            const a = renderAction(e.action);
            tdAction.title = a.raw;
            tdAction.appendChild(mkIcon(a.iconClass, a.iconExtra));
            const actionText = document.createElement("span");
            actionText.textContent = ` ${a.label}`;
            tdAction.appendChild(actionText);

            const tdPersisted = document.createElement("td");
            tdPersisted.classList.add("text-center");
            const persisted = !!e.persisted;
            const persistedIcon = persisted
                ? mkIcon("fa-check", ["text-success"])
                : mkIcon("fa-times", ["text-muted"]);
            persistedIcon.setAttribute("aria-label", persisted ? "Persisted" : "Not persisted");
            persistedIcon.title = persisted ? "Persisted" : "Not persisted";
            tdPersisted.appendChild(persistedIcon);

            const tdReason = document.createElement("td");
            tdReason.textContent = e.reason ?? "";

            tr.appendChild(tdTime);
            tr.appendChild(tdEntity);
            tr.appendChild(tdAction);
            tr.appendChild(tdPersisted);
            tr.appendChild(tdReason);
            this.tbody.appendChild(tr);
        });
    }
}
