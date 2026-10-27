import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {toNumber} from "../lq_js_common/helpers/scaling";
import {QueuingActivityWaveform} from "../graphs/queuing_activity_uplot";

// `.64.2.2`: the global `RttHistogram` ticker is a bus `Vec<u32>` of 50 buckets.
// In bridge mode `throughput_tracker::rtt_histogram` buckets each host's CURRENT
// p50 RTT at 10 ms per column (`column = median_ms / 10`), so bucket `i` counts
// hosts whose current median RTT fell in `[i*10, i*10+10)` ms, clamped at bucket
// 49 (= 500 ms). Returns the HOST-median RTT (nearest bucket midpoint) or `null`.
function rttMedianFromHistogram(data) {
    if (!Array.isArray(data) || data.length === 0) {
        return null;
    }
    let total = 0;
    for (const count of data) {
        total += Number(count) || 0;
    }
    if (total <= 0) {
        return null;
    }
    const target = total * 0.5;
    let cumulative = 0;
    for (let i = 0; i < data.length; i += 1) {
        cumulative += Number(data[i]) || 0;
        if (cumulative >= target) {
            return i * 10 + 5;
        }
    }
    return (data.length - 1) * 10 + 5;
}

function downUpEqual(left, right) {
    return toNumber(left?.down, null) === toNumber(right?.down, null)
        && toNumber(left?.up, null) === toNumber(right?.up, null);
}

/**
 * `.64.2.2` Part 1: a GLOBAL Queue Dynamics widget on the Executive Summary,
 * reusing the Circuit page's `QueuingActivityWaveform` through the SEAM (no
 * fork). Fed from the global 1 s tickers: `Throughput` (delivered + demand +
 * configured capacity) for the upper panel and `RttHistogram` for the lower.
 * No Insight license needed (plain `BaseDashlet` + vendored uPlot).
 */
export class QueueDynamicsDashlet extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 12;
        this.latestThroughput = null;
        this.latestRttHistogram = null;
        this.graph = null;
    }

    title() { return "Queue Dynamics"; }
    tooltip() {
        return "<h5>Queue Dynamics</h5><p>Live global queue dynamics on a 30-second window. Top panel: the delivered (shaped) throughput with the configured capacity line, plus the offered/demand rate where the box reports one — on a bridge box the demand is zero until the backend ships it, otherwise the demand line is omitted. Bottom panel: the global host-median round-trip time from the bridge RTT histogram.</p>";
    }
    subscribeTo() { return ["Throughput", "RttHistogram"]; }
    canBeSlowedDown() { return true; }

    buildContainer() {
        const container = super.buildContainer();
        const graph = document.createElement("div");
        graph.id = `${this.id}_queuedyn`;
        graph.style.minHeight = "240px";
        container.appendChild(graph);
        this._graphId = graph.id;
        return container;
    }

    setup() {
        super.setup();
        this.graph = new QueuingActivityWaveform(this._graphId);
        this.applyRttThresholds();
    }

    // `.64.2.2` R2: match the Circuit page's RTT bands. The canonical NON-ADMIN
    // source is `window.rttThresholds`, baked into the template for EVERY user
    // (`template.rs:172`) — the same precedence as `scaling.js` / `color_scales.js`.
    // (`window.config.rtt_thresholds` comes from `GetConfig`, which is
    // admin-gated, so it is only a fallback and is not required.)
    applyRttThresholds() {
        if (!this.graph || this._rttThresholdsDone) {
            return;
        }
        const thresholds = window.rttThresholds
            || window.rtt_thresholds
            || window.config?.rtt_thresholds;
        if (thresholds) {
            this.graph.setRttThresholds(thresholds);
            this._rttThresholdsDone = true;
            return;
        }
        // No source at all: apply the component defaults once.
        if (!this._rttThresholdsDefaulted) {
            this.graph.setRttThresholds(undefined);
            this._rttThresholdsDefaulted = true;
        }
    }

    onMessage(msg) {
        if (msg.event === "Throughput") {
            this.latestThroughput = msg.data;
            this.maybePushSample();
            return;
        }
        if (msg.event === "RttHistogram") {
            this.latestRttHistogram = msg.data;
            this.maybePushSample();
        }
    }

    maybePushSample() {
        // Suppress the all-zero first sample (before the Throughput ticker has data).
        if (!this.graph || !this.latestThroughput) {
            return;
        }
        this.applyRttThresholds();
        // `.64.2.2` (4): `Throughput` + `RttHistogram` BOTH run on the one 1s
        // cadence; `quantizeTimestamp` forces `last + 1s` on a same-slot push, so
        // pushing per message advances the 30s axis 2s/s. Coalesce to ONE push/s.
        const now = Date.now();
        if (this._lastPushMs !== undefined && now - this._lastPushMs < 1000) {
            return;
        }
        this._lastPushMs = now;
        this.pushSample();
    }

    pushSample() {
        const throughput = this.latestThroughput;
        const max = throughput?.max;
        const raw = throughput?.bps;
        const shaped = throughput?.shaped_bps;
        // REQUIRED-1(b): the real demand is `actual_bps` once the backend rider
        // ships. Until then — LEGACY mode: `bps` is the demand; BRIDGE mode:
        // `bps == shaped_bps` (both are the delivered/shaped rate), so there is NO
        // demand — zero the enqueued line rather than duplicate the delivered one.
        const hasDemand = throughput?.actual_bps !== undefined && throughput?.actual_bps !== null;
        const enqueued = hasDemand
            ? throughput.actual_bps
            : (downUpEqual(raw, shaped) ? { down: 0, up: 0 } : raw);
        this.graph.pushSample({
            timestamp: Date.now(),
            // Delivered (solid) = the shaped/mapped rate.
            throughputBps: {
                down: toNumber(shaped?.down, 0),
                up: toNumber(shaped?.up, 0),
            },
            // Offered/demand (the enqueued line); zeroed when the box reports none.
            actualThroughputBps: {
                down: toNumber(enqueued?.down, 0),
                up: toNumber(enqueued?.up, 0),
            },
            // REQUIRED-3: `throughput.rs` builds max=DownUpOrder::new(uplink,
            // downlink) so `max.down` is the UPLINK -> swap (Mbps -> bps).
            ceilingBps: {
                down: toNumber(max?.up, 0) * 1_000_000,
                up: toNumber(max?.down, 0) * 1_000_000,
            },
            // REQUIRED-5: the histogram has no LAN/WAN split => ONE series: the
            // global host-median RTT mapped to `internet`, `subscriber` null.
            // NOTE: the component's legend labels that series "Internet RTT (P50)",
            // but here it carries the global HOST-median RTT (the tooltip says so).
            rttP50Ms: { subscriber: null, internet: rttMedianFromHistogram(this.latestRttHistogram) },
        });
    }
}
