// Pure helpers for the v3 `[dataplane]` bandwidth editor (dependency-free so
// the node manager test suite can pin the behaviour without a DOM).
//
// The v3 single source for a box's root shaper bandwidth is the `[dataplane]`
// section (`download_mbps` / `upload_mbps`; card
// `docs/lqos_conf_single_source_card.md` §3). During the transition the legacy
// `[queues] downlink/uplink_bandwidth_mbps` still resolve, so this module
// READS `[dataplane]` first (falling back to `[queues]`) and, on save, MIRRORS
// the value into both so no still-legacy reader diverges.

/** The compiled default when neither section names a value (Mbps). */
export const FALLBACK_BANDWIDTH_MBPS = 1000;

/**
 * Reads the box root bandwidth in Mbps.
 *
 * @param {unknown} config The whole node config (or any object).
 * @returns {{download: number, upload: number}} `[dataplane]` first, then the
 *   legacy `[queues]`, then {@link FALLBACK_BANDWIDTH_MBPS}.
 */
export function readBandwidthMbps(config) {
    const dataplane = (config && typeof config.dataplane === "object" && config.dataplane) || {};
    const queues = (config && typeof config.queues === "object" && config.queues) || {};
    return {
        download: dataplane.download_mbps ?? queues.downlink_bandwidth_mbps ?? FALLBACK_BANDWIDTH_MBPS,
        upload: dataplane.upload_mbps ?? queues.uplink_bandwidth_mbps ?? FALLBACK_BANDWIDTH_MBPS,
    };
}

/**
 * Writes an edited bandwidth: the v3 `[dataplane]` single source AND a mirror
 * into the legacy `[queues]` keys (preserving every other `[queues]` field).
 * Mutates and returns `config`.
 *
 * @param {object} config The whole node config.
 * @param {number} download Download Mbps.
 * @param {number} upload Upload Mbps.
 * @returns {object} `config`.
 */
export function applyBandwidthMbps(config, download, upload) {
    if (!config || typeof config !== "object") {
        return config;
    }
    config.dataplane = {download_mbps: download, upload_mbps: upload};
    const queues = (config.queues && typeof config.queues === "object") ? config.queues : {};
    queues.downlink_bandwidth_mbps = download;
    queues.uplink_bandwidth_mbps = upload;
    config.queues = queues;
    return config;
}
