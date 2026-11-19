import {saveConfig, loadConfig, renderConfigMenu, sendWsRequest} from "./config/config_helper";
import {normalizeDpdkStatus, normalizeWorkerCount} from "./config/dpdk_status.mjs";
import {
    applyWorkerCapacityMbps,
    parseWorkerCapacityInput,
    readWorkerCapacityMbps,
} from "./config/tuning_hygiene.mjs";
import {renderMarkupText} from "./config/markup_text.mjs";
import {t, tHtml} from "./i18n/index.js";

function validateConfig() {
    // Validate numeric fields
    const netdevBudgetUsecs = parseInt(document.getElementById("netdevBudgetUsecs").value);
    if (isNaN(netdevBudgetUsecs) || netdevBudgetUsecs < 0) {
        alert(t("config.tuning.validation.netdev_budget_usecs"));
        return false;
    }

    const netdevBudgetPackets = parseInt(document.getElementById("netdevBudgetPackets").value);
    if (isNaN(netdevBudgetPackets) || netdevBudgetPackets < 0) {
        alert(t("config.tuning.validation.netdev_budget_packets"));
        return false;
    }

    const rxUsecs = parseInt(document.getElementById("rxUsecs").value);
    if (isNaN(rxUsecs) || rxUsecs < 0) {
        alert(t("config.tuning.validation.rx_polling"));
        return false;
    }

    const txUsecs = parseInt(document.getElementById("txUsecs").value);
    if (isNaN(txUsecs) || txUsecs < 0) {
        alert(t("config.tuning.validation.tx_polling"));
        return false;
    }

    const workers = normalizeWorkerCount(document.getElementById("dpdkWorkers").value);
    if (!workers.ok) {
        alert(workers.error);
        return false;
    }

    const workerCapacity = parseWorkerCapacityInput(
        document.getElementById("dpdkWorkerCapacity").value,
    );
    if (!workerCapacity.ok) {
        alert(workerCapacity.error);
        return false;
    }

    return true;
}

function updateConfig() {
    // Update only the tuning section
    window.config.tuning = {
        set_cpu_governor_performance: document.getElementById("setCpuGovernorPerformance").checked,
        stop_irq_balance: document.getElementById("stopIrqBalance").checked,
        netdev_budget_usecs: parseInt(document.getElementById("netdevBudgetUsecs").value),
        netdev_budget_packets: parseInt(document.getElementById("netdevBudgetPackets").value),
        rx_usecs: parseInt(document.getElementById("rxUsecs").value),
        tx_usecs: parseInt(document.getElementById("txUsecs").value),
        disable_rxvlan: document.getElementById("disableRxVlan").checked,
        disable_txvlan: document.getElementById("disableTxVlan").checked,
        disable_offload: document.getElementById("disableOffload").value
            .split(' ')
            .map(s => s.trim())
            .filter(s => s.length > 0)
    };

    // A0: the optional [tuning] worker capacity. Carry the whole tuning
    // object (this function just replaced it above, preserving every existing
    // field) and set/delete ONLY worker_capacity_mbps: blank = delete =
    // compiled 1,125 Mbps effective default. The deprecated
    // placement_budget_mbps alias is removed on save.
    const workerCapacity = parseWorkerCapacityInput(
        document.getElementById("dpdkWorkerCapacity").value,
    );
    applyWorkerCapacityMbps(window.config, workerCapacity.value);

    // The DPDK worker count lives in the [queues] section (the installer
    // translates it to LIBREQOS_DPDK_WORKERS). Only touch this key so a save
    // here never resets the rest of the queues configuration.
    window.config.queues = window.config.queues && typeof window.config.queues === "object"
        ? window.config.queues
        : {};
    window.config.queues.workers = normalizeWorkerCount(document.getElementById("dpdkWorkers").value).value;
}

function kindBadgeClass(label) {
    // translatable: false — "Running", "Effective" and "Configured" are the
    // provenance labels `config/dpdk_status.mjs` emits (they mirror
    // `DpdkStatusKind` on the Rust side); translating a rendered label here
    // would break the comparison without translating the module that owns it.
    switch (label) {
        case "Running":
            return "text-bg-success";
        case "Effective":
            return "text-bg-info";
        case "Configured":
            return "text-bg-secondary";
        default:
            return "text-bg-light";
    }
}

function renderDpdkStatus(rows) {
    const target = document.getElementById("dpdkStatusTable");
    if (!target) {
        return;
    }
    target.replaceChildren();
    if (!rows.length) {
        const empty = document.createElement("div");
        empty.className = "text-secondary";
        empty.textContent = t("config.tuning.dpdk.unavailable");
        target.appendChild(empty);
        return;
    }

    const list = document.createElement("div");
    list.className = "d-flex flex-column gap-2";
    rows.forEach((row) => {
        const item = document.createElement("div");
        item.className = "border rounded p-2";

        const head = document.createElement("div");
        head.className = "d-flex justify-content-between align-items-start gap-2";
        const label = document.createElement("div");
        label.className = "fw-semibold";
        label.textContent = row.label;
        const badge = document.createElement("span");
        badge.className = `badge rounded-pill ${kindBadgeClass(row.kind)}`;
        badge.textContent = row.kind;
        head.append(label, badge);

        const value = document.createElement("div");
        value.textContent = row.value ?? t("config.tuning.dpdk.value_unavailable");

        const source = document.createElement("div");
        source.className = "text-secondary";
        source.textContent = row.source;

        item.append(head, value, source);
        if (row.note) {
            const note = document.createElement("div");
            note.className = "text-secondary fst-italic";
            note.textContent = row.note;
            item.appendChild(note);
        }
        list.appendChild(item);
    });
    target.appendChild(list);
}

function loadDpdkStatus() {
    sendWsRequest(
        "DpdkStatusResult",
        {DpdkStatus: {}},
        (msg) => {
            renderDpdkStatus(normalizeDpdkStatus(msg && msg.data));
        },
        () => {
            renderDpdkStatus([]);
        },
    );
}

// Render the configuration menu
renderConfigMenu('tuning');
// These paragraphs wrap a literal file name or command in `<code>`/`<strong>`,
// which the DOM applier cannot keep: they are empty in the page and rendered here.
renderMarkupText({
    dpdkWorkersLabel: tHtml("config.tuning.dpdk.workers_label"),
    dpdkWorkersHelp: tHtml("config.tuning.dpdk.workers_help"),
    dpdkWorkersNote: tHtml("config.tuning.dpdk.workers_note"),
    dpdkWorkerCapacityLabel: tHtml("config.tuning.dpdk.worker_capacity_label"),
    dpdkWorkerCapacityHelp: tHtml("config.tuning.dpdk.worker_capacity_help"),
    netdevSubtitle: tHtml("config.tuning.netdev.subtitle"),
    setCpuGovernorPerformanceHelp: tHtml("config.tuning.governor_help"),
    stopIrqBalanceHelp: tHtml("config.tuning.irqbalance_help"),
    offloadHelp: tHtml("config.tuning.disable_offload_help"),
});

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    if (window.config) {
        const tunables = window.config.tuning ?? {};
        window.config.tuning = tunables;

        // Boolean fields
        document.getElementById("setCpuGovernorPerformance").checked = tunables.set_cpu_governor_performance ?? true;
        document.getElementById("stopIrqBalance").checked = tunables.stop_irq_balance ?? false;
        document.getElementById("disableRxVlan").checked = tunables.disable_rxvlan ?? false;
        document.getElementById("disableTxVlan").checked = tunables.disable_txvlan ?? false;

        // Numeric fields
        document.getElementById("netdevBudgetUsecs").value = tunables.netdev_budget_usecs ?? 8000;
        document.getElementById("netdevBudgetPackets").value = tunables.netdev_budget_packets ?? 300;
        document.getElementById("rxUsecs").value = tunables.rx_usecs ?? 8;
        document.getElementById("txUsecs").value = tunables.tx_usecs ?? 8;

        // Array field (convert to space-separated string)
        document.getElementById("disableOffload").value =
            (tunables.disable_offload ?? ["gso", "tso", "lro", "sg", "gro"]).join(' ');

        // DPDK worker count (stored in [queues]).
        document.getElementById("dpdkWorkers").value = window.config.queues?.workers ?? "";

        // Optional [tuning] worker capacity (Mbps, effective); default 1125.
        document.getElementById("dpdkWorkerCapacity").value =
            readWorkerCapacityMbps(window.config) ?? 1125;

        // Add save button click handler
        document.getElementById('saveButton').addEventListener('click', () => {
            if (validateConfig()) {
                updateConfig();
                saveConfig(() => {
                    alert(t("config.tuning.saved"));
                    loadDpdkStatus();
                });
            }
        });

        loadDpdkStatus();
    } else {
        // translatable: false — "Configuration not found in window.config" is a console diagnostic, never rendered
        console.error("Configuration not found in window.config");
    }
});
