import assert from "node:assert/strict";
import test from "node:test";

import {normalizeDefaultSqm, isFlatCompileMode, SUPPORTED_DEFAULT_SQM} from "./queues_hygiene.mjs";

test("keeps the supported v3 default SQM", () => {
    assert.deepEqual(normalizeDefaultSqm("cake diffserv4"), {
        value: SUPPORTED_DEFAULT_SQM,
        legacy: false,
        original: "cake diffserv4",
    });
});

test("normalizes legacy kernel SQM values and flags them", () => {
    for (const legacy of ["fq_codel", "cake diffserv4 ack-filter"]) {
        const result = normalizeDefaultSqm(legacy);
        assert.equal(result.value, SUPPORTED_DEFAULT_SQM);
        assert.equal(result.legacy, true);
        assert.equal(result.original, legacy);
    }
});

test("treats blank or absent values as a non-legacy default", () => {
    for (const absent of ["", "   ", null, undefined, 42]) {
        const result = normalizeDefaultSqm(absent);
        assert.equal(result.value, SUPPORTED_DEFAULT_SQM);
        assert.equal(result.legacy, false);
    }
});

test("only flat compile mode enables the flat-mode queue-sizing controls", () => {
    assert.equal(isFlatCompileMode({topology: {compile_mode: "flat"}}), true);
    for (const mode of ["ap_only", "ap_site", "full", ""]) {
        assert.equal(isFlatCompileMode({topology: {compile_mode: mode}}), false, mode);
    }
    assert.equal(isFlatCompileMode({}), false);
    assert.equal(isFlatCompileMode({topology: {}}), false);
    assert.equal(isFlatCompileMode(null), false);
    assert.equal(isFlatCompileMode(undefined), false);
});
