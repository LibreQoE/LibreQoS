import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const source = readFileSync(join(here, "template.js"), "utf8");

test("the frontend scheduler classifier is retired", () => {
    assert.equal(
        source.includes("schedulerLooksRecovered"),
        false,
        "schedulerLooksRecovered must be retired (it hid a populated error)"
    );
    assert.equal(
        source.includes("schedulerHasLiveError"),
        false,
        "schedulerHasLiveError must be retired"
    );
});

test("the served worst-of severity is the only tone source", () => {
    assert.ok(
        source.includes("summary_severity"),
        "the descriptor must read the served summary_severity"
    );
    assert.ok(
        source.includes("function schedulerSummarySeverity"),
        "the served-severity mapping must exist"
    );
    assert.ok(
        source.includes("schedulerSummarySeverity(data)"),
        "the descriptor must branch on the served severity"
    );
});

test("the operator-facing entry is titled System Health", () => {
    assert.ok(
        source.includes('"System Health"'),
        'the nav entry must be titled "System Health"'
    );
    assert.equal(
        source.includes('"Scheduler status is stale"'),
        false,
        "the retired Scheduler label must not remain"
    );
});
