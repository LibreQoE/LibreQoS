import {lerpGreenToRedViaOrange} from "../helpers/scaling.js";
import {isColorBlindMode} from "../helpers/colorblind.js";
import {heatRowAriaLabel, typeMonoMarkup} from "../executive_heat_labels.mjs";

export const MAX_HEATMAP_ROWS = 10;

function lerpViridis(t) {
    const stops = [
        [68, 1, 84],
        [59, 82, 139],
        [33, 145, 140],
        [94, 201, 98],
        [253, 231, 37],
    ];
    if (t <= 0) return "#440154";
    if (t >= 1) return "#FDE725";
    const idx = t * (stops.length - 1);
    const i = Math.floor(idx);
    const frac = idx - i;
    const c0 = stops[i];
    const c1 = stops[i + 1];
    const r = Math.round(c0[0] + frac * (c1[0] - c0[0]));
    const g = Math.round(c0[1] + frac * (c1[1] - c0[1]));
    const b = Math.round(c0[2] + frac * (c1[2] - c0[2]));
    return "#" + ((1 << 24) + (r << 16) + (g << 8) + b).toString(16).slice(1);
}

export function formatCount(value) {
    if (value === undefined || value === null) return "—";
    const num = Number(value);
    if (!Number.isFinite(num)) return "—";
    return num.toLocaleString();
}

export function clampPercent(value) {
    const num = Number(value) || 0;
    return Math.min(100, Math.max(0, num));
}

export const colorByCapacity = (pct) => {
    const clamped = clampPercent(pct);
    return isColorBlindMode()
        ? lerpViridis(clamped / 100)
        : lerpGreenToRedViaOrange(100 - clamped, 100);
};

export function isIpLike(name) {
    if (!name) return false;
    return /^[0-9a-fA-F:.]+$/.test(name) && (name.includes(".") || name.includes(":"));
}

export function buildHeatmapRows(data) {
    const rows = [];
    const sites = (data?.sites || [])
        .filter(site => site.blocks)
        .filter(site => !isIpLike(site.site_name))
        .filter(site => site.depth === undefined || site.depth <= 2)
        .filter(site => {
            const t = (site.node_type || "").toLowerCase();
            return t === "site" || t === "ap" || t === "";
        });
    sites.forEach(site => {
        const siteLabel = site.site_name || "Site";
        rows.push({
            label: siteLabel,
            site_name: site.site_name || "",
            badge: "Site",
            blocks: site.blocks,
            qoq_blocks: site.qoq_blocks,
        });
    });
    const circuits = (data?.circuits || []);
    circuits.forEach(circuit => {
        const name = circuit.circuit_name || circuit.circuit_id || `Circuit ${circuit.circuit_hash}`;
        rows.push({
            label: name,
            circuit_id: circuit.circuit_id || "",
            badge: "Circuit",
            blocks: circuit.blocks,
            qoq_blocks: circuit.qoq_blocks,
        });
    });
    const asns = (data?.asns || []);
    asns.forEach(asn => {
        const label = asn.asn_name
            ? `${asn.asn_name} (ASN ${asn.asn})`
            : `ASN ${asn.asn}`;
        rows.push({
            label,
            badge: "ASN",
            blocks: asn.blocks,
        });
    });

    return rows;
}

export function formatLatest(value, unit = "", precision = 0) {
    if (value === null || value === undefined || Number.isNaN(value)) return "—";
    const suffix = unit ? ` ${unit}` : "";
    if (precision === 0) {
        return `${Math.round(value)}${suffix}`;
    }
    return `${value.toFixed(precision)}${suffix}`;
}

export function latestValue(values) {
    if (!values || !values.length) return null;
    for (let i = values.length - 1; i >= 0; i--) {
        const val = values[i];
        if (val !== null && val !== undefined) {
            const num = Number(val);
            if (Number.isFinite(num)) {
                return num;
            }
        }
    }
    return null;
}

export function nonNullCount(values) {
    if (!values || !values.length) return 0;
    return values.filter(v => v !== null && v !== undefined).length;
}

function escapeHeatAttr(text) {
    return String(text ?? "")
        .replace(/&/g, "&amp;")
        .replace(/"/g, "&quot;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;");
}

function tooltipHtmlFromLines(lines) {
    return (Array.isArray(lines) ? lines : [])
        .filter((line) => !!String(line ?? "").trim())
        .map((line, idx) => `<div${idx === 0 ? ` class="fw-semibold"` : ""}>${escapeHeatAttr(line)}</div>`)
        .join("");
}

function tooltipAttrsFromLines(lines) {
    const filtered = (Array.isArray(lines) ? lines : [])
        .map((line) => String(line ?? "").trim())
        .filter(Boolean);
    const html = tooltipHtmlFromLines(filtered);
    const aria = filtered.join(". ");
    return `data-bs-toggle="tooltip" data-bs-placement="top" data-bs-container="body" data-bs-trigger="hover focus" data-bs-html="true" tabindex="0" title="${escapeHeatAttr(html)}" aria-label="${escapeHeatAttr(aria)}"`;
}

function noDataTooltipLines(blockIndex, meaning) {
    return [
        `15-minute block ${blockIndex}`,
        "No data",
        meaning || "",
    ];
}

function scalarTooltipLines(blockIndex, valueLabel, valueText, meaning, extra = "") {
    return [
        `15-minute block ${blockIndex}`,
        `${valueLabel}: ${valueText}`,
        meaning,
        extra,
    ];
}

function splitTooltipLines(blockIndex, topLabel, topValue, bottomLabel, bottomValue, meaning) {
    return [
        `15-minute block ${blockIndex}`,
        `${topLabel}: ${topValue}`,
        `${bottomLabel}: ${bottomValue}`,
        meaning,
    ];
}

function rttTooltipLines(blockIndex, ulP50, ulP90, dlP50, dlP90, meaning) {
    return [
        `15-minute block ${blockIndex}`,
        `Upload p50 RTT: ${ulP50}`,
        `Upload p90 RTT: ${ulP90}`,
        `Download p50 RTT: ${dlP50}`,
        `Download p90 RTT: ${dlP90}`,
        meaning,
    ];
}

function latestDetailMarkup(latest, formatValue, describeValue) {
    const detail = describeValue ? describeValue(latest) : "";
    return `
        <div>${formatValue(latest)}</div>
        ${detail ? `<div class="text-body-secondary">${escapeHeatAttr(detail)}</div>` : ""}
    `;
}

export function heatmapRow(values, colorFn, formatValue, describeValue = null) {
    const length = Array.isArray(values) && values.length ? values.length : 15;
    let cells = "";
    for (let i = 0; i < length; i++) {
        const val = values && values[i] !== undefined ? values[i] : null;
        if (val === null || val === undefined) {
            cells += `<div class="exec-heat-cell empty" ${tooltipAttrsFromLines(noDataTooltipLines(i + 1, "This block has no metric data yet."))}></div>`;
            continue;
        }
        const numeric = Number(val) || 0;
        const color = colorFn(numeric);
        const title = formatValue(numeric);
        const detail = describeValue ? describeValue(numeric) : "";
        const tooltipLines = scalarTooltipLines(
            i + 1,
            "Value",
            title,
            "This cell shows the metric value for this 15-minute block.",
            detail ? `Context: ${detail}` : "",
        );
        cells += `<div class="exec-heat-cell" style="background:${color}" ${tooltipAttrsFromLines(tooltipLines)}></div>`;
    }
    return cells;
}

export function heatRow(label, badge, values, colorFn, formatValue, link = null, describeValue = null) {
    const latest = latestValue(values);
    const formattedLatest = latestDetailMarkup(latest, formatValue, describeValue);
    const redactClass =
        badge === "Site" || badge === "Circuit" ? " redactable" : "";
    const labelMarkup = link ? `<a href="${link}">${label}</a>` : label;
    const latestLabel = describeValue ? `${formatValue(latest)} ${describeValue(latest)}`.trim() : formatValue(latest);
    return `
        <div class="exec-heat-row" role="listitem" aria-label="${escapeHeatAttr(heatRowAriaLabel(label, badge, latestLabel))}">
            <div class="exec-heat-label text-truncate" title="${escapeHeatAttr(label)}">
                ${typeMonoMarkup(badge)}<div class="fw-semibold text-truncate${redactClass}">${labelMarkup}</div>
            </div>
            <div class="exec-heat-cells" role="img" aria-label="${escapeHeatAttr(`${label} heatmap history`)}">${heatmapRow(values, colorFn, formatValue, describeValue)}</div>
            <div class="text-muted small text-end exec-latest">${formattedLatest}</div>
        </div>
    `;
}

function heatmapTooltipIsActive(target) {
    if (!target) return false;
    if (target.querySelector('[data-bs-toggle="tooltip"][aria-describedby]')) {
        return true;
    }
    if (document.activeElement && target.contains(document.activeElement)) {
        return true;
    }
    try {
        return !!target.querySelector('[data-bs-toggle="tooltip"]:hover');
    } catch (_err) {
        return false;
    }
}

function flushDeferredHeatmapHtml(target) {
    if (!target) return false;
    const pendingHtml = target.__lqosPendingHeatmapHtml;
    if (pendingHtml === undefined || pendingHtml === null) {
        return false;
    }
    if (heatmapTooltipIsActive(target)) {
        return false;
    }
    delete target.__lqosPendingHeatmapHtml;
    if (target.__lqosLastHeatmapHtml === pendingHtml) {
        return false;
    }
    target.__lqosApplyHeatmapHtml?.(pendingHtml);
    return true;
}

function ensureDeferredHeatmapFlushListeners(target) {
    if (!target || target.__lqosHeatmapFlushListenersAttached) {
        return;
    }
    const scheduleFlush = () => {
        window.setTimeout(() => {
            flushDeferredHeatmapHtml(target);
        }, 0);
    };
    target.addEventListener("mouseleave", scheduleFlush);
    target.addEventListener("focusout", scheduleFlush);
    target.__lqosHeatmapFlushListenersAttached = true;
}

export function replaceHeatmapHtml(target, nextHtml, applyHtml) {
    if (!target || typeof applyHtml !== "function") {
        return false;
    }
    target.__lqosApplyHeatmapHtml = applyHtml;
    ensureDeferredHeatmapFlushListeners(target);
    if (target.__lqosLastHeatmapHtml === nextHtml) {
        delete target.__lqosPendingHeatmapHtml;
        return false;
    }
    if (heatmapTooltipIsActive(target)) {
        target.__lqosPendingHeatmapHtml = nextHtml;
        return false;
    }
    delete target.__lqosPendingHeatmapHtml;
    applyHtml(nextHtml);
    return true;
}

function heatmapRowQuadrants(quads, colorFn, formatValue, describeValue = null) {
    const length = Array.isArray(quads?.ul_p50) && quads.ul_p50.length ? quads.ul_p50.length : 15;
    let cells = "";
    for (let i = 0; i < length; i++) {
        const ulP50 = quads?.ul_p50?.[i];
        const ulP90 = quads?.ul_p90?.[i];
        const dlP50 = quads?.dl_p50?.[i];
        const dlP90 = quads?.dl_p90?.[i];

        const allMissing =
            (ulP50 === null || ulP50 === undefined) &&
            (ulP90 === null || ulP90 === undefined) &&
            (dlP50 === null || dlP50 === undefined) &&
            (dlP90 === null || dlP90 === undefined);
        if (allMissing) {
            cells += `<div class="exec-heat-cell empty" ${tooltipAttrsFromLines(noDataTooltipLines(i + 1, "This block has no RTT samples yet."))}></div>`;
            continue;
        }

        const fmt = (v) => formatValue(v);
        const describe = (v) => describeValue ? ` (${describeValue(v)})` : "";
        const tooltipLines = rttTooltipLines(
            i + 1,
            `${fmt(ulP50)}${describe(ulP50)}`,
            `${fmt(ulP90)}${describe(ulP90)}`,
            `${fmt(dlP50)}${describe(dlP50)}`,
            `${fmt(dlP90)}${describe(dlP90)}`,
            "Top row is upload, bottom row is download. Left is p50, right is p90.",
        );

        const quad = (v) => {
            if (v === null || v === undefined) {
                return `<div class="exec-quad empty"></div>`;
            }
            const numeric = Number(v);
            if (!Number.isFinite(numeric)) {
                return `<div class="exec-quad empty"></div>`;
            }
            const color = colorFn(numeric);
            return `<div class="exec-quad" style="background:${color}"></div>`;
        };

        // Quadrants: upload at top, download at bottom. Left=p50, right=p90.
        cells += `
            <div class="exec-heat-cell quad" ${tooltipAttrsFromLines(tooltipLines)}>
                <div class="exec-quad-grid">
                    ${quad(ulP50)}
                    ${quad(ulP90)}
                    ${quad(dlP50)}
                    ${quad(dlP90)}
                </div>
            </div>
        `;
    }
    return cells;
}

export function rttHeatRow(label, badge, blocks, colorFn, formatValue, link = null, describeValue = null, options = {}) {
    const quads = {
        ul_p50: blocks?.rtt_p50_up || [],
        ul_p90: blocks?.rtt_p90_up || [],
        dl_p50: blocks?.rtt_p50_down || [],
        dl_p90: blocks?.rtt_p90_down || [],
    };
    const latest = latestValue(blocks?.rtt || []);
    const formattedLatest = latestDetailMarkup(latest, formatValue, describeValue);
    const redactClass =
        badge === "Site" || badge === "Circuit" ? " redactable" : "";
    const labelMarkup = link ? `<a href="${link}">${label}</a>` : label;
    const latestLabel = describeValue ? `${formatValue(latest)} ${describeValue(latest)}`.trim() : formatValue(latest);
    // `.64.3.1` (A1): the Executive Median RTT WIDGET renders p50 ONLY (upload p50
    // over download p50, a 1x2 split); the detail page keeps the full 2x2 quad
    // (p50 / p90) — scoped via `options.p50Only` so only the widget changes.
    const cells = options.p50Only
        ? heatmapRowSplit(quads.ul_p50, quads.dl_p50, colorFn, formatValue, {
            topLabel: "Upload p50 RTT",
            bottomLabel: "Download p50 RTT",
            meaning: "Top is upload p50, bottom is download p50 (median RTT).",
        })
        : heatmapRowQuadrants(quads, colorFn, formatValue, describeValue);
    return `
        <div class="exec-heat-row" role="listitem" aria-label="${escapeHeatAttr(heatRowAriaLabel(label, badge, latestLabel, "RTT"))}">
            <div class="exec-heat-label text-truncate" title="${escapeHeatAttr(label)}">
                ${typeMonoMarkup(badge)}<div class="fw-semibold text-truncate${redactClass}">${labelMarkup}</div>
            </div>
            <div class="exec-heat-cells" role="img" aria-label="${escapeHeatAttr(`${label} RTT heatmap history`)}">${cells}</div>
            <div class="text-muted small text-end exec-latest">${formattedLatest}</div>
        </div>
    `;
}

function heatmapRowSplit(topValues, bottomValues, colorFn, formatValue, options = {}) {
    const length = Array.isArray(topValues) && topValues.length
        ? topValues.length
        : (Array.isArray(bottomValues) && bottomValues.length ? bottomValues.length : 15);
    const topLabel = options.topLabel || "Upload";
    const bottomLabel = options.bottomLabel || "Download";
    const meaning = options.meaning || "Top is upload, bottom is download.";
    let cells = "";
    for (let i = 0; i < length; i++) {
        const top = topValues?.[i];
        const bottom = bottomValues?.[i];

        const allMissing =
            (top === null || top === undefined) &&
            (bottom === null || bottom === undefined);
        if (allMissing) {
            cells += `<div class="exec-heat-cell empty" ${tooltipAttrsFromLines(noDataTooltipLines(i + 1, meaning))}></div>`;
            continue;
        }

        const numOrNull = (v) => (v === null || v === undefined ? null : Number(v));
        const fmt = (v) => formatValue(numOrNull(v));
        const tooltipLines = splitTooltipLines(
            i + 1,
            topLabel,
            fmt(top),
            bottomLabel,
            fmt(bottom),
            meaning,
        );

        const part = (v) => {
            if (v === null || v === undefined) {
                return `<div class="exec-split empty"></div>`;
            }
            const numeric = Number(v);
            if (!Number.isFinite(numeric)) {
                return `<div class="exec-split empty"></div>`;
            }
            const color = colorFn(numeric);
            return `<div class="exec-split" style="background:${color}"></div>`;
        };

        // Top = upload, bottom = download.
        cells += `
            <div class="exec-heat-cell split" ${tooltipAttrsFromLines(tooltipLines)}>
                <div class="exec-split-grid">
                    ${part(top)}
                    ${part(bottom)}
                </div>
            </div>
        `;
    }
    return cells;
}

function splitHeatRow(label, badge, topValues, bottomValues, colorFn, formatValue, link = null, options = {}) {
    const latestTop = latestValue(topValues);
    const latestBottom = latestValue(bottomValues);
    const formattedLatest = `
        <span class="exec-latest-pair" title="Upload / Download">
            <span>${formatValue(latestTop)}</span>
            <span class="exec-latest-sep">/</span>
            <span>${formatValue(latestBottom)}</span>
        </span>
    `;
    const redactClass =
        badge === "Site" || badge === "Circuit" ? " redactable" : "";
    const labelMarkup = link ? `<a href="${link}">${label}</a>` : label;
    const latestLabel = `${formatValue(latestTop)} / ${formatValue(latestBottom)}`;
    return `
        <div class="exec-heat-row" role="listitem" aria-label="${escapeHeatAttr(heatRowAriaLabel(label, badge, latestLabel))}">
            <div class="exec-heat-label text-truncate" title="${escapeHeatAttr(label)}">
                ${typeMonoMarkup(badge)}<div class="fw-semibold text-truncate${redactClass}">${labelMarkup}</div>
            </div>
            <div class="exec-heat-cells" role="img" aria-label="${escapeHeatAttr(`${label} heatmap history`)}">${heatmapRowSplit(topValues, bottomValues, colorFn, formatValue, options)}</div>
            <div class="text-muted small text-end exec-latest">${formattedLatest}</div>
        </div>
    `;
}

export function retransmitHeatRow(label, badge, blocks, colorFn, formatValue, link = null) {
    const topValues = blocks?.retransmit_up || [];
    const bottomValues = blocks?.retransmit_down || [];
    const latestTop = latestValue(topValues);
    const latestBottom = latestValue(bottomValues);
    const both = latestTop !== null && latestBottom !== null;

    let max = null;
    let direction = null;
    if (both) {
        if (latestTop >= latestBottom) {
            max = latestTop;
            direction = latestTop > latestBottom ? "upload" : null;
        } else {
            max = latestBottom;
            direction = "download";
        }
    } else if (latestTop !== null) {
        max = latestTop;
        direction = "upload";
    } else if (latestBottom !== null) {
        max = latestBottom;
        direction = "download";
    }

    const arrow = direction === "upload" ? "↑" : direction === "download" ? "↓" : "";
    const formattedLatest = max === null
        ? `<span class="exec-latest-pair">—</span>`
        : `<span class="exec-latest-pair" title="Upload ${formatValue(latestTop)} · Download ${formatValue(latestBottom)}"><span>${formatValue(max)}</span>${arrow ? `<span class="exec-latest-dir">${arrow}</span>` : ""}</span>`;
    const latestLabel = both
        ? `${formatValue(latestTop)} / ${formatValue(latestBottom)}${direction ? ` (higher: ${direction})` : ""}`
        : formatValue(max);
    const redactClass =
        badge === "Site" || badge === "Circuit" ? " redactable" : "";
    const labelMarkup = link ? `<a href="${link}">${label}</a>` : label;
    return `
        <div class="exec-heat-row" role="listitem" aria-label="${escapeHeatAttr(heatRowAriaLabel(label, badge, latestLabel))}">
            <div class="exec-heat-label text-truncate" title="${escapeHeatAttr(label)}">
                ${typeMonoMarkup(badge)}<div class="fw-semibold text-truncate${redactClass}">${labelMarkup}</div>
            </div>
            <div class="exec-heat-cells" role="img" aria-label="${escapeHeatAttr(`${label} retransmit heatmap history`)}">${heatmapRowSplit(topValues, bottomValues, colorFn, formatValue, {
                topLabel: "Upload retransmits",
                bottomLabel: "Download retransmits",
                meaning: "Top is upload, bottom is download. Higher percentages mean more TCP retransmits during this 15-minute block.",
            })}</div>
            <div class="text-muted small text-end exec-latest">${formattedLatest}</div>
        </div>
    `;
}

export function utilizationHeatRow(label, badge, blocks, colorFn, formatValue, link = null) {
    const topValues = blocks?.upload || [];
    const bottomValues = blocks?.download || [];
    const latestTop = latestValue(topValues);
    const latestBottom = latestValue(bottomValues);
    const both = latestTop !== null && latestBottom !== null;

    let max = null;
    let direction = null;
    if (both) {
        if (latestTop >= latestBottom) {
            max = latestTop;
            direction = latestTop > latestBottom ? "upload" : null;
        } else {
            max = latestBottom;
            direction = "download";
        }
    } else if (latestTop !== null) {
        max = latestTop;
        direction = "upload";
    } else if (latestBottom !== null) {
        max = latestBottom;
        direction = "download";
    }

    const arrow = direction === "upload" ? "↑" : direction === "download" ? "↓" : "";
    const formattedLatest = max === null
        ? `<span class="exec-latest-pair">—</span>`
        : `<span class="exec-latest-pair" title="Upload ${formatValue(latestTop)} · Download ${formatValue(latestBottom)}"><span>${formatValue(max)}</span>${arrow ? `<span class="exec-latest-dir">${arrow}</span>` : ""}</span>`;
    const latestLabel = both
        ? `${formatValue(latestTop)} / ${formatValue(latestBottom)}${direction ? ` (higher: ${direction})` : ""}`
        : formatValue(max);
    const redactClass =
        badge === "Site" || badge === "Circuit" ? " redactable" : "";
    const labelMarkup = link ? `<a href="${link}">${label}</a>` : label;
    return `
        <div class="exec-heat-row" role="listitem" aria-label="${escapeHeatAttr(heatRowAriaLabel(label, badge, latestLabel))}">
            <div class="exec-heat-label text-truncate" title="${escapeHeatAttr(label)}">
                ${typeMonoMarkup(badge)}<div class="fw-semibold text-truncate${redactClass}">${labelMarkup}</div>
            </div>
            <div class="exec-heat-cells" role="img" aria-label="${escapeHeatAttr(`${label} utilization heatmap history`)}">${heatmapRowSplit(topValues, bottomValues, colorFn, formatValue, {
                topLabel: "Upload utilization",
                bottomLabel: "Download utilization",
                meaning: "Top is upload, bottom is download. This shows how much of planned or available capacity was in use during this 15-minute block.",
            })}</div>
            <div class="text-muted small text-end exec-latest">${formattedLatest}</div>
        </div>
    `;
}
