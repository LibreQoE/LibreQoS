// Provides a generic dashboard system for use in LibreQoS and Insight
import {DashboardLayout} from "./layout";
import {resetWS, subscribeWS} from "./ws";
import {heading5Icon} from "../helpers/content_builders";
import {openDashboardEditor} from "./dashboard_editor";

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

        // Set up the div
        this.parentDiv = document.getElementById(this.divName);
        if (this.parentDiv === null) {
            throw new Error("No element found with the id '" + this.divName + "'");
        }

        // Content
        this.dashlets = [];
        this.layout = new DashboardLayout(cookieName, defaultLayout);
        this.dashletIdentities = this.layout.dashlets;

        // Data Wrangling
        this.channels = [];

        // Auto Refresh Handling
        this.paused = false;
        let cadence = localStorage.getItem("dashCadence");
        if (cadence === null) {
            this.cadence = 1;
            localStorage.setItem("dashCadence", this.cadence.toString());
        } else {
            this.cadence = parseInt(cadence);
        }
        this.tickCounter = 0;
    }


    build() {
        this.#filterWidgetList();
        let childDivs = this.#buildWidgetChildDivs();
        this.childIds = [];
        childDivs.forEach((d) => { this.childIds.push(d.id); });
        this.#clearRenderedDashboard();
        childDivs.forEach((d) => { this.parentDiv.appendChild(d) });
        this.#buildChannelList(this.dashlets);
        this.#webSocketSubscription();
    }

    #filterWidgetList() {
        this.dashlets = [];
        for (let i=0; i<this.dashletIdentities.length; i++) {
            let widget = this.widgetFactory(this.dashletIdentities[i].tag, i);
            if (widget == null) continue; // Skip build
            widget.size = this.dashletIdentities[i].size;
            this.dashlets.push(widget);
        }
    }

    #buildWidgetChildDivs() {
        let childDivs = [];
        for (let i=0; i<this.dashlets.length; i++) {
            let div = this.dashlets[i].buildContainer();
            childDivs.push(div);
        }
        return childDivs;
    }

    #clearRenderedDashboard() {
        while (this.parentDiv.children.length > 1) {
            this.parentDiv.removeChild(this.parentDiv.lastChild);
        }
    }

    #buildChannelList() {
        this.channels = [];
        for (let i=0; i<this.dashlets.length; i++) {
            let channels = this.dashlets[i].subscribeTo();
            for (let j=0; j<channels.length; j++) {
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

    #webSocketSubscription() {
        if (this.channels.length === 0) {
            // We're not in WS mode
            for (let i=0; i<this.dashlets.length; i++) {
                this.dashlets[i].setupOnce({});
            }
            return;
        }
        resetWS();
        subscribeWS(this.channels, (msg) => {
            if (msg.event === "join") {
                // The DOM will be present now, setup events
                for (let i=0; i<this.dashlets.length; i++) {
                    this.dashlets[i].setupOnce(msg);
                }
            } else {
                // Propagate the message
                if (!this.paused) {
                    this.tickCounter++;
                    this.tickCounter %= this.cadence;
                    for (let i = 0; i < this.dashlets.length; i++) {
                        if (this.dashlets[i].canBeSlowedDown()) {
                            if (this.tickCounter === 0) {
                                this.dashlets[i].onMessage(msg);
                            }
                        } else {
                            this.dashlets[i].onMessage(msg);
                        }
                    }
                }
            }
        });
    }
}