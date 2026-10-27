import {isColorBlindMode} from "../helpers/colorblind";
import {isDarkMode} from "../helpers/dark_mode";
import {scaleNumber} from "../lq_js_common/helpers/scaling";

// `.66.7`: the Live Traffic Pulse canvas renderer. Replaces the ECharts graph:
// a plain <canvas> + requestAnimationFrame at display rate (~60 fps), DPR-aware,
// driven by the EXISTING 1 Hz `Throughput` feed. Smoothness comes from the
// render: each frame the window scrolls by elapsed time and the newest edge is
// linearly interpolated between the two bracketing samples (with a one-sample
// render lag so the "next" value is always known). No spline — crisp segments.
const WINDOW_MS = 30_000; // the visible pulse window
const LAG_MS = 1_000; // one sample of render delay (needed to interpolate the edge)

export function pulsePalette() {
    if (isColorBlindMode()) {
        // Okabe-Ito: blue / sky / vermillion.
        return {mapped: "#0072B2", mappedUp: "#56B4E9", unmapped: "#D55E00"};
    }
    return {mapped: "#2f8bff", mappedUp: "#22d3ee", unmapped: "#ff6f61"};
}

function withAlpha(hex, alpha) {
    const h = String(hex).replace("#", "");
    const r = parseInt(h.slice(0, 2), 16);
    const g = parseInt(h.slice(2, 4), 16);
    const b = parseInt(h.slice(4, 6), 16);
    return `rgba(${r},${g},${b},${alpha})`;
}

function lerp(a, b, f) {
    return a + (b - a) * f;
}

export class LiveTrafficPulseCanvas {
    constructor(canvasEl) {
        this.canvas = canvasEl;
        this.ctx = canvasEl.getContext("2d");
        // Time-indexed ring: samples whose timestamp is within the last 30 s
        // (+ a small margin so the left edge can interpolate).
        this.samples = [];
        this._frameId = null;
        this._stopped = false;
        this._downMax = 0;
        this._upMax = 0;
        this._w = 1;
        this._h = 1;
        this._dpr = 1;

        this._onVisibility = () => {
            if (document.hidden) {
                this._stop();
            } else {
                this._start();
            }
        };
        document.addEventListener("visibilitychange", this._onVisibility);

        this._resizeObserver = typeof ResizeObserver !== "undefined"
            ? new ResizeObserver(() => this._resize())
            : null;
        if (this._resizeObserver) {
            this._resizeObserver.observe(canvasEl);
        }

        this._resize();
        this._start();
    }

    // `.66.7`: a new 1 Hz sample. `sample.t` is a wall-clock ms timestamp.
    push(sample) {
        this.samples.push(sample);
        const cutoff = sample.t - WINDOW_MS - LAG_MS;
        while (this.samples.length > 0 && this.samples[0].t < cutoff) {
            this.samples.shift();
        }
    }

    dispose() {
        this._stopped = true;
        this._stop();
        document.removeEventListener("visibilitychange", this._onVisibility);
        if (this._resizeObserver) {
            this._resizeObserver.disconnect();
            this._resizeObserver = null;
        }
        if (this.ctx) {
            this.ctx.setTransform(1, 0, 0, 1, 0, 0);
            this.ctx.clearRect(0, 0, this.canvas.width, this.canvas.height);
        }
    }

    _resize() {
        const rect = this.canvas.getBoundingClientRect();
        const dpr = window.devicePixelRatio || 1;
        this._w = Math.max(1, Math.round(rect.width));
        this._h = Math.max(1, Math.round(rect.height));
        this._dpr = dpr;
        this.canvas.width = Math.max(1, Math.round(this._w * dpr));
        this.canvas.height = Math.max(1, Math.round(this._h * dpr));
    }

    _start() {
        if (this._stopped || this._frameId !== null || document.hidden) {
            return;
        }
        this._frameId = window.requestAnimationFrame(this._frame);
    }

    _stop() {
        if (this._frameId !== null) {
            window.cancelAnimationFrame(this._frameId);
            this._frameId = null;
        }
    }

    _frame = () => {
        this._frameId = null;
        if (this._stopped || document.hidden) {
            return;
        }
        this._draw(Date.now());
        this._frameId = window.requestAnimationFrame(this._frame);
    };

    // Linear interpolation of all four series at time `t` (holds at the ends).
    #sampleAt(t) {
        const s = this.samples;
        if (s.length === 0) {
            return null;
        }
        if (t <= s[0].t) {
            return {...s[0], t};
        }
        const last = s[s.length - 1];
        if (t >= last.t) {
            return {...last, t};
        }
        let i = 0;
        while (i < s.length - 1 && s[i + 1].t <= t) {
            i += 1;
        }
        const a = s[i];
        const b = s[i + 1];
        const dt = b.t - a.t;
        const f = dt > 0 ? (t - a.t) / dt : 0;
        return {
            t,
            classPresent: a.classPresent && b.classPresent,
            mappedDown: lerp(a.mappedDown, b.mappedDown, f),
            mappedUp: lerp(a.mappedUp, b.mappedUp, f),
            unmappedDown: lerp(a.unmappedDown, b.unmappedDown, f),
            unmappedUp: lerp(a.unmappedUp, b.unmappedUp, f),
        };
    }

    _draw(now) {
        const ctx = this.ctx;
        if (!ctx) {
            return;
        }
        const w = this._w;
        const h = this._h;
        const dpr = this._dpr;
        ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
        ctx.clearRect(0, 0, w, h);

        const dark = isDarkMode();
        const p = pulsePalette();
        const muted = dark ? "#8b98a8" : "#5b6674";

        const mid = h / 2;
        const plotH = Math.max(8, h / 2 - 8);
        const axisW = 58;
        const plotW = Math.max(8, w - axisW);

        // `.66.7`: render one sample behind (LAG_MS) so the newest edge can be
        // linearly interpolated; the window scrolls by elapsed wall time.
        const renderNow = now - LAG_MS;
        const t0 = renderNow - WINDOW_MS;

        // x-axis anchors (crisp text, always drawn).
        ctx.font = "9px system-ui, -apple-system, Segoe UI, sans-serif";
        ctx.fillStyle = muted;
        ctx.textAlign = "left";
        ctx.fillText("30s ago", axisW, h - 2);
        ctx.textAlign = "right";
        ctx.fillText("Now", w - 2, h - 2);
        ctx.textAlign = "center";
        for (let k = 1; k <= 5; k += 1) {
            const x = axisW + (k * 5 / 30) * plotW;
            ctx.fillText(`${30 - k * 5}s`, x, h - 2);
        }
        ctx.textAlign = "left";

        if (this.samples.length < 2) {
            this._downMax = 0;
            this._upMax = 0;
            return;
        }

        // Build the visible point list: window ends + every sample inside.
        const times = [t0];
        for (const s of this.samples) {
            if (s.t > t0 && s.t < renderNow) {
                times.push(s.t);
            }
        }
        times.push(renderNow);
        const points = times
            .map((t) => this.#sampleAt(t))
            .filter((s) => s !== null)
            .map((s) => ({...s, x: axisW + ((s.t - t0) / WINDOW_MS) * plotW}));
        if (points.length < 2) {
            return;
        }

        // Independent half-scales: the download half is normalized to the
        // download peak, the upload half to the upload peak, with the zero line
        // centered. This keeps both directions readable on asymmetric links
        // (a download-dominant ISP no longer leaves the lower half blank).
        let downMax = 1;
        let upMax = 1;
        for (const s of points) {
            downMax = Math.max(downMax, s.mappedDown + s.unmappedDown);
            upMax = Math.max(upMax, s.mappedUp + s.unmappedUp);
        }
        const downTarget = downMax * 1.12;
        const upTarget = upMax * 1.12;
        if (downTarget > this._downMax || this._downMax <= 0) {
            this._downMax = downTarget;
        } else {
            this._downMax += (downTarget - this._downMax) * 0.05;
        }
        if (upTarget > this._upMax || this._upMax <= 0) {
            this._upMax = upTarget;
        } else {
            this._upMax += (upTarget - this._upMax) * 0.05;
        }
        const downScale = plotH / this._downMax;
        const upScale = plotH / this._upMax;
        const yDown = (v) => mid - v * downScale;
        const yUp = (v) => mid + v * upScale;

        // y-axis labels (positive above the zero line, negative below).
        ctx.fillStyle = muted;
        ctx.textAlign = "right";
        const yLabel = (v) => {
            const abs = Math.abs(v);
            const text = abs < 1 ? "0" : scaleNumber(abs, 1).replace(/([TGMK])$/, " $1bps");
            return v < 0 ? `-${text}` : text;
        };
        ctx.fillText(yLabel(this._downMax), axisW - 6, mid - plotH + 3);
        ctx.fillText(yLabel(this._downMax * 0.5), axisW - 6, mid - plotH * 0.5 + 3);
        ctx.fillText("0", axisW - 6, mid + 3);
        ctx.fillText(yLabel(-this._upMax * 0.5), axisW - 6, mid + plotH * 0.5 + 3);
        ctx.fillText(yLabel(-this._upMax), axisW - 6, mid + plotH + 3);
        ctx.textAlign = "left";

        // Faint grid lines at the half/full ticks.
        ctx.strokeStyle = dark ? "rgba(255,255,255,0.05)" : "rgba(0,0,0,0.05)";
        ctx.lineWidth = 1;
        for (const frac of [1, 0.5, -0.5, -1]) {
            const y = mid - frac * plotH;
            ctx.beginPath();
            ctx.moveTo(axisW, y);
            ctx.lineTo(w, y);
            ctx.stroke();
        }

        // Faint zero line.
        ctx.strokeStyle = dark ? "rgba(255,255,255,0.08)" : "rgba(0,0,0,0.08)";
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.moveTo(axisW, mid);
        ctx.lineTo(w, mid);
        ctx.stroke();

        const downGrad = ctx.createLinearGradient(0, mid - plotH, 0, mid);
        downGrad.addColorStop(0, p.mapped);
        downGrad.addColorStop(1, withAlpha(p.mapped, 0.12));
        const upGrad = ctx.createLinearGradient(0, mid, 0, mid + plotH);
        upGrad.addColorStop(0, withAlpha(p.mappedUp, 0.12));
        upGrad.addColorStop(1, p.mappedUp);

        // ── download (up on screen): mapped fill, then the coral unmapped band ──
        this.#fillBand(points, (s) => yDown(s.mappedDown), mid, downGrad);
        this.#fillStackedBand(
            points,
            (s) => yDown(s.mappedDown),
            (s) => yDown(s.mappedDown + (s.classPresent ? s.unmappedDown : 0)),
            withAlpha(p.unmapped, 0.85),
        );
        // ── upload (down on screen) ──
        this.#fillBand(points, (s) => yUp(s.mappedUp), mid, upGrad);
        this.#fillStackedBand(
            points,
            (s) => yUp(s.mappedUp),
            (s) => yUp(s.mappedUp + (s.classPresent ? s.unmappedUp : 0)),
            withAlpha(p.unmapped, 0.85),
        );

        // Silhouette outlines (crisp, no spline).
        const strokeTop = (getY, color, width) => {
            ctx.strokeStyle = color;
            ctx.lineWidth = width;
            ctx.beginPath();
            points.forEach((s, i) => {
                const y = getY(s);
                if (i === 0) {
                    ctx.moveTo(s.x, y);
                } else {
                    ctx.lineTo(s.x, y);
                }
            });
            ctx.stroke();
        };
        strokeTop((s) => yDown(s.mappedDown + (s.classPresent ? s.unmappedDown : 0)), withAlpha(p.mapped, dark ? 0.9 : 0.8), 1.25);
        strokeTop((s) => yDown(s.mappedDown), withAlpha(p.unmapped, 0.55), 1);
        strokeTop((s) => yUp(s.mappedUp + (s.classPresent ? s.unmappedUp : 0)), withAlpha(p.mappedUp, dark ? 0.9 : 0.8), 1.25);
        strokeTop((s) => yUp(s.mappedUp), withAlpha(p.unmapped, 0.55), 1);

        // Latest-edge glow (a subtle breathing pulse).
        const edge = points[points.length - 1];
        const breathe = 3 + 1.6 * (0.5 + 0.5 * Math.sin(now / 320));
        const glow = (x, y, color) => {
            ctx.save();
            ctx.shadowColor = color;
            ctx.shadowBlur = 12;
            ctx.fillStyle = color;
            ctx.beginPath();
            ctx.arc(x, y, breathe, 0, Math.PI * 2);
            ctx.fill();
            ctx.restore();
        };
        glow(edge.x, yDown(edge.mappedDown + (edge.classPresent ? edge.unmappedDown : 0)), p.mapped);
        glow(edge.x, yUp(edge.mappedUp + (edge.classPresent ? edge.unmappedUp : 0)), p.mappedUp);
    }

    // Fill between the baseline (`baseY`) and a value curve.
    #fillBand(points, getY, baseY, fillStyle) {
        const ctx = this.ctx;
        ctx.beginPath();
        ctx.moveTo(points[0].x, baseY);
        for (const s of points) {
            ctx.lineTo(s.x, getY(s));
        }
        ctx.lineTo(points[points.length - 1].x, baseY);
        ctx.closePath();
        ctx.fillStyle = fillStyle;
        ctx.fill();
    }

    // Fill between two value curves (the coral band inside the silhouette).
    #fillStackedBand(points, lowerY, upperY, fillStyle) {
        const ctx = this.ctx;
        ctx.beginPath();
        points.forEach((s, i) => {
            const y = lowerY(s);
            if (i === 0) {
                ctx.moveTo(s.x, y);
            } else {
                ctx.lineTo(s.x, y);
            }
        });
        for (let i = points.length - 1; i >= 0; i -= 1) {
            ctx.lineTo(points[i].x, upperY(points[i]));
        }
        ctx.closePath();
        ctx.fillStyle = fillStyle;
        ctx.fill();
    }
}
