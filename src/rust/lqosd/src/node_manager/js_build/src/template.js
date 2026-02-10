import {clearDiv} from "./helpers/builders";
import {initRedact} from "./helpers/redact";
import {initDayNightMode} from "./helpers/dark_mode";
import {initColorBlind} from "./helpers/colorblind";
import {get_ws_client} from "./pubsub/ws";

const wsClient = get_ws_client();
const listenOnce = (eventName, handler) => {
    const wrapped = (msg) => {
        wsClient.off(eventName, wrapped);
        handler(msg);
    };
    wsClient.on(eventName, wrapped);
};

function escapeAttr(text) {
    if (text === undefined || text === null) return "";
    return String(text)
        .replaceAll('&', '&amp;')
        .replaceAll('"', '&quot;')
        .replaceAll("'", '&apos;')
        .replaceAll('<', '&lt;')
        .replaceAll('>', '&gt;');
}

function loadSchedulerStatus() {
    const container = document.getElementById('schedulerStatus');
    if (!container) return;
    listenOnce("SchedulerStatus", (msg) => {
        if (!msg || !msg.data) return;
        const data = msg.data;
        const color = data.available ? 'text-success' : 'text-danger';
        const icon = data.available ? 'fa-check-circle' : 'fa-times-circle';
        container.innerHTML = `
            <a class="nav-link ${color}" href="#" id="schedulerStatusLink">
                <i class="fa fa-fw fa-centerline ${icon}"></i> Scheduler
            </a>`;

        // Click opens details modal only; no tooltip
        $('#schedulerStatus').off('click').on('click', '#schedulerStatusLink', (e) => {
            e.preventDefault();
            openSchedulerModal();
        });
    });
    wsClient.send({ SchedulerStatus: {} });
}

function openSchedulerModal() {
    const modalEl = document.getElementById('schedulerModal');
    if (!modalEl) return;
    const myModal = new bootstrap.Modal(modalEl, { focus: true });
    myModal.show();
    $("#schedulerDetailsBody").html("<i class='fa fa-spinner fa-spin'></i> Loading scheduler status...");
    listenOnce("SchedulerDetails", (msg) => {
        if (!msg || !msg.data) {
            $("#schedulerDetailsBody").text('Failed to load scheduler details');
            return;
        }
        $("#schedulerDetailsBody").text(msg.data.details);
    });
    wsClient.send({ SchedulerDetails: {} });
}

function getDeviceCounts() {
    listenOnce("DeviceCount", (msg) => {
        if (!msg || !msg.data) return;
        $("#shapedDeviceCount").text(msg.data.shaped_devices);
        $("#unknownIpCount").text(msg.data.unknown_ips);
    });
    wsClient.send({ DeviceCount: {} });
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

let lastSearchTerm = "";
let searchHandlerReady = false;

function renderSearchResults(data) {
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
}

function doSearch(search) {
    if (search.length > 2) {
        lastSearchTerm = search;
        if (!searchHandlerReady) {
            wsClient.on("SearchResults", (msg) => {
                if (!msg || msg.term !== lastSearchTerm) {
                    return;
                }
                renderSearchResults(msg.results || []);
            });
            searchHandlerReady = true;
        }
        wsClient.send({ Search: { term: search } });
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
        listenOnce("ReloadResult", (msg) => {
            if (!msg) {
                $("#reloadLibreResult").text("Failed to reload LibreQoS");
                return;
            }
            $("#reloadLibreResult").text(msg.message || "");
        });
        wsClient.send({ ReloadLibreQoS: {} });
    }
}

function setupDynamicUrls() {
    // Get the current host and protocol from the browser
    const currentHost = window.location.hostname;
    const currentProtocol = window.location.protocol;
    
    // Construct API URL (port 9122)
    // The Swagger UI lives at /api-docs/ (dash, trailing slash)
    const apiUrl = `${currentProtocol}//${currentHost}:9122/api-docs/`;
    
    // Construct Chat URL (port 9121)
    const chatUrl = `${currentProtocol}//${currentHost}:9121/`;
    
    // Update API link only if it has the placeholder
    const apiLink = document.getElementById('apiLink');
    if (apiLink) {
        const hrefAttr = apiLink.getAttribute('href');
        if (hrefAttr === '%%API_URL%%') {
            apiLink.href = apiUrl;
        }
    }
    
    // Update Chat link if it exists (only created when chatbot is available)
    const chatLink = document.getElementById('chatLink');
    if (chatLink) {
        // If server rendered a disabled span, swap it for an active link.
        if (chatLink.tagName && chatLink.tagName.toLowerCase() !== 'a') {
            const parentLi = chatLink.closest('li');
            const a = document.createElement('a');
            a.className = 'nav-link';
            a.id = 'chatLink';
            a.href = 'chatbot.html';
            a.innerHTML = '<i class="fa fa-fw fa-centerline fa-comments nav-icon"></i> Ask Libby';
            if (parentLi) parentLi.replaceChild(a, chatLink); else chatLink.replaceWith(a);
        } else {
            const hrefAttr = chatLink.getAttribute('href');
            if (hrefAttr === '%%CHAT_URL%%' || !hrefAttr) {
                // Prefer embedded chatbot page
                chatLink.href = 'chatbot.html';
            }
        }
    }
}

function initUrgentIssues() {
    const containerId = 'urgentStatus';
    const linkId = 'urgentStatusLink';
    const badgeId = 'urgentBadge';

    function ensurePlaceholder() {
        return document.getElementById(containerId) !== null;
    }

    function renderStatus(count) {
        const cont = document.getElementById(containerId);
        if (!cont) return;
        const cls = count > 0 ? 'text-danger' : 'text-secondary';
        const icon = count > 0 ? 'fa-bell' : 'fa-bell-slash';
        cont.innerHTML = `
            <a class="nav-link ${cls}" href="#" id="${linkId}">
                <i class="fa fa-fw fa-centerline ${icon}"></i> Urgent Issues
                <span id="${badgeId}" class="badge bg-danger ${count>0?'':'d-none'}">${count}</span>
            </a>`;
        $("#" + containerId).off("click").on("click", `#${linkId}`, (e) => {
            e.preventDefault();
            showModal();
        });
    }

    function poll() {
        if (!ensurePlaceholder()) return;
        listenOnce("UrgentStatus", (msg) => {
            const count = msg && msg.data ? msg.data.count : 0;
            renderStatus(count || 0);
        });
        wsClient.send({ UrgentStatus: {} });
    }

    function showModal() {
        const modalEl = document.getElementById('urgentModal');
        if (!modalEl) return;
        new bootstrap.Modal(modalEl, { focus: true }).show();
        const holder = document.getElementById('urgentListContainer');
        if (!holder) return;
        holder.innerHTML = `<div class="text-center text-muted"><i class='fa fa-spinner fa-spin'></i> Loading...</div>`;
        listenOnce("UrgentList", (msg) => {
            const items = msg && msg.data ? msg.data.items || [] : [];
            if (items.length === 0) {
                holder.innerHTML = '<div class="text-center text-success">No urgent issues.</div>';
                return;
            }
            const table = document.createElement('table');
            table.className = 'table table-sm table-striped';
            const tbody = document.createElement('tbody');
            items.forEach((it) => {
                const tr = document.createElement('tr');
                const td = document.createElement('td');
                const when = new Date(it.ts * 1000).toLocaleString();
                const sev = it.severity === 'Error' ? 'danger' : 'warning';
                td.innerHTML = `
                    <div>
                        <span class="badge bg-${sev}">${it.severity}</span>
                        <strong class="ms-2">${it.code}</strong>
                        <span class="text-muted ms-2">(${it.source})</span>
                        <span class="text-muted float-end">${when}</span>
                        <a href="#" class="text-secondary float-end ms-3 urgent-clear" data-id="${it.id}" title="Acknowledge"><i class="fa fa-times"></i></a>
                    </div>
                    <div class="mt-1" style="white-space: pre-wrap;">${it.message}</div>
                    ${it.context ? `<pre class="mt-2">${it.context}</pre>` : ''}
                    `;
                tr.appendChild(td);
                tbody.appendChild(tr);
            });
            table.appendChild(tbody);
            holder.innerHTML = '';
            holder.appendChild(table);
            $(holder).off('click').on('click', 'a.urgent-clear', function (e) {
                e.preventDefault();
                const id = $(this).data('id');
                listenOnce("UrgentClearResult", () => {
                    showModal();
                    poll();
                });
                wsClient.send({ UrgentClear: { id } });
            });
        });
        wsClient.send({ UrgentList: {} });
    }

    if (!document.getElementById(containerId)) {
        const ul = document.querySelector('.sidebar .navbar-nav');
        if (ul) {
            const li = document.createElement('li');
            li.className = 'nav-item';
            li.id = containerId;
            ul.appendChild(li);
        }
    }

    $(document).off('click', '#urgentClearAll').on('click', '#urgentClearAll', () => {
        listenOnce("UrgentClearAllResult", () => {
            showModal();
            poll();
        });
        wsClient.send({ UrgentClearAll: {} });
    });

    poll();
    setInterval(poll, 30000);
}

function initSchedulerTooltips() {
    // Initialize Bootstrap tooltips for scheduler status elements
    const schedulerElements = document.querySelectorAll('[data-bs-toggle="tooltip"]');
    schedulerElements.forEach(element => {
        new bootstrap.Tooltip(element);
    });
}

initLogout();
initDayNightMode();
initRedact();
initColorBlind();
getDeviceCounts();
setupSearch();
setupReload();
setupDynamicUrls();
window.lqosInitUrgentIssues = initUrgentIssues;
initSchedulerTooltips();
loadSchedulerStatus();
