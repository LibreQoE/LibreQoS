import {DashletBaseInsight} from "./insight_dashlet_base";
import {LiveTrafficPulseCanvas, pulsePalette} from "../graphs/live_traffic_pulse_canvas";
import {get_ws_client} from "../pubsub/ws";
import {scaleNumber} from "../lq_js_common/helpers/scaling";

// `.66.7`: the real Live Traffic Pulse, replacing the Last-30-Seconds Traffic
// widget (`throughputRing30`). Driven by the EXISTING 1 Hz `Throughput` event;
// a canvas + rAF interpolates the motion client-side (no backend feed).
const WINDOW_MS = 30_000; // the pulse window (a TIME-indexed ring, last 30 s)
const BACKGROUND_MAX = 30;
const DELTA_24H_SECONDS = 86400;
const DELTA_TIMEOUT_MS = 6000;

export class LiveTrafficPulseDash extends DashletBaseInsight {
    constructor(slot) {
        super(slot);
        // `.64.3.1` A4: the Executive traffic widget is full-width with a pinned
        // 180 px body.
        this.size = 12;
        this._samples = [];
        this._background = [];
        this._deltaBaselineBps = null;
        this._deltaUnavailable = true; // until a successful 24h read
        this._deltaRequested = false;
        this._deltaTimer = null;
    }

    canBeSlowedDown() { return true; }

    title() { return "Live Traffic Pulse"; }

    tooltip() {
        return "<h5>Live Traffic Pulse</h5><p>A mirrored 30-second Mapped/Unmapped demand "
            + "stream (download up, upload down). Mapped = a shaped circuit was resolved; "
            + "Unmapped = the best-effort class. The download and upload halves are "
            + "scaled independently around zero, so each direction uses the full half-height. "
            + "Peak / 30s avg come from the live window; Δ vs 24h uses licensed Insight history when available.</p>";
    }

    subscribeTo() { return ["Throughput"]; }
    keepAliveWhenHidden() { return true; }
    keepSubscribedWhenHidden() { return true; }
    supportsZoom() { return false; }

    buildContainer() {
        const container = super.buildContainer();
        const body = document.createElement("div");
        body.className = "exec-widget-body pulse-body";
        body.innerHTML = `
            <div class="pulse-numbers">
                <div class="pulse-metric pulse-metric-down">
                    <span class="pulse-arrow" data-pulse="down-arrow">↓</span>
                    <div class="pulse-metric-copy">
                        <div class="pulse-metric-value" data-pulse="down">—</div>
                        <div class="pulse-metric-label">Downstream <span class="pulse-metric-sub">(current)</span></div>
                    </div>
                </div>
                <div class="pulse-metric pulse-metric-up">
                    <span class="pulse-arrow" data-pulse="up-arrow">↑</span>
                    <div class="pulse-metric-copy">
                        <div class="pulse-metric-value" data-pulse="up">—</div>
                        <div class="pulse-metric-label">Upstream <span class="pulse-metric-sub">(current)</span></div>
                    </div>
                </div>
                <div class="pulse-metric pulse-metric-unmapped" data-pulse="unmapped-wrap">
                    <span class="pulse-arrow" data-pulse="unmapped-arrow">∿</span>
                    <div class="pulse-metric-copy">
                        <div class="pulse-metric-value" data-pulse="unmapped">—</div>
                        <div class="pulse-metric-label">Unmapped <span class="pulse-metric-sub" data-pulse="unmapped-pct"></span></div>
                        <div class="pulse-note" data-pulse="no-class" style="display:none">Total · class split unavailable</div>
                    </div>
                </div>
                <div class="pulse-baseline" data-pulse="baseline"></div>
            </div>
            <div class="pulse-chart"><canvas class="pulse-canvas" data-pulse="chart"></canvas></div>`;
        container.appendChild(body);
        this._chartEl = body.querySelector('[data-pulse="chart"]');
        this._el = {};
        body.querySelectorAll("[data-pulse]").forEach((el) => {
            this._el[el.dataset.pulse] = el;
        });

        // Header: live status next to the title, mapped/unmapped legend right.
        const handle = container.querySelector(".dashbox-title");
        if (handle) {
            const status = document.createElement("span");
            status.className = "pulse-status";
            status.innerHTML = '<span class="pulse-status-dot"></span> Last 30 seconds';
            handle.insertBefore(status, handle.children.length ? handle.children[0] : null);
            const legend = document.createElement("span");
            legend.className = "dashbox-title-controls pulse-legend";
            legend.innerHTML = `
                <span class="pulse-legend-item"><span class="pulse-legend-swatch mapped"></span>Mapped ↓</span>
                <span class="pulse-legend-item"><span class="pulse-legend-swatch mapped-up"></span>Mapped ↑</span>
                <span class="pulse-legend-item"><span class="pulse-legend-swatch unmapped"></span>Unmapped</span>`;
            handle.appendChild(legend);
            this._el["legend-mapped"] = legend.querySelector(".mapped");
            this._el["legend-mapped-up"] = legend.querySelector(".mapped-up");
            this._el["legend-unmapped"] = legend.querySelector(".unmapped");
        }
        return container;
    }

    setup() {
        super.setup();
        // `.66.7`: a plain canvas + rAF (the ECharts pulse graph is retired). It
        // reads the palette each frame, so a CVD toggle only needs the DOM row
        // recoloured below.
        this.pulse = new LiveTrafficPulseCanvas(this._chartEl);
        this._colorBlindHandler = () => this.#applyPalette();
        window.addEventListener("colorBlindModeChanged", this._colorBlindHandler);
        this.#applyPalette();
        this.#request24h();
        this.#updateBaseline();
    }

    dispose() {
        if (this._colorBlindHandler) {
            window.removeEventListener("colorBlindModeChanged", this._colorBlindHandler);
            this._colorBlindHandler = null;
        }
        if (this._deltaTimer) {
            clearTimeout(this._deltaTimer);
            this._deltaTimer = null;
        }
        if (this.pulse) {
            this.pulse.dispose();
            this.pulse = null;
        }
        super.dispose();
    }

    onMessage(msg) {
        if (msg.event !== "Throughput") {
            return;
        }
        this.#ingest(msg.data);
    }

    onBackgroundMessage(msg) {
        if (msg.event !== "Throughput") {
            return;
        }
        this._background.push(msg.data);
        if (this._background.length > BACKGROUND_MAX) {
            this._background.shift();
        }
    }

    flushBackgroundMessages() {
        if (this._background.length === 0) {
            return false;
        }
        // `.66.7`: replay the hidden-window samples on a ~1 Hz timeline so the
        // time-indexed ring reconstructs the gap (not all stacked at `now`).
        const n = this._background.length;
        const now = Date.now();
        this._background.forEach((data, i) => {
            this.#ingest(data, now - (n - 1 - i) * 1000);
        });
        this._background = [];
        return true;
    }

    #ingest(data, t = Date.now()) {
        if (!data) {
            return;
        }
        // `.66.2` (i): the class split is additive. When absent (older backend /
        // legacy) the WHOLE silhouette has no class source, so fall back to the
        // pre-existing TOTAL (`bps`) as the silhouette with NO coral and an
        // explicit "class split unavailable" label — NEVER reuse bps/shaped_bps
        // AS the class split.
        const classPresent = data.mapped_bps != null && data.unmapped_bps != null;
        const total = data.bps != null ? data.bps : {down: 0, up: 0};
        const mapped = classPresent ? data.mapped_bps : total;
        const unmapped = classPresent ? data.unmapped_bps : {down: 0, up: 0};
        const sample = {
            t,
            classPresent,
            mappedDown: Number(mapped.down) || 0,
            mappedUp: Number(mapped.up) || 0,
            unmappedDown: Number(unmapped.down) || 0,
            unmappedUp: Number(unmapped.up) || 0,
            // `.117`: prefer the backend's class-derived share; fall back to
            // the same arithmetic only for an older backend without it.
            unmappedPct: Number.isFinite(Number(data.unmapped_pct))
                ? Number(data.unmapped_pct)
                : null,
        };
        this._samples.push(sample);
        // `.66.7`: a TIME-indexed ring — retain the last 30 s (not a fixed count).
        const cutoff = t - WINDOW_MS;
        while (this._samples.length > 0 && this._samples[0].t < cutoff) {
            this._samples.shift();
        }
        if (this.pulse) {
            this.pulse.push(sample);
        }
        this.#updateNumbers();
        this.#updateBaseline();
    }

    #applyPalette() {
        if (!this._el) {
            return;
        }
        const p = pulsePalette();
        if (this._el["down-arrow"]) this._el["down-arrow"].style.color = p.mapped;
        if (this._el["up-arrow"]) this._el["up-arrow"].style.color = p.mappedUp;
        if (this._el["unmapped-arrow"]) this._el["unmapped-arrow"].style.color = p.unmapped;
        this._el.down.style.color = p.mapped;
        this._el.up.style.color = p.mappedUp;
        this._el.unmapped.style.color = p.unmapped;
        if (this._el["legend-mapped"]) this._el["legend-mapped"].style.background = p.mapped;
        if (this._el["legend-mapped-up"]) this._el["legend-mapped-up"].style.background = p.mappedUp;
        if (this._el["legend-unmapped"]) this._el["legend-unmapped"].style.background = p.unmapped;
    }

    #updateNumbers() {
        const last = this._samples[this._samples.length - 1];
        if (!last || !this._el) {
            return;
        }
        const totalDown = last.mappedDown + last.unmappedDown;
        const totalUp = last.mappedUp + last.unmappedUp;
        this._el.down.textContent = this.#fmt(totalDown);
        this._el.up.textContent = this.#fmt(totalUp);
        if (last.classPresent) {
            this._el["unmapped-wrap"].style.display = "";
            this._el["no-class"].style.display = "none";
            const unmappedTotal = last.unmappedDown + last.unmappedUp;
            this._el.unmapped.textContent = this.#fmt(unmappedTotal);
            const total = totalDown + totalUp;
            const pct = Number.isFinite(last.unmappedPct)
                ? last.unmappedPct
                : (total > 0 ? (unmappedTotal / total) * 100 : 0);
            this._el["unmapped-pct"].textContent = total > 0
                ? `(${pct.toFixed(1)}% of total)`
                : "";
            // Fade the unmapped metric block with a shallow floor:
            // 0% -> 0.15, 2.5% -> 0.25, 5% -> 0.50, 10%+ -> 1.0.
            // The chart band keeps its normal alpha; its height already
            // encodes the unmapped volume.
            let opacity;
            if (pct <= 2.5) {
                opacity = 0.15 + (pct / 2.5) * 0.10;
            } else if (pct <= 5) {
                opacity = 0.25 + ((pct - 2.5) / 2.5) * 0.25;
            } else if (pct <= 10) {
                opacity = 0.5 + ((pct - 5) / 5) * 0.5;
            } else {
                opacity = 1;
            }
            this._el["unmapped-wrap"].style.opacity = Math.min(1, Math.max(0, opacity)).toFixed(2);
        } else {
            // REQUIRED (i): no class counters ⇒ the whole silhouette is the
            // pre-existing total; hide the coral/Unmapped number and label it.
            this._el["unmapped-wrap"].style.display = "none";
            this._el["no-class"].style.display = "";
        }
    }

    #updateBaseline() {
        if (!this._el || !this._el.baseline) {
            return;
        }
        if (this._samples.length === 0) {
            this._el.baseline.textContent = "";
            return;
        }
        let peak = 0;
        let sum = 0;
        for (const s of this._samples) {
            peak = Math.max(peak, s.mappedDown + s.unmappedDown, s.mappedUp + s.unmappedUp);
            sum += s.mappedDown + s.unmappedDown + s.mappedUp + s.unmappedUp;
        }
        const avg = sum / this._samples.length;
        const delta = this.#deltaHtml(avg);
        this._el.baseline.innerHTML = `
            <div class="pulse-baseline-row"><span>Peak</span><b>${this.#fmt(peak)}</b></div>
            <div class="pulse-baseline-row"><span>30s avg</span><b>${this.#fmt(avg)}</b></div>
            ${delta ? `<div class="pulse-baseline-delta">${delta}</div>` : ""}`;
    }

    #deltaHtml(nowAvg) {
        if (this._deltaUnavailable || !(this._deltaBaselineBps > 0)) {
            return "";
        }
        const pct = ((nowAvg - this._deltaBaselineBps) / this._deltaBaselineBps) * 100;
        const up = pct >= 0;
        return `Δ vs 24h <b class="pulse-delta ${up ? "up" : "down"}">${up ? "▲" : "▼"} ${Math.abs(pct).toFixed(1)}%</b>`;
    }

    // `.66.2` (iv): the 24h baseline reuses the EXISTING `LtsThroughput` path.
    // That path is INSIGHT-LICENSE-GATED (`insight_gate()` → FORBIDDEN when
    // unentitled), so an `Error` response (or a timeout) HIDES the Δ-24h chip.
    #request24h() {
        if (this._deltaRequested) {
            return;
        }
        this._deltaRequested = true;
        const wsClient = get_ws_client();
        const seconds = DELTA_24H_SECONDS;
        let done = false;
        const cleanup = () => {
            wsClient.off("LtsThroughput", onData);
            wsClient.off("Error", onError);
            if (this._deltaTimer) {
                clearTimeout(this._deltaTimer);
                this._deltaTimer = null;
            }
        };
        const onData = (msg) => {
            if (done || !msg || msg.seconds !== seconds) {
                return;
            }
            done = true;
            cleanup();
            const rows = Array.isArray(msg.data) ? msg.data : [];
            const first = rows[0];
            const baseline = first
                ? ((Number(first.median_down) || 0) + (Number(first.median_up) || 0)) * 8
                : 0;
            if (baseline > 0) {
                this._deltaBaselineBps = baseline;
                this._deltaUnavailable = false;
            }
            this.#updateBaseline();
        };
        const onError = () => {
            if (done) {
                return;
            }
            done = true;
            cleanup();
            this._deltaBaselineBps = null;
            this._deltaUnavailable = true;
            this.#updateBaseline();
        };
        wsClient.on("LtsThroughput", onData);
        wsClient.on("Error", onError);
        wsClient.send({LtsThroughput: {seconds}});
        this._deltaTimer = setTimeout(() => {
            if (done) {
                return;
            }
            done = true;
            cleanup();
            this.#updateBaseline();
        }, DELTA_TIMEOUT_MS);
    }

    #fmt(bitsPerSecond) {
        const n = Number(bitsPerSecond) || 0;
        if (n < 1000) {
            return `${Math.round(n)} bps`;
        }
        return scaleNumber(n, 1).replace(/([TGMK])$/, " $1bps");
    }
}
