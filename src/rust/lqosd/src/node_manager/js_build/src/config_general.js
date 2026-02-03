import {saveConfig, loadConfig, loadQooProfiles, renderConfigMenu} from "./config/config_helper";

let qooProfiles = null;

function escapeHtml(value) {
    return String(value)
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;")
        .replace(/"/g, "&quot;")
        .replace(/'/g, "&#039;");
}

function getProfilesPath() {
    const dir = window.config?.lqos_directory;
    if (!dir) return "qoo_profiles.json";
    const trimmed = String(dir).replace(/\/+$/, "");
    return `${trimmed}/qoo_profiles.json`;
}

function selectedQooProfileId() {
    const select = document.getElementById("qooProfileId");
    if (!select) return "";
    return String(select.value || "");
}

function defaultQooProfileId() {
    const configuredDefault = qooProfiles?.default_profile_id;
    const trimmed = configuredDefault ? String(configuredDefault).trim() : "";
    return trimmed || "web_browsing";
}

function defaultQooProfileLabel() {
    const defaultId = defaultQooProfileId();
    const profile = qooProfiles?.profiles?.find(p => p.id === defaultId);
    return profile?.name || defaultId || "web_browsing";
}

function setQooProfilesAlert(kind, message) {
    const holder = document.getElementById("qooProfilesLoadAlert");
    if (!holder) return;
    if (!message) {
        holder.innerHTML = "";
        return;
    }
    const safeKind = kind === "danger" || kind === "warning" || kind === "info" || kind === "success"
        ? kind
        : "warning";
    holder.innerHTML = `<div class="alert alert-${safeKind} mb-3" role="alert">${escapeHtml(message)}</div>`;
}

function renderQooProfileSelect() {
    const select = document.getElementById("qooProfileId");
    if (!select) return;

    const configuredId = window.config?.qoo_profile_id ? String(window.config.qoo_profile_id).trim() : "";
    const profiles = Array.isArray(qooProfiles?.profiles) ? qooProfiles.profiles : [];

    const options = [];
    const defaultLabel = qooProfiles ? defaultQooProfileLabel() : "web_browsing";
    options.push({ value: "", label: `(default) ${defaultLabel}` });

    profiles.forEach((p) => {
        const id = p?.id ? String(p.id) : "";
        const name = p?.name ? String(p.name) : id;
        if (!id) return;
        options.push({ value: id, label: `${name} (${id})` });
    });

    const isKnown = configuredId && profiles.some(p => p.id === configuredId);
    if (configuredId && !isKnown) {
        options.push({ value: configuredId, label: `Unknown (configured): ${configuredId}` });
    }

    select.innerHTML = options
        .map(o => `<option value="${escapeHtml(o.value)}">${escapeHtml(o.label)}</option>`)
        .join("");

    select.value = configuredId || "";
}

function renderQooProfilesTable() {
    const body = document.getElementById("qooProfilesTableBody");
    if (!body) return;

    const profiles = Array.isArray(qooProfiles?.profiles) ? qooProfiles.profiles : null;
    if (!profiles || profiles.length === 0) {
        body.innerHTML = `<tr><td colspan="4" class="text-muted small">No profile data loaded.</td></tr>`;
        return;
    }

    const selected = selectedQooProfileId();
    const defaultId = defaultQooProfileId();
    const effectiveSelected = selected ? selected : defaultId;

    body.innerHTML = profiles.map((p) => {
        const id = p?.id ? String(p.id) : "";
        const name = p?.name ? String(p.name) : id;
        const description = p?.description ? String(p.description) : "";

        const isDefault = id === defaultId;
        const isSelected = id === effectiveSelected;
        const flags = [
            isDefault ? `<span class="badge bg-secondary me-1">Default</span>` : "",
            isSelected ? `<span class="badge bg-primary">Selected</span>` : "",
        ].join("");

        const rowClass = isSelected ? "table-active" : "";
        return `
            <tr class="${rowClass}">
                <td>${escapeHtml(name)}</td>
                <td><code>${escapeHtml(id)}</code></td>
                <td>${description ? escapeHtml(description) : "—"}</td>
                <td>${flags || "—"}</td>
            </tr>
        `;
    }).join("");
}

function validateConfig() {
    // Validate required fields
    const nodeName = document.getElementById("nodeName").value.trim();
    if (!nodeName) {
        alert("Node Name is required");
        return false;
    }

    const packetCaptureTime = parseInt(document.getElementById("packetCaptureTime").value);
    if (isNaN(packetCaptureTime) || packetCaptureTime < 1) {
        alert("Packet Capture Time must be a number greater than 0");
        return false;
    }

    const queueCheckPeriod = parseInt(document.getElementById("queueCheckPeriod").value);
    if (isNaN(queueCheckPeriod) || queueCheckPeriod < 100) {
        alert("Queue Check Period must be a number of at least 100 milliseconds");
        return false;
    }

    // Validate webserver listen address if provided
    const webserverListen = document.getElementById("webserverListen").value.trim();
    if (webserverListen) {
        const parts = webserverListen.split(':');
        if (parts.length !== 2 || isNaN(parseInt(parts[1]))) {
            alert("Web Server Listen Address must be in format IP:PORT");
            return false;
        }
    }

    return true;
}

function updateConfig() {
    // Update only the general configuration section
    window.config.node_name = document.getElementById("nodeName").value.trim();
    window.config.packet_capture_time = parseInt(document.getElementById("packetCaptureTime").value);
    window.config.queue_check_period_ms = parseInt(document.getElementById("queueCheckPeriod").value);
    window.config.disable_webserver = document.getElementById("disableWebserver").checked;
    window.config.disable_icmp_ping = document.getElementById("disableIcmpPing").checked;
    window.config.enable_circuit_heatmaps = document.getElementById("enableCircuitHeatmaps").checked;
    window.config.enable_site_heatmaps = document.getElementById("enableSiteHeatmaps").checked;
    window.config.enable_asn_heatmaps = document.getElementById("enableAsnHeatmaps").checked;

    const selectedProfileId = selectedQooProfileId();
    window.config.qoo_profile_id = selectedProfileId ? selectedProfileId : null;
    
    const webserverListen = document.getElementById("webserverListen").value.trim();
    window.config.webserver_listen = webserverListen ? webserverListen : null;
}

// Render the configuration menu
renderConfigMenu('general');

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    if (window.config) {
        // Required fields
        if (window.config.node_id) {
            document.getElementById("nodeId").value = window.config.node_id;
        }
        if (window.config.node_name) {
            document.getElementById("nodeName").value = window.config.node_name;
        }
        if (window.config.packet_capture_time) {
            document.getElementById("packetCaptureTime").value = window.config.packet_capture_time;
        }
        if (window.config.queue_check_period_ms) {
            document.getElementById("queueCheckPeriod").value = window.config.queue_check_period_ms;
        }

        // Optional fields with nullish coalescing
        document.getElementById("disableWebserver").checked = window.config.disable_webserver ?? false;
        document.getElementById("webserverListen").value = window.config.webserver_listen ?? "";
        document.getElementById("disableIcmpPing").checked = window.config.disable_icmp_ping ?? false;
        document.getElementById("enableCircuitHeatmaps").checked = window.config.enable_circuit_heatmaps ?? true;
        document.getElementById("enableSiteHeatmaps").checked = window.config.enable_site_heatmaps ?? true;
        document.getElementById("enableAsnHeatmaps").checked = window.config.enable_asn_heatmaps ?? true;

        const profilesPath = getProfilesPath();
        const profilesPathEl = document.getElementById("qooProfilesPath");
        if (profilesPathEl) profilesPathEl.textContent = profilesPath;

        const qooSelect = document.getElementById("qooProfileId");
        if (qooSelect) {
            qooSelect.addEventListener("change", () => {
                if (qooProfiles && Array.isArray(qooProfiles.profiles)) {
                    const selectedId = selectedQooProfileId();
                    if (selectedId && !qooProfiles.profiles.some(p => p.id === selectedId)) {
                        setQooProfilesAlert(
                            "warning",
                            `Selected QoO profile '${selectedId}' was not found in ${profilesPath}.`,
                        );
                    } else {
                        setQooProfilesAlert("", "");
                    }
                }
                renderQooProfilesTable();
            });
        }

        setQooProfilesAlert("", "");
        loadQooProfiles(
            (data) => {
                qooProfiles = data;
                renderQooProfileSelect();
                renderQooProfilesTable();

                const configuredId = window.config?.qoo_profile_id ? String(window.config.qoo_profile_id).trim() : "";
                if (configuredId) {
                    const known = Array.isArray(qooProfiles?.profiles) && qooProfiles.profiles.some(p => p.id === configuredId);
                    if (!known) {
                        setQooProfilesAlert(
                            "warning",
                            `Configured QoO profile '${configuredId}' was not found in ${profilesPath}.`,
                        );
                    }
                }
            },
            () => {
                qooProfiles = null;
                renderQooProfileSelect();
                renderQooProfilesTable();
                setQooProfilesAlert(
                    "warning",
                    `Unable to load QoO profiles. Create or fix ${profilesPath}, then reload this page.`,
                );
            },
        );

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
        console.error("Configuration not found in window.config");
    }
});
