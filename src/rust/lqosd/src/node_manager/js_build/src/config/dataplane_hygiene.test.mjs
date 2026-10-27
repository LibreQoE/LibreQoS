import {test} from "node:test";
import assert from "node:assert/strict";
import {
    FALLBACK_BANDWIDTH_MBPS,
    applyBandwidthMbps,
    readBandwidthMbps,
} from "./dataplane_hygiene.mjs";

test("readBandwidthMbps prefers [dataplane] over the legacy [queues] keys", () => {
    const config = {
        dataplane: {download_mbps: 900, upload_mbps: 450},
        queues: {downlink_bandwidth_mbps: 8000, uplink_bandwidth_mbps: 5000},
    };
    assert.deepEqual(readBandwidthMbps(config), {download: 900, upload: 450});
});

test("readBandwidthMbps falls back to the legacy [queues] keys", () => {
    const config = {queues: {downlink_bandwidth_mbps: 8000, uplink_bandwidth_mbps: 5000}};
    assert.deepEqual(readBandwidthMbps(config), {download: 8000, upload: 5000});
});

test("readBandwidthMbps falls back to the compiled default", () => {
    assert.deepEqual(readBandwidthMbps({}), {
        download: FALLBACK_BANDWIDTH_MBPS,
        upload: FALLBACK_BANDWIDTH_MBPS,
    });
    assert.deepEqual(readBandwidthMbps(null), {
        download: FALLBACK_BANDWIDTH_MBPS,
        upload: FALLBACK_BANDWIDTH_MBPS,
    });
});

test("applyBandwidthMbps writes [dataplane] AND mirrors into [queues], preserving other queues keys", () => {
    const config = {queues: {queue_mode: "shape", default_sqm: "cake diffserv4"}};
    applyBandwidthMbps(config, 2000, 1000);
    assert.deepEqual(config.dataplane, {download_mbps: 2000, upload_mbps: 1000});
    assert.equal(config.queues.downlink_bandwidth_mbps, 2000);
    assert.equal(config.queues.uplink_bandwidth_mbps, 1000);
    // Unrelated [queues] keys are untouched.
    assert.equal(config.queues.queue_mode, "shape");
    assert.equal(config.queues.default_sqm, "cake diffserv4");
});

test("applyBandwidthMbps round-trips through readBandwidthMbps", () => {
    const config = {};
    applyBandwidthMbps(config, 750, 250);
    assert.deepEqual(readBandwidthMbps(config), {download: 750, upload: 250});
});

test("a decimal root rate survives the read + the mirror losslessly", () => {
    const config = {};
    applyBandwidthMbps(config, 12.5, 0.5);
    assert.deepEqual(config.dataplane, {download_mbps: 12.5, upload_mbps: 0.5});
    assert.equal(config.queues.downlink_bandwidth_mbps, 12.5);
    assert.equal(config.queues.uplink_bandwidth_mbps, 0.5);
    assert.deepEqual(readBandwidthMbps(config), {download: 12.5, upload: 0.5});
});
