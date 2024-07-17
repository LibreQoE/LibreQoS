import {subscribeWS, resetWS} from "../pubsub/ws";
import {darkBackground, modalContent} from "../helpers/our_modals";
import {heading5Icon, theading} from "../helpers/builders";
import {DashletMenu, widgetFactory} from "./dashlet_index";

export class Dashboard {
    // Takes the name of the parent div to start building the dashboard
    constructor(divName) {
        this.divName = divName;
        this.editingDashboard = false;
        this.parentDiv = document.getElementById(divName);
        if (this.parentDiv === null) {
            console.log("Dashboard parent not found");
        }
        this.layout = new Layout();
        this.dashletIdentities = this.layout.dashlets;
        this.dashlets = [];
        this.channels = [];
        this.#editButton();
        if (localStorage.getItem("forceEditMode")) {
            localStorage.removeItem("forceEditMode");
            requestAnimationFrame(() => {
                setTimeout(() => {
                    this.editMode();
                })
            });
        }
    }

    #editButton() {
        let editDiv = document.createElement("div");
        editDiv.id = this.divName + "_edit";
        editDiv.style.position = "absolute";
        editDiv.style.right = "5px";
        editDiv.style.top = "5px";
        editDiv.style.width = "40px";
        editDiv.style.zIndex = "100";
        editDiv.style.opacity = 0.5;
        editDiv.innerHTML = "<button type='button' class='btn btn-primary btn-sm'><i class='fa fa-pencil'></i></button>";
        editDiv.onclick = () => {
            if (this.editingDashboard) {
                this.closeEditMode();
            } else {
                this.editMode();
            }
        };
        this.parentDiv.appendChild(editDiv);
    }

    build() {
        this.#filterWidgetList();
        let childDivs = this.#buildWidgetChildDivs();
        this.childIds = [];
        childDivs.forEach((d) => { this.childIds.push(d.id); });
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
        // Insert a Temporary Div to hold edit options
        let editDiv = document.createElement("div");
        editDiv.classList.add("col-12");
        editDiv.id = "dashboardEditingDiv";
        editDiv.style.padding = "10px";
        editDiv.style.borderRadius = "5px";
        editDiv.style.marginLeft = "0";
        editDiv.style.backgroundColor = "#ddddff";
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
        nuke.classList.add("btn", "btn-sm", "btn-danger");
        nuke.innerHTML = "<i class='fa fa-trash'></i> Remove All Items";
        nuke.onclick = () => { this.removeAll(); };
        c1.appendChild(nuke);
        let filler = document.createElement("button");
        filler.type = "button";
        filler.classList.add("btn", "btn-sm", "btn-warning");
        filler.innerHTML = "<i class='fa fa-plus-square'></i> One of Everything";
        filler.onclick = () => { this.addAll(); };
        filler.style.marginLeft = "5px";
        c1.appendChild(filler);

        let c2 = document.createElement("div");
        c2.classList.add("col-3");
        c2.appendChild(heading5Icon("plus", "Add Dashlet"));
        let list = document.createElement("div");
        list.classList.add("dropdown");
        list.id = "dropdown-widgets";
        let listBtn = document.createElement("button");
        listBtn.type = "button";
        listBtn.classList.add("btn", "btn-primary", "dropdown-toggle");
        listBtn.setAttribute("data-bs-toggle", "dropdown");
        listBtn.innerHTML = "<i class='fa fa-plus'></i> Add Widget";
        list.appendChild(listBtn);
        let listUl = document.createElement("ul");
        listUl.classList.add("dropdown-menu");
        DashletMenu.forEach((d) => {
            let entry = document.createElement("li");
            let item = document.createElement("a");
            item.classList.add("dropdown-item");
            item.innerText = d.name;
            let myTag = d.tag;
            item.onclick = () => {
                let didSomething = false;
                DashletMenu.forEach((d) => {
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
                    localStorage.setItem("forceEditMode", "true");
                    window.location.reload();
                }
            })
        }
        c3.appendChild(lbl);
        c3.appendChild(saveDashName);
        c3.appendChild(saveBtn);

        let c4 = document.createElement("div");
        c4.classList.add("col-3");
        c4.appendChild(heading5Icon("cloud", "Load Layout"))
        let listRemote = document.createElement("div");
        listRemote.classList.add("dropdown");
        let listBtnRemote = document.createElement("button");
        listBtnRemote.type = "button";
        listBtnRemote.classList.add("btn", "btn-secondary", "dropdown-toggle");
        listBtnRemote.setAttribute("data-bs-toggle", "dropdown");
        listBtnRemote.innerHTML = "<i class='fa fa-cloud'></i> Load Layout";
        listRemote.appendChild(listBtnRemote);
        let listUlRemote = document.createElement("ul");
        listUlRemote.classList.add("dropdown-menu");
        listUlRemote.id = "remoteDashletList";
        listRemote.appendChild(listUlRemote);
        c4.appendChild(listRemote);
        $.get("/local-api/dashletThemes", (data) => {
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
                        data: JSON.stringify({ theme: filename}),
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

        row.appendChild(c1);
        row.appendChild(c2);
        row.appendChild(c3);
        row.appendChild(c4);
        editDiv.appendChild(row);


        // Decorate all the dashboard elements with controls after a refresh period
        requestAnimationFrame(() => {
            setTimeout(() => { this.#updateEditDecorations() });
        });

        toasts.appendChild(editDiv);
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

    editModeOld() {
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

        // Apply
        this.build();
        let self = this;
        requestAnimationFrame(() => {
            setTimeout(() => { self.#updateEditDecorations() });
        });

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