import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";

export class BaseCombinedDashlet extends BaseDashlet {
    constructor(slotNumber, dashlets) {
        super(slotNumber)

        this.selectedIndex = 0;
        this.dashlets = dashlets;
        this.titles = [];
        this.subsciptions = [];
        this.divIds = [];
        dashlets.forEach((dash) => {
            this.titles.push(dash.title());
            this.subsciptions.push(dash.subscribeTo());
        });
    }

    subscribeTo() {
        const channels = [];
        for (let i = 0; i < this.subsciptions.length; i++) {
            const entry = this.subsciptions[i];
            if (Array.isArray(entry)) {
                for (let j = 0; j < entry.length; j++) {
                    channels.push(entry[j]);
                }
            } else if (entry) {
                channels.push(entry);
            }
        }
        return channels;
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

        let btn = document.createElement("span");
        //btn.classList.add("btn", "btn-secondary", "btn-sm");
        btn.innerHTML = " <i class='fa fa-chevron-down'></i>";
        //btn.style.height = "15px";
        btn.setAttribute("data-bs-toggle", "dropdown");
        dd.appendChild(btn);

        let ul = document.createElement("ul");
        ul.classList.add("dropdown-menu");

        let i =0;
        this.titles.forEach((t) => {
            let li1 = document.createElement("li");
            let link = document.createElement("a");
            link.classList.add("dropdown-item");
            link.innerText = t;
            let myI = i;
            link.onclick = () => {
                this.divIds.forEach((id) => { $("#" + id).hide() });
                let divId = this.divIds[myI];
                $("#" + divId).show();
            }
            li1.appendChild(link);
            ul.appendChild(li1);
            i++;
        })

        dd.appendChild(ul);

        title.appendChild(dd);

        div.appendChild(title);

        return div;
    }

    buildContainer() {
        let containers = [];
        let i = 0;
        this.divIds = [];
        this.dashlets.forEach((d) => {
            d.size = 12;
            d.id = this.id + "___" + i;
            this.divIds.push(this.id + "___" + i);
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
            if (i > 0) {
                c.style.display = "none";
            }
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
