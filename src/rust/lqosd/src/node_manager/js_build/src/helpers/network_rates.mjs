import {toNumber} from "../lq_js_common/helpers/scaling.js";

export const RATE_COMPARE_EPSILON_MBPS = 0.01;

export function ratePairFromNode(node, field, fallback = [0, 0]) {
    const pair = node?.[field];
    if (Array.isArray(pair)) {
        return [toNumber(pair[0], fallback[0]), toNumber(pair[1], fallback[1])];
    }
    return fallback;
}

export function configuredMax(node) {
    const configured = node?.configured_max_throughput;
    if (Array.isArray(configured)) {
        return [toNumber(configured[0], 0), toNumber(configured[1], 0)];
    }
    return ratePairFromNode(node, "max_throughput");
}

export function effectiveMax(node) {
    return ratePairFromNode(node, "effective_max_throughput", configuredMax(node));
}

export function ratesApproximatelyEqual(left, right) {
    return Math.abs(toNumber(left, 0) - toNumber(right, 0)) <= RATE_COMPARE_EPSILON_MBPS;
}

export function rateBelow(left, right) {
    return toNumber(left, 0) < toNumber(right, 0) - RATE_COMPARE_EPSILON_MBPS;
}

export function ratePairsMatch(left, right) {
    return ratesApproximatelyEqual(left?.[0], right?.[0])
        && ratesApproximatelyEqual(left?.[1], right?.[1]);
}
