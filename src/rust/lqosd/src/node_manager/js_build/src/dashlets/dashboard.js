import {subscribeWS} from "../pubsub/ws";
import {ThroughputBpsDash} from "./throughput_bps_dash";
import {ThroughputPpsDash} from "./throughput_pps_dash";
import {ShapedUnshapedDash} from "./shaped_unshaped_dash";
import {TrackedFlowsCount} from "./tracked_flow_count_dash";
import {ThroughputRingDash} from "./throughput_ring_dash";
import {RttHistoDash} from "./rtt_histo_dash";
import {darkBackground, modalContent} from "../helpers/our_modals";
import {heading5Icon, theading} from "../helpers/builders";

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
            widget.size = this.dashletIdentities[i].size;
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
        let darken = darkBackground("darkEdit");
        let content = modalContent("darkEdit");

        content.appendChild(this.#buildDashletList());
        content.appendChild(this.#buildMenu());
        content.appendChild(this.#buildAddButton());

        darken.appendChild(content);
        this.parentDiv.appendChild(darken);
    }

    #replaceDashletList() {
        let newList = this.#buildDashletList();
        let target = document.getElementById("dashletList");
        target.replaceChildren(newList);
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

    #buildDashletList() {
        let dashletList = document.createElement("div");
        dashletList.id = "dashletList";

        dashletList.appendChild(heading5Icon("dashboard", "Dashboard Items"));
        dashletList.appendChild(document.createElement("hr"));

        let table = document.createElement("table");
        table.classList.add("table");
        let thead = document.createElement("thead");
        thead.appendChild(theading("Item"));
        thead.appendChild(theading(""));
        thead.appendChild(theading(""));
        thead.appendChild(theading(""));
        thead.appendChild(theading(""));
        table.appendChild(thead);
        for (let i=0; i<this.dashletIdentities.length; i++) {
            let d = this.dashletIdentities[i];
            let tr = document.createElement("tr");

            let name = document.createElement("td");
            name.innerText = d.name;
            tr.appendChild(name);

            let up = document.createElement("td");
            if (i > 0) {
                let upBtn = document.createElement("button");
                upBtn.type = "button";
                upBtn.classList.add("btn", "btn-sm", "btn-info");
                upBtn.innerHTML = "<i class='fa fa-arrow-up'></i>";
                let myI = i;
                upBtn.onclick = () => {
                    this.clickUp(myI);
                };
                up.appendChild(upBtn);
            }
            tr.appendChild(up);

            let down = document.createElement("td");
            if (i < this.dashletIdentities.length - 1) {
                let downBtn = document.createElement("button");
                downBtn.type = "button";
                downBtn.classList.add("btn", "btn-sm", "btn-info");
                downBtn.innerHTML = "<i class='fa fa-arrow-down'></i>";
                let myI = i;
                downBtn.onclick = () => {
                    this.clickDown(myI);
                };
                down.appendChild(downBtn);
            }
            tr.appendChild(down);

            // TODO: Resize buttons

            let trash = document.createElement("td");
            let trashBtn = document.createElement("button");
            trashBtn.type = "button";
            trashBtn.classList.add("btn", "btn-sm", "btn-warn");
            trashBtn.innerHTML = "<i class='fa fa-trash'></i>";
            let myI = i;
            trashBtn.onclick = () => {
                this.clickTrash(myI);
            };
            trash.appendChild(trashBtn);
            tr.appendChild(trash);

            table.appendChild(tr);
        }
        dashletList.appendChild(table);

        /*let html = "<h5><i class=\"fa fa-dashboard nav-icon\"></i> Dashboard Items</h5>";
        html += "<hr />";
        html += "<table><tbody>";
        for (let i=0; i<this.dashletIdentities.length; i++) {
            let self = this;
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
        dashletList.innerHTML = html;*/
        return dashletList;
    }

    #buildMenu() {
        let menu = document.createElement("div");
        html = "<h5>Add Item</h5><select>";
        for (let i=0; i<DashletMenu.length; i++) {
            html += "<option value='" + DashletMenu[i].tag + "'>";
            html += DashletMenu[i].name;
            html += "</option>";
        }
        html += "</select>";
        menu.innerHTML = html;
        return menu;
    }

    #buildAddButton() {
        let addItem = document.createElement("button");
        addItem.type = "button";
        addItem.classList.add("btn", "btn-success");
        addItem.innerText = "Add to Dashboard";
        addItem.onclick = () => {
            alert("not implemented yet");
        };
        return addItem;
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