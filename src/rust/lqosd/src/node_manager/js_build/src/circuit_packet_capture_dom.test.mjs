import assert from "node:assert/strict";
import test from "node:test";

import {
    appendRedactableText,
    setIconText,
    setPacketCaptureDownloadButton,
} from "./circuit_packet_capture_dom.mjs";

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
        this.attributes = {};
        this._textContent = "";
    }

    appendChild(child) {
        this.children.push(child);
        return child;
    }

    removeChild(child) {
        const index = this.children.indexOf(child);
        if (index >= 0) {
            this.children.splice(index, 1);
        }
        return child;
    }

    get firstChild() {
        return this.children[0] ?? null;
    }

    setAttribute(name, value) {
        this.attributes[name] = String(value);
    }

    set textContent(value) {
        this._textContent = String(value);
    }

    get textContent() {
        return this._textContent;
    }

    set innerHTML(_) {
        throw new Error("packet-capture helpers must not use innerHTML");
    }
}

class FakeTextNode {
    constructor(text) {
        this.textContent = String(text);
    }
}

function installFakeDocument() {
    globalThis.document = {
        createElement: (tagName) => new FakeElement(tagName),
        createTextNode: (textValue) => new FakeTextNode(textValue),
    };
}

test("packet-capture menu text is rendered as DOM text", () => {
    installFakeDocument();
    const link = new FakeElement("a");
    const address = "<img src=x onerror=alert(1)>";

    setIconText(link, ["fa", "fa-search"], "Capture packets from ");
    appendRedactableText(link, address);

    assert.equal(link.children.length, 4);
    assert.equal(link.children[0].tagName, "I");
    assert.equal(link.children[0].attributes["aria-hidden"], "true");
    assert.deepEqual(link.children[0].classList.values, ["fa", "fa-search"]);
    assert.equal(link.children[2].textContent, "Capture packets from ");
    assert.equal(link.children[3].tagName, "SPAN");
    assert.equal(link.children[3].textContent, address);
    assert.equal(link.children[3].classList.contains("redactable"), true);
});

test("packet-capture download button resets children without innerHTML", () => {
    installFakeDocument();
    const button = new FakeElement("button");
    const address = "<svg onload=alert(1)>";
    button.appendChild(new FakeTextNode("old label"));

    setPacketCaptureDownloadButton(button, address);

    assert.equal(button.children.length, 4);
    assert.equal(button.children[0].tagName, "I");
    assert.deepEqual(button.children[0].classList.values, ["fa", "fa-download"]);
    assert.equal(button.children[2].textContent, "Download Packet Capture for ");
    assert.equal(button.children[3].textContent, address);
});
