import {BaseDashlet} from "./base_dashlet";

export class BaseCombinedDashlet extends BaseDashlet {
    constructor(slotNumber, dashlets) {
        super(slotNumber)

        this.dashlets = dashlets;
        this.titles = [];
        this.subsciptions = [];
        dashlets.forEach((dash) => {
            this.titles.push(dash.title());
            this.subsciptions.push(dash.subscribeTo());
        });
    }

    subscribeTo() {
        return this.subsciptions;
    }

    #base() {
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

        // Dropdown
        let dd = document.createElement("span");
        dd.classList.add("dropdown");

        let btn = document.createElement("button");
        btn.classList.add("btn", "btn-secondary", "dropdown-toggle", "btn-sm");
        btn.setAttribute("data-bs-toggle", "dropdown");
        dd.appendChild(btn);

        let ul = document.createElement("ul");
        ul.classList.add("dropdown-menu");

        this.titles.forEach((t) => {
            let li1 = document.createElement("li");
            li1.innerHTML = "<a class='dropdown-item'>" + t + "</a>";
            ul.appendChild(li1);
        })

        dd.appendChild(ul);

        title.appendChild(dd);

        div.appendChild(title);

        return div;
    }

    buildContainer() {
        let containers = [];
        let i = 0;
        this.dashlets.forEach((d) => {
            d.size = 12;
            d.id = this.id + "___" + i;
            let container = document.createElement("div");
            container.id = d.id;
            containers.push(container);
            i++;
        });

        let base = this.#base();
        i = 0;
        let row = document.createElement("div");
        row.classList.add("row");
        containers.forEach((c) => {
            c.style.backgroundColor = "rgba(1, 0, 0, 1)";
            row.appendChild(c);
            i++;
        });
        base.appendChild(row);

        return base;
    }

    setup() {
        super.setup();
        this.dashlets.forEach((d) => {
            d.size = this.size;
            d.setup();
        })
    }

    onMessage(msg) {
        this.dashlets.forEach((d) => {
            d.onMessage(msg);
        })
    }
}