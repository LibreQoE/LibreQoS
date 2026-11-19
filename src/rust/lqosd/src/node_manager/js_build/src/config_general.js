import {saveConfig, loadConfig, loadQooProfiles, renderConfigMenu} from "./config/config_helper";
import {t, tHtml} from "./i18n/index.js";

let qooProfiles = null;

function escapeHtml(value) {
    return String(value)
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;")
        .replace(/"/g, "&quot;")
        .replace(/'/g, "&#039;");
}

function setCobrandUploadStatus(kind, message) {
    const holder = document.getElementById("cobrandUploadStatus");
    if (!holder) return;
    const safeKind = kind === "danger" || kind === "warning" || kind === "info" || kind === "success"
        ? kind
        : "secondary";
    holder.innerHTML = `<div class="alert alert-${safeKind} mb-0">${message}</div>`;
}

function selectedCobrandFile() {
    const input = document.getElementById("cobrandFile");
    return input?.files?.[0] ?? null;
}

function selectedCobrandLooksLikePng(file) {
    if (!file) return true;
    const typeOk = file.type === "image/png";
    const extensionOk = String(file.name || "").toLowerCase().endsWith(".png");
    return typeOk || extensionOk;
}

async function uploadCobrandIfSelected() {
    const file = selectedCobrandFile();
    if (!file) {
        return;
    }
    if (!selectedCobrandLooksLikePng(file)) {
        throw new Error(t("config.general.cobrand.error.not_png"));
    }

    setCobrandUploadStatus("info", t("config.general.cobrand.uploading"));
    const response = await fetch("/local-api/config/cobrand", {
        method: "POST",
        credentials: "same-origin",
        headers: {
            "Content-Type": "image/png",
        },
        body: file,
    });

    if (!response.ok) {
        const detail = (await response.text().catch(() => "")).trim();
        const message = detail || t("config.general.cobrand.upload_failed_http", {status: response.status});
        throw new Error(message);
    }

    document.getElementById("cobrandFile").value = "";
    setCobrandUploadStatus("success", t("config.general.cobrand.uploaded"));
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
    options.push({ value: "", label: t("config.general.qoo.option_default", {profile: defaultLabel}) });

    profiles.forEach((p) => {
        const id = p?.id ? String(p.id) : "";
        const name = p?.name ? String(p.name) : id;
        if (!id) return;
        options.push({ value: id, label: `${name} (${id})` });
    });

    const isKnown = configuredId && profiles.some(p => p.id === configuredId);
    if (configuredId && !isKnown) {
        options.push({ value: configuredId, label: t("config.general.qoo.option_unknown", {id: configuredId}) });
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
        body.innerHTML = `<tr><td colspan="4" class="text-muted small">${t("config.general.qoo.no_data")}</td></tr>`;
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
            isDefault ? `<span class="badge bg-secondary me-1">${t("config.general.qoo.badge_default")}</span>` : "",
            isSelected ? `<span class="badge bg-primary">${t("config.general.qoo.badge_selected")}</span>` : "",
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
        alert(t("config.general.node.name_required"));
        return false;
    }

    const packetCaptureTime = parseInt(document.getElementById("packetCaptureTime").value);
    if (isNaN(packetCaptureTime) || packetCaptureTime < 1) {
        alert(t("config.general.node.capture_time_invalid"));
        return false;
    }

    // Validate webserver listen address if provided
    const webserverListen = document.getElementById("webserverListen").value.trim();
    if (webserverListen) {
        const parts = webserverListen.split(':');
        if (parts.length !== 2 || isNaN(parseInt(parts[1]))) {
            alert(t("config.general.node.listen_invalid"));
            return false;
        }
    }

    return true;
}

function updateConfig() {
    // Update only the general configuration section
    window.config.node_name = document.getElementById("nodeName").value.trim();
    window.config.packet_capture_time = parseInt(document.getElementById("packetCaptureTime").value);
    window.config.disable_webserver = document.getElementById("disableWebserver").checked;
    window.config.disable_icmp_ping = document.getElementById("disableIcmpPing").checked;
    window.config.exclude_efficiency_cores = document.getElementById("excludeEfficiencyCores").checked;
    window.config.enable_circuit_heatmaps = document.getElementById("enableCircuitHeatmaps").checked;
    window.config.enable_site_heatmaps = document.getElementById("enableSiteHeatmaps").checked;
    window.config.enable_asn_heatmaps = document.getElementById("enableAsnHeatmaps").checked;
    window.config.display_cobrand = document.getElementById("displayCobrand").checked;

    const selectedProfileId = selectedQooProfileId();
    window.config.qoo_profile_id = selectedProfileId ? selectedProfileId : null;
    
    const sslEnabled = !!window.config?.ssl?.enabled;
    const webserverListenInput = document.getElementById("webserverListen");
    const webserverListen = webserverListenInput.value.trim();
    if (sslEnabled) {
        window.config.webserver_listen = "127.0.0.1:9123";
    } else {
        window.config.webserver_listen = webserverListen ? webserverListen : null;
    }
}

// The prose the page renders itself: each paragraph wraps a literal path or file
// name in markup, so it cannot ride a `data-i18n` attribute (that applier writes
// text). The messages carry the markup; the placeholders carry the literals.
function renderTranslatedProse(profilesPath) {
    const defined = document.getElementById("qooProfilesNote");
    if (defined) defined.innerHTML = tHtml("config.general.qoo.defined_in", {path: profilesPath});
    const displayHelp = document.getElementById("cobrandDisplaySubtitle");
    if (displayHelp) displayHelp.innerHTML = tHtml("config.general.cobrand.display_subtitle");
    const enableHelp = document.getElementById("cobrandEnableHelp");
    if (enableHelp) enableHelp.innerHTML = tHtml("config.general.cobrand.enable_help");
    const uploadHelp = document.getElementById("cobrandUploadSubtitle");
    if (uploadHelp) uploadHelp.innerHTML = tHtml("config.general.cobrand.upload_subtitle");
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

        // Optional fields with nullish coalescing
        const sslEnabled = !!window.config?.ssl?.enabled;
        document.getElementById("disableWebserver").checked = window.config.disable_webserver ?? false;
        document.getElementById("webserverListen").value = sslEnabled
            ? "127.0.0.1:9123"
            : (window.config.webserver_listen ?? "");
        document.getElementById("webserverListen").disabled = sslEnabled;
        document.getElementById("sslWebserverNotice").classList.toggle("d-none", !sslEnabled);
        document.getElementById("disableIcmpPing").checked = window.config.disable_icmp_ping ?? false;
        document.getElementById("excludeEfficiencyCores").checked = window.config.exclude_efficiency_cores ?? true;
        document.getElementById("enableCircuitHeatmaps").checked = window.config.enable_circuit_heatmaps ?? true;
        document.getElementById("enableSiteHeatmaps").checked = window.config.enable_site_heatmaps ?? true;
        document.getElementById("enableAsnHeatmaps").checked = window.config.enable_asn_heatmaps ?? true;
        document.getElementById("displayCobrand").checked = window.config.display_cobrand ?? false;
        setCobrandUploadStatus(
            "secondary",
            tHtml("config.general.cobrand.pending"),
        );

        const cobrandInput = document.getElementById("cobrandFile");
        if (cobrandInput) {
            cobrandInput.addEventListener("change", () => {
                const file = selectedCobrandFile();
                if (!file) {
                    setCobrandUploadStatus(
                        "secondary",
                        tHtml("config.general.cobrand.pending"),
                    );
                    return;
                }
                if (!selectedCobrandLooksLikePng(file)) {
                    setCobrandUploadStatus("warning", t("config.general.cobrand.wrong_file"));
                    return;
                }
                setCobrandUploadStatus(
                    "info",
                    tHtml("config.general.cobrand.selected", {name: file.name}),
                );
            });
        }

        const profilesPath = getProfilesPath();
        renderTranslatedProse(profilesPath);

        const qooSelect = document.getElementById("qooProfileId");
        if (qooSelect) {
            qooSelect.addEventListener("change", () => {
                if (qooProfiles && Array.isArray(qooProfiles.profiles)) {
                    const selectedId = selectedQooProfileId();
                    if (selectedId && !qooProfiles.profiles.some(p => p.id === selectedId)) {
                        setQooProfilesAlert(
                            "warning",
                            t("config.general.qoo.selected_missing", {id: selectedId, path: profilesPath}),
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
                            t("config.general.qoo.configured_missing", {id: configuredId, path: profilesPath}),
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
                    t("config.general.qoo.load_failed", {path: profilesPath}),
                );
            },
        );

        // Add save button click handler
        document.getElementById('saveButton').addEventListener('click', async () => {
            if (validateConfig()) {
                try {
                    await uploadCobrandIfSelected();
                    updateConfig();
                    saveConfig((msg) => {
                        if (!msg?.ok) {
                            const message = msg?.message || t("config.general.save_failed");
                            setCobrandUploadStatus("danger", escapeHtml(message));
                            return;
                        }
                        setCobrandUploadStatus(
                            "success",
                            t("config.general.cobrand.saved"),
                        );
                    }, (msg) => {
                        const message = msg?.message || t("config.general.save_failed");
                        setCobrandUploadStatus("danger", escapeHtml(message));
                    });
                } catch (error) {
                    const message = error instanceof Error ? error.message : t("config.general.cobrand.upload_failed");
                    setCobrandUploadStatus("danger", escapeHtml(message));
                }
            }
        });
    } else {
        // translatable: false — "Configuration not found in window.config" is a console diagnostic for a missing payload, never rendered
        console.error("Configuration not found in window.config");
    }
});
