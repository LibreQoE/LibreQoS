// Pure helper for the dashboard layout: drop persisted tabs whose dashlets are
// ALL no longer in the live dashlet catalog (a retired surface — e.g. the
// K3-retired Bakery tab).  A tab with at least one known dashlet is kept.
//
// Why this exists: the main dashboard persists its layout in localStorage, and
// DashboardLayout.mergeMissingDefaultTabs only ADDS missing default tabs — it
// never prunes a tab whose dashlets were removed.  Without this, a browser that
// saved the old layout keeps rendering the dead tab after the dashlets are
// deleted.
export function pruneTabsWithoutKnownDashlets(tabs, knownTags) {
    const list = Array.isArray(tabs) ? tabs : [];
    if (!(knownTags instanceof Set) || knownTags.size === 0) {
        return list;
    }
    return list.filter((tab) => {
        const dashlets = Array.isArray(tab?.dashlets) ? tab.dashlets : [];
        return dashlets.some((dashlet) => dashlet && knownTags.has(dashlet.tag));
    });
}
