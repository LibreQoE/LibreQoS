import {toNumber} from "./lq_js_common/helpers/scaling.js";
import {clearElement} from "./helpers/dom_clear.mjs";
import {
    configuredMax,
    effectiveMax,
    rateBelow,
    ratePairsMatch,
    ratesApproximatelyEqual,
} from "./helpers/network_rates.mjs";

export {configuredMax, effectiveMax, ratePairsMatch, ratesApproximatelyEqual};

const COMPACT_REASON_LABELS = {
    attachment: "Est. Attachment",
    base: "Base rate",
    mixed: "Est. Mixed",
    override: "Est. Override",
    parent: "Est. Parent",
    queue: "Est. Queue",
};

export function immediateParentNodeFromTree(tree, node) {
    const parentIndex = node?.immediate_parent;
    if (parentIndex === null || parentIndex === undefined || !tree?.[parentIndex]) {
        return null;
    }
    return tree[parentIndex][1] || null;
}

function formatMbps(value) {
    const numeric = toNumber(value, 0);
    if (numeric === 0) {
        return "Unlimited";
    }
    return `${numeric.toFixed(1).replace(/\.0$/, "")}M`;
}

function overrideValue(rateOverrideData, direction) {
    if (!rateOverrideData?.has_override) {
        return null;
    }
    const field = direction === 0
        ? "override_download_bandwidth_mbps"
        : "override_upload_bandwidth_mbps";
    const value = rateOverrideData[field];
    if (value === null || value === undefined) {
        return null;
    }
    const numeric = Number(value);
    return Number.isFinite(numeric) ? numeric : null;
}

function topologyOverrideParentIsActive(topologyOverrideData) {
    if (!topologyOverrideData?.has_override) {
        return false;
    }
    const desired = Array.isArray(topologyOverrideData.override_parent_node_ids)
        ? topologyOverrideData.override_parent_node_ids[0]
        : null;
    return !!desired && desired === topologyOverrideData.current_parent_node_id;
}

function parentDisplayName(parentNode, topologyOverrideData) {
    return topologyOverrideData?.current_parent_node_name
        || parentNode?.name
        || "upstream parent";
}

function reasonForDirection({node, parentNode, rateOverrideData, topologyOverrideData, direction}) {
    const configured = configuredMax(node);
    const effective = effectiveMax(node);
    const configuredValue = configured[direction];
    const effectiveValue = effective[direction];
    const override = overrideValue(rateOverrideData, direction);
    const parentEffective = parentNode ? effectiveMax(parentNode)[direction] : null;
    const directionName = direction === 0 ? "Download" : "Upload";

    if (
        parentEffective !== null
        && ratesApproximatelyEqual(effectiveValue, parentEffective)
        && (rateBelow(effectiveValue, configuredValue) || (override !== null && rateBelow(effectiveValue, override)))
    ) {
        const parentName = parentDisplayName(parentNode, topologyOverrideData);
        const suffix = topologyOverrideParentIsActive(topologyOverrideData)
            ? " selected by topology override"
            : "";
        const attachment = node?.active_attachment_name
            ? ` Active attachment: ${node.active_attachment_name}.`
            : "";
        return {
            kind: "parent",
            label: "Parent",
            detail: `${directionName}: inherited from parent ${parentName}${suffix} at ${formatMbps(effectiveValue)}.${attachment}`,
        };
    }

    if (override !== null && ratesApproximatelyEqual(effectiveValue, override)) {
        return {
            kind: "override",
            label: "Override",
            detail: `${directionName}: matches operator rate override at ${formatMbps(effectiveValue)}.`,
        };
    }

    if (node?.active_attachment_name && rateBelow(effectiveValue, configuredValue)) {
        return {
            kind: "attachment",
            label: "Attachment",
            detail: `${directionName}: capped by active attachment ${node.active_attachment_name} at ${formatMbps(effectiveValue)}.`,
        };
    }

    if (rateBelow(effectiveValue, configuredValue)) {
        return {
            kind: "queue",
            label: "Queue",
            detail: `${directionName}: effective queue rate is below the configured rate at ${formatMbps(effectiveValue)}.`,
        };
    }

    return {
        kind: "base",
        label: "Base",
        detail: `${directionName}: uses configured rate at ${formatMbps(effectiveValue)}.`,
    };
}

export function buildNodeLimitSummary({
    node,
    parentNode = null,
    rateOverrideData = null,
    topologyOverrideData = null,
} = {}) {
    const directions = [0, 1].map((direction) => reasonForDirection({
        node,
        parentNode,
        rateOverrideData,
        topologyOverrideData,
        direction,
    }));
    const sameKind = directions[0].kind === directions[1].kind;
    const label = sameKind ? directions[0].label : "Mixed";
    return {
        label,
        kind: sameKind ? directions[0].kind : "mixed",
        title: directions.map((direction) => direction.detail).join(" "),
        compactReason: COMPACT_REASON_LABELS[sameKind ? directions[0].kind : "mixed"],
        directions,
    };
}

export function limitSourceBadgeClass(kind) {
    switch (kind) {
        case "parent":
            return "text-bg-warning";
        case "override":
            return "text-bg-info";
        case "attachment":
            return "text-bg-primary";
        case "mixed":
            return "text-bg-secondary";
        case "queue":
            return "text-bg-danger";
        default:
            return "text-bg-dark";
    }
}

export function renderEffectiveNowDisplay({target, effective, summary, formatRatePair}) {
    if (!target) {
        return;
    }
    clearElement(target);
    const doc = target.ownerDocument || document;
    const wrap = doc.createElement("span");
    wrap.classList.add("lqos-tree-detail-value", "lqos-tree-effective-now");

    const value = doc.createElement("span");
    value.textContent = formatRatePair(effective[0], effective[1]);
    wrap.appendChild(value);

    const badge = doc.createElement("span");
    badge.classList.add("badge", "rounded-pill", "lqos-tree-limit-source", limitSourceBadgeClass(summary.kind));
    badge.textContent = summary.compactReason || summary.label;
    badge.setAttribute("data-bs-toggle", "tooltip");
    badge.setAttribute("data-bs-placement", "top");
    badge.setAttribute("data-bs-trigger", "hover focus");
    badge.setAttribute("data-bs-container", "body");
    badge.setAttribute("tabindex", "0");
    badge.setAttribute("title", `Estimated effective-rate source. ${summary.title}`);
    badge.setAttribute("aria-label", `Estimated effective-rate source. ${summary.title}`);
    wrap.appendChild(badge);

    target.appendChild(wrap);
}
