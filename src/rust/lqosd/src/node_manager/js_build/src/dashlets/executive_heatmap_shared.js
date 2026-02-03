import {lerpGreenToRedViaOrange} from "../helpers/scaling";

export const MAX_HEATMAP_ROWS = 10;

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

export const colorByCapacity = (pct) => lerpGreenToRedViaOrange(100 - clampPercent(pct), 100);

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

export function heatmapRow(values, colorFn, formatValue) {
    const length = Array.isArray(values) && values.length ? values.length : 15;
    let cells = "";
    for (let i = 0; i < length; i++) {
        const val = values && values[i] !== undefined ? values[i] : null;
        if (val === null || val === undefined) {
            cells += `<div class="exec-heat-cell empty" title="No data"></div>`;
            continue;
        }
        const numeric = Number(val) || 0;
        const color = colorFn(numeric);
        const title = formatValue(numeric);
        cells += `<div class="exec-heat-cell" style="background:${color}" title="Block ${i + 1}: ${title}"></div>`;
    }
    return cells;
}

export function heatRow(label, badge, values, colorFn, formatValue, link = null) {
    const latest = latestValue(values);
    const formattedLatest = formatValue(latest);
    const redactClass =
        badge === "Site" || badge === "Circuit" ? " redactable" : "";
    const labelMarkup = link ? `<a href="${link}">${label}</a>` : label;
    return `
        <div class="exec-heat-row">
            <div class="exec-heat-label text-truncate" title="${label}">
                <div class="fw-semibold text-truncate${redactClass}">${labelMarkup}</div>
                ${badge ? `<span class="badge bg-light text-secondary border">${badge}</span>` : ""}
            </div>
            <div class="exec-heat-cells">${heatmapRow(values, colorFn, formatValue)}</div>
            <div class="text-muted small text-end exec-latest">${formattedLatest}</div>
        </div>
    `;
}

function heatmapRowQuadrants(quads, colorFn, formatValue) {
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
            cells += `<div class="exec-heat-cell empty" title="No data"></div>`;
            continue;
        }

        const fmt = (v) => formatValue(v);
        const title = [
            `Block ${i + 1}`,
            `UL p50: ${fmt(ulP50)}`,
            `UL p90: ${fmt(ulP90)}`,
            `DL p50: ${fmt(dlP50)}`,
            `DL p90: ${fmt(dlP90)}`,
        ].join(" • ");

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
            <div class="exec-heat-cell quad" title="${title}">
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

export function rttHeatRow(label, badge, blocks, colorFn, formatValue, link = null) {
    const quads = {
        ul_p50: blocks?.rtt_p50_up || [],
        ul_p90: blocks?.rtt_p90_up || [],
        dl_p50: blocks?.rtt_p50_down || [],
        dl_p90: blocks?.rtt_p90_down || [],
    };
    const latest = latestValue(blocks?.rtt || []);
    const formattedLatest = formatValue(latest);
    const redactClass =
        badge === "Site" || badge === "Circuit" ? " redactable" : "";
    const labelMarkup = link ? `<a href="${link}">${label}</a>` : label;
    return `
        <div class="exec-heat-row">
            <div class="exec-heat-label text-truncate" title="${label}">
                <div class="fw-semibold text-truncate${redactClass}">${labelMarkup}</div>
                ${badge ? `<span class="badge bg-light text-secondary border">${badge}</span>` : ""}
            </div>
            <div class="exec-heat-cells">${heatmapRowQuadrants(quads, colorFn, formatValue)}</div>
            <div class="text-muted small text-end exec-latest">${formattedLatest}</div>
        </div>
    `;
}

function heatmapRowSplit(topValues, bottomValues, colorFn, formatValue) {
    const length = Array.isArray(topValues) && topValues.length
        ? topValues.length
        : (Array.isArray(bottomValues) && bottomValues.length ? bottomValues.length : 15);
    let cells = "";
    for (let i = 0; i < length; i++) {
        const top = topValues?.[i];
        const bottom = bottomValues?.[i];

        const allMissing =
            (top === null || top === undefined) &&
            (bottom === null || bottom === undefined);
        if (allMissing) {
            cells += `<div class="exec-heat-cell empty" title="No data"></div>`;
            continue;
        }

        const numOrNull = (v) => (v === null || v === undefined ? null : Number(v));
        const fmt = (v) => formatValue(numOrNull(v));
        const title = [
            `Block ${i + 1}`,
            `UL: ${fmt(top)}`,
            `DL: ${fmt(bottom)}`,
        ].join(" • ");

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
            <div class="exec-heat-cell split" title="${title}">
                <div class="exec-split-grid">
                    ${part(top)}
                    ${part(bottom)}
                </div>
            </div>
        `;
    }
    return cells;
}

function splitHeatRow(label, badge, topValues, bottomValues, colorFn, formatValue, link = null) {
    const latestTop = latestValue(topValues);
    const latestBottom = latestValue(bottomValues);
    const formattedLatest = `
        <div>${formatValue(latestTop)}</div>
        <div>${formatValue(latestBottom)}</div>
    `;
    const redactClass =
        badge === "Site" || badge === "Circuit" ? " redactable" : "";
    const labelMarkup = link ? `<a href="${link}">${label}</a>` : label;
    return `
        <div class="exec-heat-row">
            <div class="exec-heat-label text-truncate" title="${label}">
                <div class="fw-semibold text-truncate${redactClass}">${labelMarkup}</div>
                ${badge ? `<span class="badge bg-light text-secondary border">${badge}</span>` : ""}
            </div>
            <div class="exec-heat-cells">${heatmapRowSplit(topValues, bottomValues, colorFn, formatValue)}</div>
            <div class="text-muted small text-end exec-latest split">${formattedLatest}</div>
        </div>
    `;
}

export function retransmitHeatRow(label, badge, blocks, colorFn, formatValue, link = null) {
    const topValues = blocks?.retransmit_up || [];
    const bottomValues = blocks?.retransmit_down || [];
    return splitHeatRow(label, badge, topValues, bottomValues, colorFn, formatValue, link);
}

export function utilizationHeatRow(label, badge, blocks, colorFn, formatValue, link = null) {
    const topValues = blocks?.upload || [];
    const bottomValues = blocks?.download || [];
    return splitHeatRow(label, badge, topValues, bottomValues, colorFn, formatValue, link);
}
