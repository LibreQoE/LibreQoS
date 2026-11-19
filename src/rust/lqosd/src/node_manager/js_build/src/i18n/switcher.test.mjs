// The switcher builds real DOM nodes, so this is the smallest fake that can
// hold its select and options: `template.js` is not unit-testable, and the
// control used to be guarded by nothing at all (REV-T1 of i18n-03).
//
// `select.choose()` stands in for the browser: a native select hands its new
// value to the `change` listener, which is the half this module owns. Which
// option a browser marks selected is the platform's job; the guard here is that
// the module marks the option the browser will display — the active tag when
// the list carries it, the first option when it does not.
//
// The wrappers arrive hidden (`languageNav` in the shell, `languageControl` on
// the standalone pages), so the fake records the reveal as well as the drop.

import assert from "node:assert/strict";
import test from "node:test";

import {initLanguageSwitcher, listedLocales} from "./switcher.mjs";

const LOCALES = [
    {tag: "en", name: "English", dir: "ltr"},
    {tag: "es", name: "Español", dir: "ltr"},
];
const GERMAN = {tag: "de", name: "Deutsch", dir: "ltr"};

class FakeElement {
    constructor(tagName) {
        this.tagName = tagName.toUpperCase();
        this.attributes = {};
        this.listeners = {};
        this.children = [];
        this.textContent = "";
        this.value = "";
        this.selected = false;
    }

    setAttribute(name, value) {
        this.attributes[name] = String(value);
    }

    addEventListener(type, handler) {
        this.listeners[type] = handler;
    }

    appendChild(child) {
        this.children.push(child);
        return child;
    }

    /** Replays a user picking `value` in the open list. */
    choose(value) {
        this.value = value;
        this.listeners.change();
    }
}

class FakeDocument {
    constructor(elements) {
        this.elements = elements;
    }

    getElementById(id) {
        return this.elements[id] ?? null;
    }

    createElement(tagName) {
        return new FakeElement(tagName);
    }
}

/** An element the switcher may reveal once it has options, or drop entirely. */
function removable() {
    return {removed: 0, hidden: true, remove() { this.removed += 1; }};
}

/** Renders the selector against a fake document and records what it did. */
function render({listed = LOCALES, active = "en", containerId} = {}) {
    const select = new FakeElement("select");
    const nav = removable();
    const control = removable();
    const doc = new FakeDocument({languageSelect: select, languageNav: nav, languageControl: control});
    const selected = [];
    let reloads = 0;
    const count = initLanguageSwitcher({
        document: doc,
        scope: {LQOS_I18N_LOCALES: listed},
        active,
        containerId,
        onSelect: (tag) => selected.push(tag),
        reload: () => { reloads += 1; },
    });
    return {count, select, nav, control, selected, reloads: () => reloads};
}

test("a missing locale list reads as empty", () => {
    assert.deepEqual(listedLocales({}), []);
    assert.deepEqual(listedLocales(undefined), []);
});

test("a page that did not load the list drops the item and does not throw", () => {
    const {count, select, nav, control} = render({listed: null});
    assert.equal(count, 0);
    assert.deepEqual(select.children, []);
    assert.equal(nav.removed, 1, "the whole item goes, not just an empty control");
    assert.equal(control.removed, 0, "a wrapper this page does not use is left alone");
});

test("one option per listed locale, in list order, in the locale's own name", () => {
    const {count, select} = render();
    assert.equal(count, 2);
    assert.deepEqual(select.children.map((option) => option.tagName), ["OPTION", "OPTION"]);
    assert.deepEqual(select.children.map((option) => option.value), ["en", "es"]);
    assert.deepEqual(select.children.map((option) => option.textContent), ["English", "Español"]);
});

test("a third locale in the list becomes a third option, in place", () => {
    const {count, select} = render({listed: [...LOCALES, GERMAN]});
    assert.equal(count, 3);
    assert.deepEqual(select.children.map((option) => option.value), ["en", "es", "de"]);
    assert.deepEqual(select.children.map((option) => option.textContent), ["English", "Español", "Deutsch"]);
});

test("the active tag is the selected option, and the only one", () => {
    const {select} = render({active: "es"});
    assert.deepEqual(select.children.map((option) => option.selected), [false, true]);
});

test("an active tag the list does not carry falls back to the first listed locale", () => {
    // A browser shows the first option when nothing is marked selected, and
    // re-picking what is already on screen fires no change event; marking the
    // first option keeps the displayed locale the one a change acts on.
    const {count, select} = render({active: "fr"});
    assert.equal(count, 2);
    assert.deepEqual(select.children.map((option) => option.selected), [true, false]);
});

test("choosing another locale persists that tag and reloads", () => {
    const {select, selected, reloads} = render({active: "en"});
    select.choose("es");
    assert.deepEqual(selected, ["es"]);
    assert.equal(reloads(), 1);
});

test("one change listener on the select covers the whole list", () => {
    const {select} = render();
    assert.deepEqual(Object.keys(select.listeners), ["change"]);
    for (const option of select.children) {
        assert.deepEqual(option.listeners, {}, "options carry no listeners of their own");
    }
});

test("a single-entry list drops the item and builds nothing", () => {
    const {count, select, nav, control} = render({listed: [LOCALES[0]]});
    assert.equal(count, 0);
    assert.deepEqual(select.children, []);
    assert.equal(nav.removed, 1);
    assert.equal(control.removed, 0);
});

// The standalone login and first-run pages ship the same select inside a wrapper
// of their own, so the degradation path has to remove the wrapper the caller
// names rather than the shell's sidebar item.
test("a page names the wrapper its control lives in", () => {
    const {count, select, nav, control} = render({
        listed: [LOCALES[0]],
        containerId: "languageControl",
    });
    assert.equal(count, 0);
    assert.deepEqual(select.children, []);
    assert.equal(control.removed, 1, "the page's control goes as a whole");
    assert.equal(nav.removed, 0, "the shell's item is not this page's to remove");
});

test("a listed pair builds the options and drops nothing", () => {
    const {count, nav, control} = render({containerId: "languageControl"});
    assert.equal(count, 2);
    assert.equal(control.removed, 0);
    assert.equal(nav.removed, 0);
});

test("the wrapper ships hidden and is revealed once there are options", () => {
    const {nav, control} = render({containerId: "languageControl"});
    assert.equal(control.hidden, false, "the page's wrapper is revealed with its options");
    assert.equal(nav.hidden, true, "a wrapper this page does not use is left as it shipped");
});

test("a page without the select is left alone", () => {
    let created = 0;
    const doc = {
        getElementById: () => null,
        createElement: () => { created += 1; },
    };
    assert.equal(initLanguageSwitcher({document: doc, scope: {}}), 0);
    assert.equal(created, 0);
});

test("the production defaults persist the choice and reload the page", () => {
    // No injected `onSelect`/`reload`/`active`: this is the call `template.js`
    // makes, so a default that stops persisting or stops reloading fails here.
    // The page is served in Spanish with `<html lang="en">` behind it, so the
    // option the browser displays is the served catalog's tag, not the mark-up
    // fallback: the server resolved this request, the attribute did not.
    const select = new FakeElement("select");
    const page = new FakeDocument({languageSelect: select, languageNav: {remove() {}}});
    page.documentElement = {lang: "en"};
    const storage = new Map();
    let reloads = 0;
    globalThis.document = page;
    globalThis.window = {LQOS_I18N: {locale: "es", messages: {}}};
    globalThis.localStorage = {
        getItem: (key) => (storage.has(key) ? storage.get(key) : null),
        setItem: (key, value) => storage.set(key, value),
    };
    globalThis.location = {reload: () => { reloads += 1; }};
    globalThis.LQOS_I18N_LOCALES = LOCALES;
    try {
        assert.equal(initLanguageSwitcher(), 2, "the defaults must render one option per listed locale");
        assert.deepEqual(
            select.children.map((option) => option.selected),
            [false, true],
            "the served catalog's tag must be the option the browser displays",
        );
        select.choose("es");
        assert.equal(storage.get("uiLanguage"), "es", "the choice must persist to localStorage");
        assert.match(page.cookie, /^lqos_lang=es;path=\/;max-age=\d+;SameSite=Lax$/);
        assert.equal(reloads, 1, "the choice must reload the page");
    } finally {
        delete globalThis.document;
        delete globalThis.window;
        delete globalThis.localStorage;
        delete globalThis.location;
        delete globalThis.LQOS_I18N_LOCALES;
    }
});
