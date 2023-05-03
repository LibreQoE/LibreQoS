import html from './template.html';
import { Page } from '../page'

export class MenuPage implements Page {
    activePanel: string;
    searchButton: HTMLButtonElement;
    searchBar: HTMLInputElement;

    constructor(activeElement: string) {
        let container = document.getElementById('main');
        if (container) {
            container.innerHTML = html;

            let activePanel = document.getElementById(activeElement);
            if (activePanel) {
                activePanel.classList.add('active');
            }

            let username = document.getElementById('menuUser');
            if (username) {
                if (window.login) {
                    username.textContent = window.login.name;
                } else {
                    username.textContent = "Unknown";
                }
            }

            this.searchBar = <HTMLInputElement>document.getElementById("txtSearch");
            this.searchButton = <HTMLButtonElement>document.getElementById("btnSearch");

            this.wireup();
        }
    }

    wireup() {
        this.searchBar.onkeyup = () => {
            let r = document.getElementById("searchResults");
            if (r) {
                r.style.display = "none";
            }
            let searchText = this.searchBar.value;
            if (searchText.length > 3) {
                this.doSearch(searchText);
            }
        }
        this.searchButton.onclick = () => {
            let searchText = this.searchBar.value;
            this.doSearch(searchText);
        }
    }

    doSearch(term: string) {
        //console.log("Searching for: " + term);
        let r = document.getElementById("searchResults");
        if (r) {
            r.style.display = "none";
        }
        window.bus.sendSearch(term);
    }

    onmessage(event: any) {
        if (event.msg) {
            switch (event.msg) {
                case "authOk": {
                    let username = document.getElementById('menuUser');
                    if (username) {
                        if (window.login) {
                            username.textContent = window.login.name;
                        } else {
                            username.textContent = "Unknown";
                        }
                    }
                } break;
                case "search": {
                    this.searchResult(event.hits);
                } break;
            }
        }
    }

    icon(type: string): string {
        switch (type) {
            case "circuit": return "<i class='fa fa-user'></i>"; break;
            case "site": return "<i class='fa fa-building'></i>"; break;
            case "ap": return "<i class='fa fa-wifi'></i>"; break;
            default: return "<i class='fa fa-question'></i>";
        }
    }

    searchResult(hits) {
        //console.log(hits);
        let r = document.getElementById("searchResults");
        if (r) {
            let html = "<table>";
            for (let i = 0; i < hits.length; i++) {
                html += "<tr onclick='window.router.goto(\"" + hits[i].url + "\")''>";
                html += "<td>" + this.icon(hits[i].icon) + "</td>";
                html += "<td>" + hits[i].name + "</td>";
                //html += hits[i].url;
                html += "</tr>";
            }
            html += "</table>";
            r.innerHTML = html;
            r.style.display = "block";
        }
    }

    ontick(): void {
        // Do nothing
    }
}