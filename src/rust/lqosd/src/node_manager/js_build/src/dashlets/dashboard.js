import {subscribeWS} from "../pubsub/ws";
import {ThroughputBpsDash} from "./throughput_bps_dash";
import {ThroughputPpsDash} from "./throughput_pps_dash";
import {ShapedUnshapedDash} from "./shaped_unshaped_dash";
import {TrackedFlowsCount} from "./tracked_flow_count_dash";
import {ThroughputRingDash} from "./throughput_ring_dash";
import {RttHistoDash} from "./rtt_histo_dash";


export class Dashboard {
    // Takes the name of the parent div to start building the dashboard
    constructor(divName) {
        this.divName = divName;
        this.parentDiv = document.getElementById(divName);
        if (this.parentDiv === null) {
            console.log("Dashboard parent not found");
        }
        let layout = new Layout();
        this.dashletIdentities = layout.dashlets;
        this.dashlets = [];
        this.channels = [];

        // Build the edit button
        let editDiv = document.createElement("div");
        editDiv.id = this.divName + "_edit";
        editDiv.style.position = "fixed";
        editDiv.style.right = "5px";
        editDiv.style.top = "5px";
        editDiv.style.width = "40px";
        editDiv.style.zIndex = "100";
        editDiv.innerHTML = "<button type='button' class='btn btn-secondary btn-sm'><i class='fa fa-pencil'></i></button>";
        editDiv.onclick = () => {
            this.editMode();
        };
        this.parentDiv.appendChild(editDiv);
    }

    build() {
        // Get the widget order, filtering invalid
        for (let i=0; i<this.dashletIdentities.length; i++) {
            let widget = this.#factory(i);
            if (widget == null) continue; // Skip build
            this.dashlets.push(widget);
        }

        // Build the widgets and get the channel list
        for (let i=0; i<this.dashlets.length; i++) {
            let div = this.dashlets[i].buildContainer();
            let channels = this.dashlets[i].subscribeTo();
            for (let j=0; j<channels.length; j++) {
                if (!this.#alreadySubscribed(channels[j])) {
                    this.channels.push(channels[j]);
                }
            }
            this.parentDiv.appendChild(div);
        }

        // Start subscribing to appropriate channels
        subscribeWS(this.channels, (msg) => {
            if (msg.event === "join") {
                // The DOM will be present now, setup events
                for (let i=0; i<this.dashlets.length; i++) {
                    this.dashlets[i].setupOnce(msg);
                }
            } else {
                // Propagate the message
                for (let i=0; i<this.dashlets.length; i++) {
                    this.dashlets[i].onMessage(msg);
                }
            }
        });
    }

    #alreadySubscribed(name) {
        for (let i=0; i<this.channels.length; i++) {
            if (this.channels[i] === name) {
                return true;
            }
        }
        return false;
    }

    #factory(count) {
        let widgetName = this.dashletIdentities[count].tag;
        let widget = null;
        switch (widgetName) {
            case "throughputBps":   widget = new ThroughputBpsDash(count); break;
            case "throughputPps":   widget = new ThroughputPpsDash(count); break;
            case "shapedUnshaped":  widget = new ShapedUnshapedDash(count); break;
            case "trackedFlowsCount": widget = new TrackedFlowsCount(count); break;
            case "throughputRing":  widget = new ThroughputRingDash(count); break;
            case "rttHistogram":    widget = new RttHistoDash(count); break;
            default: {
                console.log("I don't know how to construct a widget of type [" + widgetName + "]");
                return null;
            }
        }
        return widget;
    }

    editMode() {
        let darken = document.createElement("div");
        darken.id = "editDark";
        darken.style.zIndex = 200;
        darken.style.position = "absolute";
        darken.style.top = "0px";
        darken.style.bottom = "0px";
        darken.style.left = "0px";
        darken.style.right = "0px";
        darken.style.background = "rgba(1, 1, 1, 0.75)";

        let content = document.createElement("div");
        content.style.zIndex = 210;
        content.style.position = "absolute";
        content.style.top = "25px";
        content.style.bottom = "25px";
        content.style.left = "25px";
        content.style.right = "25px";
        content.style.background = "#eee";
        content.style.padding = "10px";
        darken.appendChild(content);

        // Close Button
        let close = document.createElement("button");
        close.classList.add("btn", "btn-primary");
        close.innerText = "Close";
        close.type = "button";
        close.onclick = () => { darken.remove() };
        close.style.marginBottom = "4px";
        content.appendChild(close);

        let dashletList = document.createElement("div");
        let html = "<h5><i class=\"fa fa-dashboard nav-icon\"></i> Dashboard Items</h5>";
        html += "<table><tbody>";
        for (let i=0; i<this.dashletIdentities.length; i++) {
            html += "<tr>";
            html += "<td>" + this.dashletIdentities[i].name + "</td>";
            html += "<td style='width: 20px'>" + this.dashletIdentities[i].size + "</td>";
            if (i > 0) {
                html += "<td style='width: 20px'><button type='button' class='btn btn-sm btn-info'><i class='fa fa-arrow-up'></i></button></td>";
            } else {
                html += "<td style='width: 20px'></td>";
            }
            if (i < this.dashletIdentities.length - 1) {
                html += "<td style='width: 20px'><button type='button' class='btn btn-sm btn-info'><i class='fa fa-arrow-down'></i></button></td>";
            } else {
                html += "<td style='width: 20px'></td>";
            }
            html += "<td style='width: 20px'><button type='button' class='btn btn-sm btn-warn'><i class='fa fa-plus-circle'></i></button></td>";
            html += "<td style='width: 20px'><button type='button' class='btn btn-sm btn-warn'><i class='fa fa-minus-circle'></i></button></td>";
            html += "<td style='width: 20px'><button type='button' class='btn btn-sm btn-warn'><i class='fa fa-trash'></i></button></td>";
            html += "</tr>";
        }
        html += "</tbody></table>";
        dashletList.innerHTML = html;
        content.appendChild(dashletList);

        // Menu
        let menu = document.createElement("div");
        html = "<h5>Add Item</h5><select>";
        for (let i=0; i<DashletMenu.length; i++) {
            html += "<option value='" + DashletMenu[i].tag + "'>";
            html += DashletMenu[i].name;
            html += "</option>";
        }
        html += "</select>";
        menu.innerHTML = html;
        content.appendChild(menu);

        // Add Button
        let addItem = document.createElement("button");
        addItem.type = "button";
        addItem.classList.add("btn", "btn-success");
        addItem.innerText = "Add to Dashboard";
        content.appendChild(addItem);

        this.parentDiv.appendChild(darken);
    }
}

// Serializable POD for dashboard layout
class Layout {
    constructor() {
        this.dashlets = DashletMenu;
    }
}

const DashletMenu = [
    { name: "Throughput Bits/Second", tag: "throughputBps", size: 3 },
    { name: "Throughput Packets/Second", tag: "throughputPps", size: 3 },
    { name: "Shaped/Unshaped Pie", tag: "shapedUnshaped", size: 3 },
    { name: "Tracked Flows Counter", tag: "trackedFlowsCount", size: 3 },
    { name: "Last 5 Minutes Throughput", tag: "throughputRing", size: 6 },
    { name: "Round-Trip Time Histogram", tag: "rttHistogram", size: 6 },
];