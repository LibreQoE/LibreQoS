import {saveConfig, loadConfig, renderConfigMenu} from "./config/config_helper";
import {normalizeDefaultSqm, isFlatCompileMode, SUPPORTED_DEFAULT_SQM} from "./config/queues_hygiene.mjs";
import {applyBandwidthMbps} from "./config/dataplane_hygiene.mjs";

// The flat-mode queue-sizing knobs are read only by `topology.compile_mode =
// "flat"`; in the other supported modes the DPDK compiler derives sizing and
// these values are ignored. We therefore render them only in flat mode.
let flatMode = false;

function validateConfig() {
    // Validate numeric fields
    const uplink = parseFloat(document.getElementById("uplinkBandwidth").value);
    if (isNaN(uplink) || uplink < 1) {
        alert("Uplink Bandwidth must be a number greater than 0");
        return false;
    }

    const downlink = parseFloat(document.getElementById("downlinkBandwidth").value);
    if (isNaN(downlink) || downlink < 1) {
        alert("Downlink Bandwidth must be a number greater than 0");
        return false;
    }

    if (flatMode) {
        const pnDownload = parseInt(document.getElementById("generatedPnDownload").value);
        if (isNaN(pnDownload) || pnDownload < 1) {
            alert("Per-Node Download must be a number greater than 0");
            return false;
        }

        const pnUpload = parseInt(document.getElementById("generatedPnUpload").value);
        if (isNaN(pnUpload) || pnUpload < 1) {
            alert("Per-Node Upload must be a number greater than 0");
            return false;
        }

        const overrideQueues = document.getElementById("overrideQueues").value;
        if (overrideQueues && (isNaN(overrideQueues) || overrideQueues < 1)) {
            alert("Override Queues must be a number greater than 0");
            return false;
        }
    }

    return true;
}

function updateConfig() {
    // Preserve any queues keys this page does not own (notably `workers`, which
    // the DPDK Dataplane section edits) so a save here never silently resets
    // them. Only the fields this page presents are written.
    const existing = (window.config && typeof window.config.queues === "object" && window.config.queues)
        ? window.config.queues
        : {};
    const next = {
        ...existing,
        // The DPDK pipeline is cake-only; a migrated kernel SQM is normalized.
        default_sqm: SUPPORTED_DEFAULT_SQM,
        queue_mode: document.getElementById("queueMode").value,
        uplink_bandwidth_mbps: parseFloat(document.getElementById("uplinkBandwidth").value),
        downlink_bandwidth_mbps: parseFloat(document.getElementById("downlinkBandwidth").value),
    };
    if (flatMode) {
        // Flat-compile-mode sizing (ignored in ap_only/ap_site/full). Outside
        // flat mode these keys are left untouched (preserved above).
        next.generated_pn_download_mbps = parseInt(document.getElementById("generatedPnDownload").value);
        next.generated_pn_upload_mbps = parseInt(document.getElementById("generatedPnUpload").value);
        next.override_available_queues = document.getElementById("overrideQueues").value ?
            parseInt(document.getElementById("overrideQueues").value) : null;
        next.use_binpacking = document.getElementById("useBinpacking").checked;
    }
    window.config.queues = next;
    // TRANSITION: keep the v3 `[dataplane]` single source in sync with the
    // capacity edited here (the control plane + the engine prefer `[dataplane]`).
    applyBandwidthMbps(
        window.config,
        next.downlink_bandwidth_mbps,
        next.uplink_bandwidth_mbps,
    );
}

function updateFlatModeVisibility() {
    const section = document.getElementById("flatModeQueueSizing");
    const hint = document.getElementById("flatModeHint");
    if (section) {
        section.classList.toggle("d-none", !flatMode);
    }
    if (hint) {
        hint.classList.toggle("d-none", flatMode);
    }
}

// Render the configuration menu
renderConfigMenu('queues');

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    if (window.config && window.config.queues) {
        const queues = window.config.queues;

        const sqm = normalizeDefaultSqm(queues.default_sqm);
        document.getElementById("defaultSqm").value = sqm.value;
        if (sqm.legacy) {
            console.warn(`Default SQM "${sqm.original}" is not supported by the DPDK pipeline; it will be saved as "${sqm.value}".`);
        }
        document.getElementById("queueMode").value = queues.queue_mode ?? ((queues.monitor_only ?? false) ? "observe" : "shape");

        document.getElementById("uplinkBandwidth").value = queues.uplink_bandwidth_mbps ?? 1000;
        document.getElementById("downlinkBandwidth").value = queues.downlink_bandwidth_mbps ?? 1000;

        // Flat-mode sizing fields (rendered only when compile_mode == "flat").
        document.getElementById("generatedPnDownload").value = queues.generated_pn_download_mbps ?? 1000;
        document.getElementById("generatedPnUpload").value = queues.generated_pn_upload_mbps ?? 1000;
        document.getElementById("overrideQueues").value = queues.override_available_queues ?? "";
        document.getElementById("useBinpacking").checked = queues.use_binpacking ?? false;

        flatMode = isFlatCompileMode(window.config);
        updateFlatModeVisibility();

        // Add save button click handler
        document.getElementById('saveButton').addEventListener('click', () => {
            if (validateConfig()) {
                updateConfig();
                saveConfig(() => {
                    alert("Configuration saved successfully!");
                });
            }
        });
    } else {
        console.error("Queue configuration not found in window.config");
    }
});
