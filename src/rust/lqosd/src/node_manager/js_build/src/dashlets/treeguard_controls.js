import {loadConfig, saveConfig} from "../config/config_helper";
import {defaultTreeguardConfig, ensureTreeguardConfig} from "../config/treeguard_defaults";
import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";

export class TreeguardControlsDashlet extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 6;
        this.treeguard = defaultTreeguardConfig();
        this.isSaving = false;
        this.liveStatus = null;
        this.configLoaded = false;
    }

    title() {
        return "TreeGuard Controls";
    }

    tooltip() {
        return "<h5>TreeGuard Controls</h5><p>Quick access to the default TreeGuard controls plus the current config-derived rollout status.</p>";
    }

    subscribeTo() {
        return ["TreeGuardStatus"];
    }

    statusLineId() {
        return `${this.id}_status`;
    }

    controlsId() {
        return `${this.id}_controls`;
    }

    saveStateId() {
        return `${this.id}_save_state`;
    }

    enabledId() {
        return `${this.id}_enabled`;
    }

    dryRunId() {
        return `${this.id}_dry_run`;
    }

    buildContainer() {
        const base = super.buildContainer();

        const statusLine = document.createElement("div");
        statusLine.id = this.statusLineId();
        statusLine.className = "text-muted small mb-3";
        statusLine.innerText = "Loading TreeGuard status...";
        base.appendChild(statusLine);

        const controls = document.createElement("div");
        controls.id = this.controlsId();
        controls.className = "d-flex flex-column flex-md-row gap-3 align-items-start align-items-md-center";
        controls.innerHTML = `
            <div class="form-check form-switch mb-0">
                <input class="form-check-input" type="checkbox" id="${this.enabledId()}">
                <label class="form-check-label" for="${this.enabledId()}">Enable TreeGuard</label>
            </div>
            <div class="form-check form-switch mb-0">
                <input class="form-check-input" type="checkbox" id="${this.dryRunId()}">
                <label class="form-check-label" for="${this.dryRunId()}">Dry Run Mode</label>
            </div>
        `;
        base.appendChild(controls);

        const saveState = document.createElement("div");
        saveState.id = this.saveStateId();
        saveState.className = "small mt-2 text-muted";
        base.appendChild(saveState);

        return base;
    }

    setup() {
        this.bindControls();
        this.syncControls();
        this.refreshConfig(false);
    }

    bindControls() {
        const enabled = document.getElementById(this.enabledId());
        const dryRun = document.getElementById(this.dryRunId());
        enabled.addEventListener("change", () => this.persistToggle("enabled", enabled.checked));
        dryRun.addEventListener("change", () => this.persistToggle("dry_run", dryRun.checked));
    }

    refreshConfig(showLoading = true, onLoaded = null, onFailed = null) {
        if (showLoading) {
            this.setSaveState("Loading config...", false);
        }
        loadConfig(
            () => {
                this.treeguard = ensureTreeguardConfig(window.config);
                this.configLoaded = true;
                this.syncControls();
                this.renderStatusLine();
                if (showLoading) {
                    this.setSaveState("", false);
                }
                if (onLoaded) {
                    onLoaded();
                }
            },
            () => {
                this.treeguard = ensureTreeguardConfig(window.config || {});
                this.syncControls();
                this.renderStatusLine();
                if (showLoading) {
                    this.setSaveState("Unable to refresh TreeGuard config.", true);
                }
                if (onFailed) {
                    onFailed();
                }
            },
        );
    }

    syncControls() {
        const enabled = document.getElementById(this.enabledId());
        const dryRun = document.getElementById(this.dryRunId());
        const controls = document.getElementById(this.controlsId());
        const statusEnabled = this.liveStatus?.enabled;
        const statusDryRun = this.liveStatus?.dry_run;
        const paused = !!this.liveStatus?.paused_for_bakery_reload;
        enabled.checked = this.configLoaded ? !!this.treeguard.enabled : !!(statusEnabled ?? this.treeguard.enabled);
        dryRun.checked = this.configLoaded ? !!this.treeguard.dry_run : !!(statusDryRun ?? this.treeguard.dry_run);
        enabled.disabled = this.isSaving || paused;
        dryRun.disabled = this.isSaving || paused;
        if (controls) {
            controls.classList.toggle("opacity-50", paused);
        }
    }

    renderStatusLine() {
        const statusLine = document.getElementById(this.statusLineId());
        const liveStatus = this.liveStatus || {};
        const enabled = (liveStatus.enabled ?? this.treeguard.enabled) ? "Enabled" : "Disabled";
        const dryRun = !!(liveStatus.dry_run ?? this.treeguard.dry_run);
        const paused = !!liveStatus.paused_for_bakery_reload;
        const pauseReason = (liveStatus.pause_reason || "Bakery full reload in progress").trim();
        const liveState = dryRun ? "dry run" : "live";
        const mode = this.treeguard.cpu?.mode === "cpu_aware" ? "CPU-aware" : "Traffic/RTT only";
        const linkScope = this.treeguard.links?.all_nodes ? "all links" : "allowlisted links";
        const circuitScope = this.treeguard.circuits?.all_circuits ? "all circuits" : "allowlisted circuits";
        const managedNodes = Number.isFinite(Number(liveStatus.managed_nodes))
            ? Math.max(0, Math.trunc(Number(liveStatus.managed_nodes)))
            : null;
        const managedCircuits = Number.isFinite(Number(liveStatus.managed_circuits))
            ? Math.max(0, Math.trunc(Number(liveStatus.managed_circuits)))
            : null;
        const warningCount = Array.isArray(liveStatus.warnings) ? liveStatus.warnings.length : 0;
        const summaryBits = [
            `Status: <strong>${enabled}</strong>`,
            liveState,
            mode,
            linkScope,
            circuitScope,
        ];
        if (paused) {
            summaryBits.push(`<span class="text-warning">paused: ${pauseReason}</span>`);
        }
        if (managedNodes !== null || managedCircuits !== null) {
            const counts = [];
            if (managedNodes !== null) counts.push(`${managedNodes} node${managedNodes === 1 ? "" : "s"}`);
            if (managedCircuits !== null) counts.push(`${managedCircuits} circuit${managedCircuits === 1 ? "" : "s"}`);
            if (counts.length > 0) {
                summaryBits.push(`managing ${counts.join(", ")}`);
            }
        }
        if (warningCount > 0) {
            summaryBits.push(`<span class="text-warning">${warningCount} warning${warningCount === 1 ? "" : "s"}</span>`);
        }
        statusLine.innerHTML = `${summaryBits.join(", ")}. <a href="config_treeguard.html" class="ms-1">Open configuration</a>`;
    }

    onMessage(msg) {
        if (msg.event !== "TreeGuardStatus") {
            return;
        }
        this.liveStatus = msg.data || null;
        if (!this.configLoaded) {
            this.syncControls();
            if (!this.isSaving) {
                this.setSaveState("", false);
            }
        }
        this.renderStatusLine();
    }

    persistToggle(field, nextValue) {
        if (!this.configLoaded) {
            this.isSaving = true;
            this.syncControls();
            this.refreshConfig(
                true,
                () => {
                    this.isSaving = false;
                    this.persistToggle(field, nextValue);
                },
                () => {
                    this.isSaving = false;
                    this.syncControls();
                },
            );
            return;
        }
        const previousValue = this.treeguard[field];
        this.treeguard[field] = nextValue;
        window.config = window.config || {};
        window.config.treeguard = ensureTreeguardConfig(window.config);
        window.config.treeguard[field] = nextValue;
        this.isSaving = true;
        this.syncControls();
        this.renderStatusLine();
        this.setSaveState("Saving...", false);

        saveConfig(
            () => {
                this.isSaving = false;
                this.syncControls();
                this.renderStatusLine();
                this.setSaveState("Saved.", false);
            },
            () => {
                this.treeguard[field] = previousValue;
                window.config.treeguard[field] = previousValue;
                this.isSaving = false;
                this.syncControls();
                this.renderStatusLine();
                this.setSaveState("Save failed.", true);
            },
        );
    }

    setSaveState(message, isError) {
        const saveState = document.getElementById(this.saveStateId());
        if (!saveState) {
            return;
        }
        saveState.innerText = message;
        saveState.classList.toggle("text-danger", !!message && isError);
        saveState.classList.toggle("text-muted", !message || !isError);
    }
}
