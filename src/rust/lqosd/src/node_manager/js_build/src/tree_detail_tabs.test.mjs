import assert from "node:assert/strict";
import test from "node:test";
import {setTreeDetailTabsVisibility} from "./tree_detail_tabs.mjs";

function fakeElement() {
    const classes = new Set();
    const element = {
        classList: {
            add: (name) => classes.add(name),
            remove: (name) => classes.delete(name),
            contains: (name) => classes.has(name),
            toggle: (name, force) => {
                if (force) classes.add(name);
                else classes.delete(name);
            },
        },
        contains: (candidate) => candidate?.parent === element,
        focusCount: 0,
        focus() {
            this.focusCount += 1;
        },
    };
    return element;
}

function treeDocument(activeElement, stormguardTabActive = false) {
    const tabsContainer = fakeElement();
    const stormguardTabItem = fakeElement();
    const stormguardTab = fakeElement();
    const overviewTab = fakeElement();
    const overviewPane = fakeElement();
    const stormguardPane = fakeElement();
    if (stormguardTabActive) stormguardTab.classList.add("active");
    const elements = {
        treeDetailTabsContainer: tabsContainer,
        treeStormguardTabItem: stormguardTabItem,
        "tree-stormguard-tab": stormguardTab,
        "tree-overview-tab": overviewTab,
        treeOverviewPane: overviewPane,
        treeStormguardPane: stormguardPane,
    };
    return {
        document: {
            activeElement,
            getElementById: (id) => elements[id] || null,
        },
        elements,
    };
}

test("hiding StormGuard tabs moves tab focus to the visible overview pane", () => {
    const focusedTab = {parent: null};
    const {document, elements} = treeDocument(focusedTab, true);
    focusedTab.parent = elements.treeDetailTabsContainer;
    let overviewWasActivated = false;

    setTreeDetailTabsVisibility(document, {
        Tab: {getOrCreateInstance: () => ({show: () => { overviewWasActivated = true; }})},
    }, false);

    assert.equal(elements.treeDetailTabsContainer.classList.contains("d-none"), true);
    assert.equal(elements.treeStormguardTabItem.classList.contains("d-none"), true);
    assert.equal(elements.treeOverviewPane.focusCount, 1);
    assert.equal(overviewWasActivated, true);
});

test("hiding StormGuard tabs does not steal focus from visible content", () => {
    const {document, elements} = treeDocument({parent: null});

    setTreeDetailTabsVisibility(document, null, false);

    assert.equal(elements.treeDetailTabsContainer.classList.contains("d-none"), true);
    assert.equal(elements.treeOverviewPane.focusCount, 0);
});

test("showing StormGuard reveals the full tab strip", () => {
    const {document, elements} = treeDocument({parent: null});
    elements.treeDetailTabsContainer.classList.add("d-none");
    elements.treeStormguardTabItem.classList.add("d-none");

    setTreeDetailTabsVisibility(document, null, true);

    assert.equal(elements.treeDetailTabsContainer.classList.contains("d-none"), false);
    assert.equal(elements.treeStormguardTabItem.classList.contains("d-none"), false);
});
