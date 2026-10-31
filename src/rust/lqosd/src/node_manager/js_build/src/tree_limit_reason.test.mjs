import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {dirname, resolve} from "node:path";
import test from "node:test";
import {fileURLToPath} from "node:url";

import {
    buildNodeLimitSummary,
    configuredMax,
    effectiveMax,
    immediateParentNodeFromTree,
    ratePairsMatch,
    ratesApproximatelyEqual,
    renderEffectiveNowDisplay,
    renderLimitedByDisplay,
} from "./tree_limit_reason.mjs";

const __dirname = dirname(fileURLToPath(import.meta.url));

class FakeElement {
    constructor(ownerDocument, tagName) {
        this.ownerDocument = ownerDocument;
        this.tagName = tagName;
        this.attributes = {};
        this.children = [];
        this.textContent = "";
        this.classNames = [];
        this.classList = {
            add: (...names) => {
                this.classNames.push(...names);
            },
        };
    }

    get lastChild() {
        return this.children[this.children.length - 1] || null;
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

    setAttribute(name, value) {
        this.attributes[name] = String(value);
    }

    getAttribute(name) {
        return this.attributes[name] ?? null;
    }

    matches(selector) {
        return selector === '[data-bs-toggle="tooltip"]'
            && this.attributes["data-bs-toggle"] === "tooltip";
    }

    querySelectorAll(selector) {
        const matches = [];
        const visit = (element) => {
            if (element.matches?.(selector)) {
                matches.push(element);
            }
            element.children?.forEach(visit);
        };
        this.children.forEach(visit);
        return matches;
    }
}

const fakeDocument = {
    createElement(tagName) {
        return new FakeElement(fakeDocument, tagName);
    },
};

function node({
    name = "Selected",
    configured = [500, 100],
    effective = configured,
    activeAttachment = null,
} = {}) {
    return {
        name,
        configured_max_throughput: configured,
        max_throughput: configured,
        effective_max_throughput: effective,
        active_attachment_name: activeAttachment,
    };
}

test("rate helpers fall back from effective to configured rates", () => {
    const selected = {
        max_throughput: [100, 50],
    };

    assert.deepEqual(configuredMax(selected), [100, 50]);
    assert.deepEqual(effectiveMax(selected), [100, 50]);
});

test("summary identifies parent-limited nodes", () => {
    const selected = node({configured: [500, 100], effective: [294, 60]});
    const parent = node({name: "7232 Rochester", configured: [294, 60], effective: [294, 60]});

    const summary = buildNodeLimitSummary({node: selected, parentNode: parent});

    assert.equal(summary.label, "Parent");
    assert.equal(summary.kind, "parent");
    assert.equal(summary.compactReason, "Est. Parent");
    assert.match(summary.title, /Download: inherited from parent 7232 Rochester at 294M\./);
    assert.match(summary.title, /Upload: inherited from parent 7232 Rochester at 60M\./);
});

test("summary notes active topology parent overrides", () => {
    const selected = node({
        configured: [500, 100],
        effective: [294, 60],
        activeAttachment: "Backhaul A",
    });
    const parent = node({name: "Old Parent", configured: [294, 60], effective: [294, 60]});

    const summary = buildNodeLimitSummary({
        node: selected,
        parentNode: parent,
        topologyOverrideData: {
            has_override: true,
            override_parent_node_ids: ["parent-1"],
            current_parent_node_id: "parent-1",
            current_parent_node_name: "Pinned Parent",
        },
    });

    assert.equal(summary.label, "Parent");
    assert.match(summary.title, /Pinned Parent selected by topology override/);
    assert.match(summary.title, /Active attachment: Backhaul A\./);
});

test("summary identifies operator rate overrides", () => {
    const selected = node({configured: [500, 100], effective: [250, 80]});

    const summary = buildNodeLimitSummary({
        node: selected,
        rateOverrideData: {
            has_override: true,
            override_download_bandwidth_mbps: 250,
            override_upload_bandwidth_mbps: 80,
        },
    });

    assert.equal(summary.label, "Override");
    assert.match(summary.title, /Download: matches operator rate override at 250M\./);
    assert.match(summary.title, /Upload: matches operator rate override at 80M\./);
});

test("summary identifies attachment caps when no parent or override explains the rate", () => {
    const selected = node({
        configured: [500, 100],
        effective: [400, 80],
        activeAttachment: "Backhaul A",
    });

    const summary = buildNodeLimitSummary({node: selected});

    assert.equal(summary.label, "Attachment");
    assert.match(summary.title, /Download: capped by active attachment Backhaul A at 400M\./);
    assert.match(summary.title, /Upload: capped by active attachment Backhaul A at 80M\./);
});

test("summary reports mixed causes when directions differ", () => {
    const selected = node({configured: [500, 100], effective: [294, 80]});
    const parent = node({name: "Parent Node", configured: [294, 100], effective: [294, 100]});

    const summary = buildNodeLimitSummary({
        node: selected,
        parentNode: parent,
        rateOverrideData: {
            has_override: true,
            override_download_bandwidth_mbps: 400,
            override_upload_bandwidth_mbps: 80,
        },
    });

    assert.equal(summary.label, "Mixed");
    assert.equal(summary.kind, "mixed");
    assert.equal(summary.compactReason, "Est. Mixed");
    assert.equal(summary.directions[0].kind, "parent");
    assert.equal(summary.directions[1].kind, "override");
});

test("summary uses explicit compact label for unexplained queue caps", () => {
    const selected = node({configured: [500, 100], effective: [450, 90]});

    const summary = buildNodeLimitSummary({node: selected});

    assert.equal(summary.label, "Queue");
    assert.equal(summary.kind, "queue");
    assert.equal(summary.compactReason, "Est. Queue");
});

test("summary does not treat a missing override direction as overridden", () => {
    const selected = node({configured: [500, 100], effective: [500, 80]});

    const summary = buildNodeLimitSummary({
        node: selected,
        rateOverrideData: {
            has_override: true,
            override_download_bandwidth_mbps: null,
            override_upload_bandwidth_mbps: 80,
        },
    });

    assert.equal(summary.label, "Mixed");
    assert.equal(summary.directions[0].kind, "base");
    assert.equal(summary.directions[1].kind, "override");
    assert.match(summary.directions[0].detail, /Download: uses configured rate at 500M\./);
});

test("summary uses base when effective and configured rates match", () => {
    const selected = node({configured: [294, 60], effective: [294, 60]});

    const summary = buildNodeLimitSummary({node: selected});

    assert.equal(summary.label, "Base");
    assert.equal(summary.kind, "base");
    assert.equal(summary.compactReason, "Base rate");
    assert.match(summary.title, /Download: uses configured rate at 294M\./);
});

test("rate comparison boundary is shared and explicit", () => {
    assert.equal(ratesApproximatelyEqual(100, 100.009), true);
    assert.equal(ratesApproximatelyEqual(100, 100.011), false);
    assert.equal(ratePairsMatch([100, 50], [100.009, 50]), true);
    assert.equal(ratePairsMatch([100, 50], [100.011, 50]), false);
});

test("parent lookup follows the selected node's immediate parent index", () => {
    const parent = node({name: "Parent", configured: [300, 60]});
    const selected = {
        ...node({name: "Child", configured: [500, 100]}),
        immediate_parent: 2,
    };
    const tree = [
        [0, node({name: "Root"})],
        [1, selected],
        [2, parent],
    ];

    assert.equal(immediateParentNodeFromTree(tree, selected), parent);
    assert.equal(immediateParentNodeFromTree(tree, {...selected, immediate_parent: null}), null);
    assert.equal(immediateParentNodeFromTree(tree, {...selected, immediate_parent: 99}), null);
});

test("effective rate renderer keeps the rate value compact", () => {
    const target = new FakeElement(fakeDocument, "dd");

    renderEffectiveNowDisplay({
        target,
        effective: [100, 50],
        formatRatePair: (down, up) => `${down}M / ${up}M`,
    });

    const wrap = target.children[0];

    assert.deepEqual(wrap.classNames, ["lqos-tree-detail-value", "lqos-tree-effective-now"]);
    assert.equal(wrap.children.length, 0);
    assert.equal(wrap.textContent, "100M / 50M");
    assert.equal(target.attributes["data-lqos-render-key"], "effective:100M / 50M");
});

test("limited-by renderer builds accessible source detail", () => {
    const target = new FakeElement(fakeDocument, "dd");
    const summary = {
        kind: "parent",
        label: "Parent",
        compactReason: "Est. Parent",
        title: "Download: inherited from parent Alpha at 100M. Upload: inherited from parent Alpha at 50M.",
    };

    renderLimitedByDisplay({
        target,
        summary,
    });

    const source = target.children[0];
    const value = source.children[0];
    const icon = source.children[1];

    assert.deepEqual(source.classNames, ["lqos-tree-limit-source"]);
    assert.equal(source.children.length, 2);
    assert.equal(value.textContent, "Est. Parent");
    assert.equal(icon.attributes["aria-hidden"], "true");
    assert.equal(source.attributes["data-bs-toggle"], "tooltip");
    assert.equal(source.attributes["data-bs-trigger"], "hover focus");
    assert.equal(source.attributes["data-bs-container"], "body");
    assert.equal(source.attributes.tabindex, "0");
    assert.equal(source.attributes.title, `Estimated effective-rate source. ${summary.title}`);
    assert.equal(source.attributes["aria-label"], `Estimated effective-rate source. ${summary.title}`);
    assert.equal(
        target.attributes["data-lqos-render-key"],
        `limited:Est. Parent:Estimated effective-rate source. ${summary.title}`,
    );
});

test("limited-by renderer preserves stable tooltip state between refreshes", () => {
    const target = new FakeElement(fakeDocument, "dd");
    const summary = {
        kind: "parent",
        label: "Parent",
        compactReason: "Est. Parent",
        title: "Download: inherited from parent Alpha at 100M. Upload: inherited from parent Alpha at 50M.",
    };
    const args = {
        target,
        summary,
    };

    renderLimitedByDisplay(args);
    const oldSource = target.children[0];
    renderLimitedByDisplay(args);

    assert.equal(target.children.length, 1);
    assert.equal(target.children[0], oldSource);
});

test("limited-by renderer disposes stale tooltip state when the source changes", () => {
    const target = new FakeElement(fakeDocument, "dd");
    const summary = {
        kind: "parent",
        label: "Parent",
        compactReason: "Est. Parent",
        title: "Download: inherited from parent Alpha at 100M. Upload: inherited from parent Alpha at 50M.",
    };

    renderLimitedByDisplay({target, summary});
    const oldSource = target.children[0];
    let disposed = false;
    globalThis.bootstrap = {
        Tooltip: {
            getInstance(el) {
                assert.equal(el, oldSource);
                return {
                    dispose() {
                        disposed = true;
                    },
                };
            },
        },
    };

    try {
        renderLimitedByDisplay({
            target,
            summary: {
                ...summary,
                compactReason: "Est. Override",
                title: "Download: matches operator rate override at 100M. Upload: matches operator rate override at 50M.",
            },
        });
    } finally {
        delete globalThis.bootstrap;
    }

    assert.equal(disposed, true);
    assert.equal(target.children.length, 1);
});

test("shared rate helpers preserve configured and effective fallback behavior", () => {
    const selected = {
        max_throughput: [100, 50],
        effective_max_throughput: [90, 40],
    };

    assert.deepEqual(configuredMax(selected), [100, 50]);
    assert.deepEqual(effectiveMax(selected), [90, 40]);
});

test("tree details markup keeps compact definition grid IDs", () => {
    const html = readFileSync(resolve(__dirname, "../../static2/tree.html"), "utf8");

    assert.match(html, /<dl class="lqos-tree-detail-grid">/);
    const expected = [
        "nodeSettingsBaseConfigured",
        "nodeSettingsEffectiveNow",
        "nodeSettingsOverride",
        "nodeSettingsLimitedBy",
        "nodeTopologyOverrideValue",
        "nodeSettingsActiveAttachment",
    ];
    expected.forEach((id) => {
        assert.match(html, new RegExp(`id="${id}"`));
    });
    assert.match(
        html,
        /<div class="lqos-tree-detail-item lqos-tree-detail-col-start">\s*<dt class="lqos-tree-detail-label">Base Configured Rate<\/dt>\s*<dd class="lqos-tree-detail-body" id="nodeSettingsBaseConfigured">-/,
    );
    assert.match(
        html,
        /<div class="lqos-tree-detail-item lqos-tree-detail-row-end">\s*<dt class="lqos-tree-detail-label">Effective Now<\/dt>\s*<dd class="lqos-tree-detail-body" id="nodeSettingsEffectiveNow">-/,
    );
    assert.match(
        html,
        /<div class="lqos-tree-detail-item lqos-tree-detail-row-end">\s*<dt class="lqos-tree-detail-label">Limited By<\/dt>\s*<dd class="lqos-tree-detail-body" id="nodeSettingsLimitedBy">-/,
    );
    assert.match(
        html,
        /<div class="lqos-tree-detail-item lqos-tree-detail-row-end lqos-tree-detail-last-row lqos-tree-detail-final-cell">\s*<dt class="lqos-tree-detail-label">Active Attachment<\/dt>\s*<dd class="lqos-tree-detail-body" id="nodeSettingsActiveAttachment">-/,
    );
    const positions = expected.map((id) => html.indexOf(`id="${id}"`));
    assert.deepEqual([...positions].sort((a, b) => a - b), positions);
});

test("tree details CSS keeps desktop detail values aligned in four columns", () => {
    const css = readFileSync(resolve(__dirname, "../../static2/node_manager.css"), "utf8");

    assert.match(css, /\.lqos-tree-detail-grid\s*{[^}]*grid-template-columns:\s*minmax\(8\.5rem, max-content\) minmax\(0, 1fr\)\s*minmax\(8\.5rem, max-content\) minmax\(0, 1fr\);/s);
    assert.match(css, /\.lqos-tree-detail-item\s*{\s*display: contents;\s*}/);
});
