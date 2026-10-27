// `lqos-74t.53`: the wire-order convention for CAKE diffserv4 tins.
//
// The wire `cake_core.tins` array is the KERNEL INTERNAL order —
// `[0]=Best Effort, [1]=Bulk, [2]=Video, [3]=Voice`
// (`cake_core::diffserv::DIFFSERV4_TIN_INDEX`, the Linux `sch_cake.c`
// `diffserv4[]` table; `bridge_rs` `shard_tin.rs`/`cut_harness.rs` document the
// same). `tc -s cake` prints the STATS-COLUMN order
// `[Bulk, Best Effort, Video, Voice]`; the bridge produces that only for the
// leafstats baseline via the `cake_diffserv4_tin` 0<->1 swap — NEVER on the wire.
//
// The UI labels the rows `[Bulk, Best Effort, RT Video, Voice]`, so a label index
// maps to the wire index below (only Bulk/Best Effort swap; Video/Voice unchanged).

const CAKE_TIN_WIRE_INDEX = [1, 0, 2, 3];

/** Map a UI tin-label index to the wire (kernel-internal) tin index. */
export function cakeTinIndexFor(labelIndex) {
    return CAKE_TIN_WIRE_INDEX[labelIndex] ?? labelIndex;
}

/** The four diffserv4 tin legend pairs (down series name -> up series name). */
export const CAKE_TIN_LEGEND_PAIRS = [
    { down: "Bulk", up: "Bulk Up" },
    { down: "Best Effort", up: "Best Effort Up" },
    { down: "RT Video", up: "RT Video Up" },
    { down: "Voice", up: "RT Voice Up" },
];
