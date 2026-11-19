import {toNumber} from "./lq_js_common/helpers/scaling.js";
import {clearElement} from "./helpers/dom_clear.mjs";
import {formatNumber} from "./i18n/format.js";
import {t} from "./i18n/index.js";
import {
    configuredMax,
    effectiveMax,
    rateBelow,
    ratePairsMatch,
    ratesApproximatelyEqual,
} from "./helpers/network_rates.mjs";

export {configuredMax, effectiveMax, ratePairsMatch, ratesApproximatelyEqual};

// The rate-source vocabulary: one entry per kind, holding the label and the
// compact table word. A `kind` is a code the summary, the page and the tests
// compare; only the words it renders come from the catalog, so a comparison can
// never depend on the active locale. Every branch holds its literal `t()` call:
// `extract.mjs` reads a key only where a call site names one, so a table of bare
// keys would read as dead text and fail the gate.
function kindWords(kind) {
    switch (kind) {
    case "attachment":
        return {label: t("tree.limit.kind.attachment"), compact: t("tree.limit.compact.attachment")};
    case "override":
        return {label: t("tree.limit.kind.override"), compact: t("tree.limit.compact.override")};
    case "parent":
        return {label: t("tree.limit.kind.parent"), compact: t("tree.limit.compact.parent")};
    case "queue":
        return {label: t("tree.limit.kind.queue"), compact: t("tree.limit.compact.queue")};
    case "mixed":
        return {label: t("tree.limit.kind.mixed"), compact: t("tree.limit.compact.mixed")};
    default:
        return {label: t("tree.limit.kind.base"), compact: t("tree.limit.compact.base")};
    }
}

function directionLabel(direction) {
    return direction === 0 ? t("common.table.download") : t("common.table.upload");
}

function renderKey(target) {
    if (target?.getAttribute) {
        return target.getAttribute("data-lqos-render-key");
    }
    return target?.attributes?.["data-lqos-render-key"] ?? null;
}

function setRenderKey(target, key) {
    target.setAttribute("data-lqos-render-key", key);
}

function clearRenderedTarget(target) {
    clearElement(target);
    target.textContent = "";
}

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
        return t("tree.limit.unlimited");
    }
    // The page's rate shorthand ("294.5M"): the decimal mark follows the active
    // locale, the grouping separator does not (the value is a single rate).
    return `${formatNumber(numeric, {maximumFractionDigits: 1, useGrouping: false})}M`;
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
        || t("tree.limit.parent_unknown");
}

function reasonForDirection({node, parentNode, rateOverrideData, topologyOverrideData, direction}) {
    const configured = configuredMax(node);
    const effective = effectiveMax(node);
    const configuredValue = configured[direction];
    const effectiveValue = effective[direction];
    const override = overrideValue(rateOverrideData, direction);
    const parentEffective = parentNode ? effectiveMax(parentNode)[direction] : null;
    const directionName = directionLabel(direction);

    if (
        parentEffective !== null
        && ratesApproximatelyEqual(effectiveValue, parentEffective)
        && (rateBelow(effectiveValue, configuredValue) || (override !== null && rateBelow(effectiveValue, override)))
    ) {
        const parentName = parentDisplayName(parentNode, topologyOverrideData);
        const rate = formatMbps(effectiveValue);
        if (topologyOverrideParentIsActive(topologyOverrideData)) {
            return {
                kind: "parent",
                label: kindWords("parent").label,
                detail: node?.active_attachment_name
                    ? t("tree.limit.detail.parent_override_attachment", {
                        direction: directionName,
                        parent: parentName,
                        rate,
                        attachment: node.active_attachment_name,
                    })
                    : t("tree.limit.detail.parent_override", {
                        direction: directionName,
                        parent: parentName,
                        rate,
                    }),
            };
        }
        return {
            kind: "parent",
            label: kindWords("parent").label,
            detail: node?.active_attachment_name
                ? t("tree.limit.detail.parent_attachment", {
                    direction: directionName,
                    parent: parentName,
                    rate,
                    attachment: node.active_attachment_name,
                })
                : t("tree.limit.detail.parent", {
                    direction: directionName,
                    parent: parentName,
                    rate,
                }),
        };
    }

    if (override !== null && ratesApproximatelyEqual(effectiveValue, override)) {
        return {
            kind: "override",
            label: kindWords("override").label,
            detail: t("tree.limit.detail.override", {
                direction: directionName,
                rate: formatMbps(effectiveValue),
            }),
        };
    }

    if (node?.active_attachment_name && rateBelow(effectiveValue, configuredValue)) {
        return {
            kind: "attachment",
            label: kindWords("attachment").label,
            detail: t("tree.limit.detail.attachment", {
                direction: directionName,
                attachment: node.active_attachment_name,
                rate: formatMbps(effectiveValue),
            }),
        };
    }

    if (rateBelow(effectiveValue, configuredValue)) {
        return {
            kind: "queue",
            label: kindWords("queue").label,
            detail: t("tree.limit.detail.queue", {
                direction: directionName,
                rate: formatMbps(effectiveValue),
            }),
        };
    }

    return {
        kind: "base",
        label: kindWords("base").label,
        detail: t("tree.limit.detail.base", {
            direction: directionName,
            rate: formatMbps(effectiveValue),
        }),
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
    const label = sameKind ? directions[0].label : kindWords("mixed").label;
    return {
        label,
        kind: sameKind ? directions[0].kind : "mixed",
        title: directions.map((direction) => direction.detail).join(" "),
        compactReason: kindWords(sameKind ? directions[0].kind : "mixed").compact,
        directions,
    };
}

export function renderEffectiveNowDisplay({target, effective, formatRatePair}) {
    if (!target) {
        return;
    }
    const display = formatRatePair(effective[0], effective[1]);
    // The render-cache key: internal, never displayed, so it is built from parts
    // rather than spelled as text a reader might mistake for a label.
    const key = ["effective", display].join(":");
    if (renderKey(target) === key) {
        return;
    }
    clearRenderedTarget(target);
    setRenderKey(target, key);
    const doc = target.ownerDocument || document;
    const wrap = doc.createElement("span");
    wrap.classList.add("lqos-tree-detail-value", "lqos-tree-effective-now");
    wrap.textContent = display;
    target.appendChild(wrap);
}

export function renderLimitedByDisplay({target, summary}) {
    if (!target) {
        return;
    }
    const display = summary.compactReason || summary.label;
    const title = t("tree.limit.source_tooltip", {details: summary.title});
    // The render-cache key: internal, never displayed.
    const key = ["limited", display, title].join(":");
    if (renderKey(target) === key) {
        return;
    }
    clearRenderedTarget(target);
    setRenderKey(target, key);
    const doc = target.ownerDocument || document;
    const source = doc.createElement("span");
    source.classList.add("lqos-tree-limit-source");
    source.setAttribute("data-bs-toggle", "tooltip");
    source.setAttribute("data-bs-placement", "top");
    // translatable: false — "hover focus" is a Bootstrap tooltip trigger token, not a label
    source.setAttribute("data-bs-trigger", "hover focus");
    source.setAttribute("data-bs-container", "body");
    source.setAttribute("tabindex", "0");
    source.setAttribute("title", title);
    source.setAttribute("aria-label", title);

    const text = doc.createElement("span");
    text.textContent = display;
    source.appendChild(text);

    const icon = doc.createElement("i");
    icon.classList.add("fa", "fa-circle-info");
    icon.setAttribute("aria-hidden", "true");
    source.appendChild(icon);

    target.appendChild(source);
}
