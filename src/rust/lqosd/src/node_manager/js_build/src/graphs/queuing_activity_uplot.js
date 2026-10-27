// §`lqos-74t.47.4` spike: the Queue Dynamics chart rendered with uPlot.
// Drop-in behind the `QueuingActivityWaveform` seam (same public API), so
// `circuit.js` is untouched. ECharts stays the predecessor behind the seam.
// Vendored offline: static2/vendor/uplot.min.{js,css} (MIT), global `uPlot`.
// Layout parity: TWO stacked panels (throughput bps / RTT ms) like the ECharts
// grids, with the RTT threshold bands drawn on the lower panel.
import {scaleNumber, toNumber} from "../lq_js_common/helpers/scaling";

function formatBps(value) {
    return `${scaleNumber(Math.abs(toNumber(value, 0)), 1)}bps`;
}

function formatRttMs(value) {
    const n = toNumber(value, NaN);
    if (!Number.isFinite(n)) {
        return "-";
    }
    return `${n.toFixed(n >= 10 ? 0 : 1)} ms`;
}

const WINDOW_MS = 30_000;
const MAX_SAMPLES = 90;
const RENDER_INTERVAL_MS = 16;
const DISPLAY_LAG_MS = 1_200;
const SAMPLE_INTERVAL_MS = 1_000;

function hexToRgba(hex, alpha) {
    if (typeof hex !== "string" || !hex.startsWith("#")) return hex;
    const n = hex.length === 4 ? `#${hex[1]}${hex[1]}${hex[2]}${hex[2]}${hex[3]}${hex[3]}` : hex;
    const r = parseInt(n.slice(1, 3), 16);
    const g = parseInt(n.slice(3, 5), 16);
    const b = parseInt(n.slice(5, 7), 16);
    return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

function throughputPaletteColor(direction, fallback) {
    const idx = direction === "up" ? 1 : 0;
    return window.graphPalette?.[idx] || fallback;
}

function getWaveformTheme(direction = "down") {
    const isDark = document.documentElement.getAttribute("data-bs-theme") !== "light";
    const throughputColor = throughputPaletteColor(direction, direction === "up" ? "#32d3bd" : "#4992ff");
    if (isDark) {
        return {
            axisText: "rgba(216, 226, 244, 0.62)",
            axisLine: "rgba(216, 226, 244, 0.28)",
            splitLine: "rgba(216, 226, 244, 0.045)",
            enqueuedLine: throughputColor,
            throughputLine: throughputColor,
            throughputAreaTop: hexToRgba(throughputColor, 0.36),
            throughputAreaBottom: "rgba(9, 20, 31, 0.03)",
            rttLine: "#b7a5ff",
            ceilingInactive: "rgba(255, 255, 255, 0.32)",
            rttBandGood: "rgba(42, 168, 108, 0.08)",
            rttBandWarn: "rgba(228, 154, 44, 0.07)",
            rttBandBad: "rgba(225, 84, 84, 0.08)",
        };
    }
    return {
        axisText: "rgba(54, 67, 86, 0.72)",
        axisLine: "rgba(77, 94, 118, 0.28)",
        splitLine: "rgba(77, 94, 118, 0.08)",
        enqueuedLine: throughputColor,
        throughputLine: throughputColor,
        throughputAreaTop: hexToRgba(throughputColor, 0.24),
        throughputAreaBottom: "rgba(255, 255, 255, 0.02)",
        rttLine: "#6f63cf",
        ceilingInactive: "rgba(110, 120, 134, 0.34)",
        rttBandGood: "rgba(46, 163, 111, 0.06)",
        rttBandWarn: "rgba(214, 148, 40, 0.06)",
        rttBandBad: "rgba(196, 82, 82, 0.06)",
    };
}

function normalizeRttThresholds(rawThresholds) {
    const green = Math.max(0, Math.round(toNumber(rawThresholds?.green_ms ?? rawThresholds?.greenMs, 0)));
    const yellow = Math.max(green, Math.round(toNumber(rawThresholds?.yellow_ms ?? rawThresholds?.yellowMs, 100)));
    const red = Math.max(yellow, 1, Math.round(toNumber(rawThresholds?.red_ms ?? rawThresholds?.redMs, 200)));
    return {green_ms: green, yellow_ms: yellow, red_ms: red};
}

function rttAxisMax(thresholds, observedRttMax) {
    const red = Math.max(1, toNumber(thresholds?.red_ms, 200));
    const headroom = Math.max(20, Math.round(red * 0.25));
    const desired = Math.max(red + headroom, observedRttMax > 0 ? Math.ceil(observedRttMax * 1.1) : 0);
    return Math.ceil(desired / 10) * 10;
}

/// A stable "nice" bps axis max (1/1.25/1.5/2/2.5/3/4/5/6/8 x 10^k). Computed
/// once per data sample (1 Hz) — NOT per frame — so the y-scale stays steady
/// under `setData(data, true)`.
function niceAxisMax(max) {
    if (!Number.isFinite(max) || max <= 0) return 1;
    const target = max * 1.05;
    const mag = Math.pow(10, Math.floor(Math.log10(target)));
    const n = target / mag;
    const step = n <= 1 ? 1 : n <= 1.25 ? 1.25 : n <= 1.5 ? 1.5 : n <= 2 ? 2 : n <= 2.5 ? 2.5 : n <= 3 ? 3 : n <= 4 ? 4 : n <= 5 ? 5 : n <= 6 ? 6 : n <= 8 ? 8 : 10;
    return step * mag;
}

function formatAxisNumber(value) {
    if (!Number.isFinite(value)) return "";
    const abs = Math.abs(value);
    if (abs >= 1e9) return (value / 1e9).toFixed(1) + "G";
    if (abs >= 1e6) return (value / 1e6).toFixed(1) + "M";
    if (abs >= 1e3) return (value / 1e3).toFixed(1) + "K";
    return value.toFixed(0);
}

const AXIS_FONT = "11px system-ui, sans-serif";

/// §`lqos-74t.47.4`: the uPlot drop-in for `QueuingActivityWaveform`.
export class QueuingActivityWaveform {
    constructor(id) {
        this.id = id;
        this.dom = document.getElementById(id);
        if (!this.dom) {
            throw new Error(`QueuingActivityWaveform(uPlot): missing DOM element '${id}'`);
        }
        this.dom.classList.remove("muted");
        this.dom.classList.add("lqos-qd-uplot");
        // Drop the server-rendered loading overlay (ECharts replaced it on init).
        this.dom.replaceChildren();
        this.downColors = getWaveformTheme("down");
        this.upColors = getWaveformTheme("up");
        this.samples = [];
        this.sampleClockBase = null;
        this.lastSampleTimestamp = null;
        this.lastPruneTimestamp = 0;
        this.cached = null;
        this.renderNow = Date.now();
        this.displayNow = this.renderNow - DISPLAY_LAG_MS;
        this.renderRaf = null;
        this.lastRenderTimestamp = 0;
        this.renderingEnabled = true;
        this.rttThresholds = normalizeRttThresholds();
        this.observedRttMax = 0;
        this.rttAxisMaxValue = rttAxisMax(this.rttThresholds, 0);
        this.bpsAxisMax = 1;
        this.xRange = [this.displayNow - WINDOW_MS, this.displayNow];
        // §`lqos-74t.47.4`: the predecessor drew every series with
        // `smooth:false, step:"start"` — a held/blocky staircase. uPlot's stepped
        // path (`align:1` = value held until the next x) matches it.
        this.stepped = uPlot.paths.stepped({align: 1});

        const width = Math.max(320, this.dom.clientWidth || 320);
        const height = Math.max(120, this.dom.clientHeight || 336);
        const gap = 6;
        const topH = Math.max(60, Math.round((height - gap) * 0.52));
        const botH = Math.max(60, height - gap - topH);

        this.topDom = document.createElement("div");
        this.botDom = document.createElement("div");
        this.topDom.className = "lqos-qd-uplot-top";
        this.botDom.className = "lqos-qd-uplot-bottom";
        this.dom.style.display = "flex";
        this.dom.style.flexDirection = "column";
        this.dom.style.gap = `${gap}px`;
        this.dom.appendChild(this.topDom);
        this.dom.appendChild(this.botDom);

        const sync = uPlot.sync("lqos-qd-" + id);
        this.uTop = new uPlot(this.topOptions(width, topH, sync), this.buildTopData(), this.topDom);
        this.uBot = new uPlot(this.botOptions(width, botH, sync), this.buildBotData(), this.botDom);
        this.scheduleRenderLoop();
        // §`lqos-74t.47.4`: this class is not a `DashboardGraph` (no ECharts), so
        // wire theme + resize directly instead of `window.graphList` (whose theme
        // handler calls `echarts.init` on every graph's DOM).
        this.themeObserver = new MutationObserver(() => this.onThemeChange());
        this.themeObserver.observe(document.documentElement, {attributes: true, attributeFilter: ["data-bs-theme"]});
        this.resizeRaf = null;
        this.resizeHandler = () => {
            if (this.resizeRaf) window.cancelAnimationFrame(this.resizeRaf);
            this.resizeRaf = window.requestAnimationFrame(() => {
                this.resizeRaf = null;
                this.resize();
            });
        };
        window.addEventListener("resize", this.resizeHandler);
    }

    /// A theme-aware fill gradient: reads the CURRENT colors at draw time so a
    /// theme toggle is reflected without rebuilding the series.
    fillFn(kind) {
        return (u) => {
            const c = kind === "up" ? this.upColors : this.downColors;
            const top = kind === "rtt" ? "rgba(73,146,255,0.18)" : c.throughputAreaTop;
            const bottom = kind === "rtt" ? "rgba(9,20,31,0.03)" : c.throughputAreaBottom;
            const g = u.ctx.createLinearGradient(0, u.bbox.top, 0, u.bbox.top + u.bbox.height);
            g.addColorStop(0, top);
            g.addColorStop(1, bottom);
            return g;
        };
    }

    /// §`lqos-74t.47.4`: the axis-trigger hover tooltip the predecessor had —
    /// a `-Ns` header + one row per series (colored dot, `name: value`), with
    /// bits/s vs ms formatting. Each panel shows its own series.
    makeTooltipPlugin(kind) {
        const self = this;
        let tip = null;
        const buildRows = (idx) => {
            const c = self.cached;
            if (!c) return null;
            if (kind === "bps") {
                return [
                    ["Enqueued (DL)", c.enqDown[idx], self.downColors.enqueuedLine],
                    ["Throughput (DL)", c.tpDown[idx], self.downColors.throughputLine],
                    ["Ceiling Base (DL)", c.ceilDown[idx], self.downColors.ceilingInactive],
                    ["Enqueued (UL)", c.enqUp[idx], self.upColors.enqueuedLine],
                    ["Throughput (UL)", c.tpUp[idx], self.upColors.throughputLine],
                    ["Ceiling Base (UL)", c.ceilUp[idx], self.upColors.ceilingInactive],
                ].map(([name, v, color]) => ({name, text: formatBps(v), color}));
            }
            return [
                ["Access RTT (P50)", c.lan[idx], self.downColors.throughputLine],
                ["Internet RTT (P50)", c.wan[idx], self.upColors.throughputLine],
            ].map(([name, v, color]) => ({name, text: formatRttMs(v), color}));
        };
        return {
            hooks: {
                init: (u) => {
                    tip = document.createElement("div");
                    tip.style.cssText = "position:absolute;pointer-events:none;z-index:10;display:none;"
                        + "background:var(--bs-body-bg, rgba(9,20,31,0.94));color:var(--bs-body-color, #d8e2f4);"
                        + "border:1px solid var(--bs-border-color, rgba(216,226,244,0.18));border-radius:6px;"
                        + "padding:6px 8px;font:11px/1.45 system-ui, sans-serif;white-space:nowrap;"
                        + "box-shadow:0 4px 14px rgba(0,0,0,0.35);";
                    u.over.appendChild(tip);
                },
                setCursor: (u) => {
                    if (!tip) return;
                    const idx = u.cursor.idx;
                    const rows = idx == null ? null : buildRows(idx);
                    if (!rows || !self.cached) {
                        tip.style.display = "none";
                        return;
                    }
                    const when = self.cached.xs[idx];
                    // Use the sliding `displayNow` (the x-axis clock), not the
                    // constructor-only `renderNow`, or the header always reads "now".
                    const secs = Math.max(0, Math.round((self.displayNow - when) / 1000));
                    let html = `<div style="font-weight:700;margin-bottom:3px;">${secs === 0 ? "now" : `-${secs}s`}</div>`;
                    for (const r of rows) {
                        html += `<div><span style="display:inline-block;width:9px;height:9px;border-radius:10px;margin-right:5px;background-color:${r.color};"></span>${r.name}: <b>${r.text}</b></div>`;
                    }
                    tip.innerHTML = html;
                    tip.style.display = "block";
                    const w = u.over.clientWidth;
                    const left = u.cursor.left + 12 + tip.offsetWidth > w ? u.cursor.left - 12 - tip.offsetWidth : u.cursor.left + 12;
                    tip.style.left = `${Math.max(2, left)}px`;
                    tip.style.top = `${u.cursor.top + 12}px`;
                },
            },
        };
    }

    axisCommon(scale, side, colors) {
        return {
            scale, side,
            stroke: colors.axisText,
            grid: {stroke: colors.splitLine, width: 1},
            ticks: {stroke: colors.axisText, width: 1},
            font: AXIS_FONT,
        };
    }

    topOptions(width, height, sync) {
        const down = this.downColors;
        const up = this.upColors;
        return {
            width, height,
            padding: [8, 60, 6, 8],
            legend: {show: false},
            cursor: {show: true, x: true, y: false, points: {show: false}, sync},
            scales: {
                x: {time: false, range: () => this.xRange},
                bps: {range: () => [0, this.bpsAxisMax]},
            },
            axes: [
                {...this.axisCommon("x", 2, down), show: false},
                {...this.axisCommon("bps", 1, down), values: (u, splits) => splits.map((v) => formatAxisNumber(v) + "bps")},
            ],
            plugins: [this.makeTooltipPlugin("bps")],
            series: [
                {},
                {label: "Enqueued (DL)", scale: "bps", stroke: down.enqueuedLine, width: 1.4, dash: [2, 4], points: {show: false}, paths: this.stepped},
                {label: "Throughput (DL)", scale: "bps", stroke: down.throughputLine, width: 1.6, points: {show: false}, fill: this.fillFn("down"), paths: this.stepped},
                {label: "Ceiling Base (DL)", scale: "bps", stroke: down.ceilingInactive, width: 1, dash: [4, 4], points: {show: false}, paths: this.stepped},
                {label: "Enqueued (UL)", scale: "bps", stroke: up.enqueuedLine, width: 1.4, dash: [2, 4], points: {show: false}, paths: this.stepped},
                {label: "Throughput (UL)", scale: "bps", stroke: up.throughputLine, width: 1.6, points: {show: false}, fill: this.fillFn("up"), paths: this.stepped},
                {label: "Ceiling Base (UL)", scale: "bps", stroke: up.ceilingInactive, width: 1, dash: [4, 4], points: {show: false}, paths: this.stepped},
            ],
        };
    }

    botOptions(width, height, sync) {
        const down = this.downColors;
        const up = this.upColors;
        return {
            width, height,
            padding: [8, 60, 22, 8],
            legend: {show: false},
            cursor: {show: true, x: true, y: false, points: {show: false}, sync},
            scales: {
                x: {time: false, range: () => this.xRange},
                rtt: {range: () => [0, this.rttAxisMaxValue]},
            },
            axes: [
                {
                    ...this.axisCommon("x", 2, down),
                    values: (u, splits) => splits.map((v) => {
                        const s = Math.round((this.displayNow - v) / 1000);
                        return s <= 0 ? "now" : `-${s}s`;
                    }),
                },
                {...this.axisCommon("rtt", 1, down), grid: {show: false}, values: (u, splits) => splits.map((v) => `${v} ms`)},
            ],
            series: [
                {},
                {label: "Access RTT (P50)", scale: "rtt", stroke: down.throughputLine, width: 1.6, points: {show: false}, fill: this.fillFn("rtt"), paths: this.stepped},
                {label: "Internet RTT (P50)", scale: "rtt", stroke: up.throughputLine, width: 1.6, points: {show: false}, paths: this.stepped},
            ],
            plugins: [
                this.makeTooltipPlugin("rtt"),
                {hooks: {drawClear: (u) => this.drawRttBands(u)}},
            ],
        };
    }

    drawRttBands(u) {
        const t = this.rttThresholds;
        const max = this.rttAxisMaxValue;
        const {ctx} = u;
        const {left, top, width, height} = u.bbox;
        const y = (v) => u.valToPos(Math.max(0, Math.min(v, max)), "rtt", true);
        const yMax = u.valToPos(max, "rtt", true);
        const yBottom = u.valToPos(0, "rtt", true);
        ctx.save();
        ctx.beginPath();
        ctx.rect(left, top, width, height);
        ctx.clip();
        const fill = (a, b, color) => {
            const yTop = Math.min(a, b);
            const yBot = Math.max(a, b);
            if (yBot - yTop > 0.5) {
                ctx.fillStyle = color;
                ctx.fillRect(left, yTop, width, yBot - yTop);
            }
        };
        fill(y(t.yellow_ms), yBottom, this.downColors.rttBandGood);
        fill(y(t.red_ms), y(t.yellow_ms), this.downColors.rttBandWarn);
        fill(yMax, y(t.red_ms), this.downColors.rttBandBad);
        ctx.restore();
    }

    buildTopData() {
        const c = this.cached;
        if (!c || !c.xs || c.xs.length === 0) return [[]];
        return [c.xs, c.enqDown, c.tpDown, c.ceilDown, c.enqUp, c.tpUp, c.ceilUp];
    }

    buildBotData() {
        const c = this.cached;
        if (!c || !c.xs || c.xs.length === 0) return [[]];
        return [c.xs, c.lan, c.wan];
    }

    quantizeTimestamp(arrivalTimestamp) {
        const arrival = toNumber(arrivalTimestamp, Date.now());
        if (this.sampleClockBase === null) {
            this.sampleClockBase = arrival;
            this.lastSampleTimestamp = arrival;
            return arrival;
        }
        let quantized = this.sampleClockBase + Math.round((arrival - this.sampleClockBase) / SAMPLE_INTERVAL_MS) * SAMPLE_INTERVAL_MS;
        if (this.lastSampleTimestamp !== null && quantized <= this.lastSampleTimestamp) {
            quantized = this.lastSampleTimestamp + SAMPLE_INTERVAL_MS;
        }
        this.lastSampleTimestamp = quantized;
        return quantized;
    }

    pushSample(sample) {
        const normalized = {
            timestamp: this.quantizeTimestamp(sample.timestamp || Date.now()),
            throughputBps: {down: toNumber(sample.throughputBps?.down, 0), up: toNumber(sample.throughputBps?.up, 0)},
            actualThroughputBps: {down: toNumber(sample.actualThroughputBps?.down, 0), up: toNumber(sample.actualThroughputBps?.up, 0)},
            ceilingBps: {down: toNumber(sample.ceilingBps?.down, 0), up: toNumber(sample.ceilingBps?.up, 0)},
            rttP50Ms: {
                subscriber: toNumber(sample.rttP50Ms?.subscriber, null),
                internet: toNumber(sample.rttP50Ms?.internet, null),
            },
        };
        this.samples.push(normalized);
        if (this.samples.length > MAX_SAMPLES) this.samples.shift();
        this.pruneSamples(normalized.timestamp);
        this.rebuildCached();
    }

    pruneSamples(referenceTimestamp = Date.now()) {
        if (referenceTimestamp - this.lastPruneTimestamp < SAMPLE_INTERVAL_MS) return;
        this.lastPruneTimestamp = referenceTimestamp;
        const cutoff = referenceTimestamp - WINDOW_MS - SAMPLE_INTERVAL_MS * 2;
        while (this.samples.length > 0 && this.samples[0].timestamp < cutoff) this.samples.shift();
    }

    rebuildCached() {
        const n = this.samples.length;
        const xs = new Array(n + 1);
        const enqDown = new Array(n + 1), tpDown = new Array(n + 1), ceilDown = new Array(n + 1);
        const enqUp = new Array(n + 1), tpUp = new Array(n + 1), ceilUp = new Array(n + 1);
        const lan = new Array(n + 1), wan = new Array(n + 1);
        let observed = 0;
        let bpsObserved = 0;
        for (let i = 0; i < n; i++) {
            const s = this.samples[i];
            xs[i] = s.timestamp;
            enqDown[i] = s.actualThroughputBps.down; tpDown[i] = s.throughputBps.down; ceilDown[i] = s.ceilingBps.down;
            enqUp[i] = s.actualThroughputBps.up; tpUp[i] = s.throughputBps.up; ceilUp[i] = s.ceilingBps.up;
            lan[i] = s.rttP50Ms.subscriber; wan[i] = s.rttP50Ms.internet;
            for (const v of [lan[i], wan[i]]) if (Number.isFinite(v)) observed = Math.max(observed, v);
            for (const v of [enqDown[i], tpDown[i], ceilDown[i], enqUp[i], tpUp[i], ceilUp[i]]) if (Number.isFinite(v)) bpsObserved = Math.max(bpsObserved, v);
        }
        const j = Math.max(0, n - 1);
        xs[n] = n > 0 ? xs[j] : Date.now();
        enqDown[n] = enqDown[j] ?? 0; tpDown[n] = tpDown[j] ?? 0; ceilDown[n] = ceilDown[j] ?? 0;
        enqUp[n] = enqUp[j] ?? 0; tpUp[n] = tpUp[j] ?? 0; ceilUp[n] = ceilUp[j] ?? 0;
        lan[n] = lan[j] ?? null; wan[n] = wan[j] ?? null;
        this.cached = {xs, enqDown, tpDown, ceilDown, enqUp, tpUp, ceilUp, lan, wan};
        this.observedRttMax = observed;
        this.rttAxisMaxValue = rttAxisMax(this.rttThresholds, observed);
        this.bpsAxisMax = niceAxisMax(bpsObserved);
    }

    scheduleRenderLoop() {
        if (!this.renderingEnabled || this.renderRaf !== null) return;
        const step = (timestamp) => {
            if ((!this.uTop && !this.uBot) || !this.renderingEnabled) {
                this.renderRaf = null;
                return;
            }
            if (!this.lastRenderTimestamp || timestamp - this.lastRenderTimestamp >= RENDER_INTERVAL_MS) {
                this.lastRenderTimestamp = timestamp;
                this.render();
            }
            this.renderRaf = window.requestAnimationFrame(step);
        };
        this.renderRaf = window.requestAnimationFrame(step);
    }

    render() {
        if (!this.uTop || !this.uBot) return;
        this.renderNow = Date.now();
        const displayNow = this.renderNow - DISPLAY_LAG_MS;
        this.displayNow = displayNow;
        this.xRange = [displayNow - WINDOW_MS, displayNow];
        const c = this.cached;
        if (c && c.xs && c.xs.length) {
            c.xs[c.xs.length - 1] = displayNow;
            this.uTop.setData(this.buildTopData(), true);
            this.uBot.setData(this.buildBotData(), true);
        }
    }

    setRenderingEnabled(enabled) {
        const next = !!enabled;
        if (this.renderingEnabled === next) return;
        this.renderingEnabled = next;
        if (!this.renderingEnabled) {
            if (this.renderRaf) {
                window.cancelAnimationFrame(this.renderRaf);
                this.renderRaf = null;
            }
            return;
        }
        this.lastRenderTimestamp = 0;
        this.scheduleRenderLoop();
        this.render();
    }

    setRttThresholds(rawThresholds) {
        this.rttThresholds = normalizeRttThresholds(rawThresholds);
        this.rttAxisMaxValue = rttAxisMax(this.rttThresholds, this.observedRttMax);
        if (this.uBot) this.uBot.setData(this.buildBotData(), true);
    }

    legendColors() {
        return {
            subscriber: this.downColors.throughputLine,
            internet: this.upColors.throughputLine,
            full: this.downColors.rttLine,
        };
    }

    onThemeChange() {
        this.downColors = getWaveformTheme("down");
        this.upColors = getWaveformTheme("up");
        this.rebuildCharts();
    }

    /// Recreate both uPlot instances with the current theme colors. Theme toggles
    /// are rare; recreating refreshes strokes, fills, axes and the RTT bands
    /// together and avoids uPlot's `setSeries`/`redraw` mutation pitfalls.
    rebuildCharts() {
        const width = Math.max(320, this.dom.clientWidth || 320);
        const height = Math.max(120, this.dom.clientHeight || 336);
        const gap = 6;
        const topH = Math.max(60, Math.round((height - gap) * 0.52));
        const botH = Math.max(60, height - gap - topH);
        if (this.uTop) {
            this.uTop.destroy();
            this.uTop = null;
        }
        if (this.uBot) {
            this.uBot.destroy();
            this.uBot = null;
        }
        const sync = uPlot.sync("lqos-qd-" + this.id);
        this.uTop = new uPlot(this.topOptions(width, topH, sync), this.buildTopData(), this.topDom);
        this.uBot = new uPlot(this.botOptions(width, botH, sync), this.buildBotData(), this.botDom);
    }

    resize() {
        if (!this.dom) return;
        const width = Math.max(320, this.dom.clientWidth || 320);
        const height = Math.max(120, this.dom.clientHeight || 336);
        const gap = 6;
        const topH = Math.max(60, Math.round((height - gap) * 0.52));
        const botH = Math.max(60, height - gap - topH);
        if (this.uTop) this.uTop.setSize({width, height: topH});
        if (this.uBot) this.uBot.setSize({width, height: botH});
    }

    dispose() {
        if (this.renderRaf) {
            window.cancelAnimationFrame(this.renderRaf);
            this.renderRaf = null;
        }
        if (this.themeObserver) {
            this.themeObserver.disconnect();
            this.themeObserver = null;
        }
        if (this.resizeHandler) {
            window.removeEventListener("resize", this.resizeHandler);
            this.resizeHandler = null;
        }
        if (this.resizeRaf) {
            window.cancelAnimationFrame(this.resizeRaf);
            this.resizeRaf = null;
        }
        if (this.uTop) {
            this.uTop.destroy();
            this.uTop = null;
        }
        if (this.uBot) {
            this.uBot.destroy();
            this.uBot = null;
        }
    }
}
