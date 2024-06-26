export class BaseDashlet {
    constructor(slotNumber) {
        this.slotNumber = slotNumber;
        this.id = "dash_" + slotNumber;
        this.size = 3;
        this.setupDone = false;
    }

    sizeClasses() {
        switch (this.size) {
            case 1: return ["col-1"];
            case 2: return ["col-2"];
            case 3: return ["col-3"];
            case 4: return ["col-4"];
            case 5: return ["col-5"];
            case 6: return ["col-6"];
            case 7: return ["col-7"];
            case 8: return ["col-8"];
            case 9: return ["col-9"];
            case 10: return ["col-10"];
            case 11: return ["col-11"];
            case 12: return ["col-12"];
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