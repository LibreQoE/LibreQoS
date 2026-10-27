import assert from "node:assert/strict";
import {test} from "node:test";
import {pruneTabsWithoutKnownDashlets} from "./layout_prune.mjs";

const known = new Set(["throughputBps", "cpu", "ram"]);

test("prunes a tab whose dashlets are all retired", () => {
    const tabs = [
        {id: "overview", dashlets: [{tag: "throughputBps"}, {tag: "cpu"}]},
        {id: "bakery", dashlets: [{tag: "bakeryPipeline"}, {tag: "bakeryActivity"}]},
    ];
    const out = pruneTabsWithoutKnownDashlets(tabs, known);
    assert.deepEqual(out.map((tab) => tab.id), ["overview"]);
});

test("keeps a tab with at least one known dashlet", () => {
    const tabs = [{id: "mixed", dashlets: [{tag: "gone"}, {tag: "cpu"}]}];
    const out = pruneTabsWithoutKnownDashlets(tabs, known);
    assert.deepEqual(out.map((tab) => tab.id), ["mixed"]);
});

test("an empty known set is a no-op (never prune blindly)", () => {
    const tabs = [{id: "bakery", dashlets: [{tag: "bakeryPipeline"}]}];
    assert.equal(pruneTabsWithoutKnownDashlets(tabs, new Set()).length, 1);
});

test("tolerates malformed tab and dashlet shapes", () => {
    const tabs = [null, {}, {id: "x", dashlets: null}, {id: "y", dashlets: [{}, {tag: "cpu"}]}];
    const out = pruneTabsWithoutKnownDashlets(tabs, known);
    assert.deepEqual(out.map((tab) => tab && tab.id), ["y"]);
});

test("non-array input yields an empty array", () => {
    assert.deepEqual(pruneTabsWithoutKnownDashlets(null, known), []);
});
