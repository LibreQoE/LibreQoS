// Provides a generic dashboard system for use in LibreQoS and Insight
import {DashboardLayout} from "./layout";
import {get_ws_client} from "../../pubsub/ws";
import {heading5Icon} from "../helpers/content_builders";
import {openDashboardEditor} from "./dashboard_editor";

const DIAGNOSTIC_CHANNELS = new Set(["Cpu", "Ram", "RttHistogram"]);

export class Dashboard {
    // Takes the target DIV in which to build the dashboard,
    // and the name of the dashboard for cookie and builder
    // purposes
    constructor(divName, cookieName, defaultLayout, widgetFactory, dashletMenu, hasCadence = true, savedDashUrl = "/local-api/dashletThemes") {
        this.divName = divName;
        this.cookieName = cookieName;
        window.cookieName = cookieName;
        this.widgetFactory = widgetFactory;
        this.dashletMenu = dashletMenu;
        this.savedDashUrl = savedDashUrl;
        this.hasCadence = hasCadence;

        // Set up the div
        this.parentDiv = document.getElementById(this.divName);
        if (this.parentDiv === null) {
            throw new Error("No element found with the id '" + this.divName + "'");
        }

        // Editor Support
        this.#editButton(hasCadence);
        if (localStorage.getItem("forceEditMode")) {
            localStorage.removeItem("forceEditMode");
            requestAnimationFrame(() => {
                setTimeout(() => {
                    this.editMode();
                })
            });
        }

        // Content
        this.dashlets = [];
        this.tabDashlets = {}; // dashlets organized by tab
        this.layout = new DashboardLayout(cookieName, defaultLayout);
        // During edit mode, this holds the active tab's dashlets being edited
        this.dashletIdentities = [];

        // Data Wrangling
        this.channels = [];
        this.tabChannels = {};
        this.pendingImmediateDashlets = new Set();
        this.wsClient = get_ws_client();
        this.channelDisposers = new Map();
        this.subscribedChannels = new Set();
        this.lastMessages = new Map();

        // Auto Refresh Handling
        this.paused = false;
        if (!this.hasCadence) {
            this.cadence = 1;
        } else {
            let cadence = localStorage.getItem("dashCadence");
            if (cadence === null) {
                this.cadence = 1;
                localStorage.setItem("dashCadence", this.cadence.toString());
            } else {
                const parsedCadence = parseInt(cadence);
                this.cadence = Number.isFinite(parsedCadence) && parsedCadence > 0 ? parsedCadence : 1;
            }
        }
        this.tickCounter = 0;
    }

    #urlDebugEnabled() {
        try {
            const params = new URLSearchParams(window.location.search || "");
            if (!params.has("debug")) {
                return false;
            }
            const value = (params.get("debug") || "").trim().toLowerCase();
            return value === "" || value === "1" || value === "true" || value === "dashboard";
        } catch (_) {
            return false;
        }
    }

    #debugEnabled() {
        return localStorage.getItem("debugDashboard") === "1" || this.#urlDebugEnabled();
    }

    #debugDumpEnabled() {
        return this.#urlDebugEnabled();
    }

    #debug(event, details = {}) {
        if (!this.#debugEnabled()) {
            return;
        }
        if (!window.__lqosDashboardTrace) {
            window.__lqosDashboardTrace = [];
        }
        const entry = {
            ts: new Date().toISOString(),
            event,
            ...details,
        };
        window.__lqosDashboardTrace.push(entry);
        if (window.__lqosDashboardTrace.length > 1000) {
            window.__lqosDashboardTrace.shift();
        }
        console.debug("[dashboard]", entry);
    }

    #markDashletState(dashlet, patch = {}) {
        if (!dashlet) {
            return;
        }
        if (!dashlet.__dashboardDebugState) {
            dashlet.__dashboardDebugState = {};
        }
        Object.assign(dashlet.__dashboardDebugState, patch);
        const el = document.getElementById(dashlet.id);
        if (!el) {
            return;
        }
        if (dashlet.__dashboardDebugState.lastSetupAt) {
            el.dataset.debugLastSetupAt = dashlet.__dashboardDebugState.lastSetupAt;
        }
        if (dashlet.__dashboardDebugState.lastMessageAt) {
            el.dataset.debugLastMessageAt = dashlet.__dashboardDebugState.lastMessageAt;
        }
        if (dashlet.__dashboardDebugState.lastEvent) {
            el.dataset.debugLastEvent = dashlet.__dashboardDebugState.lastEvent;
        }
        if (dashlet.__dashboardDebugState.lastReplayAt) {
            el.dataset.debugLastReplayAt = dashlet.__dashboardDebugState.lastReplayAt;
        }
        if (dashlet.__dashboardDebugState.lastReplayEvent) {
            el.dataset.debugLastReplayEvent = dashlet.__dashboardDebugState.lastReplayEvent;
        }
    }

    #paneStateSnapshot(pane) {
        if (!pane) {
            return {
                hasPane: false,
            };
        }
        const rect = typeof pane.getBoundingClientRect === "function"
            ? pane.getBoundingClientRect()
            : { width: 0, height: 0 };
        const computed = window.getComputedStyle ? window.getComputedStyle(pane) : null;
        return {
            hasPane: true,
            paneId: pane.id,
            paneClientWidth: pane.clientWidth,
            paneClientHeight: pane.clientHeight,
            paneRectWidth: rect.width,
            paneRectHeight: rect.height,
            paneDisplay: computed ? computed.display : "",
            paneVisibility: computed ? computed.visibility : "",
        };
    }

    #dashletDebugSnapshot(dashlet) {
        let channels = [];
        if (dashlet && typeof dashlet.subscribeTo === "function") {
            const subscribed = dashlet.subscribeTo();
            channels = Array.isArray(subscribed) ? [...subscribed] : [subscribed];
        }
        return {
            id: dashlet?.id || "",
            title: dashlet?.title || "",
            tag: dashlet?.tag || "",
            tabIndex: dashlet?.tabIndex,
            keepSubscribedWhenHidden: !!dashlet?.keepSubscribedWhenHidden?.(),
            renderWhenHidden: !!dashlet?.renderWhenHidden?.(),
            subscribed: this.#dashletShouldStayActive(dashlet),
            renderable: dashlet?.tabIndex === this.layout.activeTab || !!dashlet?.renderWhenHidden?.(),
            channels,
            debugState: dashlet?.__dashboardDebugState || {},
        };
    }

    #downloadDebugDump() {
        const tabs = Array.from(document.querySelectorAll(`#${this.divName}_tabs .nav-link`)).map((tab, index) => ({
            index,
            title: tab.textContent ? tab.textContent.trim() : "",
            active: tab.classList.contains("active"),
        }));
        const panes = Array.from(document.querySelectorAll(`#${this.divName}_tab_content .tab-pane`)).map((pane, index) => ({
            index,
            classes: Array.from(pane.classList),
            ...this.#paneStateSnapshot(pane),
        }));
        const desiredChannels = this.wsClient?.desiredChannels instanceof Map
            ? Array.from(this.wsClient.desiredChannels.keys())
            : [];
        const subscribedChannels = Array.from(this.subscribedChannels);
        const dump = {
            generatedAt: new Date().toISOString(),
            href: window.location.href,
            userAgent: navigator.userAgent,
            activeTab: this.layout.activeTab,
            cadence: this.cadence,
            paused: this.paused,
            desiredChannels,
            subscribedChannels,
            lastMessageChannels: Array.from(this.lastMessages.keys()),
            tabs,
            panes,
            dashlets: this.dashlets.map((dashlet) => this.#dashletDebugSnapshot(dashlet)),
            dashboardTrace: window.__lqosDashboardTrace || [],
            wsTrace: window.__lqosDashboardWsTrace || [],
        };
        const stamp = dump.generatedAt.replace(/[:.]/g, "-");
        const fileName = `${this.cookieName || "dashboard"}-debug-${stamp}.json`;
        const blob = new Blob([JSON.stringify(dump, null, 2)], {type: "application/json"});
        const url = URL.createObjectURL(blob);
        const link = document.createElement("a");
        link.href = url;
        link.download = fileName;
        document.body.appendChild(link);
        link.click();
        document.body.removeChild(link);
        setTimeout(() => URL.revokeObjectURL(url), 1000);
        this.#debug("debug-dump-download", {
            fileName,
            subscribedChannels,
            desiredChannels,
        });
    }


    build() {
        this.destroy();
        window.__lqosDashboardDashletHook = (entry) => {
            this.#debug("dashlet-hook", entry);
        };
        window.__lqosDashboardRenderHook = (entry) => {
            this.#debug("render-hook", entry);
        };
        this.#filterWidgetList();
        this.#buildTabUI();
        this.#buildTabContents();
        this.#buildChannelList(this.dashlets);
        const renderableDashlets = this.#renderableDashlets();
        this.#debug("build", {
            activeTab: this.layout.activeTab,
            channels: this.channels,
            renderableDashlets: renderableDashlets.map((d) => d.id),
        });
        for (let i = 0; i < renderableDashlets.length; i++) {
            renderableDashlets[i].setupOnce({});
            this.#markDashletState(renderableDashlets[i], {
                lastSetupAt: new Date().toISOString(),
            });
        }
        this.#queueImmediateDashlets(renderableDashlets);
        this.#syncWebSocketChannels();
        this.#replayCachedMessages(renderableDashlets);
    }

    destroy() {
        this.#disposeWebSocketChannels();
        if (window.__lqosDashboardDashletHook) {
            delete window.__lqosDashboardDashletHook;
        }
        if (window.__lqosDashboardRenderHook) {
            delete window.__lqosDashboardRenderHook;
        }
        for (let i = 0; i < this.dashlets.length; i++) {
            if (this.dashlets[i] && this.dashlets[i].dispose) {
                this.dashlets[i].dispose();
            }
        }
        this.dashlets = [];
        this.tabDashlets = {};
        this.channels = [];
        this.tabChannels = {};
        this.childIds = [];
        this.pendingImmediateDashlets.clear();
        this.lastMessages.clear();
        this.#clearRenderedDashboard();
    }

    #buildTabUI() {
        // Create tab navigation
        let tabNav = document.createElement('ul');
        tabNav.classList.add('nav', 'nav-tabs', 'mb-3');
        tabNav.id = this.divName + '_tabs';
        
        this.layout.tabs.forEach((tab, index) => {
            let li = document.createElement('li');
            li.classList.add('nav-item');
            
            let a = document.createElement('a');
            a.classList.add('nav-link');
            if (index === this.layout.activeTab) {
                a.classList.add('active');
            }
            a.href = '#';
            a.innerText = tab.name;
            a.onclick = (e) => {
                e.preventDefault();
                this.switchTab(index);
            };
            
            li.appendChild(a);
            tabNav.appendChild(li);
        });
        
        this.parentDiv.appendChild(tabNav);
    }

    #buildTabContents() {
        // Create tab content container
        let tabContent = document.createElement('div');
        tabContent.classList.add('tab-content');
        tabContent.id = this.divName + '_tab_content';
        
        this.childIds = [];
        
        this.layout.tabs.forEach((tab, tabIndex) => {
            let tabPane = document.createElement('div');
            tabPane.classList.add('tab-pane', 'row');
            tabPane.id = this.divName + '_tab_' + tabIndex;
            
            if (tabIndex === this.layout.activeTab) {
                tabPane.classList.add('active');
                tabPane.style.display = 'flex';
            } else {
                tabPane.style.display = 'none';
            }
            
            // Build dashlets for this tab
            let tabDashlets = this.tabDashlets[tabIndex] || [];
            tabDashlets.forEach(dashlet => {
                let div = dashlet.buildContainer();
                this.childIds.push(div.id);
                tabPane.appendChild(div);
            });
            
            tabContent.appendChild(tabPane);
        });
        
        this.parentDiv.appendChild(tabContent);
    }

    switchTab(tabIndex) {
        if (tabIndex === this.layout.activeTab) return;
        const previousTabIndex = this.layout.activeTab;
        this.#debug("switch-tab", {
            from: previousTabIndex,
            to: tabIndex,
        });

        // Update active tab in layout
        this.layout.activeTab = tabIndex;
        this.layout.save(this.layout);
        
        // Update tab navigation
        let tabs = document.querySelectorAll('#' + this.divName + '_tabs .nav-link');
        tabs.forEach((tab, index) => {
            if (index === tabIndex) {
                tab.classList.add('active');
            } else {
                tab.classList.remove('active');
            }
        });
        
        // Update tab content visibility
        let tabPanes = document.querySelectorAll('#' + this.divName + '_tab_content .tab-pane');
        tabPanes.forEach((pane, index) => {
            if (index === tabIndex) {
                pane.classList.add('active');
                pane.style.display = 'flex';
                this.#debug("tab-pane-visible", {
                    tabIndex,
                    ...this.#paneStateSnapshot(pane),
                });
                let tabDashlets = this.tabDashlets[index] || [];
                const skipReplayDashletIds = new Set();
                tabDashlets.forEach((dashlet) => {
                    dashlet.setupOnce({});
                    this.#markDashletState(dashlet, {
                        lastSetupAt: new Date().toISOString(),
                    });
                });
                tabDashlets.forEach((dashlet) => {
                    if (dashlet.flushBackgroundMessages && dashlet.flushBackgroundMessages()) {
                        skipReplayDashletIds.add(dashlet.id);
                    }
                });
                this.#queueImmediateDashlets(tabDashlets);
                this.#replayCachedMessages(tabDashlets, skipReplayDashletIds);
                // Notify dashlets in this tab that they're now visible
                tabDashlets.forEach(dashlet => {
                    if (dashlet.onTabActivated) {
                        dashlet.onTabActivated();
                    }
                });
                
                // Resize all ECharts instances in the newly visible tab
                setTimeout(() => {
                    this.#debug("tab-pane-resize-pass", {
                        tabIndex,
                        ...this.#paneStateSnapshot(pane),
                    });
                    const charts = pane.querySelectorAll('.dashgraph, .dashgraphZoomed');
                    charts.forEach(chartDiv => {
                        if (typeof echarts !== 'undefined') {
                            const chart = echarts.getInstanceByDom(chartDiv);
                            if (chart) {
                                chart.resize();
                            }
                        }
                    });
                }, 0);
            } else {
                pane.classList.remove('active');
                pane.style.display = 'none';
            }
        });

        const previousDashlets = this.tabDashlets[previousTabIndex] || [];
        previousDashlets.forEach((dashlet) => {
            if (dashlet.onTabDeactivated) {
                dashlet.onTabDeactivated();
            }
        });

        this.#buildChannelList();
        this.#syncWebSocketChannels();
    }

    #filterWidgetList() {
        this.dashlets = [];
        this.tabDashlets = {};
        let globalIndex = 0;
        
        // Process each tab
        this.layout.tabs.forEach((tab, tabIndex) => {
            this.tabDashlets[tabIndex] = [];
            const dashlets = Array.isArray(tab.dashlets) ? tab.dashlets : [];
            tab.dashlets = dashlets;

            dashlets.forEach((dashletDef) => {
                let widget = this.widgetFactory(dashletDef.tag, globalIndex);
                if (widget == null) return; // Skip build
                
                widget.size = dashletDef.size;
                widget.tabIndex = tabIndex;
                
                this.dashlets.push(widget);
                this.tabDashlets[tabIndex].push(widget);
                globalIndex++;
            });
        });
    }


    #clearRenderedDashboard() {
        while (this.parentDiv.children.length > 0) {
            this.parentDiv.removeChild(this.parentDiv.lastChild);
        }
    }

    #dashletShouldStayActive(dashlet) {
        return dashlet.keepSubscribedWhenHidden() || dashlet.tabIndex === this.layout.activeTab;
    }

    #subscribedDashlets() {
        return this.dashlets.filter((dashlet) => this.#dashletShouldStayActive(dashlet));
    }

    #renderableDashlets() {
        return this.dashlets.filter(
            (dashlet) => dashlet.tabIndex === this.layout.activeTab || dashlet.renderWhenHidden(),
        );
    }

    #backgroundDashlets() {
        return this.#subscribedDashlets().filter(
            (dashlet) =>
                dashlet.tabIndex !== this.layout.activeTab
                && dashlet.keepSubscribedWhenHidden()
                && !dashlet.renderWhenHidden(),
        );
    }

    #queueImmediateDashlets(dashlets) {
        if (!Array.isArray(dashlets) || this.cadence <= 1) {
            return;
        }
        for (let i = 0; i < dashlets.length; i++) {
            const dashlet = dashlets[i];
            if (dashlet && dashlet.canBeSlowedDown()) {
                this.pendingImmediateDashlets.add(dashlet.id);
                this.#debug("queue-immediate", {
                    dashletId: dashlet.id,
                });
            }
        }
    }

    #debugInterestingChannels(context, channels) {
        if (!Array.isArray(channels)) {
            return;
        }
        const interesting = channels.filter((channel) => DIAGNOSTIC_CHANNELS.has(channel));
        if (interesting.length === 0) {
            return;
        }
        this.#debug("interesting-channels", {
            context,
            channels: interesting,
            activeTab: this.layout.activeTab,
            renderableDashlets: this.#renderableDashlets().map((d) => d.id),
            subscribedDashlets: this.#subscribedDashlets().map((d) => d.id),
        });
    }

    #dashletSubscribedToMessage(dashlet, eventName) {
        const channels = dashlet.subscribeTo();
        if (Array.isArray(channels)) {
            return channels.includes(eventName);
        }
        return channels === eventName;
    }

    #replayCachedMessages(dashlets, skipDashletIds = new Set()) {
        if (!Array.isArray(dashlets)) {
            return;
        }
        for (let i = 0; i < dashlets.length; i++) {
            const dashlet = dashlets[i];
            if (skipDashletIds.has(dashlet.id)) {
                continue;
            }
            const channels = dashlet.subscribeTo();
            if (!Array.isArray(channels)) {
                if (channels && this.lastMessages.has(channels)) {
                    dashlet.onMessage(this.lastMessages.get(channels));
                    this.#markDashletState(dashlet, {
                        lastReplayAt: new Date().toISOString(),
                        lastReplayEvent: channels,
                    });
                    this.#debug("replay-cached", {
                        dashletId: dashlet.id,
                        eventName: channels,
                    });
                    this.pendingImmediateDashlets.delete(dashlet.id);
                }
                continue;
            }
            for (let j = 0; j < channels.length; j++) {
                const channel = channels[j];
                if (this.lastMessages.has(channel)) {
                    dashlet.onMessage(this.lastMessages.get(channel));
                    this.#markDashletState(dashlet, {
                        lastReplayAt: new Date().toISOString(),
                        lastReplayEvent: channel,
                    });
                    this.#debug("replay-cached", {
                        dashletId: dashlet.id,
                        eventName: channel,
                    });
                    this.pendingImmediateDashlets.delete(dashlet.id);
                }
            }
        }
    }

    #buildChannelList() {
        this.channels = [];
        this.tabChannels = {};
        this.layout.tabs.forEach((_, tabIndex) => {
            this.tabChannels[tabIndex] = [];
        });

        for (let i = 0; i < this.dashlets.length; i++) {
            const dashlet = this.dashlets[i];
            const channels = dashlet.subscribeTo();
            const tabChannels = this.tabChannels[dashlet.tabIndex] || [];
            for (let j = 0; j < channels.length; j++) {
                if (!tabChannels.includes(channels[j])) {
                    tabChannels.push(channels[j]);
                }
            }
            this.tabChannels[dashlet.tabIndex] = tabChannels;
            if (!this.#dashletShouldStayActive(dashlet)) {
                continue;
            }
            for (let j = 0; j < channels.length; j++) {
                if (!this.#alreadySubscribed(channels[j])) {
                    this.channels.push(channels[j]);
                }
            }
        }
    }

    #alreadySubscribed(name) {
        for (let i=0; i<this.channels.length; i++) {
            if (this.channels[i] === name) {
                return true;
            }
        }
        return false;
    }

    #disposeWebSocketChannels() {
        this.#debug("dispose-channels", {
            channels: Array.from(this.subscribedChannels),
        });
        if (this.subscribedChannels.size > 0) {
            this.wsClient.unsubscribe(Array.from(this.subscribedChannels));
        }
        for (const disposer of this.channelDisposers.values()) {
            disposer();
        }
        this.channelDisposers.clear();
        this.subscribedChannels.clear();
    }

    #syncWebSocketChannels() {
        const hadEstablishedSubscription =
            !!this.wsClient.ws && this.wsClient.handshake_done && this.subscribedChannels.size > 0;
        const nextChannels = new Set(this.channels);
        const addedChannels = [];
        const removedChannels = [];

        for (const channel of nextChannels) {
            if (!this.subscribedChannels.has(channel)) {
                addedChannels.push(channel);
            }
        }
        for (const channel of this.subscribedChannels) {
            if (!nextChannels.has(channel)) {
                removedChannels.push(channel);
            }
        }

        for (let i = 0; i < removedChannels.length; i++) {
            const channel = removedChannels[i];
            const disposer = this.channelDisposers.get(channel);
            if (disposer) {
                disposer();
                this.channelDisposers.delete(channel);
            }
            this.subscribedChannels.delete(channel);
        }
        if (removedChannels.length > 0) {
            this.wsClient.unsubscribe(removedChannels);
        }
        if (removedChannels.length > 0) {
            this.#debug("channels-removed", {
                channels: removedChannels,
            });
        }
        this.#debugInterestingChannels("removed", removedChannels);

        for (let i = 0; i < addedChannels.length; i++) {
            const channel = addedChannels[i];
            const disposer = this.wsClient.on(channel, (msg) => {
                this.#handleWebSocketMessage(msg);
            });
            this.channelDisposers.set(channel, disposer);
            this.subscribedChannels.add(channel);
        }
        if (addedChannels.length > 0) {
            this.wsClient.subscribe(addedChannels);
        }
        if (hadEstablishedSubscription && addedChannels.length > 0) {
            this.wsClient.refreshSubscriptions("dashboard-channel-add");
        }
        this.#debugInterestingChannels("added", addedChannels);
        this.#debug("channels-sync", {
            added: addedChannels,
            removed: removedChannels,
            subscribed: Array.from(this.subscribedChannels),
        });
    }

    #handleWebSocketMessage(msg) {
        if (!msg || !msg.event || this.paused) {
            return;
        }

        this.lastMessages.set(msg.event, msg);
        this.tickCounter++;
        this.tickCounter %= this.cadence;
        const renderableDashlets = this.#renderableDashlets();
        const backgroundDashlets = this.#backgroundDashlets();
        this.#debug("message", {
            eventName: msg.event,
            tickCounter: this.tickCounter,
            renderableDashlets: renderableDashlets.map((d) => d.id),
            backgroundDashlets: backgroundDashlets.map((d) => d.id),
            subscribedDashlets: this.#subscribedDashlets().map((d) => d.id),
        });
        if (DIAGNOSTIC_CHANNELS.has(msg.event)) {
            this.#debug("interesting-message", {
                eventName: msg.event,
                activeTab: this.layout.activeTab,
                renderableDashlets: renderableDashlets.map((d) => d.id),
                backgroundDashlets: backgroundDashlets.map((d) => d.id),
                subscribedDashlets: this.#subscribedDashlets().map((d) => d.id),
            });
        }

        for (let i = 0; i < renderableDashlets.length; i++) {
            const dashlet = renderableDashlets[i];
            if (!this.#dashletSubscribedToMessage(dashlet, msg.event)) {
                continue;
            }
            const shouldDeliverImmediately =
                this.pendingImmediateDashlets.has(dashlet.id);
            if (dashlet.canBeSlowedDown()) {
                if (shouldDeliverImmediately || this.tickCounter === 0) {
                    dashlet.onMessage(msg);
                    this.#markDashletState(dashlet, {
                        lastMessageAt: new Date().toISOString(),
                        lastEvent: msg.event,
                    });
                    this.#debug("deliver", {
                        dashletId: dashlet.id,
                        eventName: msg.event,
                        immediate: shouldDeliverImmediately,
                        slowed: true,
                    });
                    this.pendingImmediateDashlets.delete(dashlet.id);
                }
            } else {
                dashlet.onMessage(msg);
                this.#markDashletState(dashlet, {
                    lastMessageAt: new Date().toISOString(),
                    lastEvent: msg.event,
                });
                this.#debug("deliver", {
                    dashletId: dashlet.id,
                    eventName: msg.event,
                    immediate: false,
                    slowed: false,
                });
            }
        }

        for (let i = 0; i < backgroundDashlets.length; i++) {
            const dashlet = backgroundDashlets[i];
            if (!this.#dashletSubscribedToMessage(dashlet, msg.event)) {
                continue;
            }
            dashlet.onBackgroundMessage(msg);
            this.#debug("background-deliver", {
                dashletId: dashlet.id,
                eventName: msg.event,
            });
        }
    }

    #editButton(hasCadence) {
        let parent = document.getElementById("controls");
        if (parent === null) return;
        let editDiv = document.createElement("span");
        editDiv.id = this.divName + "_edit";
        editDiv.innerHTML = "<button type='button' class='btn btn-secondary btn-sm' id='btnEditDash'><i class='fa fa-pencil'></i> Edit</button>";
        editDiv.onclick = () => {
            // New Editor - pass the full layout with tabs
            let availableElements = [];
            this.dashletMenu.forEach((d) => {
                let category = "Uncategorized";
                if (d.category !== null) category = d.category;
                availableElements.push({
                    name: d.name,
                    size: d.size,
                    tag: d.tag,
                    category,
                });
            });

            openDashboardEditor(this.layout, availableElements, (newLayout) => {
                console.log("New dashboard layout:", newLayout);
                this.layout.save(newLayout);
                window.location.reload();
            }, this.cookieName);
        };

        // Cadence Picker
        let cadenceDiv = document.createElement("div");
        cadenceDiv.id = this.divName + "_cadence";
        let cadenceLabel = document.createElement("label");
        cadenceLabel.htmlFor = "cadencePicker";
        cadenceLabel.innerText = "Refresh Rate: ";
        let cadencePicker = document.createElement("input");
        cadencePicker.id = "cadencePicker";
        cadencePicker.type = "number";
        cadencePicker.min = "1";
        cadencePicker.max = "60";
        cadencePicker.value = this.cadence;
        cadencePicker.onchange = () => {
            const parsedCadence = parseInt(cadencePicker.value);
            this.cadence = Number.isFinite(parsedCadence) && parsedCadence > 0 ? parsedCadence : 1;
            localStorage.setItem("dashCadence", this.cadence.toString());
        }
        cadenceDiv.appendChild(cadenceLabel);
        cadenceDiv.appendChild(cadencePicker);

        // Pause Button
        let pauseDiv = document.createElement("span");
        pauseDiv.id = this.divName + "_pause";
        pauseDiv.innerHTML = "<button type='button' class='btn btn-secondary btn-sm ms-2'><i class='fa fa-pause'></i> Pause</button>";
        pauseDiv.onclick = () => {
            this.paused = !this.paused;
            let target = document.getElementById(this.divName + "_pause");
            if (this.paused) {
                target.innerHTML = "<button type='button' class='btn btn-secondary btn-sm'><i class='fa fa-play'></i> Resume</button>";
            } else {
                target.innerHTML = "<button type='button' class='btn btn-secondary btn-sm'><i class='fa fa-pause'></i> Pause</button>";
            }
        };

        let debugDumpDiv = null;
        if (this.#debugDumpEnabled()) {
            debugDumpDiv = document.createElement("span");
            debugDumpDiv.id = this.divName + "_debug_dump";
            debugDumpDiv.innerHTML = "<button type='button' class='btn btn-outline-secondary btn-sm ms-2'><i class='fa fa-download'></i> Debug Dump</button>";
            debugDumpDiv.onclick = () => {
                this.#downloadDebugDump();
            };
        }

        parent.appendChild(editDiv);
        if (debugDumpDiv !== null) {
            parent.appendChild(debugDumpDiv);
        }
        if (hasCadence) {
            parent.appendChild(pauseDiv);
            parent.appendChild(cadenceDiv);
        }
    }

    editMode() {
        // Insert a Temporary Div to hold edit options
        let editDiv = document.createElement("div");
        editDiv.classList.add("col-12", "bg-secondary-subtle", "mb-2");
        editDiv.id = "dashboardEditingDiv";
        editDiv.style.padding = "10px";
        editDiv.style.borderRadius = "5px";
        editDiv.style.marginLeft = "0";
        let toasts = document.getElementById("toasts");
        this.editingDashboard = true;
        // Work on a copy of the active tab's dashlets so indices align with visible widgets
        this.dashletIdentities = JSON.parse(JSON.stringify(this.#getActiveTabDashlets()));

        // Add the editing elements
        let row = document.createElement("div");
        row.classList.add("row");

        let c1 = document.createElement("div");
        c1.classList.add("col-3");
        c1.appendChild(heading5Icon("gear", "Dashboard Options"));
        let nuke = document.createElement("button");
        nuke.type = "button";
        nuke.classList.add("btn", "btn-sm", "btn-secondary");
        nuke.innerHTML = "<i class='fa fa-trash'></i> Remove All";
        nuke.onclick = () => { this.removeAll(); };
        c1.appendChild(nuke);
        let filler = document.createElement("button");
        filler.type = "button";
        filler.classList.add("btn", "btn-sm", "btn-secondary");
        filler.innerHTML = "<i class='fa fa-plus-square'></i> Add All";
        filler.onclick = () => { this.addAll(); };
        filler.style.marginLeft = "5px";
        c1.appendChild(filler);

        let c2 = document.createElement("div");
        c2.classList.add("col-3");
        c2.appendChild(heading5Icon("plus", "Add Dashboard Item"));
        let list = document.createElement("div");
        list.classList.add("dropdown");
        list.id = "dropdown-widgets";
        let listBtn = document.createElement("button");
        listBtn.type = "button";
        listBtn.classList.add("btn", "btn-secondary", "btn-sm", "dropdown-toggle");
        listBtn.setAttribute("data-bs-toggle", "dropdown");
        listBtn.innerHTML = "<i class='fa fa-plus'></i> Add Widget";
        list.appendChild(listBtn);
        let listUl = document.createElement("ul");
        listUl.classList.add("dropdown-menu", "dropdown-menu-sized");
        this.dashletMenu.forEach((d) => {
            let entry = document.createElement("li");
            let item = document.createElement("a");
            item.classList.add("dropdown-item");
            item.innerText = d.name;
            let myTag = d.tag;
            item.onclick = () => {
                let didSomething = false;
                this.dashletMenu.forEach((d) => {
                    if (d.tag === myTag) {
                        this.dashletIdentities.push({ tag: d.tag, size: d.size });
                        didSomething = true;
                    }
                });
                if (didSomething) {
                    this.#replaceDashletList();
                }
            };
            entry.appendChild(item);
            listUl.appendChild(entry);
        });
        list.appendChild(listUl);
        c2.appendChild(list);

        let c3 = document.createElement("div");
        c3.classList.add("col-3");
        c3.appendChild(heading5Icon("save", "Save Layout"));

        const wsClient = get_ws_client();
        const listenOnce = (eventName, handler) => {
            const wrapped = (msg) => {
                wsClient.off(eventName, wrapped);
                handler(msg);
            };
            wsClient.on(eventName, wrapped);
        };

        let saveGroup = document.createElement("div");
        saveGroup.classList.add("input-group");
        let saveAppend = document.createElement("div");
        saveAppend.classList.add("input-group-append");

        let lbl = document.createElement("label");
        lbl.htmlFor = "saveDashName";
        let saveDashName = document.createElement("input");
        saveDashName.id = "saveDashName";
        saveDashName.type = "text";
        saveDashName.classList.add("form-control", "border-0", "small");
        let saveBtn = document.createElement("button");
        saveBtn.type = "button";
        saveBtn.classList.add("btn", "btn-secondary", "btn-sm");
        saveBtn.innerHTML = "<i class='fa fa-save'></i>";
        saveBtn.style.marginLeft = "4px";
        saveBtn.onclick = () => {
            let name = $("#saveDashName").val();
            if (name.length < 1) return;
            listenOnce("DashletSaveResult", (msg) => {
                if (!msg || !msg.ok) {
                    alert(msg && msg.error ? msg.error : "Failed to save dashboard layout");
                    return;
                }
                localStorage.setItem("forceEditMode", "true");
                window.location.reload();
            });
            wsClient.send({
                DashletSave: {
                    name: name,
                    entries: this.dashletIdentities
                }
            });
        }
        saveGroup.appendChild(saveDashName);
        saveAppend.appendChild(saveBtn);
        saveGroup.appendChild(saveAppend);
        c3.appendChild(saveGroup);

        let c4 = document.createElement("div");
        c4.classList.add("col-3");
        c4.appendChild(heading5Icon("save", "Saved Layouts"));
        listenOnce("DashletThemes", (data) => {
            let list = document.createElement("ul");
            list.classList.add("list-group", "list-group-numbered");
            data.entries.forEach((d) => {
                let i = document.createElement("li");
                i.classList.add("list-group-item","list-group-item-action");
                let ln = document.createElement("a");
                ln.href = "#";
                ln.innerHTML = "<i class='fa fa-save'></i> " + d.name;
                ln.onclick = () => {
                    let resp = confirm("Load [" + d.name + "] from [" + d.path + "]?");
                    if (resp) {
                        listenOnce("DashletTheme", (x) => {
                            if (!x || !x.entries) {
                                alert("Failed to load dashboard layout");
                                return;
                            }
                            this.dashletIdentities = x.entries;
                            this.layout.save(this.dashletIdentities);
                            localStorage.setItem("forceEditMode", "true");
                            window.location.reload();
                        });
                        wsClient.send({ DashletGet: { name: d.name } });
                    }
                };
                let dl = document.createElement("a");
                dl.classList.add("badge","text-bg-danger");
                dl.style.float = "right";
                dl.href = "#";
                dl.innerHTML = "<i class='fa fa-trash'></i> Delete";
                dl.onclick = () => {
                    let resp = confirm("Delete [" + d.name + "] from [" + d.path + "]?");
                    if (resp) {
                        listenOnce("DashletDeleteResult", (msg) => {
                            if (!msg || !msg.ok) {
                                alert(msg && msg.error ? msg.error : "Failed to delete dashboard layout");
                                return;
                            }
                            localStorage.setItem("forceEditMode", "true");
                            window.location.reload();
                        });
                        wsClient.send({ DashletDelete: { name: d.name } });
                    }
                }
                i.appendChild(ln);
                i.appendChild(dl);
                list.appendChild(i);
            });
            c4.appendChild(list);
        });
        wsClient.send({ DashletThemes: {} });

        row.appendChild(c1);
        row.appendChild(c2);
        row.appendChild(c3);
        row.appendChild(c4);
        editDiv.appendChild(row);
        toasts.appendChild(editDiv);

        // Insert the inline edit buttons
        let editDivParent = document.getElementById(this.childIds[0]);
        if (editDivParent != null) {
            let div = document.getElementById(this.childIds[0]);
            let clientRect = div.getClientRects()[0];
            let clientLeft = (clientRect.left - 10) + "px";
            let clientTop  = (clientRect.top - 20) + "px";
            for (let i = 0; i < this.dashlets.length; i++) {
                let div = document.getElementById(this.childIds[i]);
                if (div !== null) {
                    let clientRect = div.getClientRects()[0];
                    let clientTop  = (clientRect.top - 20) + "px";
                    let clientLeft = (clientRect.left + 10) + "px";
                    let clientMiddleX = (clientRect.left + 80) + "px";
                    let clientMiddleY = (clientRect.top + 18) + "px";

                    // Up Button
                    let myI = i;
                    editDiv.appendChild(this.#dashEditButton(
                        clientLeft,
                        clientTop,
                        "arrow-up",
                        "secondary",
                        () => { if (myI > 0) this.clickUp(myI); }
                    ));

                    // Down Button
                    let myI2 = i;
                    editDiv.appendChild(this.#dashEditButton(
                        (clientRect.left + 20) + "px",
                        clientTop,
                        "arrow-down",
                        "secondary",
                        () => { if (myI2 < (this.dashlets.length - 1)) this.clickDown(myI2); }
                    ));

                    // Trash Button
                    let myI3 = i;
                    editDiv.appendChild(this.#dashEditButton(
                        clientMiddleX,
                        clientMiddleY,
                        "trash",
                        "danger",
                        () => {
                            this.clickTrash(myI3);
                        }
                    ));

                    // Expand Button
                    let myI4 = i;
                    editDiv.appendChild(this.#dashEditButton(
                        clientLeft,
                        clientTop,
                        "plus-circle",
                        "secondary",
                        () => {
                            this.zoomIn(myI4);
                        }
                    ));

                    // Contract Button
                    let myI5 = i;
                    editDiv.appendChild(this.#dashEditButton(
                        (clientRect.left + 40) + "px",
                        clientTop,
                        "minus-circle",
                        "secondary",
                        () => {
                            this.zoomOut(myI5);
                        }
                    ));
                } else {
                    console.log("Warning: NULL div found in dashlet list");
                }
            }

            editDivParent.appendChild(editDiv);
        }
    }

    #dashEditButton(left, top, iconSuffix, style, closure) {
        let div = document.createElement("div");
        div.style.position = "absolute";
        div.style.width = "20px";
        div.style.zIndex = "200";
        div.style.height = "20px";
        div.style.top = top;
        div.style.left = left;
        div.classList.add("dashEditButton");
        let button = document.createElement("button");
        button.type = "button";
        button.classList.add("btn", "btn-sm", "btn-" + style);
        button.innerHTML = "<i class='fa fa-" + iconSuffix +"'></i>";
        button.onclick = closure;
        div.appendChild(button);
        return div;
    }

    closeEditMode() {
        let editor = document.getElementById("dashboardEditingDiv");
        if (editor != null) {
            editor.remove();
        }
        this.editingDashboard = false;
    }

    clickUp(i) {
        let toMove = this.dashletIdentities[i];
        let toReplace = this.dashletIdentities[i-1];
        this.dashletIdentities[i-1] = toMove;
        this.dashletIdentities[i] = toReplace;
        this.#replaceDashletList();
    }

    clickDown(i) {
        let toMove = this.dashletIdentities[i];
        let toReplace = this.dashletIdentities[i+1];
        this.dashletIdentities[i+1] = toMove;
        this.dashletIdentities[i] = toReplace;
        this.#replaceDashletList();
    }

    clickTrash(i) {
        this.dashletIdentities.splice(i, 1);
        this.#replaceDashletList();
    }

    zoomIn(i) {
        if (this.dashletIdentities[i].size < 12) {
            this.dashletIdentities[i].size += 1;
        }
        this.#replaceDashletList();
    }

    zoomOut(i) {
        if (this.dashletIdentities[i].size > 1) {
            this.dashletIdentities[i].size -= 1;
        }
        this.#replaceDashletList();
    }

    removeAll() {
        this.dashletIdentities = [];
        this.#replaceDashletList();
    }

    addAll() {
        // Add all known widgets into current tab as tag/size pairs
        this.dashletIdentities = this.dashletMenu.map(d => ({ tag: d.tag, size: d.size }));
        this.#replaceDashletList();
    }

    #getActiveTabDashlets() {
        const t = this.layout.activeTab || 0;
        if (this.layout.tabs && this.layout.tabs[t] && Array.isArray(this.layout.tabs[t].dashlets)) {
            return this.layout.tabs[t].dashlets;
        }
        return [];
    }

    #replaceDashletList() {
        // Persist the current tab’s dashlets and reload to apply
        const t = this.layout.activeTab || 0;
        if (this.layout.tabs && this.layout.tabs[t]) {
            this.layout.tabs[t].dashlets = this.dashletIdentities;
        }
        this.layout.save(this.layout);
        localStorage.setItem("forceEditMode", "true");
        window.location.reload();
    }
}
