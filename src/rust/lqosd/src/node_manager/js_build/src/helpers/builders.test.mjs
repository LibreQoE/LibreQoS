import assert from "node:assert/strict";
import test from "node:test";

import {safeRelativeHref, simpleLinkRow} from "./safe_dom.mjs";

class FakeClassList {
    constructor() {
        this.values = [];
    }

    add(...classes) {
        this.values.push(...classes);
    }

    contains(className) {
        return this.values.includes(className);
    }
}

class FakeElement {
    constructor(tagName) {
        this.tagName = tagName.toUpperCase();
        this.children = [];
        this.classList = new FakeClassList();
        this._textContent = "";
        this._href = "";
    }

    appendChild(child) {
        this.children.push(child);
    }

    set href(value) {
        this._href = String(value);
    }

    get href() {
        return this._href;
    }

    set textContent(value) {
        this._textContent = String(value);
    }

    get textContent() {
        return this._textContent;
    }
}

test("safeRelativeHref rejects scriptable URL protocols", () => {
    assert.equal(safeRelativeHref("javascript:alert(1)"), "#");
    assert.equal(safeRelativeHref(" data:text/html,boom"), "#");
    assert.equal(safeRelativeHref("https://example.invalid/circuit"), "#");
    assert.equal(safeRelativeHref("//example.invalid/circuit"), "#");
    assert.equal(safeRelativeHref("circuit.html?circuit=123"), "circuit.html?circuit=123");
});

test("simpleLinkRow renders operator text as text content", () => {
    globalThis.document = {
        createElement: (tagName) => new FakeElement(tagName),
    };

    const row = simpleLinkRow(
        "javascript:alert(1)",
        "<img src=x onerror=alert(1)>",
        true,
    );
    const link = row.children[0];

    assert.equal(row.tagName, "TD");
    assert.equal(link.tagName, "A");
    assert.equal(link.href, "#");
    assert.equal(link.textContent, "<img src=x onerror=alert(1)>");
    assert.equal(link.classList.contains("redactable"), true);
});
