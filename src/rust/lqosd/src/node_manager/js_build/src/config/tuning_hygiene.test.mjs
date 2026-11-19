import {test} from "node:test";
import assert from "node:assert/strict";
import {
    applyWorkerCapacityMbps,
    parseWorkerCapacityInput,
    readWorkerCapacityMbps,
} from "./tuning_hygiene.mjs";
import {clearCatalogs, EN, ES, installCatalogs, installKeyEchoCatalogs} from "../../i18n/test_catalogs.mjs";

test.afterEach(() => {
    clearCatalogs();
});

test("the worker-capacity error maps to a key both catalogs define", () => {
    installKeyEchoCatalogs();
    const {error} = parseWorkerCapacityInput("nope");
    assert.equal(typeof EN.messages[error], "string", `${error} is missing from en.json`);
    assert.equal(typeof ES.messages[error], "string", `${error} is missing from es.json`);
});

test("the worker-capacity error follows the Spanish catalog", () => {
    installCatalogs(ES);
    assert.equal(
        parseWorkerCapacityInput("0").error,
        "La capacidad por worker debe ser un número entero positivo de Mbps.",
    );
});

test("blank worker capacity means absent/default", () => {
    assert.deepEqual(parseWorkerCapacityInput(""), {ok: true, value: null});
    assert.deepEqual(parseWorkerCapacityInput("  "), {ok: true, value: null});
});

test("worker capacity accepts a positive integer Mbps", () => {
    assert.deepEqual(parseWorkerCapacityInput("1125"), {ok: true, value: 1125});
    assert.deepEqual(parseWorkerCapacityInput(" 2000 "), {ok: true, value: 2000});
});

test("worker capacity rejects decimal, zero, negative and junk", () => {
    for (const bad of ["12.5", "0", "-1", "nope", "1e3"]) {
        assert.equal(parseWorkerCapacityInput(bad).ok, false, bad);
    }
});

test("apply carries the whole tuning object and sets the canonical key", () => {
    const config = {tuning: {netdev_budget_usecs: 8000}, queues: {workers: 10}};
    applyWorkerCapacityMbps(config, 1125);
    assert.equal(config.tuning.netdev_budget_usecs, 8000);
    assert.equal(config.tuning.worker_capacity_mbps, 1125);
    assert.deepEqual(config.queues, {workers: 10});
    assert.equal(readWorkerCapacityMbps(config), 1125);
});

test("apply with null deletes the key but keeps the rest of tuning", () => {
    const config = {
        tuning: {netdev_budget_usecs: 8000, worker_capacity_mbps: 1125},
    };
    applyWorkerCapacityMbps(config, null);
    assert.equal(config.tuning.netdev_budget_usecs, 8000);
    assert.equal("worker_capacity_mbps" in config.tuning, false);
    assert.equal(readWorkerCapacityMbps(config), null);
});

test("deprecated placement_budget_mbps is read, then migrated on save", () => {
    const config = {tuning: {placement_budget_mbps: 1500}};
    assert.equal(readWorkerCapacityMbps(config), 1500);
    applyWorkerCapacityMbps(config, 1500);
    assert.equal(config.tuning.worker_capacity_mbps, 1500);
    assert.equal("placement_budget_mbps" in config.tuning, false);
});

test("canonical worker_capacity_mbps wins over the deprecated alias", () => {
    const config = {tuning: {worker_capacity_mbps: 1125, placement_budget_mbps: 2000}};
    assert.equal(readWorkerCapacityMbps(config), 1125);
});

test("round-trips through the config object without inventing a section", () => {
    const config = {};
    applyWorkerCapacityMbps(config, 2000);
    assert.deepEqual(config.tuning, {worker_capacity_mbps: 2000});
    applyWorkerCapacityMbps(config, null);
    assert.deepEqual(config.tuning, {});
});
