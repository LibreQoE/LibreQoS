import {saveConfig, loadConfig, renderConfigMenu, sendWsRequest} from "./config/config_helper";
import {normalizeDpdkStatus, normalizeWorkerCount} from "./config/dpdk_status.mjs";
import {
    applyPlacementBudgetMbps,
    parsePlacementBudgetInput,
    readPlacementBudgetMbps,
} from "./config/tuning_hygiene.mjs";

function validateConfig() {
    // Validate numeric fields
    const netdevBudgetUsecs = parseInt(document.getElementById("netdevBudgetUsecs").value);
    if (isNaN(netdevBudgetUsecs) || netdevBudgetUsecs < 0) {
        alert("Netdev Budget (μs) must be a positive number");
        return false;
    }

    const netdevBudgetPackets = parseInt(document.getElementById("netdevBudgetPackets").value);
    if (isNaN(netdevBudgetPackets) || netdevBudgetPackets < 0) {
        alert("Netdev Budget (Packets) must be a positive number");
        return false;
    }

    const rxUsecs = parseInt(document.getElementById("rxUsecs").value);
    if (isNaN(rxUsecs) || rxUsecs < 0) {
        alert("RX Polling Frequency (μs) must be a positive number");
        return false;
    }

    const txUsecs = parseInt(document.getElementById("txUsecs").value);
    if (isNaN(txUsecs) || txUsecs < 0) {
        alert("TX Polling Frequency (μs) must be a positive number");
        return false;
    }

    const workers = normalizeWorkerCount(document.getElementById("dpdkWorkers").value);
    if (!workers.ok) {
        alert(workers.error);
        return false;
    }

    const placementBudget = parsePlacementBudgetInput(
        document.getElementById("dpdkPlacementBudget").value,
    );
    if (!placementBudget.ok) {
        alert(placementBudget.error);
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

    // A0: the optional [tuning] placement budget. Carry the whole tuning
    // object (this function just replaced it above, preserving every existing
    // field) and set/delete ONLY placement_budget_mbps: blank = delete =
    // compiled 2.0G pre-headroom default.
    const placementBudget = parsePlacementBudgetInput(
        document.getElementById("dpdkPlacementBudget").value,
    );
    applyPlacementBudgetMbps(window.config, placementBudget.value);

    // The DPDK worker count lives in the [queues] section (the installer
    // translates it to LIBREQOS_DPDK_WORKERS). Only touch this key so a save
    // here never resets the rest of the queues configuration.
    window.config.queues = window.config.queues && typeof window.config.queues === "object"
        ? window.config.queues
        : {};
    window.config.queues.workers = normalizeWorkerCount(document.getElementById("dpdkWorkers").value).value;
}

function kindBadgeClass(label) {
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
        empty.textContent = "DPDK status is unavailable.";
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
        value.textContent = row.value ?? "Unavailable";

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

        // Optional [tuning] placement budget (Mbps pre-headroom); blank =
        // compiled default 2000 (effective 1500 at 25% headroom).
        document.getElementById("dpdkPlacementBudget").value =
            readPlacementBudgetMbps(window.config) ?? "";

        // Add save button click handler
        document.getElementById('saveButton').addEventListener('click', () => {
            if (validateConfig()) {
                updateConfig();
                saveConfig(() => {
                    alert("Configuration saved successfully!");
                    loadDpdkStatus();
                });
            }
        });

        loadDpdkStatus();
    } else {
        console.error("Configuration not found in window.config");
    }
});
