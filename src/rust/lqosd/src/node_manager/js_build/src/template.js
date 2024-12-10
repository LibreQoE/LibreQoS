import {clearDiv} from "./helpers/builders";
import {initRedact} from "./helpers/redact";
import {initDayNightMode} from "./helpers/dark_mode";

function getDeviceCounts() {
    $.get("/local-api/deviceCount", (data) => {
        //console.log(data);
        $("#shapedDeviceCount").text(data.shaped_devices);
        $("#unknownIpCount").text(data.unknown_ips);
    })
}

function initLogout() {
    $("#btnLogout").on('click', () => {
        //console.log("Logout");
        const cookies = document.cookie.split(";");

        for (let i = 0; i < cookies.length; i++) {
            const cookie = cookies[i];
            const eqPos = cookie.indexOf("=");
            const name = eqPos > -1 ? cookie.substr(0, eqPos) : cookie;
            document.cookie = name + "=;expires=Thu, 01 Jan 1970 00:00:00 GMT";
        }
        window.location.reload();
    });
}

function doSearch(search) {
    if (search.length > 2) {
        $.ajax({
            type: "POST",
            url: "/local-api/search",
            data: JSON.stringify({term: search}),
            contentType: 'application/json',
            success: (data) => {
                let searchResults = document.getElementById("searchResults");
                searchResults.style.visibility = "visible";
                let list = document.createElement("table");
                list.classList.add("table", "table-striped");
                let tbody = document.createElement("tbody");
                data.forEach((item) => {
                    let r = document.createElement("tr");
                    let c = document.createElement("td");

                    if (item.Circuit !== undefined) {
                        c.innerHTML = "<a class='nav-link' href='/circuit.html?id=" + encodeURI(item.Circuit.id) + "'><i class='fa fa-user'></i> " + item.Circuit.name + "</a>";
                    } else if (item.Device !== undefined) {
                        c.innerHTML = "<a class='nav-link' href='/circuit.html?id=" + encodeURI(item.Device.circuit_id) + "'><i class='fa fa-computer'></i> " + item.Device.name + "</a>";
                    } else if (item.Site !== undefined) {
                        c.innerHTML = "<a class='nav-link' href='/tree.html?parent=" + item.Site.idx + "'><i class='fa fa-building'></i> " + item.Site.name + "</a>";
                    } else {
                        console.log(item);
                        c.innerText = item;
                    }
                    r.appendChild(c);
                    tbody.appendChild(r);
                });
                clearDiv(searchResults);
                list.appendChild(tbody);
                searchResults.appendChild(list);
            },
        })
    } else {
        // Close the search panel
        let searchResults = document.getElementById("searchResults");
        searchResults.style.visibility = "hidden";
    }
}

function setupSearch() {
    $("#btnSearch").on('click', () => {
        const search = $("#txtSearch").val();
        doSearch(search);
    });
    $("#txtSearch").on('keyup', () => {
        const search = $("#txtSearch").val();
        doSearch(search);
    });
}

function setupReload() {
    let link = document.getElementById("lnkReloadLqos");
    link.onclick = () => {
        const myModal = new bootstrap.Modal(document.getElementById('reloadModal'), { focus: true });
        myModal.show();
        $("#reloadLibreResult").html("<i class='fa fa-spinner fa-spin'></i> Reloading LibreQoS...");
        $.get("/local-api/reloadLqos", (result) => {
            $("#reloadLibreResult").text(result);
        });
    }
}

initLogout();
initDayNightMode();
initRedact();
getDeviceCounts();
setupSearch();
setupReload();