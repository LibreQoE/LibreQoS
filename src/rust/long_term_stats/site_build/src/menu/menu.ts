import html from './template.html';
import { Page } from '../page'
import { siteIcon } from '../helpers';
import { request_search } from "../../wasm/wasm_pipe";

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
        request_search(term);
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
                case "SearchResult": {
                    this.searchResult(event.SearchResult.hits);
                } break;
            }
        }
    }

    searchResult(hits) {
        //console.log(hits);
        let r = document.getElementById("searchResults");
        if (r) {
            let html = "<table>";
            for (let i = 0; i < hits.length; i++) {
                html += "<tr onclick='window.router.goto(\"" + hits[i].url + "\")''>";
                html += "<td>" + siteIcon(hits[i].icon) + "</td>";
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