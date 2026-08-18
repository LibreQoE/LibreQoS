import assert from "node:assert/strict";
import {afterEach, test} from "node:test";
import {
    ensureMapLibreAssets,
    MAPLIBRE_SCRIPT_SRC,
    MAPLIBRE_STYLESHEET_HREF,
    resetMapLibreAssetLoaderForTest,
} from "./world_map_assets.mjs";

class FakeElement {
    constructor(tagName) {
        this.tagName = tagName.toUpperCase();
        this.listeners = new Map();
        this.attributes = new Map();
    }

    set src(value) {
        this.attributes.set("src", value);
    }

    get src() {
        return this.attributes.get("src");
    }

    set href(value) {
        this.attributes.set("href", value);
    }

    get href() {
        return this.attributes.get("href");
    }

    set rel(value) {
        this.attributes.set("rel", value);
    }

    get rel() {
        return this.attributes.get("rel");
    }

    addEventListener(eventName, handler) {
        this.listeners.set(eventName, handler);
    }

    dispatch(eventName) {
        const handler = this.listeners.get(eventName);
        if (handler) handler();
    }
}

function setupDocument() {
    const elements = [];
    globalThis.window = {};
    globalThis.document = {
        head: {
            appendChild: (element) => {
                elements.push(element);
                return element;
            },
        },
        createElement: (tagName) => new FakeElement(tagName),
        querySelector: (selector) => {
            const scriptMatch = selector.match(/^script\[src="(.+)"\]$/);
            if (scriptMatch) {
                return elements.find((element) => element.tagName === "SCRIPT" && element.src === scriptMatch[1]) || null;
            }
            const linkMatch = selector.match(/^link\[href="(.+)"\]$/);
            if (linkMatch) {
                return elements.find((element) => element.tagName === "LINK" && element.href === linkMatch[1]) || null;
            }
            return null;
        },
    };
    return elements;
}

afterEach(() => {
    resetMapLibreAssetLoaderForTest();
    delete globalThis.document;
    delete globalThis.window;
});

test("ensureMapLibreAssets injects local stylesheet and script once", async () => {
    const elements = setupDocument();
    const promise = ensureMapLibreAssets();

    const links = elements.filter((element) => element.tagName === "LINK");
    const scripts = elements.filter((element) => element.tagName === "SCRIPT");
    assert.equal(links.length, 1);
    assert.equal(links[0].href, MAPLIBRE_STYLESHEET_HREF);
    assert.equal(links[0].rel, "stylesheet");
    assert.equal(scripts.length, 1);
    assert.equal(scripts[0].src, MAPLIBRE_SCRIPT_SRC);

    scripts[0].onload();
    await promise;

    await ensureMapLibreAssets();
    assert.equal(elements.filter((element) => element.tagName === "LINK").length, 1);
    assert.equal(elements.filter((element) => element.tagName === "SCRIPT").length, 1);
});

test("ensureMapLibreAssets waits on an existing local script tag", async () => {
    const elements = setupDocument();
    const existing = document.createElement("script");
    existing.src = MAPLIBRE_SCRIPT_SRC;
    document.head.appendChild(existing);

    const promise = ensureMapLibreAssets();
    assert.equal(elements.filter((element) => element.tagName === "SCRIPT").length, 1);

    existing.dispatch("load");
    await promise;
});

test("ensureMapLibreAssets resolves when an existing local script is already loaded", async () => {
    const elements = setupDocument();
    const existing = document.createElement("script");
    existing.src = MAPLIBRE_SCRIPT_SRC;
    document.head.appendChild(existing);
    window.maplibregl = { Map: class {} };

    await ensureMapLibreAssets();

    assert.equal(elements.filter((element) => element.tagName === "SCRIPT").length, 1);
});

test("ensureMapLibreAssets rejects and allows retry after script load failure", async () => {
    let elements = setupDocument();
    const first = ensureMapLibreAssets();
    const firstScript = elements.find((element) => element.tagName === "SCRIPT");
    firstScript.onerror();
    await assert.rejects(first, /Unable to load MapLibre/);

    elements = setupDocument();
    const second = ensureMapLibreAssets();
    const secondScript = elements.find((element) => element.tagName === "SCRIPT");
    secondScript.onload();
    await second;
});

test("ensureMapLibreAssets only adds the stylesheet when MapLibre is already present", async () => {
    const elements = setupDocument();
    window.maplibregl = { Map: class {} };

    await ensureMapLibreAssets();

    assert.equal(elements.filter((element) => element.tagName === "LINK").length, 1);
    assert.equal(elements.filter((element) => element.tagName === "SCRIPT").length, 0);
});
