// Structural contract for the shell chrome i18n-03 left English: the alerts
// modal, the reload modal and the Display menu's switch descriptions.
//
// `template.js` is a bundle entrypoint that runs DOM initialization at import,
// so its render functions cannot be called from a unit test. What can be
// pinned is the wiring: every chrome string the page renders reads a catalog
// key, and every one of those keys has a distinct Spanish message, which is
// what makes the localized chrome reach the screen under `es`.
//
// The relative-time half is here too: the hand-rolled ladder this unit deleted
// must not come back, and the timestamp must go through `Intl`.

import assert from "node:assert/strict";
import {readFile} from "node:fs/promises";
import test from "node:test";
import {fileURLToPath} from "node:url";

const [page, timing, shell, english, spanish] = await Promise.all([
    readFile(fileURLToPath(new URL("../template.js", import.meta.url)), "utf8"),
    readFile(fileURLToPath(new URL("../scheduler_timing.mjs", import.meta.url)), "utf8"),
    readFile(fileURLToPath(new URL("../../../static2/template.html", import.meta.url)), "utf8"),
    readFile(fileURLToPath(new URL("../../../static2/i18n/en.json", import.meta.url)), "utf8").then(JSON.parse),
    readFile(fileURLToPath(new URL("../../../static2/i18n/es.json", import.meta.url)), "utf8").then(JSON.parse),
]);

// Rendered by `template.js`: the alerts modal's controls and states, and the
// reload flow's confirm, progress and result messages. The alerts nav label is
// not here: it moved to the tested `i18n/alerts_aria_label.mjs` helper.
const RENDERED_KEYS = [
    "common.alerts.empty",
    "common.alerts.active",
    "common.alerts.acknowledge",
    "common.alerts.acknowledge_code",
    "common.alerts.acknowledging",
    "common.alerts.acknowledge_failed",
    "common.alerts.acknowledge_timeout",
    "common.alerts.acknowledging_all",
    "common.alerts.acknowledge_all_failed",
    "common.alerts.acknowledge_all_timeout",
    "common.alerts.refresh_failed",
    "common.action.retry",
    "common.state.loading",
    "common.reload.confirm",
    "common.reload.resyncing",
    "common.reload.failed",
    "common.reload.saving_queue_mode",
];

// Rendered by the shell markup: the Display switches' tooltips and labels, the
// sidebar's language label and select name, the alerts modal's acknowledge-all
// control, the reload modal's note, and the sidebar's last inline English (the
// navbar toggle's name, the logo's alternative text, and the queue-mode group
// plus its reload note).
const MARKUP_KEYS = [
    "common.alerts.acknowledge_all",
    "common.reload.tooltip",
    "common.reload.bridge_note",
    "common.language.label",
    "common.nav.toggle",
    "common.nav.logo_alt",
    "common.nav.queue_mode_aria",
    "common.nav.queue_mode_note",
    "common.display.dark_mode_tooltip",
    "common.display.dark_mode_aria",
    "common.display.redact_aria",
    "common.display.color_blind_tooltip",
    "common.display.color_blind_aria",
    "common.display.klingon_aria",
];

// Rendered by the scheduler/bridge chrome: one key per surface (the nav label,
// a status badge, the modal panels, the runs line, the bridge detail, the queue
// dialogs). The extractor only notices a key that loses its *last* reader, so
// these pin the surfaces whose keys are also read elsewhere.
const SCHEDULER_KEYS = [
    "scheduler.nav.healthy",
    "scheduler.badge.degraded",
    "scheduler.panel.integration_named",
    "scheduler.runs.last_completed",
    "scheduler.integration.consecutive_failures",
    "scheduler.transport.cached",
    "scheduler.what_to_check.retry_unavailable",
    "bridge.health.workers",
    "bridge.health.last_ok",
    "queue_mode.confirm.observe",
    "queue_mode.confirm.shape",
];

// Rendered by the module that owns the readout: `scheduler_timing.mjs` renders
// both timing keys and is imported by `template.js`. It lives outside the page
// bundle so the rendered string can be asserted without importing a bundle that
// initializes the DOM (`scheduler_timing.test.mjs` does exactly that).
const TIMING_KEYS = ["scheduler.timing.elapsed_remaining"];

// A keyed element with a body: the same shape `standalone_pages_contract`
// guards on the two pages that bypass the shell.
const TEXT_ELEMENT = /<([a-z][a-z0-9]*)([^>]*\bdata-i18n="([^"]+)"[^>]*)>([\s\S]*?)<\/\1>/gi;

function messageTexts(message) {
    return typeof message === "string" ? [message] : Object.values(message);
}

function placeholdersOf(message) {
    const names = messageTexts(message)
        .flatMap((text) => [...text.matchAll(/\{([A-Za-z0-9_]+)\}/g)].map((match) => match[1]));
    return [...new Set(names)].sort();
}

test("every chrome string the page renders reads a catalog key", () => {
    for (const key of RENDERED_KEYS) {
        const call = new RegExp(`\\b(?:tAttr|t|tn|tHtml)\\(\\s*"${key.replaceAll(".", "\\.")}"`);
        assert.match(page, call, `template.js must read "${key}" from the catalog`);
    }
});

test("every scheduler/bridge string the page renders reads a catalog key", () => {
    for (const key of SCHEDULER_KEYS) {
        const call = new RegExp(`\\b(?:tAttr|t|tn|tHtml)\\(\\s*"${key.replaceAll(".", "\\.")}"`);
        assert.match(page, call, `template.js must read "${key}" from the catalog`);
    }
});

test("every timing key the readout renders comes from its catalog", () => {
    for (const key of TIMING_KEYS) {
        const call = new RegExp(`\\b(?:tAttr|t|tn|tHtml)\\(\\s*"${key.replaceAll(".", "\\.")}"`);
        assert.match(timing, call, `scheduler_timing.mjs must read "${key}" from the catalog`);
    }
});

test("every chrome string the markup renders carries its key", () => {
    for (const key of MARKUP_KEYS) {
        const attribute = new RegExp(`data-i18n(?:-title|-aria-label|-alt)?="${key.replaceAll(".", "\\.")}"`);
        assert.match(shell, attribute, `template.html must carry data-i18n for "${key}"`);
    }
});

test("the shell's keyed elements keep their inline English", () => {
    // An empty fallback plus a key the served catalog lacks renders empty
    // markup, which is the one thing the i18n frame must never do.
    const keyed = [...shell.matchAll(TEXT_ELEMENT)];
    assert.ok(keyed.length > 0, "template.html must key its prose elements");
    for (const element of keyed) {
        const text = element[4].replace(/<[^>]*>/g, "").trim();
        assert.notEqual(text, "", `template.html: "${element[3]}" has no English text to fall back to`);
    }
});

test("each chrome key has a Spanish message, not the English fallback", () => {
    for (const key of [...RENDERED_KEYS, ...MARKUP_KEYS]) {
        const source = english.messages[key];
        const translated = spanish.messages[key];
        assert.ok(source !== undefined, `en.json is missing "${key}"`);
        assert.ok(translated !== undefined, `es.json is missing "${key}"`);
        assert.notDeepEqual(translated, source, `es.json "${key}" still carries the English text`);
        assert.deepEqual(
            placeholdersOf(translated),
            placeholdersOf(source),
            `es.json "${key}" must keep the placeholders of en.json`,
        );
    }
});

test("the delete-level relative-time ladder stays deleted", () => {
    assert.match(
        page,
        /import \{relativeTimeText\} from "\.\/i18n\/relative_time\.mjs";/,
        "template.js must use the tested relative-time helper",
    );
    assert.match(page, /relativeTimeText\(updatedUnix\)/, "the updated stamp goes through the helper");
    assert.match(page, /relativeTimeText\(unixTs\)/, "the age stamp goes through the helper");
    // The compact forms this unit deleted: "5s ago", "just now", "Updated 5m ago".
    assert.doesNotMatch(page, /\b[smhd] ago\b/, "no hand-built compact relative time may remain");
    assert.doesNotMatch(page, /just now/, "no hand-built 'just now' may remain");
    assert.doesNotMatch(
        page,
        /\$\{\s*(?:delta|deltaSeconds|deltaMinutes|deltaHours|deltaDays)\s*\}/,
        "no hand-built delta interpolation may remain",
    );
});

test("the alerts modal's timestamp is Intl's, not the browser default", () => {
    assert.match(
        page,
        /const alertTime = Number\(it\.ts\) \* 1000;/,
        "the alert row's date must be derived from the alert timestamp",
    );
    assert.match(
        page,
        /const when = Number\.isFinite\(alertTime\) \? formatDateTime\(alertTime\) : "—";/,
        "the alert row's date must follow the UI locale and survive a missing timestamp",
    );
    assert.doesNotMatch(page, /new Date\(it\.ts \* 1000\)/, "no hand-rolled date rendering may remain");
});

test("the alert row escapes every server-supplied field it interpolates", () => {
    assert.match(
        page,
        /import \{[^}]*\bescapeAttr\b[^}]*\bescapeHtml\b[^}]*\} from "\.\/i18n\/escape\.mjs";/,
        "the render templates must use the tested escapers",
    );
    for (const field of ["severity", "code", "source", "message", "context"]) {
        assert.match(
            page,
            new RegExp(`escapeHtml\\(it\\.${field}\\)`),
            `the alert row must escape it.${field}: the server only strips ANSI`,
        );
    }
    assert.match(
        page,
        /data-id="\$\{escapeAttr\(it\.id\)\}"/,
        "the acknowledge button's data-id must be escaped",
    );
});

test("the alerts nav label comes from the tested plural helper", () => {
    assert.match(
        page,
        /import \{alertsAriaLabel\} from "\.\/i18n\/alerts_aria_label\.mjs";/,
        "the nav label selection must live in its unit-tested module",
    );
    assert.match(
        page,
        /const ariaLabel = alertsAriaLabel\(count\);/,
        "the alert count must go through the helper, not an inline branch",
    );
});

test("the reload prompt composes the shared bridge note", () => {
    assert.match(
        page,
        /const message = [^\n]*common\.reload\.confirm[^\n]*common\.reload\.bridge_note/,
        "the prompt must build one message from the question and the shared note",
    );
    assert.match(
        page,
        /window\.confirm\(message\)/,
        "the prompt must confirm the composed message",
    );
    for (const [tag, catalog] of [["en", english], ["es", spanish]]) {
        const note = catalog.messages["common.reload.bridge_note"];
        const confirm = catalog.messages["common.reload.confirm"];
        assert.ok(!confirm.includes(note), `${tag}.json "common.reload.confirm" repeats "common.reload.bridge_note"`);
    }
});
