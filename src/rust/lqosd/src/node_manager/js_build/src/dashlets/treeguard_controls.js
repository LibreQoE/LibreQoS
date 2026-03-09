import {loadConfig, saveConfig} from "../config/config_helper";
import {defaultTreeguardConfig, ensureTreeguardConfig} from "../config/treeguard_defaults";
import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";

export class TreeguardControlsDashlet extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 6;
        this.treeguard = defaultTreeguardConfig();
        this.isSaving = false;
    }

    title() {
        return "TreeGuard Controls";
    }

    tooltip() {
        return "<h5>TreeGuard Controls</h5><p>Quick access to the default TreeGuard controls plus the current config-derived rollout status.</p>";
    }

    subscribeTo() {
        return [];
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
        this.refreshConfig();
    }

    bindControls() {
        const enabled = document.getElementById(this.enabledId());
        const dryRun = document.getElementById(this.dryRunId());
        enabled.addEventListener("change", () => this.persistToggle("enabled", enabled.checked));
        dryRun.addEventListener("change", () => this.persistToggle("dry_run", dryRun.checked));
    }

    refreshConfig() {
        this.setSaveState("Loading config...", false);
        loadConfig(
            () => {
                this.treeguard = ensureTreeguardConfig(window.config);
                this.syncControls();
                this.renderStatusLine();
                this.setSaveState("", false);
            },
            () => {
                this.treeguard = ensureTreeguardConfig(window.config || {});
                this.syncControls();
                this.renderStatusLine();
                this.setSaveState("Unable to refresh TreeGuard config.", true);
            },
        );
    }

    syncControls() {
        const enabled = document.getElementById(this.enabledId());
        const dryRun = document.getElementById(this.dryRunId());
        enabled.checked = !!this.treeguard.enabled;
        dryRun.checked = !!this.treeguard.dry_run;
        enabled.disabled = this.isSaving;
        dryRun.disabled = this.isSaving;
    }

    renderStatusLine() {
        const statusLine = document.getElementById(this.statusLineId());
        const liveState = this.treeguard.dry_run ? "dry run" : "live";
        const mode = this.treeguard.cpu?.mode === "cpu_aware" ? "CPU-aware" : "Traffic/RTT only";
        const linkScope = this.treeguard.links?.all_nodes ? "all links" : "allowlisted links";
        const circuitScope = this.treeguard.circuits?.all_circuits ? "all circuits" : "allowlisted circuits";
        const enabled = this.treeguard.enabled ? "Enabled" : "Disabled";
        statusLine.innerHTML = `
            Status: <strong>${enabled}</strong>, ${liveState}, ${mode}, ${linkScope}, ${circuitScope}.
            <a href="config_treeguard.html" class="ms-1">Open configuration</a>
        `;
    }

    persistToggle(field, nextValue) {
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
