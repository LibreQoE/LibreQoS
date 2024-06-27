import {subscribeWS, resetWS} from "../pubsub/ws";
import {darkBackground, modalContent} from "../helpers/our_modals";
import {heading5Icon, theading} from "../helpers/builders";
import {DashletMenu, widgetFactory} from "./dashlet_index";

export class Dashboard {
    // Takes the name of the parent div to start building the dashboard
    constructor(divName) {
        this.divName = divName;
        this.parentDiv = document.getElementById(divName);
        if (this.parentDiv === null) {
            console.log("Dashboard parent not found");
        }
        this.layout = new Layout();
        this.dashletIdentities = this.layout.dashlets;
        this.dashlets = [];
        this.channels = [];
        this.#editButton();
    }

    #editButton() {
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
        this.#filterWidgetList();
        let childDivs = this.#buildWidgetChildDivs();
        this.#clearRenderedDashboard();
        childDivs.forEach((d) => { this.parentDiv.appendChild(d) });
        this.#buildChannelList();
        this.#webSocketSubscription();
    }

    #filterWidgetList() {
        this.dashlets = [];
        for (let i=0; i<this.dashletIdentities.length; i++) {
            let widget = widgetFactory(this.dashletIdentities[i].tag, i);
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

    #webSocketSubscription() {
        resetWS();
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

    editMode() {
        let darken = darkBackground("darkEdit");
        let content = modalContent("darkEdit");

        // Add Items Group
        let row = document.createElement("div");
        row.classList.add("row");

        let col1 = document.createElement("div");
        col1.classList.add("col-6");
        col1.style.minWidth = "300px";
        col1.appendChild(this.#buildDashletList());

        let options = document.createElement("div");
        options.appendChild(document.createElement("hr"));
        options.appendChild(heading5Icon("gear", "Options"))
        let nuke = document.createElement("button");
        nuke.type = "button";
        nuke.classList.add("btn", "btn-danger");
        nuke.innerHTML = "<i class='fa fa-trash'></i> Remove All Items";
        nuke.onclick = () => { this.removeAll(); };
        options.appendChild(nuke);

        let filler = document.createElement("button");
        filler.type = "button";
        filler.classList.add("btn", "btn-warning");
        filler.innerHTML = "<i class='fa fa-plus-square'></i> One of Everything";
        filler.onclick = () => { this.addAll(); };
        filler.style.marginLeft = "5px";
        options.appendChild(filler);
        col1.appendChild(options);

        let col2 = document.createElement("div");
        col2.classList.add("col-6");
        col2.style.minWidth = "300px";
        col2.appendChild(this.#buildMenu());

        // Themes from the server
        col2.appendChild(document.createElement("hr"));
        col2.appendChild(heading5Icon("cloud", "Saved Dashboard Layouts"))
        let remoteList = document.createElement("select");
        remoteList.id = "remoteThemes";
        let remoteBtn = document.createElement("button");
        remoteBtn.classList.add("btn", "btn-success");
        remoteBtn.style.marginLeft = "5px";
        remoteBtn.innerHTML = "<i class='fa fa-load'></i> Load Theme From Server";
        remoteBtn.id = "remoteLoadBtn";
        remoteBtn.onclick = () => {
            let layoutName = {
                theme: $('#remoteThemes').find(":selected").val().toString()
            }
            $.ajax({
                type: "POST",
                url: "/local-api/dashletGet",
                data: JSON.stringify(layoutName),
                contentType: 'application/json',
                success: (data) => {
                    this.dashletIdentities = data;
                    this.#replaceDashletList();
                    alert("Layout Loaded");
                }
            });
        }

        let delBtn = document.createElement("button");
        delBtn.classList.add("btn", "btn-danger");
        delBtn.innerHTML = "<i class='fa fa-trash'></i> Delete Remote Theme";
        delBtn.style.marginLeft = "4px";
        delBtn.onclick = () => {
            let layoutName = $('#remoteThemes').find(":selected").val();
            if (confirm("Are you sure you wish to delete " + layoutName)) {
                let layoutNameObj = {
                    theme: layoutName.toString()
                }
                $.ajax({
                    type: "POST",
                    url: "/local-api/dashletDelete",
                    data: JSON.stringify(layoutNameObj),
                    contentType: 'application/json',
                    success: () => {
                        $.get("/local-api/dashletThemes", (data) => {
                            alert("Layout deleted: " + layoutName);
                            this.fillServerLayoutList(data);
                        });
                    }
                });
            }
        }
        col2.appendChild(remoteList);
        col2.appendChild(remoteBtn);
        col2.appendChild(delBtn);

        $.get("/local-api/dashletThemes", (data) => {
            this.fillServerLayoutList(data);
        });

        // Save theme to the server
        col2.appendChild(document.createElement("hr"));
        col2.appendChild(heading5Icon("save", "Save"));
        let lbl = document.createElement("label");
        lbl.htmlFor = "saveDashName";
        let saveDashName = document.createElement("input");
        saveDashName.id = "saveDashName";
        saveDashName.type = "text";
        let saveBtn = document.createElement("button");
        saveBtn.type = "button";
        saveBtn.classList.add("btn", "btn-success");
        saveBtn.innerHTML = "<i class='fa fa-save'></i> Save to Server";
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
                    $.get("/local-api/dashletThemes", (data) => {
                        this.fillServerLayoutList(data);
                        alert("Layout Saved");
                    });
                }
            })
        }
        col2.appendChild(lbl);
        col2.appendChild(saveDashName);
        col2.appendChild(saveBtn);

        row.appendChild(col1);
        row.appendChild(col2);
        content.appendChild(row);

        darken.appendChild(content);
        document.body.appendChild(darken);
    }

    fillServerLayoutList(data) {
        let parent = document.getElementById("remoteThemes");
        while (parent.children.length > 0) {
            parent.removeChild(parent.lastChild);
        }
        for (let i=0; i<data.length; i++) {
            let e = document.createElement("option");
            e.innerText = data[i];
            e.value = data[i];
            parent.appendChild(e);
        }
        if (data.length === 0) {
            let e = document.createElement("option");
            e.innerText = "No Layouts Saved";
            e.value = "No Layouts Saved";
            parent.appendChild(e);
            $("#remoteLoadBtn").prop('disabled', true);
        } else {
            $("#remoteLoadBtn").prop('disabled', false);
        }
    }

    #clearRenderedDashboard() {
        while (this.parentDiv.children.length > 1) {
            this.parentDiv.removeChild(this.parentDiv.lastChild);
        }
    }

    #replaceDashletList() {
        resetWS();
        let newList = this.#buildDashletList();
        let target = document.getElementById("dashletList");
        target.replaceChildren(newList);

        // Apply
        this.build();

        // Persist
        this.layout.save(this.dashletIdentities);
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
        this.dashletIdentities = DashletMenu;
        this.#replaceDashletList();
    }

    #buildDashletList() {
        let dashletList = document.createElement("div");
        dashletList.id = "dashletList";
        dashletList.style.maxHeight = "450px";
        dashletList.style.overflowY = "auto";

        dashletList.appendChild(heading5Icon("dashboard", "Dashboard Items"));
        dashletList.appendChild(document.createElement("hr"));

        let table = document.createElement("table");
        table.classList.add("table");
        let thead = document.createElement("thead");
        thead.appendChild(theading("Item"));
        thead.appendChild(theading("Size"));
        thead.appendChild(theading(""));
        thead.appendChild(theading(""));
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

            let size = document.createElement("td");
            size.innerText = d.size;
            tr.appendChild(size);

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

            let bigger = document.createElement("td");
            let biggerBtn = document.createElement("button");
            biggerBtn.type = "button";
            biggerBtn.classList.add("btn", "btn-sm", "btn-info");
            biggerBtn.innerHTML = "<i class='fa fa-plus-circle'></i>";
            let biggerI = i;
            biggerBtn.onclick = () => { this.zoomIn(biggerI) }
            bigger.appendChild(biggerBtn);
            tr.appendChild(bigger);

            let smaller = document.createElement("td");
            let smallerBtn = document.createElement("button");
            smallerBtn.type = "button";
            smallerBtn.classList.add("btn", "btn-sm", "btn-info");
            smallerBtn.innerHTML = "<i class='fa fa-minus-circle'></i>";
            let smallerI = i;
            smallerBtn.onclick = () => { this.zoomOut(smallerI) }
            smaller.appendChild(smallerBtn);
            tr.appendChild(smaller);

            let trash = document.createElement("td");
            let trashBtn = document.createElement("button");
            trashBtn.type = "button";
            trashBtn.classList.add("btn", "btn-sm", "btn-warning");
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

        return dashletList;
    }

    #buildMenu() {
        let row = document.createElement("div");
        row.classList.add("row");
        let left = document.createElement("div");
        left.classList.add("col-6");

        let menu = document.createElement("div");
        menu.appendChild(heading5Icon("plus", "Add Dashboard Item"));
        menu.appendChild(document.createElement("hr"));

        let list = document.createElement("select");
        list.id = "newWidgetList";
        list.size = DashletMenu.length;
        list.style.width = "100%";
        list.classList.add("listBox");
        list.size = 8;
        DashletMenu.forEach((d) => {
            let entry = document.createElement("option");
            entry.value = d.tag;
            entry.innerText = d.name;
            entry.classList.add("listItem");
            list.appendChild(entry);
        });
        left.appendChild(list);

        let right = document.createElement("div");
        right.classList.add("col-6");
        let btn = document.createElement("button");
        btn.classList.add("btn", "btn-secondary");
        btn.innerHTML = "<i class='fa fa-plus'></i> Add Widget";
        btn.onclick = () => {
            let widgetId = $('#newWidgetList').find(":selected").val();
            if (widgetId === null || widgetId === undefined || widgetId === "") return;
            let didSomething = false;
            DashletMenu.forEach((d) => {
                if (d.tag === widgetId) {
                    this.dashletIdentities.push(d);
                    didSomething = true;
                }
            });
            if (didSomething) {
                this.#replaceDashletList();
            }
        }
        right.appendChild(btn);

        row.appendChild(left);
        row.appendChild(right);
        menu.appendChild(row);

        return menu;
    }
}

// Serializable POD for dashboard layout
class Layout {
    constructor() {
        let template = localStorage.getItem("dashboardLayout");
        if (template !== null) {
            this.dashlets = JSON.parse(template);
        } else {
            this.dashlets = DashletMenu;
        }
    }

    save(dashletIdentities) {
        this.dashlets = dashletIdentities;
        let template = JSON.stringify(dashletIdentities);
        localStorage.setItem("dashboardLayout", template);
    }
}