// Contract for the two standalone pages. They bypass the shell, so they load
// no `template.js`: their own bundle has to run the translation pass, and every
// user-visible sentence has to carry a `data-i18n` key with the English text
// still inline as the fallback. Both pages are `done` in migration.json, so the
// key list below is the ratchet — adding a string means adding it here, to both
// catalogs, and to the page.
//
// The bundle also reads catalog keys directly (validation and error sentences),
// and those never appear in markup. They get the same presence, Spanish-must-
// differ and placeholder checks, scanned from the source rather than listed, so
// a new lookup in a page bundle is covered without editing this file.
//
// The language control is here too: both pages load the generated locale list
// and build the control through the switcher the shell uses, so a standalone
// page cannot grow a second copy of the option list.
//
// Order matters too: both bundles are classic scripts with no `defer`, so a
// bundle loaded before the markup it keys translates only what the parser has
// already reached.

import assert from "node:assert/strict";
import {readFile} from "node:fs/promises";
import test from "node:test";
import {fileURLToPath} from "node:url";

const PAGES = [
    {
        name: "login",
        html: new URL("../../../static2/login.html", import.meta.url),
        bundle: new URL("../login.js", import.meta.url),
        keys: [
            "common.footer.copyright",
            "common.footer.rights",
            "common.language.label",
            "common.page_title",
            "login.action",
            "login.error_default",
            "login.intro",
            "login.password",
            "login.username",
            "login.users_hint",
        ],
    },
    {
        name: "first-run",
        html: new URL("../../../static2/first-run.html", import.meta.url),
        bundle: new URL("../first-run.js", import.meta.url),
        keys: [
            "common.footer.copyright",
            "common.footer.rights",
            "common.language.label",
            "common.page_title",
            "first_run.action",
            "first_run.cookie_warning",
            "first_run.create_admin",
            "first_run.error_default",
            "first_run.heading",
            "first_run.intro",
            "first_run.password",
            "first_run.setup_code_help",
            "first_run.setup_code_label",
            "first_run.username",
        ],
    },
];

// The copyright fragment is a proper name and a year prefix; the words that can
// be translated live in `common.footer.rights`, and the year itself is written
// at load time, so no message carries one.
const UNTRANSLATED_BY_DESIGN = new Set(["common.footer.copyright"]);
const KEY_ATTRIBUTE = /data-i18n(?:-title|-placeholder|-aria-label|-alt)?\s*=\s*["']([^"']+)["']/g;
const TEXT_ELEMENT = /<([a-z][a-z0-9]*)([^>]*\bdata-i18n="([^"]+)"[^>]*)>([\s\S]*?)<\/\1>/gi;
const BUNDLE_KEY = /\b(?:tAttr|tHtml|tn|t)\(\s*["']([^"']+)["']/g;

const [english, spanish, stylesheet] = await Promise.all([
    readFile(fileURLToPath(new URL("../../../static2/i18n/en.json", import.meta.url)), "utf8").then(JSON.parse),
    readFile(fileURLToPath(new URL("../../../static2/i18n/es.json", import.meta.url)), "utf8").then(JSON.parse),
    readFile(fileURLToPath(new URL("../../../static2/node_manager.css", import.meta.url)), "utf8"),
]);

const loaded = await Promise.all(PAGES.map(async (page) => ({
    ...page,
    markup: await readFile(fileURLToPath(page.html), "utf8"),
    script: await readFile(fileURLToPath(page.bundle), "utf8"),
})));

function keysUsedBy(markup) {
    return [...new Set([...markup.matchAll(KEY_ATTRIBUTE)].map((match) => match[1]))].sort();
}

function messageTexts(message) {
    return typeof message === "string" ? [message] : Object.values(message);
}

function placeholdersOf(message) {
    const names = messageTexts(message)
        .flatMap((text) => [...text.matchAll(/\{([A-Za-z0-9_]+)\}/g)].map((match) => match[1]));
    return [...new Set(names)].sort();
}

/** The catalog keys the page's bundle reads, sorted, without repeats. */
function bundleKeysUsedBy(script) {
    return [...new Set([...script.matchAll(BUNDLE_KEY)].map((match) => match[1]))].sort();
}

/**
 * The page's language control, read from its mark-up: the wrapper `div` that
 * holds the select, that wrapper's id, and the block from the wrapper tag
 * through `</select>`. Derived rather than restated so the assertions below
 * tie the mark-up to the bundle instead of to a copied literal.
 */
function controlOf(markup) {
    const select = markup.search(/<select[^>]*id="languageSelect"/);
    assert.notEqual(select, -1, "the page must ship the switcher's select");
    const start = markup.lastIndexOf("<div", select);
    const id = markup.slice(start, select).match(/\sid="([^"]+)"/);
    assert.ok(id, "the control's wrapper must carry an id for the switcher to name");
    const close = markup.indexOf("</select>", select);
    assert.notEqual(close, -1, "the select the page ships must be closed");
    return {id: id[1], block: markup.slice(start, close + "</select>".length)};
}

test("both page bundles run the translation pass once", () => {
    for (const page of loaded) {
        assert.match(
            page.script,
            /import \{[^}]*\bapplyDomTranslations\b[^}]*\} from "\.\/i18n\/index\.js";/,
            `${page.name} must import the applier from the i18n runtime`,
        );
        assert.equal(
            (page.script.match(/^applyDomTranslations\(\);$/gm) ?? []).length,
            1,
            `${page.name} must translate its own markup, because it loads no shell bundle`,
        );
    }
});

test("both page bundles build the language control from the shared switcher", () => {
    for (const page of loaded) {
        assert.match(
            page.script,
            /import \{initLanguageSwitcher\} from "\.\/i18n\/switcher\.mjs";/,
            `${page.name} must use the switcher the sidebar uses, not a copy`,
        );
        assert.equal(
            page.script.includes("LQOS_I18N_LOCALES"),
            false,
            `${page.name} must not read the generated locale list itself`,
        );
        // The wrapper id comes from the mark-up the page ships, so the call and
        // the wrapper cannot drift apart: a page that renames its wrapper
        // leaves the switcher naming an element that is not there, and the
        // control would simply stay empty.
        const call = page.script.match(/initLanguageSwitcher\(\{([^}]*)\}\);/);
        assert.ok(call, `${page.name} must build the control through the shared switcher`);
        const {id} = controlOf(page.markup);
        assert.ok(
            call[1].includes(`containerId: "${id}"`),
            `${page.name} must build the control into the wrapper it ships (#${id})`,
        );
    }
});

test("both pages ship the same language control", () => {
    // The switcher builds into whichever wrapper the page names and fills the
    // one select id it knows, so the two copies of the control have to stay
    // interchangeable: a changed class, `for` target, or catalog key on one
    // page is an operator-visible difference between two screens of the same
    // pre-login flow, and nothing else here would notice.
    const [login, firstRun] = loaded.map((page) => controlOf(page.markup).block);
    assert.equal(firstRun, login, "login and first-run must ship one control, not two variants");
});

test("each page loads the locale list and ships the select before its bundle", () => {
    for (const page of loaded) {
        const source = page.bundle.pathname.split("/").pop().replaceAll(".", "\\.");
        const bundle = page.markup.search(new RegExp(`<script src="${source}"`));
        const list = page.markup.indexOf('script src="i18n/locales.js%CACHEBUSTERS%"');
        const control = page.markup.indexOf('id="languageControl"');
        const select = page.markup.indexOf('id="languageSelect"');
        assert.ok(list !== -1, `${page.name} must load the generated locale list`);
        assert.ok(control !== -1, `${page.name} must ship the control's wrapper`);
        assert.ok(select !== -1, `${page.name} must ship the switcher's empty select`);
        assert.ok(bundle !== -1, `${page.name} must load its bundle`);
        assert.ok(list < bundle, `${page.name}: the locale list must load before the bundle`);
        assert.ok(control < bundle, `${page.name}: the wrapper must precede the bundle`);
        assert.ok(select < bundle, `${page.name}: the select must precede the bundle`);
    }
});

test("each page ships its control hidden, named, and its select reachable", () => {
    // The wrapper is the half that keeps a bundle-less page clean: hidden in
    // markup, revealed by the switcher once it has options. The select itself
    // must stay reachable, so no attribute may take it out of the tab order.
    // `d-flex` outranks `[hidden]` in Bootstrap's own sheet, so the hidden state
    // only holds because the stylesheet restates it at id specificity.
    assert.match(
        stylesheet,
        /#languageControl\[hidden\][^{]*\{[^}]*display: none !important;/s,
        "the stylesheet must keep the wrapper hidden against Bootstrap's d-flex",
    );
    for (const page of loaded) {
        const wrapper = page.markup.match(/<div[^>]*id="languageControl"[^>]*>/);
        assert.ok(wrapper, `${page.name} must ship the control's wrapper`);
        assert.match(
            wrapper[0],
            /\shidden(\s|=|>)/,
            `${page.name}: the wrapper must ship hidden, so a browser without the bundle shows no empty select`,
        );
        // Unlike the shell, these pages carry no `aria-label` on the select, so
        // the visible label is the only accessible name it has: the pair that
        // ties the label to the select is the whole name, and it must not be
        // droppable without a failure.
        assert.match(
            page.markup,
            /<label[^>]*for="languageSelect"[^>]*data-i18n="common\.language\.label"/,
            `${page.name}: the visible label must point at the select and read from the catalog`,
        );
        const select = page.markup.match(/<select[^>]*id="languageSelect"[^>]*>/);
        assert.ok(select, `${page.name} must ship the switcher's select`);
        for (const attribute of ["tabindex", "disabled", "hidden"]) {
            assert.equal(
                new RegExp(`\\s${attribute}(\\s|=|>)`).test(select[0]),
                false,
                `${page.name}: the select must not carry ${attribute}`,
            );
        }
    }
});

test("each page keys exactly the strings it carries", () => {
    for (const page of loaded) {
        assert.deepEqual(
            keysUsedBy(page.markup),
            [...page.keys].sort(),
            `${page.name} changed its catalog keys; update migration.json and the catalogs`,
        );
    }
});

test("every page key has a Spanish message", () => {
    for (const page of loaded) {
        for (const key of page.keys) {
            const source = english.messages[key];
            const translated = spanish.messages[key];
            assert.ok(source !== undefined, `en.json is missing "${key}"`);
            assert.ok(translated !== undefined, `es.json is missing "${key}"`);
            if (UNTRANSLATED_BY_DESIGN.has(key)) continue;
            assert.notDeepEqual(translated, source, `es.json "${key}" still carries the English text`);
        }
    }
});

test("every key the page bundles read has a distinct Spanish message", () => {
    for (const page of loaded) {
        const keys = bundleKeysUsedBy(page.script);
        assert.ok(keys.length > 0, `${page.name}'s bundle must read its own sentences from the catalog`);
        for (const key of keys) {
            const source = english.messages[key];
            const translated = spanish.messages[key];
            assert.ok(source !== undefined, `en.json is missing "${key}", which ${page.name} reads`);
            assert.ok(translated !== undefined, `es.json is missing "${key}", which ${page.name} reads`);
            assert.notDeepEqual(
                translated,
                source,
                `es.json "${key}" still carries the English text ${page.name} shows`,
            );
            assert.deepEqual(
                placeholdersOf(translated),
                placeholdersOf(source),
                `es.json "${key}" must keep the placeholders of en.json`,
            );
        }
    }
});

test("each bundle runs after the last mark-up element it translates", () => {
    for (const page of loaded) {
        const source = page.bundle.pathname.split("/").pop().replaceAll(".", "\\.");
        const bundleTag = page.markup.search(new RegExp(`<script src="${source}"`));
        assert.notEqual(bundleTag, -1, `${page.name} must load its bundle`);
        const keyed = [...page.markup.matchAll(KEY_ATTRIBUTE)];
        assert.ok(keyed.length > 0, `${page.name} must key its mark-up`);
        const lastKeyed = Math.max(...keyed.map((match) => match.index));
        assert.ok(
            bundleTag > lastKeyed,
            `${page.name}: the bundle runs before its last keyed element, so the translation pass misses it`,
        );
    }
});

test("no translatable attribute is left without its key", () => {
    for (const page of loaded) {
        for (const tag of page.markup.matchAll(/<[^>]+>/g)) {
            for (const attribute of ["title", "aria-label", "placeholder", "alt"]) {
                if (!new RegExp(`\\s${attribute}\\s*=`).test(tag[0])) continue;
                assert.match(
                    tag[0],
                    new RegExp(`data-i18n-${attribute}=`),
                    `${page.name}: ${tag[0]} carries ${attribute} prose with no catalog key`,
                );
            }
        }
    }
});

test("the English fallback stays inline in every keyed element", () => {
    for (const page of loaded) {
        const textElements = [...page.markup.matchAll(TEXT_ELEMENT)];
        assert.ok(textElements.length > 0, `${page.name} must key its prose elements`);
        for (const element of textElements) {
            const text = element[4].replace(/<[^>]*>/g, "").trim();
            assert.notEqual(text, "", `${page.name}: "${element[3]}" has no English text to fall back to`);
        }
    }
});
