import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { coreRowKind } from "./worker_occupancy_model.mjs";

const sourceDirectory = new URL(".", import.meta.url);
const scriptPath = fileURLToPath(new URL("cpu_workers.js", sourceDirectory));

test("a row click re-renders with the retained utilization", async () => {
    const source = await readFile(scriptPath, "utf8");

    // Regression (74t171): the click handler used to re-render from the cached
    // last sections but DROPPED lastUtilization, so every DPDK worker row
    // flipped to "waiting for the first sample" after any click.
    assert.match(
        source,
        /renderCpuUsage\(lastCpuUsage, lastTopology, lastWorkers, lastUtilization\)/,
        "the cached re-render must pass lastUtilization",
    );
    assert.doesNotMatch(
        source,
        /renderCpuUsage\(lastCpuUsage, lastTopology, lastWorkers\)/,
        "no cached re-render may drop utilization",
    );

    // The failure mode this guards: a worker row without its utilization is
    // rendered as pending ("—"), never as an OS/usage row and never left with
    // a stale value.
    assert.equal(coreRowKind(true, true), "worker");
    assert.equal(coreRowKind(true, false), "worker-pending");
});
