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
        this.widgetFactory = widgetFactory;
        this.dashletMenu = dashletMenu;
        this.savedDashUrl = savedDashUrl;

        // Set up the div
        this.parentDiv = document.getElementById(this.divName);
        if (this.parentDiv === null) {
            throw new Error("No element found with the id '" + this.divName + "'");
        }

        // Editor Support
        this.editingDashboard = false;
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

    #editButton(hasCadence) {
        let parent = document.getElementById("controls");
        if (parent === null) return;
        let editDiv = document.createElement("span");
        editDiv.id = this.divName + "_edit";
        editDiv.innerHTML = "<button type='button' class='btn btn-secondary btn-sm' id='btnEditDash'><i class='fa fa-pencil'></i> Edit</button>";
        editDiv.onclick = () => {
            // New Editor
            let initialElements = [];
            this.dashlets.forEach((e) => {
                initialElements.push({
                    size: e.size,
                    name: e.title(),
                });
            });

            let availableElements = [];
            this.dashletMenu.forEach((d) => {
                availableElements.push({
                    name: d.name,
                    size: d.size,
                });
            });

            openDashboardEditor(initialElements, availableElements, function(newLayout) {
                console.log("New dashboard layout:", newLayout);
            });

            // Old Editor
            if (this.editingDashboard) {
                let e = document.getElementById("btnEditDash");
                e.innerHTML = "<i class='fa fa-pencil'></i> Edit";
                this.closeEditMode();
            } else {
                let e = document.getElementById("btnEditDash");
                e.innerHTML = "<i class='fa fa-close'></i> Finish Edit";
                this.editMode();
            }
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
            this.cadence = parseInt(cadencePicker.value);
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

        parent.appendChild(editDiv);
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
                        this.dashletIdentities.push(d);
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
            let request = {
                name: name,
                entries: this.dashletIdentities
            }
            $.ajax({
                type: "POST",
                url: "/local-api/dashletSave",
                data: JSON.stringify(request),
                contentType : 'application/json',
                success: () => {
                    localStorage.setItem("forceEditMode", "true");
                    window.location.reload();
                }
            })
        }
        saveGroup.appendChild(saveDashName);
        saveAppend.appendChild(saveBtn);
        saveGroup.appendChild(saveAppend);
        c3.appendChild(saveGroup);

        //c3.appendChild(lbl);
        //c3.appendChild(saveDashName);
        //c3.appendChild(saveBtn);

        let c4 = document.createElement("div");
        c4.classList.add("col-3");
        c4.appendChild(heading5Icon("cloud", "Load Layout"))
        let listRemote = document.createElement("div");
        listRemote.classList.add("dropdown");
        let listBtnRemote = document.createElement("button");
        listBtnRemote.type = "button";
        listBtnRemote.classList.add("btn", "btn-secondary", "dropdown-toggle", "btn-sm");
        listBtnRemote.setAttribute("data-bs-toggle", "dropdown");
        listBtnRemote.innerHTML = "<i class='fa fa-cloud'></i> Load Layout";
        listRemote.appendChild(listBtnRemote);
        let listUlRemote = document.createElement("ul");
        listUlRemote.classList.add("dropdown-menu");
        listUlRemote.id = "remoteDashletList";
        listRemote.appendChild(listUlRemote);
        c4.appendChild(listRemote);
        if (this.savedDashUrl.length > 0) {
            $.get(this.savedDashUrl, (data) => {
                let parent = document.getElementById("remoteDashletList");
                data.forEach((d) => {
                    let li = document.createElement("li");
                    let link = document.createElement("a");
                    link.innerText = d;
                    let filename = d;
                    link.onclick = () => {
                        console.log("Loading " + d);
                        $.ajax({
                            type: "POST",
                            url: "/local-api/dashletGet",
                            data: JSON.stringify({theme: filename}),
                            contentType: 'application/json',
                            success: (data) => {
                                this.dashletIdentities = data;
                                this.layout.save(this.dashletIdentities);
                                localStorage.setItem("forceEditMode", "true");
                                window.location.reload();
                            }
                        });
                    }
                    li.appendChild(link);
                    parent.appendChild(li);
                });
            });
        }

        row.appendChild(c1);
        row.appendChild(c2);
        if (this.savedDashUrl.length > 0) {
            row.appendChild(c3);
            row.appendChild(c4);
        }
        editDiv.appendChild(row);


        // Decorate all the dashboard elements with controls after a refresh period
        requestAnimationFrame(() => {
            setTimeout(() => { this.#updateEditDecorations() });
        });

        toasts.appendChild(editDiv);
    }

    #replaceDashletList() {
        resetWS();

        // Apply
        this.build();
        let self = this;
        requestAnimationFrame(() => {
            setTimeout(() => { self.#updateEditDecorations() });
        });

        // Persist
        this.layout.save(this.dashletIdentities);
    }

    #updateEditDecorations() {
        let oldEditDiv = document.getElementById("divEditorElements");
        if (oldEditDiv !== null) editDiv.remove();

        let editDivParent = document.getElementById("dashboardEditingDiv");
        let editDiv = document.createElement("div");
        editDiv.id = "divEditorElements";

        for (let i=0; i<this.childIds.length; i++) {
            let dashDiv = document.getElementById(this.childIds[i]);
            if (dashDiv != null) {
                let clientRect = dashDiv.getBoundingClientRect();
                let clientLeft = (clientRect.left + 4) + "px";
                let clientRight = (clientRect.right - 34) + "px";
                let clientTop = (clientRect.top) + "px";
                let clientBottom = (clientRect.bottom) + "px";
                let clientMiddleY = (((clientRect.bottom - clientRect.top) / 2) + clientRect.top - 10) + "px";
                let clientMiddleX = (((clientRect.right - clientRect.left) / 2) + clientRect.left - 10) + "px";

                // Left Navigation Arrow
                if (i > 0) {
                    let myI = i;
                    editDiv.appendChild(this.#dashEditButton(
                        clientLeft,
                        clientMiddleY,
                        "arrow-circle-left",
                        "warning",
                        () => {
                            this.clickUp(myI);
                        }
                    ));
                }

                // Right Navigation Arrow
                if (i < this.childIds.length-1) {
                    let myI = i;
                    editDiv.appendChild(this.#dashEditButton(
                        clientRight,
                        clientMiddleY,
                        "arrow-circle-right",
                        "warning",
                        () => {
                            this.clickDown(myI);
                        }
                    ));
                }

                // Trash Button
                let myI = i;
                editDiv.appendChild(this.#dashEditButton(
                    clientMiddleX,
                    clientMiddleY,
                    "trash",
                    "danger",
                    () => {
                        this.clickTrash(myI);
                    }
                ));

                // Expand Button
                let myI2 = i;
                editDiv.appendChild(this.#dashEditButton(
                    clientLeft,
                    clientTop,
                    "plus-circle",
                    "secondary",
                    () => {
                        this.zoomIn(myI2);
                    }
                ));

                // Contract Button
                let myI3 = i;
                editDiv.appendChild(this.#dashEditButton(
                    (clientRect.left + 40) + "px",
                    clientTop,
                    "minus-circle",
                    "secondary",
                    () => {
                        this.zoomOut(myI3);
                    }
                ));
            } else {
                console.log("Warning: NULL div found in dashlet list");
            }
        }

        editDivParent.appendChild(editDiv);
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
        this.layout.save(this.dashletIdentities);
        localStorage.setItem("forceEditMode", "true");
        window.location.reload();
    }

    zoomIn(i) {
        console.log(i);
        if (this.dashletIdentities[i].size < 12) {
            this.dashletIdentities[i].size += 1;
        }
        this.layout.save(this.dashletIdentities);
        localStorage.setItem("forceEditMode", "true");
        window.location.reload();
    }

    zoomOut(i) {
        if (this.dashletIdentities[i].size > 1) {
            this.dashletIdentities[i].size -= 1;
        }
        this.layout.save(this.dashletIdentities);
        localStorage.setItem("forceEditMode", "true");
        window.location.reload();
    }

    removeAll() {
        this.dashletIdentities = [];
        this.layout.save(this.dashletIdentities);
        localStorage.setItem("forceEditMode", "true");
        window.location.reload();
    }

    addAll() {
        this.dashletIdentities = DashletMenu;
        this.layout.save(this.dashletIdentities);
        localStorage.setItem("forceEditMode", "true");
        window.location.reload();
    }
}