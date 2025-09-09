import {clearDiv} from "./helpers/builders";
import {initRedact} from "./helpers/redact";
import {initDayNightMode} from "./helpers/dark_mode";
import {initColorBlind} from "./helpers/colorblind";

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
                // Position panel near the search input for consistent placement
                const inp = document.getElementById("txtSearch");
                if (inp && searchResults) {
                    const rect = inp.getBoundingClientRect();
                    // Use fixed positioning relative to viewport
                    searchResults.style.position = 'fixed';
                    searchResults.style.top = (rect.bottom + 8) + 'px';
                    searchResults.style.left = rect.left + 'px';
                    const widthPx = Math.max(320, rect.width + 200);
                    searchResults.style.minWidth = widthPx + 'px';
                    searchResults.style.width = widthPx + 'px';
                    // Ensure it's not shifted or hidden by existing CSS
                    searchResults.style.transform = 'none';
                    searchResults.style.zIndex = '2000';
                    searchResults.style.padding = '6px';
                }
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

// Simple debounce helper
function debounce(fn, delay) {
    let timer = null;
    return function(...args) {
        clearTimeout(timer);
        timer = setTimeout(() => fn.apply(this, args), delay);
    }
}

function setupSearch() {
    const hideResults = () => {
        const panel = document.getElementById("searchResults");
        if (panel) panel.style.visibility = "hidden";
    };
    const showResults = () => {
        const panel = document.getElementById("searchResults");
        if (panel) panel.style.visibility = "visible";
    };

    $("#btnSearch").on('click', () => {
        const search = $("#txtSearch").val();
        doSearch(search);
    });
    const debouncedSearch = debounce(() => {
        const search = $("#txtSearch").val();
        doSearch(search);
    }, 300);
    $("#txtSearch").on('keyup', debouncedSearch);

    // Reposition results on resize/scroll to keep anchored under input on index
    const repositionResults = () => {
        const inp = document.getElementById('txtSearch');
        const panel = document.getElementById('searchResults');
        if (!inp || !panel || panel.style.visibility !== 'visible') return;
        const rect = inp.getBoundingClientRect();
        const widthPx = Math.max(320, rect.width + 200);
        panel.style.position = 'fixed';
        panel.style.top = (rect.bottom + 8) + 'px';
        panel.style.left = rect.left + 'px';
        panel.style.width = widthPx + 'px';
        panel.style.minWidth = widthPx + 'px';
        panel.style.transform = 'none';
        panel.style.zIndex = '2000';
        panel.style.padding = '6px';
    };
    window.addEventListener('resize', repositionResults);
    window.addEventListener('scroll', repositionResults, true);

    // Focus shows results if available
    $("#txtSearch").on('focus', () => {
        if ($("#txtSearch").val().length > 2) showResults();
    });
    // Blur hides results after short delay to allow clicking results
    $("#txtSearch").on('blur', () => {
        setTimeout(hideResults, 150);
    });

    // Add this new key handler for '/' to focus search
    $(document).on('keydown', (e) => {
        if (e.key === '/' && !$(e.target).is('input, textarea, select')) {
            e.preventDefault();
            $('#txtSearch').focus();
            showResults();
        } else if (e.key === 'Escape') {
            hideResults();
            if ($(e.target).is('#txtSearch')) {
                $('#txtSearch').blur();
            }
        }
    });

    // Click-away to close results
    $(document).on('click', (e) => {
        if ($(e.target).closest('#searchResults, #txtSearch, #btnSearch').length === 0) {
            hideResults();
        }
    });
    // Prevent clicks inside the results from bubbling
    $("#searchResults").on('click', (e) => { e.stopPropagation(); });
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
initColorBlind();
getDeviceCounts();
setupSearch();
setupReload();
