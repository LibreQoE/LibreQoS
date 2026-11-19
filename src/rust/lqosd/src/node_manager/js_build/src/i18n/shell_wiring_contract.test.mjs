// Structural contract for the shell's i18n wiring. Three one-liners make the
// frame translated at all — the translation pass, the switcher's initializer,
// and the generated locale list the switcher reads — and deleting any of them
// used to leave every declared check green (REV-T1 of i18n-03), because no test
// read the page's call sites. The control's behaviour itself is unit-tested in
// `switcher.test.mjs`; this file guards that it is plugged in, and that it sits
// in the sidebar rather than inside the Display menu (i18n-06).

import assert from "node:assert/strict";
import {readFile} from "node:fs/promises";
import test from "node:test";
import {fileURLToPath} from "node:url";

const srcDir = new URL("./", import.meta.url);
const shellPath = new URL("../../../static2/template.html", import.meta.url);

const [page, shell, runtime, switcher] = await Promise.all([
    readFile(fileURLToPath(new URL("../template.js", import.meta.url)), "utf8"),
    readFile(fileURLToPath(shellPath), "utf8"),
    readFile(fileURLToPath(new URL("index.js", srcDir)), "utf8"),
    readFile(fileURLToPath(new URL("switcher.mjs", srcDir)), "utf8"),
]);

test("the frame is translated exactly once, from the runtime's applier", () => {
    assert.match(
        page,
        /import \{[^}]*\bapplyDomTranslations\b[^}]*\} from "\.\/i18n\/index\.js";/,
        "the page must import the applier from the i18n runtime",
    );
    assert.match(runtime, /export function applyDomTranslations\(/);
    assert.equal(
        (page.match(/^applyDomTranslations\(\);$/gm) ?? []).length,
        1,
        "the shell must run the translation pass once, on load",
    );
});

test("the switcher is the module's, and it is built before the translation pass", () => {
    assert.match(
        page,
        /import \{initLanguageSwitcher\} from "\.\/i18n\/switcher\.mjs";/,
        "the page must call the switcher that `switcher.test.mjs` covers",
    );
    assert.equal(
        page.includes("function initLanguageSwitcher"),
        false,
        "the switcher belongs to its module, not the page",
    );
    assert.equal(
        page.includes("LQOS_I18N_LOCALES"),
        false,
        "the page must not read the generated locale list itself",
    );
    assert.ok(switcher.includes("LQOS_I18N_LOCALES"), "the switcher reads the generated list");
    const init = page.indexOf("initLanguageSwitcher();");
    const translate = page.indexOf("applyDomTranslations();");
    assert.ok(init !== -1, "the page must initialize the switcher");
    assert.ok(
        init < translate,
        "the language control's options must exist before the frame is translated",
    );
});

test("the shell loads the locale list before the page bundle", () => {
    const loader = shell.indexOf('script src="i18n/locales.js%CACHEBUSTERS%"');
    const bundle = shell.indexOf('script src="template.js%CACHEBUSTERS%"');
    assert.ok(loader !== -1, "the shell must load the generated locale list");
    assert.ok(bundle !== -1, "the shell must load the page bundle");
    assert.ok(loader < bundle, "the list must load before the page bundle");
});

test("the language control is its own sidebar item, above the Display menu", () => {
    assert.match(shell, /<select[^>]*id="languageSelect"[^>]*><\/select>/, "the shell carries the switcher's native select, empty: its options come from the generated list");
    assert.match(shell, /id="languageNav"/, "the item the switcher drops when a build lists fewer than two locales");

    const select = shell.indexOf('id="languageSelect"');
    const display = shell.indexOf('<details class="nav-display"');
    assert.ok(display !== -1, "the shell must still carry the Display menu");
    assert.ok(select < display, "the language control must render before the Display item");

    const itemStart = shell.lastIndexOf("<li", select);
    const itemEnd = shell.indexOf("</li>", select);
    assert.ok(itemStart !== -1 && itemEnd > select, "the control must sit inside a sidebar item");
    const item = shell.slice(itemStart, itemEnd);
    assert.ok(
        !item.includes("<details") && !item.includes("nav-display-body"),
        "the control must not be inside a collapsible Display body",
    );
    const itemTag = shell.slice(itemStart, shell.indexOf(">", itemStart) + 1);
    assert.match(
        itemTag,
        /\shidden(\s|=|>)/,
        "the item ships hidden: a page whose bundle never runs must not show an empty select",
    );
});

test("the select ships without an attribute that would take it out of the tab order", () => {
    // Reachable-by-tab is a property of this markup, not an inference from a
    // browser run: `hidden` belongs to the wrapper the switcher reveals, never
    // to the control, and a disabled or tab-excluded select would be a focus
    // stop that cannot do its job.
    const match = shell.match(/<select[^>]*id="languageSelect"[^>]*>/);
    assert.ok(match, "the shell must carry the switcher's select");
    for (const attribute of ["tabindex", "disabled", "hidden"]) {
        assert.equal(
            new RegExp(`\\s${attribute}(\\s|=|>)`).test(match[0]),
            false,
            `the select must not carry ${attribute}`,
        );
    }
});

test("the select is named in the catalog's words, and its label is visible", () => {
    // Native select: the name has to come from the markup, because the control
    // has no text of its own once an option is chosen.
    assert.match(shell, /aria-label="Language"[^>]*data-i18n-aria-label="common\.language\.label"/);
    assert.match(
        shell,
        /<label[^>]*for="languageSelect"[^>]*data-i18n="common\.language\.label"/,
        "the visible label must point at the select and read from the catalog",
    );
});
