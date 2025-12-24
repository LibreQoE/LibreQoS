const CURRENT_VERSION = 3;

export class DashboardLayout {
    constructor(cookieName, defaultLayout) {
        this.cookieName = cookieName;
        // Keep a normalized copy of the default (tabs + dashlets only)
        this._defaultTabs = DashboardLayout.normalizeToTabs(defaultLayout);
        let template = localStorage.getItem(cookieName);
        if (template !== null) {
            let parsed = JSON.parse(template);
            // Check if it's the new format with version
            if (parsed.version >= 2) {
                this.version = parsed.version;
                this.tabs = Array.isArray(parsed.tabs) ? parsed.tabs : [];
                this.activeTab = parsed.activeTab || 0;
            } else {
                // Old format saved in localStorage (pre-tabs)
                // Treat as a signal to reset to the new default layout,
                // so users see the new Overview tab instead of a single-tab conversion.
                this.version = CURRENT_VERSION;
                if (defaultLayout && defaultLayout.version >= 2) {
                    this.tabs = defaultLayout.tabs || [];
                    this.activeTab = defaultLayout.activeTab || 0;
                } else {
                    this.tabs = [{
                        name: "Dashboard",
                        dashlets: defaultLayout || []
                    }];
                    this.activeTab = 0;
                }
                try { localStorage.removeItem(this.cookieName); } catch (e) {}
            }
        } else {
            // No saved layout - use default
            if (defaultLayout && defaultLayout.version >= 2) {
                // New format default
                this.version = defaultLayout.version;
                this.tabs = defaultLayout.tabs || [];
                this.activeTab = defaultLayout.activeTab || 0;
            } else {
                // Old format default - convert to new format
                this.version = CURRENT_VERSION;
                this.tabs = [{
                    name: "Dashboard",
                    dashlets: defaultLayout || []
                }];
                this.activeTab = 0;
            }
        }
    }

    save(layoutData) {
        // Support both old style (just dashlets array) and new style (full layout object)
        if (Array.isArray(layoutData)) {
            // Old style - convert to new format
            this.tabs[this.activeTab].dashlets = layoutData;
        } else {
            // New style - full layout object
            this.version = layoutData.version || CURRENT_VERSION;
            this.tabs = layoutData.tabs || [];
            this.activeTab = layoutData.activeTab || 0;
        }
        // Only persist if layout differs from default tabs/dashlets
        const isDefault = DashboardLayout.tabsEqual(this.tabs, this._defaultTabs);
        if (isDefault) {
            // Ensure default users don't retain a saved copy that blocks future defaults
            localStorage.removeItem(this.cookieName);
            return;
        }

        const template = JSON.stringify({
            version: this.version,
            tabs: this.tabs,
            activeTab: this.activeTab
        });
        localStorage.setItem(this.cookieName, template);
    }

    // Get all dashlets across all tabs (for backward compatibility)
    get dashlets() {
        let allDashlets = [];
        this.tabs.forEach(tab => {
            allDashlets = allDashlets.concat(tab.dashlets);
        });
        return allDashlets;
    }

    // Get dashlets for current active tab
    getCurrentTabDashlets() {
        return this.tabs[this.activeTab]?.dashlets || [];
    }
}

// Static helpers
DashboardLayout.normalizeToTabs = function(defaultLayout) {
    if (!defaultLayout) return [];
    if (defaultLayout.version >= 2 && Array.isArray(defaultLayout.tabs)) {
        return defaultLayout.tabs.map(t => ({
            name: t.name || "Dashboard",
            dashlets: Array.isArray(t.dashlets) ? t.dashlets.map(d => ({ tag: d.tag, size: d.size })) : []
        }));
    }
    // Old format: list of dashlets
    if (Array.isArray(defaultLayout)) {
        return [{ name: "Dashboard", dashlets: defaultLayout.map(d => ({ tag: d.tag, size: d.size })) }];
    }
    return [];
};

DashboardLayout.tabsEqual = function(tabsA, tabsB) {
    if (!Array.isArray(tabsA) || !Array.isArray(tabsB)) return false;
    if (tabsA.length !== tabsB.length) return false;
    for (let i = 0; i < tabsA.length; i++) {
        const a = tabsA[i];
        const b = tabsB[i];
        const aDash = Array.isArray(a.dashlets) ? a.dashlets : [];
        const bDash = Array.isArray(b.dashlets) ? b.dashlets : [];
        if (aDash.length !== bDash.length) return false;
        for (let j = 0; j < aDash.length; j++) {
            const ad = aDash[j] || {};
            const bd = bDash[j] || {};
            if (ad.tag !== bd.tag || Number(ad.size) !== Number(bd.size)) {
                return false;
            }
        }
    }
    return true;
};
