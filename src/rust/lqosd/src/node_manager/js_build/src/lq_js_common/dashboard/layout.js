const CURRENT_VERSION = 3;

export class DashboardLayout {
    constructor(cookieName, defaultLayout) {
        this.cookieName = cookieName;
        this.activeTabStorageKey = `${cookieName}.activeTabId`;
        // Keep a normalized copy of the default (tabs + dashlets only)
        this._defaultTabs = DashboardLayout.normalizeToTabs(defaultLayout);
        const persistedActiveTabId = this.loadPersistedActiveTabId();
        let template = localStorage.getItem(cookieName);
        if (template !== null) {
            let parsed = JSON.parse(template);
            // Check if it's the new format with version
            if (parsed.version >= 2) {
                this.version = parsed.version;
                this.tabs = DashboardLayout.mergeMissingDefaultTabs(
                    Array.isArray(parsed.tabs) ? parsed.tabs : [],
                    this._defaultTabs
                );
                this.activeTab = DashboardLayout.resolveActiveTabIndex(
                    this.tabs,
                    parsed.activeTab,
                    persistedActiveTabId
                );
            } else {
                // Old format saved in localStorage (pre-tabs)
                // Treat as a signal to reset to the new default layout,
                // so users see the new Overview tab instead of a single-tab conversion.
                this.version = CURRENT_VERSION;
                if (defaultLayout && defaultLayout.version >= 2) {
                    this.tabs = DashboardLayout.normalizeToTabs(defaultLayout);
                    this.activeTab = DashboardLayout.resolveActiveTabIndex(
                        this.tabs,
                        defaultLayout.activeTab,
                        persistedActiveTabId
                    );
                } else {
                    this.tabs = DashboardLayout.normalizeToTabs(defaultLayout);
                    this.activeTab = DashboardLayout.resolveActiveTabIndex(
                        this.tabs,
                        0,
                        persistedActiveTabId
                    );
                }
                try { localStorage.removeItem(this.cookieName); } catch (e) {}
            }
        } else {
            // No saved layout - use default
            if (defaultLayout && defaultLayout.version >= 2) {
                // New format default
                this.version = defaultLayout.version;
                this.tabs = DashboardLayout.normalizeToTabs(defaultLayout);
                this.activeTab = DashboardLayout.resolveActiveTabIndex(
                    this.tabs,
                    defaultLayout.activeTab,
                    persistedActiveTabId
                );
            } else {
                // Old format default - convert to new format
                this.version = CURRENT_VERSION;
                this.tabs = DashboardLayout.normalizeToTabs(defaultLayout);
                this.activeTab = DashboardLayout.resolveActiveTabIndex(
                    this.tabs,
                    0,
                    persistedActiveTabId
                );
            }
        }
        this.persistActiveTabId();
    }

    save(layoutData) {
        // Support both old style (just dashlets array) and new style (full layout object)
        if (Array.isArray(layoutData)) {
            // Old style - convert to new format
            this.tabs[this.activeTab].dashlets = layoutData;
        } else {
            // New style - full layout object
            this.version = layoutData.version || CURRENT_VERSION;
            this.tabs = DashboardLayout.normalizeTabs(layoutData.tabs || []);
            this.activeTab = DashboardLayout.resolveActiveTabIndex(
                this.tabs,
                layoutData.activeTab,
                this.getActiveTabId()
            );
        }
        this.persistActiveTabId();
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

    getActiveTabId() {
        return this.tabs[this.activeTab]?.id || null;
    }

    loadPersistedActiveTabId() {
        try {
            return localStorage.getItem(this.activeTabStorageKey);
        } catch (e) {
            return null;
        }
    }

    persistActiveTabId() {
        try {
            const activeTabId = this.getActiveTabId();
            if (activeTabId) {
                localStorage.setItem(this.activeTabStorageKey, activeTabId);
            } else {
                localStorage.removeItem(this.activeTabStorageKey);
            }
        } catch (e) {}
    }
}

// Static helpers
DashboardLayout.normalizeToTabs = function(defaultLayout) {
    if (!defaultLayout) return [];
    if (defaultLayout.version >= 2 && Array.isArray(defaultLayout.tabs)) {
        return DashboardLayout.normalizeTabs(defaultLayout.tabs);
    }
    // Old format: list of dashlets
    if (Array.isArray(defaultLayout)) {
        return [DashboardLayout.normalizeTab({
            id: "dashboard",
            name: "Dashboard",
            dashlets: defaultLayout
        })];
    }
    return [];
};

DashboardLayout.makeTabId = function(name = "tab") {
    const base = String(name || "tab")
        .trim()
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, "-")
        .replace(/^-+|-+$/g, "") || "tab";
    return `${base}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
};

DashboardLayout.normalizeTab = function(tab, fallbackName = "Dashboard", fallbackId = null) {
    const name = tab?.name || fallbackName;
    return {
        id: tab?.id || fallbackId || DashboardLayout.makeTabId(name),
        name,
        dashlets: Array.isArray(tab?.dashlets)
            ? tab.dashlets.map((dashlet) => ({ tag: dashlet.tag, size: dashlet.size }))
            : []
    };
};

DashboardLayout.normalizeTabs = function(tabs) {
    if (!Array.isArray(tabs) || tabs.length === 0) {
        return [];
    }
    return tabs.map((tab, index) =>
        DashboardLayout.normalizeTab(tab, `Tab ${index + 1}`)
    );
};

DashboardLayout.resolveActiveTabIndex = function(tabs, requestedIndex, requestedTabId) {
    if (!Array.isArray(tabs) || tabs.length === 0) {
        return 0;
    }

    if (requestedTabId) {
        const indexById = tabs.findIndex((tab) => tab?.id === requestedTabId);
        if (indexById >= 0) {
            return indexById;
        }
    }

    const numericIndex = Number.parseInt(requestedIndex, 10);
    if (Number.isFinite(numericIndex) && numericIndex >= 0 && numericIndex < tabs.length) {
        return numericIndex;
    }

    return 0;
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

DashboardLayout.mergeMissingDefaultTabs = function(existingTabs, defaultTabs) {
    if (!Array.isArray(existingTabs) || existingTabs.length === 0) {
        return Array.isArray(defaultTabs) ? DashboardLayout.normalizeTabs(defaultTabs) : [];
    }

    const normalizedExisting = DashboardLayout.normalizeTabs(existingTabs);
    if (!Array.isArray(defaultTabs) || defaultTabs.length === 0) {
        return normalizedExisting;
    }

    const merged = normalizedExisting.slice();
    const knownIds = new Set(normalizedExisting.map((tab) => tab?.id || ""));
    const knownNames = new Set(normalizedExisting.map((tab) => tab?.name || ""));
    defaultTabs.forEach((tab, index) => {
        const normalizedTab = DashboardLayout.normalizeTab(tab, `Tab ${index + 1}`);
        if (
            !normalizedTab.name
            || knownIds.has(normalizedTab.id)
            || knownNames.has(normalizedTab.name)
        ) {
            return;
        }
        knownIds.add(normalizedTab.id);
        knownNames.add(normalizedTab.name);
        merged.push(normalizedTab);
    });
    return merged;
};
