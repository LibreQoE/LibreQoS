import {test} from "node:test";
import assert from "node:assert/strict";
import {
    applyPlacementBudgetMbps,
    parsePlacementBudgetInput,
    readPlacementBudgetMbps,
} from "./tuning_hygiene.mjs";

test("blank placement budget means absent/default", () => {
    assert.deepEqual(parsePlacementBudgetInput(""), {ok: true, value: null});
    assert.deepEqual(parsePlacementBudgetInput("  "), {ok: true, value: null});
});

test("placement budget accepts a positive integer Mbps", () => {
    assert.deepEqual(parsePlacementBudgetInput("2000"), {ok: true, value: 2000});
    assert.deepEqual(parsePlacementBudgetInput(" 1500 "), {ok: true, value: 1500});
});

test("placement budget rejects decimal, zero, negative and junk", () => {
    for (const bad of ["12.5", "0", "-1", "nope", "1e3"]) {
        assert.equal(parsePlacementBudgetInput(bad).ok, false, bad);
    }
});

test("apply carries the whole tuning object and sets the key", () => {
    const config = {tuning: {netdev_budget_usecs: 8000}, queues: {workers: 10}};
    applyPlacementBudgetMbps(config, 2000);
    assert.equal(config.tuning.netdev_budget_usecs, 8000);
    assert.equal(config.tuning.placement_budget_mbps, 2000);
    assert.deepEqual(config.queues, {workers: 10});
    assert.equal(readPlacementBudgetMbps(config), 2000);
});

test("apply with null deletes the key but keeps the rest of tuning", () => {
    const config = {
        tuning: {netdev_budget_usecs: 8000, placement_budget_mbps: 2000},
    };
    applyPlacementBudgetMbps(config, null);
    assert.equal(config.tuning.netdev_budget_usecs, 8000);
    assert.equal("placement_budget_mbps" in config.tuning, false);
    assert.equal(readPlacementBudgetMbps(config), null);
});

test("round-trips through the config object without inventing a section", () => {
    const config = {};
    applyPlacementBudgetMbps(config, 2000);
    assert.deepEqual(config.tuning, {placement_budget_mbps: 2000});
    applyPlacementBudgetMbps(config, null);
    assert.deepEqual(config.tuning, {});
});
