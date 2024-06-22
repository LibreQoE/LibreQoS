export class BaseDashlet {
    constructor(slotNumber) {
        this.slotNumber = slotNumber;
        this.id = "dash_" + slotNumber;
        this.size = 3;
        this.setupDone = false;
    }

    sizeClasses() {
        switch (this.size) {
            case 3: return ["col-3"];
            case 6: return ["col-6"];
            default: return ["col-3"];
        }
    }

    title() {
        return "Someone forgot to set a title";
    }

    subscribeTo() {
        return [];
    }

    onMessage(msg) {
        console.log(msg);
    }

    setupOnce(msg) {
        if (!this.setupDone) {
            this.setup(msg);
        }
        this.setupDone = true;
    }

    setup() {}

    graphDivId() {
        return this.id + "_graph";
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

        div.appendChild(title);

        return div;
    }
}