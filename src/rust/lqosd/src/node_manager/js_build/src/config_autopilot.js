import {
    loadAllShapedDevices,
    loadConfig,
    loadNetworkJson,
    renderConfigMenu,
    saveConfig,
} from "./config/config_helper";

let networkData = null;
let shapedDevices = null;
let selectedNodes = [];
let selectedCircuits = [];

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
        loadAllShapedDevices(
            (data) => {
                if (!Array.isArray(data)) {
                    console.warn("Shaped devices response was not an array:", data);
                    shapedDevices = [];
                } else {
                    shapedDevices = data;
                }
                populateCircuitSelector();
                resolve();
            },
            (err) => {
                console.error("Error loading shaped devices:", err);
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

    const circuits = new Set();
    if (Array.isArray(shapedDevices)) {
        shapedDevices.forEach((d) => {
            if (d && typeof d.circuit_id === "string" && d.circuit_id.trim() !== "") {
                circuits.add(d.circuit_id.trim());
            }
        });
    }

    Array.from(circuits)
        .sort((a, b) => a.localeCompare(b))
        .forEach((circuitId) => {
            const option = document.createElement("option");
            option.value = circuitId;
            option.textContent = circuitId;
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
    const tickSeconds = parseInt(document.getElementById("tickSeconds").value);
    if (Number.isNaN(tickSeconds) || tickSeconds < 1) {
        alert("Tick Interval must be at least 1 second");
        return false;
    }

    const cpuHighPct = parseInt(document.getElementById("cpuHighPct").value);
    const cpuLowPct = parseInt(document.getElementById("cpuLowPct").value);
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

    const idleMinMinutes = parseInt(document.getElementById("idleMinMinutes").value);
    if (!validateNonNegativeInt("Idle Minimum Duration", idleMinMinutes)) return false;

    const linksRttMissingSeconds = parseInt(document.getElementById("linksRttMissingSeconds").value);
    if (!validateNonNegativeInt("Links RTT Missing Timeout", linksRttMissingSeconds)) return false;

    const minStateDwellMinutes = parseInt(document.getElementById("minStateDwellMinutes").value);
    if (!validateNonNegativeInt("Minimum State Dwell", minStateDwellMinutes)) return false;

    const maxLinkChangesPerHour = parseInt(document.getElementById("maxLinkChangesPerHour").value);
    if (!validateNonNegativeInt("Max Link Changes Per Hour", maxLinkChangesPerHour)) return false;

    const reloadCooldownMinutes = parseInt(document.getElementById("reloadCooldownMinutes").value);
    if (!validateNonNegativeInt("Reload Cooldown", reloadCooldownMinutes)) return false;

    const circuitsRttMissingSeconds = parseInt(
        document.getElementById("circuitsRttMissingSeconds").value,
    );
    if (!validateNonNegativeInt("Circuits RTT Missing Timeout", circuitsRttMissingSeconds)) return false;

    const minSwitchDwellMinutes = parseInt(document.getElementById("minSwitchDwellMinutes").value);
    if (!validateNonNegativeInt("Minimum Switch Dwell", minSwitchDwellMinutes)) return false;

    const maxSwitchesPerHour = parseInt(document.getElementById("maxSwitchesPerHour").value);
    if (!validateNonNegativeInt("Max Switches Per Hour", maxSwitchesPerHour)) return false;

    const minScore = parseFloat(document.getElementById("minScore").value);
    if (!validatePercent("Minimum QoO Score", minScore)) return false;

    return true;
}

function updateConfig() {
    window.config.autopilot = {
        enabled: document.getElementById("enabled").checked,
        dry_run: document.getElementById("dryRun").checked,
        tick_seconds: parseInt(document.getElementById("tickSeconds").value),
        cpu: {
            mode: document.getElementById("cpuMode").value,
            cpu_high_pct: parseInt(document.getElementById("cpuHighPct").value),
            cpu_low_pct: parseInt(document.getElementById("cpuLowPct").value),
        },
        links: {
            enabled: document.getElementById("linksEnabled").checked,
            nodes: selectedNodes,
            idle_util_pct: parseFloat(document.getElementById("idleUtilPct").value),
            idle_min_minutes: parseInt(document.getElementById("idleMinMinutes").value),
            rtt_missing_seconds: parseInt(document.getElementById("linksRttMissingSeconds").value),
            unvirtualize_util_pct: parseFloat(document.getElementById("unvirtualizeUtilPct").value),
            min_state_dwell_minutes: parseInt(document.getElementById("minStateDwellMinutes").value),
            max_link_changes_per_hour: parseInt(document.getElementById("maxLinkChangesPerHour").value),
            reload_cooldown_minutes: parseInt(document.getElementById("reloadCooldownMinutes").value),
        },
        circuits: {
            enabled: document.getElementById("circuitsEnabled").checked,
            circuits: selectedCircuits,
            switching_enabled: document.getElementById("switchingEnabled").checked,
            independent_directions: document.getElementById("independentDirections").checked,
            rtt_missing_seconds: parseInt(document.getElementById("circuitsRttMissingSeconds").value),
            min_switch_dwell_minutes: parseInt(document.getElementById("minSwitchDwellMinutes").value),
            max_switches_per_hour: parseInt(document.getElementById("maxSwitchesPerHour").value),
            persist_sqm_overrides: document.getElementById("persistSqmOverrides").checked,
        },
        qoo: {
            enabled: document.getElementById("qooEnabled").checked,
            min_score: parseFloat(document.getElementById("minScore").value),
        },
    };
}

renderConfigMenu("autopilot");

Promise.all([
    loadNetworkData().catch(() => null),
    loadCircuitsData().catch(() => null),
    new Promise((resolve) => {
        loadConfig(() => resolve());
    }),
]).then(() => {
    const ap = window.config?.autopilot ?? {};
    const cpu = ap.cpu ?? {};
    const links = ap.links ?? {};
    const circuits = ap.circuits ?? {};
    const qoo = ap.qoo ?? {};

    document.getElementById("enabled").checked = ap.enabled ?? false;
    document.getElementById("dryRun").checked = ap.dry_run ?? true;
    document.getElementById("tickSeconds").value = ap.tick_seconds ?? 1;

    document.getElementById("cpuMode").value = cpu.mode ?? "cpu_aware";
    document.getElementById("cpuHighPct").value = cpu.cpu_high_pct ?? 75;
    document.getElementById("cpuLowPct").value = cpu.cpu_low_pct ?? 55;

    document.getElementById("linksEnabled").checked = links.enabled ?? true;
    document.getElementById("idleUtilPct").value = links.idle_util_pct ?? 2.0;
    document.getElementById("idleMinMinutes").value = links.idle_min_minutes ?? 15;
    document.getElementById("linksRttMissingSeconds").value = links.rtt_missing_seconds ?? 120;
    document.getElementById("unvirtualizeUtilPct").value = links.unvirtualize_util_pct ?? 5.0;
    document.getElementById("minStateDwellMinutes").value = links.min_state_dwell_minutes ?? 30;
    document.getElementById("maxLinkChangesPerHour").value = links.max_link_changes_per_hour ?? 4;
    document.getElementById("reloadCooldownMinutes").value = links.reload_cooldown_minutes ?? 10;

    selectedNodes = Array.isArray(links.nodes) ? links.nodes.slice() : [];
    selectedNodes.sort((a, b) => a.localeCompare(b));
    updateNodesList();

    document.getElementById("circuitsEnabled").checked = circuits.enabled ?? true;
    document.getElementById("switchingEnabled").checked = circuits.switching_enabled ?? true;
    document.getElementById("independentDirections").checked =
        circuits.independent_directions ?? true;
    document.getElementById("circuitsRttMissingSeconds").value = circuits.rtt_missing_seconds ?? 120;
    document.getElementById("minSwitchDwellMinutes").value = circuits.min_switch_dwell_minutes ?? 30;
    document.getElementById("maxSwitchesPerHour").value = circuits.max_switches_per_hour ?? 4;
    document.getElementById("persistSqmOverrides").checked =
        circuits.persist_sqm_overrides ?? true;

    selectedCircuits = Array.isArray(circuits.circuits) ? circuits.circuits.slice() : [];
    selectedCircuits.sort((a, b) => a.localeCompare(b));
    updateCircuitsList();

    document.getElementById("qooEnabled").checked = qoo.enabled ?? true;
    document.getElementById("minScore").value = qoo.min_score ?? 80.0;

    document.getElementById("addNodeBtn").addEventListener("click", addNode);
    document.getElementById("nodeSelector").addEventListener("keypress", (e) => {
        if (e.key === "Enter") {
            e.preventDefault();
            addNode();
        }
    });

    document.getElementById("addCircuitBtn").addEventListener("click", addCircuitFromSelector);
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
            alert("Autopilot configuration saved successfully!");
        });
    });
});

