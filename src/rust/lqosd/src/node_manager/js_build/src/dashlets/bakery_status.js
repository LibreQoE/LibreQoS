import {DashletBaseInsight} from "./insight_dashlet_base";

export class BakeryStatusDashlet extends DashletBaseInsight {
    constructor(slot) {
        super(slot);
        this.lastUpdate = {};
    }

    title() {
        return "Bakery Status";
    }

    tooltip() {
        return "<h5>Bakery Status</h5><p>Real-time queue management statistics. Shows queues created/expired and circuit activity this cycle.</p>";
    }

    subscribeTo() {
        return ["BakeryStatus"];
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
                    <div class="stat-header">Current State</div>
                    <div id="${this.id}_sites" class="small">Sites: 0</div>
                    <div id="${this.id}_circuits" class="small">Total Circuits: 0</div>
                    <div id="${this.id}_active" class="small text-success">Active Circuits: 0</div>
                    <div id="${this.id}_lazy" class="small text-info">Lazy Circuits: 0</div>
                    
                    <div class="stat-header mt-2">This Cycle</div>
                    <div id="${this.id}_created" class="small text-success">Queues Created: 0</div>
                    <div id="${this.id}_expired" class="small text-warning">Queues Expired: 0</div>
                    <div id="${this.id}_activated" class="small text-info">Lazy Activated: 0</div>
                </div>
                <div class="col-6">
                    <div class="stat-header">Performance</div>
                    <div id="${this.id}_batch_time" class="small">Batch Time: 0ms</div>
                    <div id="${this.id}_pending" class="small">Pending Commands: 0</div>
                    
                    <div class="stat-header mt-2">TC Commands</div>
                    <div id="${this.id}_tc_commands" class="stat-value-big">0</div>
                    <div class="stat-header">Executed This Cycle</div>
                </div>
            </div>
        `;
        
        target.innerHTML = html;
    }

    onMessage(msg) {
        if (msg.event === "BakeryStatus") {
            this.lastUpdate = msg.data;
            this.updateDisplay();
        }
    }

    updateDisplay() {
        const data = this.lastUpdate;
        if (!data) return;
        
        // Update current state
        this.updateElement("_sites", `Sites: ${data.currentState.totalSites}`);
        this.updateElement("_circuits", `Total Circuits: ${data.currentState.totalCircuits}`);
        this.updateElement("_active", `Active Circuits: ${data.currentState.activeCircuits}`);
        this.updateElement("_lazy", `Lazy Circuits: ${data.currentState.lazyCircuits}`);
        
        // Update per-cycle stats
        this.updateElement("_created", `Queues Created: ${data.perCycle.queuesCreated}`);
        this.updateElement("_expired", `Queues Expired: ${data.perCycle.queuesExpired}`);
        this.updateElement("_activated", `Lazy Activated: ${data.perCycle.lazyQueuesActivated}`);
        
        // Update performance
        this.updateElement("_batch_time", `Batch Time: ${data.performance.lastBatchDurationMs}ms`);
        this.updateElement("_pending", `Pending Commands: ${data.performance.pendingCommands}`);
        this.updateElement("_tc_commands", data.perCycle.tcCommandsExecuted);
    }
    
    updateElement(suffix, value) {
        const el = document.getElementById(this.id + suffix);
        if (el) el.textContent = value;
    }

    canBeSlowedDown() {
        return false; // Always update in real-time
    }
}