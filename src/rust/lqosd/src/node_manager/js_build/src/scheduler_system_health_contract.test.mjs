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

test("the System Health nav section label is driven by the catalog", () => {
    const catalog = JSON.parse(readFileSync(join(here, "../../static2/i18n/en.json"), "utf8"));
    const html = readFileSync(join(here, "../../static2/template.html"), "utf8");
    assert.equal(
        catalog.messages["common.nav.system_health"],
        "System Health",
        "common.nav.system_health must carry the System Health nav label"
    );
    // Any extra class or attribute on the <li> is cosmetic: only the label class
    // and the key matter here.
    assert.ok(
        /nav-section-label[^>]*data-i18n="common\.nav\.system_health"/.test(html),
        "the sidebar System Health entry must be driven by the catalog key"
    );
    assert.equal(
        source.includes('"System Health"'),
        false,
        "template.js must not reintroduce a hardcoded English nav title"
    );
    assert.equal(
        source.includes(">System Health<"),
        false,
        "template.js must not embed the English nav title as markup"
    );
    assert.equal(
        source.includes('"Scheduler status is stale"'),
        false,
        "the retired Scheduler label must not remain"
    );
});
