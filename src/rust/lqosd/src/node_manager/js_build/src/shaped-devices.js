import {clearDiv, formatLastSeen} from "./helpers/builders";
import {get_ws_client, subscribeWS} from "./pubsub/ws";
import {formatRetransmit, formatRtt, formatThroughput} from "./helpers/scaling";
import {colorByQoqScore} from "./helpers/color_scales";
import {toNumber} from "./lq_js_common/helpers/scaling";

let shapedDevices = [];
let displayDevices = [];
let devicesPerPage = 24;
let page = 0;
let searchTerm = "";
let metricElsByCircuitId = new Map();
const latestByCircuitId = new Map();
const wsClient = get_ws_client();

const QOO_TOOLTIP_HTML =
    "<h5>Quality of Outcome (QoO)</h5>" +
    "<p>Quality of Outcome (QoO) is IETF IPPM “Internet Quality” (draft-ietf-ippm-qoo).<br>" +
    "https://datatracker.ietf.org/doc/draft-ietf-ippm-qoo/<br>" +
    "LibreQoS implements a latency and loss-based model to estimate quality of outcome.</p>";

const listenOnce = (eventName, handler) => {
    const wrapped = (msg) => {
        wsClient.off(eventName, wrapped);
        handler(msg);
    };
    wsClient.on(eventName, wrapped);
};

function initTooltipsWithin(rootEl) {
    if (!rootEl) return;
    if (typeof bootstrap === "undefined" || !bootstrap.Tooltip) return;
    const elements = rootEl.querySelectorAll('[data-bs-toggle="tooltip"]');
    elements.forEach((element) => {
        if (bootstrap.Tooltip.getOrCreateInstance) {
            bootstrap.Tooltip.getOrCreateInstance(element);
        } else {
            new bootstrap.Tooltip(element);
        }
    });
}

function formatPlanValue(value) {
    const asNumber = toNumber(value, 0);
    let formatted = parseFloat(asNumber).toFixed(3);
    formatted = formatted.replace(/\.?0+$/, "");
    return formatted;
}

function countCircuits() {
    let entries = {};
    shapedDevices.forEach((d) => {
        if (!entries.hasOwnProperty(d.circuit_id)) {
            entries[d.circuit_id] = 1;
        }
    });
    let count = 0;
    for (const _ in entries) {
        count++;
    }
    return count;
}

function filterDevices() {
    const term = (searchTerm || "").toLowerCase().trim();
    if (term === "") {
        displayDevices = shapedDevices;
    } else {
        displayDevices = shapedDevices.filter((d) => {
            const deviceName = (d.device_name || "").toLowerCase();
            const circuitName = (d.circuit_name || "").toLowerCase();
            const parentNode = (d.parent_node || "").toLowerCase();
            return (
                deviceName.indexOf(term) > -1 ||
                circuitName.indexOf(term) > -1 ||
                parentNode.indexOf(term) > -1
            );
        });
    }
    page = 0;
    renderCards();
}

function formatQooScore(score0to100, fallback = "-") {
    if (score0to100 === null || score0to100 === undefined) {
        return fallback;
    }
    const numeric = Number(score0to100);
    if (!Number.isFinite(numeric) || numeric === 255) {
        return fallback;
    }
    const clamped = Math.min(100, Math.max(0, Math.round(numeric)));
    const color = colorByQoqScore(clamped);
    return "<span class='muted' style='color: " + color + "'>■</span>" + clamped;
}

function formatRttFromNanosOpt(nanosOpt, fallback = "-") {
    if (nanosOpt === null || nanosOpt === undefined) {
        return fallback;
    }
    const nanos = toNumber(nanosOpt, 0);
    if (!Number.isFinite(nanos) || nanos <= 0) {
        return fallback;
    }
    return formatRtt(nanos / 1_000_000.0);
}

function registerMetricEl(circuitId, metricName, el) {
    if (!circuitId || circuitId === "") return;
    let metrics = metricElsByCircuitId.get(circuitId);
    if (!metrics) {
        metrics = {};
        metricElsByCircuitId.set(circuitId, metrics);
    }
    if (!metrics[metricName]) {
        metrics[metricName] = [];
    }
    metrics[metricName].push(el);
}

function updateMetricHtml(circuitId, metricName, html) {
    const metrics = metricElsByCircuitId.get(circuitId);
    if (!metrics || !metrics[metricName]) return;
    metrics[metricName].forEach((el) => {
        el.innerHTML = html;
    });
}

function updateMetricText(circuitId, metricName, text) {
    const metrics = metricElsByCircuitId.get(circuitId);
    if (!metrics || !metrics[metricName]) return;
    metrics[metricName].forEach((el) => {
        el.innerText = text;
    });
}

function applyCircuitUpdate(device) {
    if (!device || !device.circuit_id) return;
    const circuitId = device.circuit_id;
    latestByCircuitId.set(circuitId, device);

    updateMetricText(circuitId, "lastSeen", formatLastSeen(device.last_seen_nanos));
    updateMetricHtml(
        circuitId,
        "tpDown",
        formatThroughput(toNumber(device.bytes_per_second.down, 0) * 8, device.plan.down),
    );
    updateMetricHtml(
        circuitId,
        "tpUp",
        formatThroughput(toNumber(device.bytes_per_second.up, 0) * 8, device.plan.up),
    );
    updateMetricHtml(
        circuitId,
        "rttDown",
        formatRttFromNanosOpt(device.rtt_current_p50_nanos ? device.rtt_current_p50_nanos.down : null),
    );
    updateMetricHtml(
        circuitId,
        "rttUp",
        formatRttFromNanosOpt(device.rtt_current_p50_nanos ? device.rtt_current_p50_nanos.up : null),
    );
    updateMetricHtml(circuitId, "qooDown", formatQooScore(device.qoo ? device.qoo.down : null));
    updateMetricHtml(circuitId, "qooUp", formatQooScore(device.qoo ? device.qoo.up : null));

    const packetsDown = toNumber(device.tcp_packets.down, 0);
    const packetsUp = toNumber(device.tcp_packets.up, 0);
    const retransDown = toNumber(device.tcp_retransmits.down, 0);
    const retransUp = toNumber(device.tcp_retransmits.up, 0);
    const fractionDown = packetsDown > 0 ? retransDown / packetsDown : 0;
    const fractionUp = packetsUp > 0 ? retransUp / packetsUp : 0;
    updateMetricHtml(circuitId, "reXmitDown", formatRetransmit(fractionDown));
    updateMetricHtml(circuitId, "reXmitUp", formatRetransmit(fractionUp));
}

function buildIpListEl(device) {
    const wrapper = document.createElement("div");
    wrapper.classList.add("small", "text-body-secondary");
    const addLine = (text) => {
        const div = document.createElement("div");
        div.innerText = text;
        wrapper.appendChild(div);
    };
    if (Array.isArray(device.ipv4)) {
        device.ipv4.forEach((ip) => {
            if (ip && ip.length >= 2) addLine(ip[0] + "/" + ip[1]);
        });
    }
    if (Array.isArray(device.ipv6)) {
        device.ipv6.forEach((ip) => {
            if (ip && ip.length >= 2) addLine(ip[0] + "/" + ip[1]);
        });
    }
    if (wrapper.children.length === 0) {
        addLine("-");
    }
    return wrapper;
}

function metricTableRow(labelEl, downEl, upEl) {
    const tr = document.createElement("tr");
    tr.classList.add("small");

    const tdLabel = document.createElement("td");
    tdLabel.classList.add("text-body-secondary");
    tdLabel.style.width = "34%";
    tdLabel.appendChild(labelEl);
    tr.appendChild(tdLabel);

    const tdDown = document.createElement("td");
    tdDown.classList.add("text-end");
    tdDown.appendChild(downEl);
    tr.appendChild(tdDown);

    const tdUp = document.createElement("td");
    tdUp.classList.add("text-end");
    tdUp.appendChild(upEl);
    tr.appendChild(tdUp);

    return tr;
}

function buildDeviceCard(device) {
    const card = document.createElement("div");
    card.classList.add("executive-card", "h-100");

    // Header
    const header = document.createElement("div");
    header.classList.add("d-flex", "justify-content-between", "align-items-start", "gap-2");

    const titleWrap = document.createElement("div");
    titleWrap.style.minWidth = "0";

    if (device.circuit_id) {
        const circuitLink = document.createElement("a");
        circuitLink.href = "circuit.html?id=" + encodeURI(device.circuit_id);
        circuitLink.classList.add("redactable", "fw-semibold", "text-decoration-none");
        circuitLink.innerText = device.circuit_name || "(Unknown circuit)";
        titleWrap.appendChild(circuitLink);

        const deviceLink = document.createElement("a");
        deviceLink.href = "circuit.html?id=" + encodeURI(device.circuit_id);
        deviceLink.classList.add("redactable", "d-block", "small", "text-body-secondary", "text-decoration-none");
        deviceLink.innerText = device.device_name || "";
        titleWrap.appendChild(deviceLink);
    } else {
        const circuitName = document.createElement("div");
        circuitName.classList.add("fw-semibold");
        circuitName.innerText = device.circuit_name || "(Unknown circuit)";
        titleWrap.appendChild(circuitName);

        const deviceName = document.createElement("div");
        deviceName.classList.add("small", "text-body-secondary");
        deviceName.innerText = device.device_name || "";
        titleWrap.appendChild(deviceName);
    }

    const planBadge = document.createElement("span");
    planBadge.classList.add("badge", "text-bg-secondary", "exec-badge", "ms-auto");
    planBadge.innerText =
        formatPlanValue(device.download_max_mbps) + " / " + formatPlanValue(device.upload_max_mbps) + " Mbps";

    header.appendChild(titleWrap);
    header.appendChild(planBadge);
    card.appendChild(header);

    // Meta: parent + IPs
    const parent = document.createElement("div");
    parent.classList.add("small", "text-body-secondary", "mt-1", "redactable");
    parent.style.whiteSpace = "nowrap";
    parent.style.overflow = "hidden";
    parent.style.textOverflow = "ellipsis";
    parent.title = device.parent_node || "";
    parent.innerText = device.parent_node || "-";
    card.appendChild(parent);

    const ipList = buildIpListEl(device);
    ipList.classList.add("mt-1");
    card.appendChild(ipList);

    // Last seen
    const lastSeenRow = document.createElement("div");
    lastSeenRow.classList.add("small", "text-body-secondary", "mt-2");
    const lastSeenLabel = document.createElement("span");
    lastSeenLabel.innerText = "Last seen: ";
    const lastSeenValue = document.createElement("span");
    lastSeenValue.innerText = "-";
    lastSeenRow.appendChild(lastSeenLabel);
    lastSeenRow.appendChild(lastSeenValue);
    card.appendChild(lastSeenRow);
    registerMetricEl(device.circuit_id, "lastSeen", lastSeenValue);

    // Metrics table
    const table = document.createElement("table");
    table.classList.add("table", "table-sm", "mb-0", "mt-2");

    const thead = document.createElement("thead");
    const headRow = document.createElement("tr");
    const headMetric = document.createElement("th");
    headMetric.innerText = "";
    const headDl = document.createElement("th");
    headDl.classList.add("text-end");
    headDl.innerHTML = "DL <i class='fa fa-arrow-down'></i>";
    const headUl = document.createElement("th");
    headUl.classList.add("text-end");
    headUl.innerHTML = "UL <i class='fa fa-arrow-up'></i>";
    headRow.appendChild(headMetric);
    headRow.appendChild(headDl);
    headRow.appendChild(headUl);
    thead.appendChild(headRow);
    table.appendChild(thead);

    const tbody = document.createElement("tbody");

    const tpLabel = document.createElement("span");
    tpLabel.innerText = "Throughput";
    const tpDown = document.createElement("span");
    tpDown.innerHTML = "-";
    const tpUp = document.createElement("span");
    tpUp.innerHTML = "-";
    tbody.appendChild(metricTableRow(tpLabel, tpDown, tpUp));
    registerMetricEl(device.circuit_id, "tpDown", tpDown);
    registerMetricEl(device.circuit_id, "tpUp", tpUp);

    const rttLabel = document.createElement("span");
    rttLabel.innerText = "RTT";
    const rttDown = document.createElement("span");
    rttDown.innerHTML = "-";
    const rttUp = document.createElement("span");
    rttUp.innerHTML = "-";
    tbody.appendChild(metricTableRow(rttLabel, rttDown, rttUp));
    registerMetricEl(device.circuit_id, "rttDown", rttDown);
    registerMetricEl(device.circuit_id, "rttUp", rttUp);

    const qooLabelWrap = document.createElement("span");
    qooLabelWrap.innerHTML = "QoO <i class='fas fa-info-circle'></i>";
    qooLabelWrap.setAttribute("data-bs-toggle", "tooltip");
    qooLabelWrap.setAttribute("data-bs-placement", "top");
    qooLabelWrap.setAttribute("data-bs-html", "true");
    qooLabelWrap.setAttribute("title", QOO_TOOLTIP_HTML);
    const qooDown = document.createElement("span");
    qooDown.innerHTML = "-";
    const qooUp = document.createElement("span");
    qooUp.innerHTML = "-";
    tbody.appendChild(metricTableRow(qooLabelWrap, qooDown, qooUp));
    registerMetricEl(device.circuit_id, "qooDown", qooDown);
    registerMetricEl(device.circuit_id, "qooUp", qooUp);

    const rxLabel = document.createElement("span");
    rxLabel.innerText = "Retransmits";
    const rxDown = document.createElement("span");
    rxDown.innerHTML = "-";
    const rxUp = document.createElement("span");
    rxUp.innerHTML = "-";
    tbody.appendChild(metricTableRow(rxLabel, rxDown, rxUp));
    registerMetricEl(device.circuit_id, "reXmitDown", rxDown);
    registerMetricEl(device.circuit_id, "reXmitUp", rxUp);

    table.appendChild(tbody);
    card.appendChild(table);

    return card;
}

function ensureLayout() {
    const target = document.getElementById("deviceTable");
    if (!target) return null;

    let toolbar = document.getElementById("sdToolbar");
    let grid = document.getElementById("sdCardsGrid");
    if (toolbar && grid) {
        return {
            target,
            toolbar,
            grid,
            searchInput: document.getElementById("sdSearch"),
            perPageSelect: document.getElementById("sdPerPage"),
            prevButton: document.getElementById("sdPrevPage"),
            nextButton: document.getElementById("sdNextPage"),
            pageCounter: document.getElementById("sdPageCounter"),
            summary: document.getElementById("sdSummary"),
        };
    }

    clearDiv(target);

    toolbar = document.createElement("div");
    toolbar.id = "sdToolbar";
    toolbar.classList.add("d-flex", "flex-wrap", "align-items-center", "gap-2", "mb-1");

    const searchGroup = document.createElement("div");
    searchGroup.classList.add("input-group", "input-group-sm");
    searchGroup.style.maxWidth = "340px";

    const searchIcon = document.createElement("span");
    searchIcon.classList.add("input-group-text");
    searchIcon.innerHTML = "<i class='fa fa-search'></i>";
    searchGroup.appendChild(searchIcon);

    const searchInput = document.createElement("input");
    searchInput.type = "text";
    searchInput.classList.add("form-control");
    searchInput.id = "sdSearch";
    searchInput.placeholder = "Search circuits, devices, parents…";
    searchInput.value = searchTerm;
    searchInput.oninput = () => {
        searchTerm = $("#sdSearch").val();
        filterDevices();
    };
    searchInput.onkeydown = (event) => {
        if (event.keyCode === 13) {
            searchTerm = $("#sdSearch").val();
            filterDevices();
        }
    };
    searchGroup.appendChild(searchInput);
    toolbar.appendChild(searchGroup);

    const perPageWrap = document.createElement("div");
    perPageWrap.classList.add("d-flex", "align-items-center", "gap-1");
    const perPageLabel = document.createElement("span");
    perPageLabel.classList.add("small", "text-body-secondary");
    perPageLabel.innerText = "Per page";
    const perPageSelect = document.createElement("select");
    perPageSelect.id = "sdPerPage";
    perPageSelect.classList.add("form-select", "form-select-sm");
    [12, 24, 48, 96].forEach((n) => {
        const opt = document.createElement("option");
        opt.value = String(n);
        opt.innerText = String(n);
        if (n === devicesPerPage) opt.selected = true;
        perPageSelect.appendChild(opt);
    });
    perPageSelect.onchange = () => {
        devicesPerPage = parseInt(perPageSelect.value, 10);
        if (!Number.isFinite(devicesPerPage) || devicesPerPage <= 0) {
            devicesPerPage = 24;
        }
        page = 0;
        renderCards();
    };
    perPageWrap.appendChild(perPageLabel);
    perPageWrap.appendChild(perPageSelect);
    toolbar.appendChild(perPageWrap);

    const pagerWrap = document.createElement("div");
    pagerWrap.classList.add("d-flex", "align-items-center", "gap-2");
    const pager = document.createElement("div");
    pager.classList.add("btn-group", "btn-group-sm");

    const prev = document.createElement("button");
    prev.id = "sdPrevPage";
    prev.type = "button";
    prev.classList.add("btn", "btn-secondary");
    prev.innerHTML = "<i class='fa fa-arrow-left'></i>";
    prev.onclick = () => {
        page = Math.max(0, page - 1);
        renderCards();
    };
    const next = document.createElement("button");
    next.id = "sdNextPage";
    next.type = "button";
    next.classList.add("btn", "btn-secondary");
    next.innerHTML = "<i class='fa fa-arrow-right'></i>";
    next.onclick = () => {
        const totalPages = Math.max(1, Math.ceil(displayDevices.length / devicesPerPage));
        page = Math.min(totalPages - 1, page + 1);
        renderCards();
    };
    pager.appendChild(prev);
    pager.appendChild(next);
    pagerWrap.appendChild(pager);

    const pageCounter = document.createElement("span");
    pageCounter.id = "sdPageCounter";
    pageCounter.classList.add("small", "text-body-secondary");
    pagerWrap.appendChild(pageCounter);

    toolbar.appendChild(pagerWrap);

    const summary = document.createElement("div");
    summary.id = "sdSummary";
    summary.classList.add("small", "text-body-secondary", "ms-auto");
    toolbar.appendChild(summary);

    target.appendChild(toolbar);

    grid = document.createElement("div");
    grid.id = "sdCardsGrid";
    grid.classList.add("row", "row-cols-1", "row-cols-md-2", "row-cols-xl-3", "g-3");
    target.appendChild(grid);

    return {
        target,
        toolbar,
        grid,
        searchInput,
        perPageSelect,
        prevButton: prev,
        nextButton: next,
        pageCounter,
        summary,
    };
}

function renderCards() {
    const layout = ensureLayout();
    if (!layout) return;

    const totalPages = Math.max(1, Math.ceil(displayDevices.length / devicesPerPage));
    if (page >= totalPages) page = totalPages - 1;
    if (page < 0) page = 0;

    if (layout.perPageSelect && String(devicesPerPage) !== layout.perPageSelect.value) {
        layout.perPageSelect.value = String(devicesPerPage);
    }
    if (layout.prevButton) layout.prevButton.disabled = page <= 0;
    if (layout.nextButton) layout.nextButton.disabled = page >= totalPages - 1;
    if (layout.pageCounter) layout.pageCounter.innerText = "Page " + (page + 1) + " / " + totalPages;
    if (layout.summary) {
        if (displayDevices.length === 0) {
            layout.summary.innerText = "No matches";
        } else {
            const start = page * devicesPerPage + 1;
            const end = Math.min((page + 1) * devicesPerPage, displayDevices.length);
            layout.summary.innerText = "Showing " + start + "–" + end + " of " + displayDevices.length;
        }
    }

    metricElsByCircuitId = new Map();
    clearDiv(layout.grid);

    const start = page * devicesPerPage;
    const end = Math.min(start + devicesPerPage, displayDevices.length);
    for (let i = start; i < end; i++) {
        const device = displayDevices[i];
        const col = document.createElement("div");
        col.classList.add("col");
        col.appendChild(buildDeviceCard(device));
        layout.grid.appendChild(col);
    }

    // Fill visible cards from cached live data immediately.
    metricElsByCircuitId.forEach((_metrics, circuitId) => {
        const latest = latestByCircuitId.get(circuitId);
        if (latest) {
            applyCircuitUpdate(latest);
        }
    });

    initTooltipsWithin(layout.target);
}

function loadDevices() {
    listenOnce("DevicesAll", (msg) => {
        const data = msg && msg.data ? msg.data : [];
        shapedDevices = data;
        displayDevices = data;
        renderCards();
        $("#count").text(shapedDevices.length + " devices");
        $("#countCircuit").text(countCircuits() + " circuits");
    });
    wsClient.send({ DevicesAll: {} });
}

loadDevices();
subscribeWS(["NetworkTreeClients"], (msg) => {
    if (msg.event !== "NetworkTreeClients") return;
    const data = msg && msg.data ? msg.data : [];
    data.forEach((d) => {
        if (!d || !d.circuit_id) return;
        applyCircuitUpdate(d);
    });
});
