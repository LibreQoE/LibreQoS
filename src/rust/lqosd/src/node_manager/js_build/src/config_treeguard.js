import {
    loadAllCircuitDirectoryRows,
    loadConfig,
    loadNetworkJson,
    renderConfigMenu,
    saveConfig,
} from "./config/config_helper";
import {defaultTreeguardConfig, ensureTreeguardConfig} from "./config/treeguard_defaults";

let networkData = null;
let circuitRows = null;
let selectedNodes = [];
let selectedCircuits = [];

function updateLinksEnrollmentUi() {
    const allNodes = document.getElementById("linksAllNodes")?.checked ?? true;
    const section = document.getElementById("nodesAllowlistSection");
    if (section) {
        section.style.display = allNodes ? "none" : "";
    }
}

function updateCircuitsEnrollmentUi() {
    const allCircuits = document.getElementById("circuitsAllCircuits")?.checked ?? true;
    const section = document.getElementById("circuitsAllowlistSection");
    if (section) {
        section.style.display = allCircuits ? "none" : "";
    }
}

function loadNetworkData() {
    return new Promise((resolve, reject) => {
        loadNetworkJson(
            (data) => {
                if (typeof data === "string" && data === "Not done yet") {
                    console.error("Network.json file not found on server");
                    alert("Network configuration not found. Please ensure network.json exists.");
                    resolve();
                    return;
                }
                networkData = data;
                populateNodeSelector();
                resolve();
            },
            (err) => {
                console.error("Error loading network data:", err);
                alert("Failed to load network nodes. You can still add nodes manually.");
                reject(err);
            },
        );
    });
}

function loadCircuitsData() {
    return new Promise((resolve, reject) => {
        loadAllCircuitDirectoryRows(
            (data) => {
                if (!Array.isArray(data)) {
                    console.warn("Circuit directory response was not an array:", data);
                    circuitRows = [];
                } else {
                    circuitRows = data;
                }
                populateCircuitSelector();
                resolve();
            },
            (err) => {
                console.error("Error loading circuit directory:", err);
                alert("Failed to load circuit list. You can still add circuits manually.");
                reject(err);
            },
        );
    });
}

function populateNodeSelector() {
    const selector = document.getElementById("nodeSelector");
    selector.innerHTML = '<option value="">Select a node...</option>';

    function iterate(data, level = 0) {
        if (typeof data !== "object" || data === null) {
            return;
        }

        for (const [key, value] of Object.entries(data)) {
            const option = document.createElement("option");
            option.value = key;

            let prefix = "";
            for (let i = 0; i < level; i++) {
                prefix += "- ";
            }
            option.textContent = prefix + key;
            selector.appendChild(option);

            if (value && typeof value === "object" && value.children != null) {
                iterate(value.children, level + 1);
            }
        }
    }

    if (networkData) {
        iterate(networkData);
    }
}

function populateCircuitSelector() {
    const selector = document.getElementById("circuitSelector");
    selector.innerHTML = '<option value="">Select a circuit...</option>';

    if (!Array.isArray(circuitRows)) {
        return;
    }

    circuitRows.forEach((row) => {
        const circuitId = (row?.circuit_id ?? "").trim();
        if (!circuitId) {
            return;
        }
        const circuitName = (row?.circuit_name ?? "").trim();
        const display = circuitName && circuitName !== circuitId
            ? `${circuitName} (${circuitId})`
            : circuitId;
        const option = document.createElement("option");
        option.value = circuitId;
        option.textContent = display;
        selector.appendChild(option);
    });
}

function addNode() {
    const selector = document.getElementById("nodeSelector");
    const selected = selector.value;
    if (!selected) {
        alert("Please select a node to add");
        return;
    }
    if (selectedNodes.includes(selected)) {
        alert("This node is already allowlisted");
        return;
    }
    selectedNodes.push(selected);
    selectedNodes.sort((a, b) => a.localeCompare(b));
    updateNodesList();
    selector.value = "";
}

function removeNode(nodeName) {
    const index = selectedNodes.indexOf(nodeName);
    if (index > -1) {
        selectedNodes.splice(index, 1);
        updateNodesList();
    }
}

function updateNodesList() {
    const listContainer = document.getElementById("selectedNodesList");
    listContainer.innerHTML = "";

    if (selectedNodes.length === 0) {
        listContainer.innerHTML = '<div class="text-muted">No nodes selected</div>';
        return;
    }

    selectedNodes.forEach((node) => {
        const listItem = document.createElement("div");
        listItem.className = "list-group-item d-flex justify-content-between align-items-center";

        const nodeName = document.createElement("span");
        nodeName.textContent = node;

        const removeBtn = document.createElement("button");
        removeBtn.className = "btn btn-sm btn-outline-danger";
        removeBtn.innerHTML = '<i class="fa fa-times"></i>';
        removeBtn.onclick = () => removeNode(node);

        listItem.appendChild(nodeName);
        listItem.appendChild(removeBtn);
        listContainer.appendChild(listItem);
    });
}

function addCircuitFromSelector() {
    const selector = document.getElementById("circuitSelector");
    const selected = selector.value;
    if (!selected) {
        alert("Please select a circuit to add");
        return;
    }
    addCircuitId(selected);
    selector.value = "";
}

function addCircuitFromManual() {
    const input = document.getElementById("circuitManual");
    const circuitId = input.value.trim();
    if (!circuitId) {
        alert("Please enter a circuit ID");
        return;
    }
    addCircuitId(circuitId);
    input.value = "";
}

function addCircuitId(circuitId) {
    if (selectedCircuits.includes(circuitId)) {
        alert("This circuit is already allowlisted");
        return;
    }
    selectedCircuits.push(circuitId);
    selectedCircuits.sort((a, b) => a.localeCompare(b));
    updateCircuitsList();
}

function removeCircuit(circuitId) {
    const index = selectedCircuits.indexOf(circuitId);
    if (index > -1) {
        selectedCircuits.splice(index, 1);
        updateCircuitsList();
    }
}

function updateCircuitsList() {
    const listContainer = document.getElementById("selectedCircuitsList");
    listContainer.innerHTML = "";

    if (selectedCircuits.length === 0) {
        listContainer.innerHTML = '<div class="text-muted">No circuits selected</div>';
        return;
    }

    selectedCircuits.forEach((circuitId) => {
        const listItem = document.createElement("div");
        listItem.className = "list-group-item d-flex justify-content-between align-items-center";

        const circuitName = document.createElement("span");
        circuitName.textContent = circuitId;

        const removeBtn = document.createElement("button");
        removeBtn.className = "btn btn-sm btn-outline-danger";
        removeBtn.innerHTML = '<i class="fa fa-times"></i>';
        removeBtn.onclick = () => removeCircuit(circuitId);

        listItem.appendChild(circuitName);
        listItem.appendChild(removeBtn);
        listContainer.appendChild(listItem);
    });
}

function validatePercent(name, value, min = 0, max = 100) {
    if (Number.isNaN(value) || value < min || value > max) {
        alert(`${name} must be between ${min} and ${max}`);
        return false;
    }
    return true;
}

function validateNonNegativeInt(name, value) {
    if (Number.isNaN(value) || value < 0) {
        alert(`${name} must be a number >= 0`);
        return false;
    }
    return true;
}

function validateConfig() {
    const tickSeconds = parseInt(document.getElementById("tickSeconds").value, 10);
    if (Number.isNaN(tickSeconds) || tickSeconds < 1) {
        alert("Tick Interval must be at least 1 second");
        return false;
    }

    const cpuHighPct = parseInt(document.getElementById("cpuHighPct").value, 10);
    const cpuLowPct = parseInt(document.getElementById("cpuLowPct").value, 10);
    if (!validatePercent("CPU High Threshold", cpuHighPct)) return false;
    if (!validatePercent("CPU Low Threshold", cpuLowPct)) return false;
    if (cpuLowPct > cpuHighPct) {
        alert("CPU Low Threshold must be less than or equal to CPU High Threshold");
        return false;
    }

    const idleUtilPct = parseFloat(document.getElementById("idleUtilPct").value);
    if (!validatePercent("Idle Utilization", idleUtilPct)) return false;

    const unvirtualizeUtilPct = parseFloat(document.getElementById("unvirtualizeUtilPct").value);
    if (!validatePercent("Unvirtualize Utilization", unvirtualizeUtilPct)) return false;

    const topLevelSafeUtilPct = parseFloat(document.getElementById("topLevelSafeUtilPct").value);
    if (!validatePercent("Top-Level Safe Utilization", topLevelSafeUtilPct)) return false;

    const idleMinMinutes = parseInt(document.getElementById("idleMinMinutes").value, 10);
    if (!validateNonNegativeInt("Idle Minimum Duration", idleMinMinutes)) return false;

    const linksRttMissingSeconds = parseInt(
        document.getElementById("linksRttMissingSeconds").value,
        10,
    );
    if (!validateNonNegativeInt("Links RTT Missing Timeout", linksRttMissingSeconds)) return false;

    const minStateDwellMinutes = parseInt(
        document.getElementById("minStateDwellMinutes").value,
        10,
    );
    if (!validateNonNegativeInt("Minimum State Dwell", minStateDwellMinutes)) return false;

    const maxLinkChangesPerHour = parseInt(
        document.getElementById("maxLinkChangesPerHour").value,
        10,
    );
    if (!validateNonNegativeInt("Max Link Changes Per Hour", maxLinkChangesPerHour)) return false;

    const reloadCooldownMinutes = parseInt(
        document.getElementById("reloadCooldownMinutes").value,
        10,
    );
    if (!validateNonNegativeInt("Reload Cooldown", reloadCooldownMinutes)) return false;

    const circuitsRttMissingSeconds = parseInt(
        document.getElementById("circuitsRttMissingSeconds").value,
        10,
    );
    if (!validateNonNegativeInt("Circuits RTT Missing Timeout", circuitsRttMissingSeconds)) return false;

    const circuitsIdleUtilPct = parseFloat(document.getElementById("circuitsIdleUtilPct").value);
    if (!validatePercent("Circuits Idle Utilization", circuitsIdleUtilPct)) return false;

    const circuitsIdleMinMinutes = parseInt(
        document.getElementById("circuitsIdleMinMinutes").value,
        10,
    );
    if (!validateNonNegativeInt("Circuits Idle Minimum Duration", circuitsIdleMinMinutes)) return false;

    const circuitsUpgradeUtilPct = parseFloat(
        document.getElementById("circuitsUpgradeUtilPct").value,
    );
    if (!validatePercent("Circuits Upgrade Utilization", circuitsUpgradeUtilPct)) return false;
    if (circuitsUpgradeUtilPct < circuitsIdleUtilPct) {
        alert("Circuits Upgrade Utilization must be greater than or equal to Circuits Idle Utilization");
        return false;
    }

    const minSwitchDwellMinutes = parseInt(
        document.getElementById("minSwitchDwellMinutes").value,
        10,
    );
    if (!validateNonNegativeInt("Minimum Switch Dwell", minSwitchDwellMinutes)) return false;

    const maxSwitchesPerHour = parseInt(document.getElementById("maxSwitchesPerHour").value, 10);
    if (!validateNonNegativeInt("Max Switches Per Hour", maxSwitchesPerHour)) return false;

    const minScore = parseFloat(document.getElementById("minScore").value);
    if (!validatePercent("Minimum QoO Score", minScore)) return false;

    return true;
}

function updateConfig() {
    window.config.treeguard = {
        enabled: document.getElementById("enabled").checked,
        dry_run: document.getElementById("dryRun").checked,
        tick_seconds: parseInt(document.getElementById("tickSeconds").value, 10),
        cpu: {
            mode: document.getElementById("cpuMode").value,
            cpu_high_pct: parseInt(document.getElementById("cpuHighPct").value, 10),
            cpu_low_pct: parseInt(document.getElementById("cpuLowPct").value, 10),
        },
        links: {
            enabled: document.getElementById("linksEnabled").checked,
            all_nodes: document.getElementById("linksAllNodes").checked,
            nodes: selectedNodes,
            idle_util_pct: parseFloat(document.getElementById("idleUtilPct").value),
            idle_min_minutes: parseInt(document.getElementById("idleMinMinutes").value, 10),
            rtt_missing_seconds: parseInt(
                document.getElementById("linksRttMissingSeconds").value,
                10,
            ),
            unvirtualize_util_pct: parseFloat(document.getElementById("unvirtualizeUtilPct").value),
            min_state_dwell_minutes: parseInt(
                document.getElementById("minStateDwellMinutes").value,
                10,
            ),
            max_link_changes_per_hour: parseInt(
                document.getElementById("maxLinkChangesPerHour").value,
                10,
            ),
            reload_cooldown_minutes: parseInt(
                document.getElementById("reloadCooldownMinutes").value,
                10,
            ),
            top_level_auto_virtualize: document.getElementById("topLevelAutoVirtualize").checked,
            top_level_safe_util_pct: parseFloat(document.getElementById("topLevelSafeUtilPct").value),
        },
        circuits: {
            enabled: document.getElementById("circuitsEnabled").checked,
            all_circuits: document.getElementById("circuitsAllCircuits").checked,
            circuits: selectedCircuits,
            switching_enabled: document.getElementById("switchingEnabled").checked,
            independent_directions: document.getElementById("independentDirections").checked,
            idle_util_pct: parseFloat(document.getElementById("circuitsIdleUtilPct").value),
            idle_min_minutes: parseInt(document.getElementById("circuitsIdleMinMinutes").value, 10),
            rtt_missing_seconds: parseInt(
                document.getElementById("circuitsRttMissingSeconds").value,
                10,
            ),
            upgrade_util_pct: parseFloat(document.getElementById("circuitsUpgradeUtilPct").value),
            min_switch_dwell_minutes: parseInt(
                document.getElementById("minSwitchDwellMinutes").value,
                10,
            ),
            max_switches_per_hour: parseInt(
                document.getElementById("maxSwitchesPerHour").value,
                10,
            ),
            persist_sqm_overrides: document.getElementById("persistSqmOverrides").checked,
        },
        qoo: {
            enabled: document.getElementById("qooEnabled").checked,
            min_score: parseFloat(document.getElementById("minScore").value),
        },
    };
}

renderConfigMenu("treeguard");

Promise.all([
    loadNetworkData().catch(() => null),
    loadCircuitsData().catch(() => null),
    new Promise((resolve) => {
        loadConfig(() => resolve());
    }),
]).then(() => {
    const tg = ensureTreeguardConfig(window.config);
    const cpu = tg.cpu;
    const links = tg.links;
    const circuits = tg.circuits;
    const qoo = tg.qoo;

    document.getElementById("enabled").checked = tg.enabled;
    document.getElementById("dryRun").checked = tg.dry_run;
    document.getElementById("tickSeconds").value = tg.tick_seconds;

    document.getElementById("cpuMode").value = cpu.mode;
    document.getElementById("cpuHighPct").value = cpu.cpu_high_pct;
    document.getElementById("cpuLowPct").value = cpu.cpu_low_pct;

    document.getElementById("linksEnabled").checked = links.enabled;
    document.getElementById("linksAllNodes").checked = links.all_nodes;
    document.getElementById("topLevelAutoVirtualize").checked = links.top_level_auto_virtualize;
    document.getElementById("topLevelSafeUtilPct").value = links.top_level_safe_util_pct;
    document.getElementById("idleUtilPct").value = links.idle_util_pct;
    document.getElementById("idleMinMinutes").value = links.idle_min_minutes;
    document.getElementById("linksRttMissingSeconds").value = links.rtt_missing_seconds;
    document.getElementById("unvirtualizeUtilPct").value = links.unvirtualize_util_pct;
    document.getElementById("minStateDwellMinutes").value = links.min_state_dwell_minutes;
    document.getElementById("maxLinkChangesPerHour").value = links.max_link_changes_per_hour;
    document.getElementById("reloadCooldownMinutes").value = links.reload_cooldown_minutes;

    selectedNodes = Array.isArray(links.nodes) ? links.nodes.slice() : [];
    selectedNodes.sort((a, b) => a.localeCompare(b));
    updateNodesList();
    updateLinksEnrollmentUi();

    document.getElementById("circuitsEnabled").checked = circuits.enabled;
    document.getElementById("circuitsAllCircuits").checked = circuits.all_circuits;
    document.getElementById("switchingEnabled").checked = circuits.switching_enabled;
    document.getElementById("independentDirections").checked = circuits.independent_directions;
    document.getElementById("circuitsIdleUtilPct").value = circuits.idle_util_pct;
    document.getElementById("circuitsIdleMinMinutes").value = circuits.idle_min_minutes;
    document.getElementById("circuitsRttMissingSeconds").value = circuits.rtt_missing_seconds;
    document.getElementById("circuitsUpgradeUtilPct").value = circuits.upgrade_util_pct;
    document.getElementById("minSwitchDwellMinutes").value = circuits.min_switch_dwell_minutes;
    document.getElementById("maxSwitchesPerHour").value = circuits.max_switches_per_hour;
    document.getElementById("persistSqmOverrides").checked = circuits.persist_sqm_overrides;

    selectedCircuits = Array.isArray(circuits.circuits) ? circuits.circuits.slice() : [];
    selectedCircuits.sort((a, b) => a.localeCompare(b));
    updateCircuitsList();
    updateCircuitsEnrollmentUi();

    document.getElementById("qooEnabled").checked = qoo.enabled;
    document.getElementById("minScore").value = qoo.min_score;

    document.getElementById("addNodeBtn").addEventListener("click", addNode);
    document.getElementById("linksAllNodes").addEventListener("change", updateLinksEnrollmentUi);
    document.getElementById("nodeSelector").addEventListener("keypress", (e) => {
        if (e.key === "Enter") {
            e.preventDefault();
            addNode();
        }
    });

    document.getElementById("addCircuitBtn").addEventListener("click", addCircuitFromSelector);
    document.getElementById("circuitsAllCircuits").addEventListener("change", updateCircuitsEnrollmentUi);
    document.getElementById("circuitSelector").addEventListener("keypress", (e) => {
        if (e.key === "Enter") {
            e.preventDefault();
            addCircuitFromSelector();
        }
    });

    document.getElementById("addCircuitManualBtn").addEventListener("click", addCircuitFromManual);
    document.getElementById("circuitManual").addEventListener("keypress", (e) => {
        if (e.key === "Enter") {
            e.preventDefault();
            addCircuitFromManual();
        }
    });

    document.getElementById("saveButton").addEventListener("click", () => {
        if (!validateConfig()) {
            return;
        }
        updateConfig();
        saveConfig(() => {
            alert("TreeGuard configuration saved successfully!");
        });
    });
});
