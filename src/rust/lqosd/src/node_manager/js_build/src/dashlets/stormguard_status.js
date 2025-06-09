import {DashletBaseInsight} from "./insight_dashlet_base";

export class StormguardStatusDashlet extends DashletBaseInsight {
    constructor(slot) {
        super(slot);
        this.lastUpdate = {};
    }

    title() {
        return "Stormguard Status";
    }

    tooltip() {
        return "<h5>Stormguard Status</h5><p>Real-time bandwidth optimization statistics. Shows sites being actively managed and adjustments made this cycle.</p>";
    }

    subscribeTo() {
        return ["StormguardStatus"];
    }

    buildContainer() {
        let base = super.buildContainer();
        base.style.height = "320px";
        return base;
    }

    setup() {
        super.setup();
        const target = document.getElementById(this.id);
        
        // Create layout structure
        const html = `
            <div class="row">
                <div class="col-6">
                    <div class="stat-header">Sites Managed</div>
                    <div id="${this.id}_total" class="stat-value-big">0</div>
                    
                    <div class="stat-header mt-2">This Cycle</div>
                    <div id="${this.id}_evaluated" class="small">Evaluated: 0</div>
                    <div id="${this.id}_adjustments_up" class="small text-success">↑ Increased: 0</div>
                    <div id="${this.id}_adjustments_down" class="small text-danger">↓ Decreased: 0</div>
                </div>
                <div class="col-6">
                    <div class="stat-header">Site States</div>
                    <div id="${this.id}_warmup" class="small text-warning">Warmup: 0</div>
                    <div id="${this.id}_active" class="small text-success">Active: 0</div>
                    <div id="${this.id}_cooldown" class="small text-info">Cooldown: 0</div>
                    
                    <div class="stat-header mt-2">Performance</div>
                    <div id="${this.id}_cycle_time" class="small">Cycle: 0ms</div>
                    <div id="${this.id}_recommendations" class="small">Recommendations: 0</div>
                </div>
            </div>
        `;
        
        target.innerHTML = html;
    }

    onMessage(msg) {
        if (msg.event === "StormguardStatus") {
            this.lastUpdate = msg.data;
            this.updateDisplay();
        }
    }

    updateDisplay() {
        const data = this.lastUpdate;
        if (!data) return;
        
        // Update all fields
        this.updateElement("_total", data.currentState.totalSitesManaged);
        this.updateElement("_evaluated", `Evaluated: ${data.perCycle.sitesEvaluated}`);
        this.updateElement("_adjustments_up", `↑ Increased: ${data.perCycle.adjustmentsUp}`);
        this.updateElement("_adjustments_down", `↓ Decreased: ${data.perCycle.adjustmentsDown}`);
        
        this.updateElement("_warmup", `Warmup: ${data.currentState.sitesInWarmup}`);
        this.updateElement("_active", `Active: ${data.currentState.sitesActive}`);
        this.updateElement("_cooldown", `Cooldown: ${data.currentState.sitesInCooldown}`);
        
        this.updateElement("_cycle_time", `Cycle: ${data.performance.lastCycleDurationMs}ms`);
        this.updateElement("_recommendations", `Recommendations: ${data.performance.recommendationsGenerated}`);
    }
    
    updateElement(suffix, value) {
        const el = document.getElementById(this.id + suffix);
        if (el) el.textContent = value;
    }

    canBeSlowedDown() {
        return false; // Always update in real-time
    }
}