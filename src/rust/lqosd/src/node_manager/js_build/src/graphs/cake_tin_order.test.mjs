import assert from "node:assert/strict";
import {test} from "node:test";
import {CAKE_TIN_LEGEND_PAIRS, cakeTinIndexFor} from "./cake_tin_order.mjs";

// `lqos-74t.53` (B): the wire `cake_core.tins` is the kernel INTERNAL order
// [0]=Best Effort, [1]=Bulk, [2]=Video, [3]=Voice; the UI labels are
// [Bulk, Best Effort, RT Video, Voice] => a 0<->1 swap only.
test("cakeTinIndexFor maps UI labels to the wire kernel-internal tin order", () => {
    assert.deepEqual([0, 1, 2, 3].map(cakeTinIndexFor), [1, 0, 2, 3]);
    assert.equal(cakeTinIndexFor(2), 2, "Video is unchanged");
    assert.equal(cakeTinIndexFor(3), 3, "Voice is unchanged");
    assert.equal(cakeTinIndexFor(9), 9, "out-of-range index is identity");
});

test("CAKE_TIN_LEGEND_PAIRS covers the four diffserv4 tins (down -> up)", () => {
    assert.deepEqual(
        CAKE_TIN_LEGEND_PAIRS.map((pair) => pair.down),
        ["Bulk", "Best Effort", "RT Video", "Voice"],
    );
    // The Voice up series is "RT Voice Up" (not "Voice Up"), so the mirror needs
    // an explicit pair map — a suffix rule would be wrong.
    assert.equal(CAKE_TIN_LEGEND_PAIRS[3].up, "RT Voice Up");
    assert.ok(CAKE_TIN_LEGEND_PAIRS.every((pair) => pair.up.length > 0));
});
