// Unit tests for the HTML escapers the render templates use on server text.
// The mutation they guard against is a raw `${value}` inside an `innerHTML`
// template, which no source-regex contract can see.

import assert from "node:assert/strict";
import test from "node:test";

import {escapeAttr, escapeHtml} from "./escape.mjs";

// One value per escaped character, in the order the implementation replaces them.
const RAW = `a<b>c"d'e&f`;
const ESCAPED = "a&lt;b&gt;c&quot;d&apos;e&amp;f";

test("both escapers neutralise the five HTML-significant characters", () => {
    for (const [name, escape] of [["escapeHtml", escapeHtml], ["escapeAttr", escapeAttr]]) {
        assert.equal(escape(RAW), ESCAPED, name);
    }
});

test("an escaped value cannot close the attribute or element it lands in", () => {
    const hostile = `"><img src=x onerror="alert(1)">`;
    const escaped = escapeAttr(hostile);
    assert.doesNotMatch(escaped, /[<>"']/, "no raw delimiter may survive escaping");
    assert.match(escaped, /^&quot;&gt;&lt;img src=x onerror=&quot;alert\(1\)&quot;&gt;$/);
});

test("a missing value renders as the empty string, not the word undefined", () => {
    for (const escape of [escapeHtml, escapeAttr]) {
        assert.equal(escape(undefined), "");
        assert.equal(escape(null), "");
        assert.equal(escape(0), "0");
        assert.equal(escape(42), "42");
    }
});

test("there is one encoding of the apostrophe", () => {
    // `tHtml()` imports this escaper, so a catalog message and a parameter take
    // the same path; a second implementation with `&#39;` would differ in the
    // attribute the shell's markup carries.
    assert.equal(escapeHtml("it's"), "it&apos;s");
    assert.doesNotMatch(escapeHtml("it's"), /&#39;/);
});
