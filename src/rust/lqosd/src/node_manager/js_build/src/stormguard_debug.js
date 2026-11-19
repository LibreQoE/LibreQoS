// StormGuard Debug: two websocket feeds (enforced limits and the inputs
// StormGuard evaluates) printed as tables.
//
// The rendering lives in a bundle rather than an inline `<script>` because the
// status badge, the empty states and the "Updated" line have to follow the
// active locale; an inline IIFE cannot reach the catalog. Values arrive from
// the server, so the site name, state and action tokens are escaped before they
// reach `innerHTML`.

import {formatNumber, formatTime} from "./i18n/format.js";
import {escapeHtml} from "./i18n/escape.mjs";
import {t} from "./i18n/index.js";

const statusBadge = document.getElementById("sg-status");
const updated = document.getElementById("sg-updated");
const table = document.getElementById("sg-table");
const debugTable = document.getElementById("sg-debug");

const EMPTY = "—";
const NBSP = "\u00a0";

function wsProto() {
    return window.location.protocol.startsWith("https") ? "wss://" : "ws://";
}

function setStatus(label, cls) {
    statusBadge.textContent = label;
    statusBadge.className = "badge " + cls;
}

function formatMbps(v) {
    if (v === null || v === undefined) return EMPTY;
    return formatNumber(v, {maximumFractionDigits: 2});
}

function formatPct(v) {
    if (v === null || v === undefined) return EMPTY;
    return formatNumber(v * 100, {maximumFractionDigits: 2}) + NBSP + "%";
}

function formatRtt(v) {
    if (v === null || v === undefined) return EMPTY;
    return formatNumber(v, {minimumFractionDigits: 2, maximumFractionDigits: 2});
}

function formatSeconds(v, digits) {
    if (v === null || v === undefined) return EMPTY;
    return formatNumber(v, {maximumFractionDigits: digits}) + NBSP + "s";
}

function renderStatusRows(rows) {
    if (!Array.isArray(rows) || rows.length === 0) {
        table.innerHTML = `<tr><td colspan="3" class="text-muted">${t("stormguard_debug.empty_status")}</td></tr>`;
        return;
    }

    table.innerHTML = rows
        .map((row) => {
            const site = escapeHtml(row[0] ?? t("stormguard_debug.unknown_site"));
            const down = row[1] ?? 0;
            const up = row[2] ?? 0;
            return `<tr><td class="fw-semibold">${site}</td><td>${formatMbps(down)}</td><td>${formatMbps(up)}</td></tr>`;
        })
        .join("");
}

function renderDebugRows(data) {
    if (!Array.isArray(data) || data.length === 0) {
        debugTable.innerHTML = `<tr><td colspan="11" class="text-muted">${t("stormguard_debug.empty_debug")}</td></tr>`;
        return;
    }

    const rows = [];
    data.forEach((entry) => {
        ["download", "upload"].forEach((dir) => {
            const d = entry[dir] || {};
            const site = escapeHtml(entry.site || "?");
            const direction = dir === "download"
                ? `<i class='fa fa-arrow-down text-primary'></i>${t("stormguard_debug.dir.down")}`
                : `<i class='fa fa-arrow-up text-success'></i>${t("stormguard_debug.dir.up")}`;
            const action = d.last_action
                ? `<span class="badge bg-primary-subtle text-primary">${escapeHtml(d.last_action)}</span>`
                : EMPTY;
            const state = escapeHtml(d.state || "?");
            const saturation = escapeHtml(d.saturation_current || "?") + " / " + escapeHtml(d.saturation_max || "?");
            const passiveRtt = "tcp " + formatRtt(d.passive_rtt_ms) + " &nbsp; ping " + formatRtt(d.active_ping_rtt_ms);
            const baselineRtt = t("stormguard_debug.baseline") + " " + formatRtt(d.baseline_rtt_ms)
                + " &nbsp; Δ " + formatRtt(d.delay_ms) + " (" + escapeHtml(d.strategy || "?") + ")";
            const canIncrease = d.can_increase ? "bg-success-subtle text-success" : "bg-light text-muted";
            const canDecrease = d.can_decrease ? "bg-danger-subtle text-danger" : "bg-light text-muted";
            rows.push(
                `<tr><td class="fw-semibold">${site}</td>` +
                `<td>${direction}</td>` +
                `<td>${state}</td>` +
                `<td>${action}<div class="text-muted small">${formatSeconds(d.last_action_age_secs, 0)}</div></td>` +
                `<td>${formatMbps(d.queue_mbps)} (${formatMbps(d.min_mbps)} / ${formatMbps(d.max_mbps)})</td>` +
                `<td>${saturation}</td>` +
                `<td>${formatMbps(d.throughput_mbps)} / ${formatMbps(d.throughput_ma_mbps)}</td>` +
                `<td>${formatPct(d.retrans)} / ${formatPct(d.retrans_ma)}</td>` +
                `<td>${formatRtt(d.rtt)} / ${formatRtt(d.rtt_ma)}<div class="text-muted small">${passiveRtt}</div>` +
                `<div class="text-muted small">${baselineRtt}</div></td>` +
                `<td>${formatSeconds(d.cooldown_remaining_secs, 1)}</td>` +
                `<td><span class="badge ${canIncrease}">+</span><span class="badge ${canDecrease}">−</span></td></tr>`
            );
        });
    });

    debugTable.innerHTML = rows.join("");
}

function markUpdated() {
    updated.textContent = t("common.state.updated", {time: formatTime(new Date())});
}

function connect() {
    setStatus(t("stormguard_debug.status.connecting"), "bg-secondary");
    const ws = new WebSocket(wsProto() + window.location.host + "/websocket/ws");

    ws.onopen = () => {
        setStatus(t("stormguard_debug.status.live"), "bg-success");
        ws.send('{ "channel" : "StormguardStatus"}');
        ws.send('{ "channel" : "StormguardDebug"}');
    };

    ws.onclose = () => {
        setStatus(t("stormguard_debug.status.disconnected"), "bg-warning");
        setTimeout(connect, 3000);
    };

    ws.onerror = () => {
        setStatus(t("stormguard_debug.status.error"), "bg-danger");
        ws.close();
    };

    ws.onmessage = (event) => {
        try {
            const msg = JSON.parse(event.data);
            if (msg && msg.event === "StormguardStatus") {
                renderStatusRows(msg.data || []);
                markUpdated();
            }
            if (msg && msg.event === "StormguardDebug") {
                renderDebugRows(msg.data || []);
                markUpdated();
            }
        } catch (err) {
            // translatable: false — "Failed to parse StormGuard message" is a console diagnostic, never rendered
            console.error("Failed to parse StormGuard message", err);
        }
    };
}

connect();
