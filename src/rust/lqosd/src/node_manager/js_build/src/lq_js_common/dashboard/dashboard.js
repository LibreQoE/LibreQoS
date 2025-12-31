// Provides a generic dashboard system for use in LibreQoS and Insight
import {DashboardLayout} from "./layout";
import {subscribeWS} from "./ws";
import {get_ws_client} from "../../pubsub/ws";
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
        this.#clearRenderedDashboard();
        this.#buildTabUI();
        this.#buildTabContents();
        this.#buildChannelList(this.dashlets);
        this.#webSocketSubscription();
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
                // Notify dashlets in this tab that they're now visible
                let tabDashlets = this.tabDashlets[index] || [];
                tabDashlets.forEach(dashlet => {
                    if (dashlet.onTabActivated) {
                        dashlet.onTabActivated();
                    }
                });
                
                // Resize all ECharts instances in the newly visible tab
                setTimeout(() => {
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
