import assert from "node:assert/strict";
import test from "node:test";

import {dpdkKindLabel, normalizeDpdkStatus, normalizeWorkerCount} from "./dpdk_status.mjs";

test("labels known kinds and rejects unknown ones", () => {
    assert.equal(dpdkKindLabel("running"), "Running");
    assert.equal(dpdkKindLabel("effective"), "Effective");
    assert.equal(dpdkKindLabel("configured"), "Configured");
    assert.equal(dpdkKindLabel("bogus"), "Unknown");
    assert.equal(dpdkKindLabel(undefined), "Unknown");
});

test("worker count accepts blank and positive whole numbers", () => {
    assert.deepEqual(normalizeWorkerCount(""), {ok: true, value: null});
    assert.deepEqual(normalizeWorkerCount(null), {ok: true, value: null});
    assert.deepEqual(normalizeWorkerCount("4"), {ok: true, value: 4});
});

test("worker count rejects zero, negatives, and non-numbers", () => {
    assert.equal(normalizeWorkerCount("0").ok, false);
    assert.equal(normalizeWorkerCount("-2").ok, false);
    assert.equal(normalizeWorkerCount("two").ok, false);
    assert.equal(normalizeWorkerCount("2.5").ok, false);
});

test("normalizes a status payload and preserves honest-empty values", () => {
    const rows = normalizeDpdkStatus({
        fields: [
            {id: "hugepages", label: "Hugepages", value: "1024 total", kind: "running", source: "/proc/meminfo"},
            {id: "descriptors", label: "RX / TX descriptors", value: null, kind: "effective", source: "bridge_launch", note: "not exposed"},
        ],
    });
    assert.equal(rows.length, 2);
    assert.equal(rows[0].kind, "Running");
    assert.equal(rows[1].value, null);
    assert.equal(rows[1].kind, "Effective");
    assert.equal(rows[1].note, "not exposed");
});

test("normalizes malformed payloads to an empty list", () => {
    assert.deepEqual(normalizeDpdkStatus(null), []);
    assert.deepEqual(normalizeDpdkStatus({}), []);
    assert.deepEqual(normalizeDpdkStatus({fields: "nope"}), []);
});
