import {enableTooltips} from "../helpers/content_builders";

export class BaseDashlet {
    constructor(slotNumber) {
        this.slotNumber = slotNumber;
        this.id = "dash_" + slotNumber;
        this.size = 3;
        this.setupDone = false;
        // For multi-period LTS support
        this.buttons = [];
        this.graphs = [];
        this.graphDivs = [];
        this.zoomed = false;
    }

    canBeSlowedDown() {
        return false;
    }

    sizeClasses() {
        switch (this.size) {
            case 1: return  ["col-xs-12", "col-sm-12", "col-md-6", "col-lg-3", "col-xl-1", "col-xxl-1"];
            case 2: return  ["col-xs-12", "col-sm-12", "col-md-6", "col-lg-3", "col-xl-2", "col-xxl-2"];
            case 3: return  ["col-xs-12", "col-sm-12", "col-md-6", "col-lg-3", "col-xl-3", "col-xxl-3"];
            case 4: return  ["col-xs-12", "col-sm-12", "col-md-6", "col-lg-6", "col-xl-4", "col-xxl-4"];
            case 5: return  ["col-xs-12", "col-sm-12", "col-md-6", "col-lg-6", "col-xl-5", "col-xxl-5"];
            case 6: return  ["col-xs-12", "col-sm-12", "col-md-6", "col-lg-6", "col-xl-6", "col-xxl-6"];
            case 7: return  ["col-xs-12", "col-sm-12", "col-md-12", "col-lg-12", "col-xl-7", "col-xxl-7"];
            case 8: return  ["col-xs-12", "col-sm-12", "col-md-12", "col-lg-12", "col-xl-8", "col-xxl-8"];
            case 9: return  ["col-xs-12", "col-sm-12", "col-md-12", "col-lg-12", "col-xl-9", "col-xxl-9"];
            case 10: return ["col-xs-12", "col-sm-12", "col-md-12", "col-lg-12", "col-xl-10", "col-xxl-10"];
            case 11: return ["col-xs-12", "col-sm-12", "col-md-12", "col-lg-12", "col-xl-11", "col-xxl-11"];
            case 12: return ["col-xs-12", "col-sm-12", "col-md-12", "col-lg-12", "col-xl-12", "col-xxl-12"];
            default: return ["col-3"];
        }
    }

    title() {
        return "Someone forgot to set a title";
    }

    tooltip() {
        return null;
    }

    subscribeTo() {
        return [];
    }

    onMessage(msg) {
    }

    setupOnce(msg) {
        if (!this.setupDone) {
            this.setup(msg);
        }
        this.setupDone = true;
        enableTooltips();
    }

    setup() {}

    graphDivId() {
        if (this.zoomed) {
            return this.id + "_zoomed_graph";
        } else {
            return this.id + "_graph";
        }
    }

    graphDiv() {
        let graphDiv = document.createElement("div");
        graphDiv.id = this.id + "_graph";
        graphDiv.classList.add("dashgraph");
        return graphDiv;
    }

    buildContainer() {
        let div = document.createElement("div");
        div.id = this.id;
        let sizeClasses = this.sizeClasses();
        for (let i=0; i<sizeClasses.length; i++) {
            div.classList.add(sizeClasses[i]);
        }
        div.classList.add("dashbox");

        let title = document.createElement("h5");
        title.classList.add("dashbox-title");
        title.innerText = this.title();

        let tt = this.tooltip();
        if (tt !== null) {
            let tooltip = document.createElement("span");
            tooltip.style.marginLeft = "5px";
            let button = document.createElement("a");
            //button.type = "button";
            //button.classList.add("btn", "btn-sm", "btn-info");
            button.title = tt;
            button.setAttribute("data-bs-toggle", "tooltip");
            button.setAttribute("data-bs-placement", "top");
            button.setAttribute("data-bs-html", "true");
            button.innerHTML = "<i class='fas fa-info-circle'></i>";
            tooltip.appendChild(button);
            title.appendChild(tooltip);
        }

        if (this.supportsZoom()) {
            let zoom = document.createElement("span");
            zoom.style.marginLeft = "5px";
            let button = document.createElement("a");
            button.title = "Zoom";
            button.innerHTML = "<i class='fas fa-search-plus'></i>";
            button.onclick = () => {
                if (!this.zoomed) {
                    let zoomDiv = document.createElement("div");
                    zoomDiv.classList.add("zoomed");
                    zoomDiv.id = this.id + "_zoomed";
                    zoomDiv.classList.add("dashbox");

                    let title = document.createElement("h5");
                    title.classList.add("dashbox-title");
                    title.innerText = this.title();

                    let button = document.createElement("a");
                    button.title = "Zoom";
                    button.innerHTML = "<i class='fas fa-search-minus'></i>";
                    button.style.marginLeft = "5px";
                    button.onclick = () => {
                        document.getElementById(zoomDiv.id).remove();
                        this.zoomed = !this.zoomed;
                    };
                    title.appendChild(button);

                    zoomDiv.appendChild(title);

                    let graphDiv = document.createElement("div");
                    graphDiv.id = zoomDiv.id + "_graph";
                    graphDiv.classList.add("dashgraphZoomed");
                    zoomDiv.appendChild(graphDiv);

                    document.getElementById("content").insertBefore(zoomDiv, document.getElementById("content").firstChild);

                    requestAnimationFrame(() => {
                        this.setup();
                        zoomDiv.scrollIntoView({behavior: "smooth"});
                    });
                }
                this.zoomed = !this.zoomed;
            }
            zoom.appendChild(button);
            title.appendChild(zoom);
        }

        div.appendChild(title);

        return div;
    }

    supportsZoom() {
        return false;
    }

    makePeriodBtn(name) {
        let btn = document.createElement("button");
        btn.classList.add("btn", "btn-sm", "btn-outline-primary", "tiny", "me-1");
        btn.innerText = name;
        btn.id = this.graphDivId() + "_btn_" + name;
        btn.onclick = () => {
            this.buttons.forEach((b) => {
                b.classList.remove("active");
                let targetName = "#" + b.id.replace("_btn", "");
                if (targetName.lastIndexOf("Live") > 0) {
                    targetName = "#" + this.graphDivId();
                }
                if (b === btn) {
                    b.classList.add("active");
                    $(targetName).show();
                } else {
                    $(targetName).hide();
                }
            });
            this.graphs.forEach((g) => {
                g.chart.resize();
            });
        }
        this.buttons.push(btn);
        return btn;
    }
}
