/**
 * Shows StormGuard detail tabs only when StormGuard is available and prevents focus from
 * remaining in a tab strip that has just been hidden.
 */
export function setTreeDetailTabsVisibility(document, bootstrap, visible) {
    const tabsContainer = document.getElementById("treeDetailTabsContainer");
    const stormguardTabItem = document.getElementById("treeStormguardTabItem");
    const stormguardTab = document.getElementById("tree-stormguard-tab");
    const overviewTab = document.getElementById("tree-overview-tab");
    const overviewPane = document.getElementById("treeOverviewPane");
    const stormguardPane = document.getElementById("treeStormguardPane");
    const focusWillBeHidden = !visible
        && (tabsContainer?.contains(document.activeElement) || stormguardPane?.contains(document.activeElement));

    stormguardTabItem?.classList.toggle("d-none", !visible);
    if (!visible) {
        if (stormguardTab?.classList.contains("active")) {
            bootstrap?.Tab.getOrCreateInstance(overviewTab)?.show();
        }
        tabsContainer?.classList.add("d-none");
        if (focusWillBeHidden) overviewPane?.focus();
        return;
    }
    tabsContainer?.classList.remove("d-none");
}
