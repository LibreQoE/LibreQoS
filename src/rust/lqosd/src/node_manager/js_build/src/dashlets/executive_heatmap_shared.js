import {lerpGreenToRedViaOrange} from "../helpers/scaling";

export const MAX_HEATMAP_ROWS = 20;

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
    sites.forEach(site => rows.push({
        label: site.site_name || "Site",
        badge: "Site",
        blocks: site.blocks,
    }));
    const circuits = (data?.circuits || []);
    circuits.forEach(circuit => {
        const name = circuit.circuit_name || circuit.circuit_id || `Circuit ${circuit.circuit_hash}`;
        rows.push({
            label: name,
            badge: "Circuit",
            blocks: circuit.blocks,
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

export function heatRow(label, badge, values, colorFn, formatValue) {
    const latest = latestValue(values);
    const formattedLatest = formatValue(latest);
    return `
        <div class="exec-heat-row">
            <div class="exec-heat-label text-truncate" title="${label}">
                <div class="fw-semibold text-truncate">${label}</div>
                ${badge ? `<span class="badge bg-light text-secondary border">${badge}</span>` : ""}
            </div>
            <div class="exec-heat-cells">${heatmapRow(values, colorFn, formatValue)}</div>
            <div class="text-muted small text-end exec-latest">${formattedLatest}</div>
        </div>
    `;
}
