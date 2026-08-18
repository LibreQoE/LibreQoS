import {clearDiv, formatLastSeen} from "./helpers/builders";
import {get_ws_client} from "./pubsub/ws";
import {
    formatRetransmit,
    formatRtt,
    formatThroughput,
    retransmitFractionFromSample,
} from "./helpers/scaling";
import {colorByQoqScore} from "./helpers/color_scales";
import {toNumber} from "./lq_js_common/helpers/scaling";

let shapedDevices = [];
let devicesPerPage = 24;
let page = 0;
let searchTerm = "";
let shapedDevicesKind = "Static";
let metricElsByCircuitId = new Map();
const latestByCircuitId = new Map();
const planByCircuitId = new Map();
let totalRows = 0;
let totalCircuits = 0;
let circuitMetricsWatchSignature = null;
const wsClient = get_ws_client();
const DYNAMIC_VIEW_MODE_STORAGE_KEY = "lqosShapedDevicesDynamicViewMode";
let dynamicViewMode = loadDynamicViewMode();

const QOO_TOOLTIP_HTML =
    "<h5>Quality of Outcome (QoO)</h5>" +
    "<p>Quality of Outcome (QoO) is IETF IPPM “Internet Quality” (draft-ietf-ippm-qoo).<br>" +
    "https://datatracker.ietf.org/doc/draft-ietf-ippm-qoo/<br>" +
    "LibreQoS implements a latency and loss-based model to estimate quality of outcome.</p>";

function sendWsRequest(responseEvent, request) {
    return new Promise((resolve, reject) => {
        let done = false;
        const responseHandler = (msg) => {
            if (done) return;
            done = true;
            wsClient.off(responseEvent, responseHandler);
            wsClient.off("Error", errorHandler);
            resolve(msg);
        };
        const errorHandler = (msg) => {
            if (done) return;
            done = true;
            wsClient.off(responseEvent, responseHandler);
            wsClient.off("Error", errorHandler);
            reject(msg);
        };
        wsClient.on(responseEvent, responseHandler);
        wsClient.on("Error", errorHandler);
        wsClient.send(request);
    });
}

function sendPrivateRequest(command) {
    wsClient.send({Private: command});
}

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

function normalizeDynamicViewMode(mode) {
    return mode === "list" ? "list" : "cards";
}

function loadDynamicViewMode() {
    try {
        if (window.localStorage) {
            return normalizeDynamicViewMode(window.localStorage.getItem(DYNAMIC_VIEW_MODE_STORAGE_KEY));
        }
    } catch (_error) {
        // Ignore storage failures and fall back to the default cards view.
    }
    return "cards";
}

function persistDynamicViewMode(mode) {
    try {
        if (window.localStorage) {
            window.localStorage.setItem(
                DYNAMIC_VIEW_MODE_STORAGE_KEY,
                normalizeDynamicViewMode(mode),
            );
        }
    } catch (_error) {
        // Ignore storage failures and keep the current in-memory preference.
    }
}

function isDynamicListView() {
    return shapedDevicesKind === "Dynamic" && dynamicViewMode === "list";
}

function countCircuits() {
    return totalCircuits;
}

function updateKindTabs() {
    const staticTab = document.getElementById("sdTabStatic");
    const dynamicTab = document.getElementById("sdTabDynamic");
    if (!staticTab || !dynamicTab) {
        return;
    }
    const isDynamic = shapedDevicesKind === "Dynamic";
    staticTab.classList.toggle("active", !isDynamic);
    dynamicTab.classList.toggle("active", isDynamic);
    staticTab.setAttribute("aria-selected", (!isDynamic).toString());
    dynamicTab.setAttribute("aria-selected", isDynamic.toString());
}

function renderCounts() {
    const countEl = $("#count");
    const circuitEl = $("#countCircuit");
    if (shapedDevicesKind === "Dynamic") {
        countEl.text(totalRows + " dynamic circuits");
        circuitEl.text("");
        circuitEl.addClass("d-none");
    } else {
        countEl.text(totalRows + " devices");
        circuitEl.text(totalCircuits + " circuits");
        circuitEl.removeClass("d-none");
    }
}

function setShapedDevicesKind(kind) {
    if (!kind || (kind !== "Static" && kind !== "Dynamic")) {
        kind = "Static";
    }
    if (shapedDevicesKind === kind) {
        return;
    }
    shapedDevicesKind = kind;
    page = 0;
    searchTerm = "";
    const searchBox = document.getElementById("sdSearch");
    if (searchBox) {
        searchBox.value = "";
    }
    circuitMetricsWatchSignature = null;
    updateKindTabs();
    requestShapedDevicesPage();
}

function setDynamicViewMode(mode) {
    const normalized = normalizeDynamicViewMode(mode);
    if (dynamicViewMode === normalized) {
        return;
    }
    dynamicViewMode = normalized;
    persistDynamicViewMode(dynamicViewMode);
    renderDevices();
}

function filterDevices() {
    page = 0;
    requestShapedDevicesPage();
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

function formatIpAddress(address, family) {
    if (address === null || address === undefined) return "";
    if (Array.isArray(address)) {
        if (family === 4 && address.length === 4) {
            return address.map((part) => String(part)).join(".");
        }
        if (family === 6) {
            if (address.length === 16) {
                const groups = [];
                for (let i = 0; i < 16; i += 2) {
                    const high = Number(address[i]) || 0;
                    const low = Number(address[i + 1]) || 0;
                    const value = ((high << 8) | low) >>> 0;
                    groups.push(value.toString(16));
                }
                return groups.join(":");
            }
            if (address.length === 8) {
                return address.map((part) => Number(part).toString(16)).join(":");
            }
            return address.map((part) => String(part)).join(":");
        }
        return address.map((part) => String(part)).join(".");
    }
    return String(address);
}

function formatIpTuple(tuple, defaultPrefix) {
    if (!tuple || tuple.length === 0) return "";
    const family = defaultPrefix === 128 ? 6 : 4;
    const addr = formatIpAddress(tuple[0], family);
    const prefix = Number.isFinite(tuple[1]) ? tuple[1] : defaultPrefix;
    if (!addr) return "";
    return `${addr}/${prefix}`;
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
    const plan = planByCircuitId.get(circuitId) || {down: 0, up: 0};

    updateMetricText(circuitId, "lastSeen", formatLastSeen(device.last_seen_nanos));
    updateMetricHtml(
        circuitId,
        "tpDown",
        formatThroughput(toNumber(device.bytes_per_second.down, 0) * 8, toNumber(plan.down, 0)),
    );
    updateMetricHtml(
        circuitId,
        "tpUp",
        formatThroughput(toNumber(device.bytes_per_second.up, 0) * 8, toNumber(plan.up, 0)),
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

    const fractionDown = retransmitFractionFromSample(device.tcp_retransmit_sample?.down);
    const fractionUp = retransmitFractionFromSample(device.tcp_retransmit_sample?.up);
    updateMetricHtml(circuitId, "reXmitDown", formatRetransmit(fractionDown));
    updateMetricHtml(circuitId, "reXmitUp", formatRetransmit(fractionUp));
}

function buildIpListEl(device) {
    const wrapper = document.createElement("div");
    wrapper.classList.add("small", "text-body-secondary", "redactable");
    const addLine = (text) => {
        const div = document.createElement("div");
        div.innerText = text;
        wrapper.appendChild(div);
    };
    if (Array.isArray(device.ipv4)) {
        device.ipv4.forEach((ip) => {
            const formatted = formatIpTuple(ip, 32);
            if (formatted) addLine(formatted);
        });
    }
    if (Array.isArray(device.ipv6)) {
        device.ipv6.forEach((ip) => {
            const formatted = formatIpTuple(ip, 128);
            if (formatted) addLine(formatted);
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

function buildDeviceIdentity(device, options = {}) {
    const {
        circuitClasses = [],
        deviceClasses = [],
    } = options;
    const wrapper = document.createElement("div");
    wrapper.style.minWidth = "0";

    if (device.circuit_id) {
        const circuitLink = document.createElement("a");
        circuitLink.href = "circuit.html?id=" + encodeURI(device.circuit_id);
        circuitClasses.forEach((className) => circuitLink.classList.add(className));
        circuitLink.innerText = device.circuit_name || "(Unknown circuit)";
        wrapper.appendChild(circuitLink);

        if (device.device_name) {
            const deviceLink = document.createElement("a");
            deviceLink.href = "circuit.html?id=" + encodeURI(device.circuit_id);
            deviceClasses.forEach((className) => deviceLink.classList.add(className));
            deviceLink.innerText = device.device_name;
            wrapper.appendChild(deviceLink);
        }
    } else {
        const circuitName = document.createElement("div");
        circuitClasses.forEach((className) => circuitName.classList.add(className));
        circuitName.innerText = device.circuit_name || "(Unknown circuit)";
        wrapper.appendChild(circuitName);

        if (device.device_name) {
            const deviceName = document.createElement("div");
            deviceClasses.forEach((className) => deviceName.classList.add(className));
            deviceName.innerText = device.device_name;
            wrapper.appendChild(deviceName);
        }
    }

    return wrapper;
}

function buildPlanBadge(device, extraClasses = []) {
    const badge = document.createElement("span");
    badge.classList.add("badge", "text-bg-secondary", "exec-badge", ...extraClasses);
    badge.innerText =
        formatPlanValue(device.download_max_mbps) + " / " + formatPlanValue(device.upload_max_mbps) + " Mbps";
    return badge;
}

function buildMetricValueEl() {
    const span = document.createElement("span");
    span.innerHTML = "-";
    return span;
}

function buildMetricPairCell(circuitId, downMetricName, upMetricName) {
    const wrapper = document.createElement("div");
    wrapper.classList.add("lqos-direction-metric");

    const addLine = (label, metricName) => {
        const line = document.createElement("div");
        line.classList.add("lqos-direction-metric-line");

        const labelEl = document.createElement("span");
        labelEl.classList.add("lqos-direction-metric-label");
        labelEl.innerText = label;
        line.appendChild(labelEl);

        const valueEl = buildMetricValueEl();
        line.appendChild(valueEl);
        wrapper.appendChild(line);
        registerMetricEl(circuitId, metricName, valueEl);
    };

    addLine("DL", downMetricName);
    addLine("UL", upMetricName);

    return wrapper;
}

function appendListCell(row, content, classNames = []) {
    const td = document.createElement("td");
    classNames.forEach((className) => td.classList.add(className));
    if (typeof content === "string") {
        td.innerText = content;
    } else if (content) {
        td.appendChild(content);
    }
    row.appendChild(td);
    return td;
}

function buildDeviceCard(device) {
    const card = document.createElement("div");
    card.classList.add("executive-card", "h-100");

    // Header
    const header = document.createElement("div");
    header.classList.add("d-flex", "justify-content-between", "align-items-start", "gap-2");

    const titleWrap = buildDeviceIdentity(device, {
        circuitClasses: ["redactable", "fw-semibold", "text-decoration-none"],
        deviceClasses: ["redactable", "d-block", "small", "text-body-secondary", "text-decoration-none"],
    });

    const planBadge = buildPlanBadge(device, ["ms-auto"]);

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
    table.classList.add("lqos-table", "lqos-table-tight", "mb-0", "mt-2");

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

function buildDeviceListRow(device) {
    const row = document.createElement("tr");
    row.classList.add("align-middle");

    const identity = buildDeviceIdentity(device, {
        circuitClasses: ["redactable", "fw-semibold", "text-decoration-none"],
        deviceClasses: ["redactable", "d-block", "small", "text-body-secondary", "text-decoration-none"],
    });
    appendListCell(row, identity);

    const parent = document.createElement("div");
    parent.classList.add("small", "redactable");
    parent.innerText = device.parent_node || "-";
    appendListCell(row, parent);

    appendListCell(row, buildPlanBadge(device));
    appendListCell(row, buildMetricPairCell(device.circuit_id, "tpDown", "tpUp"), ["text-nowrap"]);
    appendListCell(row, buildMetricPairCell(device.circuit_id, "rttDown", "rttUp"), ["text-nowrap"]);
    appendListCell(row, buildMetricPairCell(device.circuit_id, "qooDown", "qooUp"), ["text-nowrap"]);
    appendListCell(row, buildMetricPairCell(device.circuit_id, "reXmitDown", "reXmitUp"), ["text-nowrap"]);

    const lastSeenValue = document.createElement("span");
    lastSeenValue.innerText = "-";
    appendListCell(row, lastSeenValue, ["text-nowrap"]);
    registerMetricEl(device.circuit_id, "lastSeen", lastSeenValue);

    appendListCell(row, buildIpListEl(device), ["text-nowrap"]);

    return row;
}

function ensureLayout() {
    const target = document.getElementById("deviceTable");
    if (!target) return null;

    let toolbar = document.getElementById("sdToolbar");
    let grid = document.getElementById("sdCardsGrid");
    let listWrap = document.getElementById("sdListWrap");
    let listBody = document.getElementById("sdListBody");
    if (toolbar && grid && listWrap && listBody) {
        return {
            target,
            toolbar,
            grid,
            listWrap,
            listBody,
            searchInput: document.getElementById("sdSearch"),
            perPageSelect: document.getElementById("sdPerPage"),
            viewToggleWrap: document.getElementById("sdViewToggleWrap"),
            viewCardsButton: document.getElementById("sdViewCards"),
            viewListButton: document.getElementById("sdViewList"),
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
    const perPageLabel = document.createElement("label");
    perPageLabel.classList.add("small", "text-body-secondary");
    perPageLabel.innerText = "Per page";
    const perPageSelect = document.createElement("select");
    perPageSelect.id = "sdPerPage";
    perPageSelect.classList.add("form-select", "form-select-sm");
    perPageLabel.htmlFor = perPageSelect.id;
    perPageSelect.setAttribute("aria-label", "Devices per page");
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
        requestShapedDevicesPage();
    };
    perPageWrap.appendChild(perPageLabel);
    perPageWrap.appendChild(perPageSelect);
    toolbar.appendChild(perPageWrap);

    const viewToggleWrap = document.createElement("div");
    viewToggleWrap.id = "sdViewToggleWrap";
    viewToggleWrap.classList.add("d-none", "d-flex", "align-items-center", "gap-1");
    const viewLabel = document.createElement("span");
    viewLabel.classList.add("small", "text-body-secondary");
    viewLabel.innerText = "View";
    const viewToggle = document.createElement("div");
    viewToggle.classList.add("btn-group", "btn-group-sm", "lqos-shaped-devices-view-toggle");
    viewToggle.setAttribute("role", "group");
    viewToggle.setAttribute("aria-label", "Dynamic circuits view");

    const viewCards = document.createElement("button");
    viewCards.id = "sdViewCards";
    viewCards.type = "button";
    viewCards.classList.add("btn", "btn-outline-secondary");
    viewCards.innerHTML = "<i class='fa fa-th-large me-1'></i>Cards";
    viewCards.onclick = () => {
        setDynamicViewMode("cards");
    };

    const viewList = document.createElement("button");
    viewList.id = "sdViewList";
    viewList.type = "button";
    viewList.classList.add("btn", "btn-outline-secondary");
    viewList.innerHTML = "<i class='fa fa-list me-1'></i>List";
    viewList.onclick = () => {
        setDynamicViewMode("list");
    };

    viewToggle.appendChild(viewCards);
    viewToggle.appendChild(viewList);
    viewToggleWrap.appendChild(viewLabel);
    viewToggleWrap.appendChild(viewToggle);
    toolbar.appendChild(viewToggleWrap);

    const pagerWrap = document.createElement("div");
    pagerWrap.classList.add("d-flex", "align-items-center", "gap-2");
    const pager = document.createElement("div");
    pager.classList.add("btn-group", "btn-group-sm");

    const prev = document.createElement("button");
    prev.id = "sdPrevPage";
    prev.type = "button";
    prev.classList.add("btn", "btn-secondary");
    prev.innerHTML = "<i class='fa fa-arrow-left'></i>";
    prev.setAttribute("aria-label", "Previous devices page");
    prev.title = "Previous page";
    prev.onclick = () => {
        page = Math.max(0, page - 1);
        requestShapedDevicesPage();
    };
    const next = document.createElement("button");
    next.id = "sdNextPage";
    next.type = "button";
    next.classList.add("btn", "btn-secondary");
    next.innerHTML = "<i class='fa fa-arrow-right'></i>";
    next.setAttribute("aria-label", "Next devices page");
    next.title = "Next page";
    next.onclick = () => {
        const totalPages = Math.max(1, Math.ceil(totalRows / devicesPerPage));
        page = Math.min(totalPages - 1, page + 1);
        requestShapedDevicesPage();
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

    listWrap = document.createElement("div");
    listWrap.id = "sdListWrap";
    listWrap.classList.add("table-responsive", "lqos-table-wrap", "d-none");

    const listTable = document.createElement("table");
    listTable.classList.add("lqos-table", "lqos-table-tight", "mb-0", "lqos-shaped-devices-list-table");

    const listHead = document.createElement("thead");
    const listHeadRow = document.createElement("tr");
    [
        "Circuit",
        "Parent",
        "Plan",
        "Throughput",
        "RTT",
        "QoO",
        "Retransmits",
        "Last Seen",
        "IPs",
    ].forEach((label) => {
        const th = document.createElement("th");
        th.innerText = label;
        listHeadRow.appendChild(th);
    });
    listHead.appendChild(listHeadRow);
    listTable.appendChild(listHead);

    listBody = document.createElement("tbody");
    listBody.id = "sdListBody";
    listTable.appendChild(listBody);
    listWrap.appendChild(listTable);
    target.appendChild(listWrap);

    return {
        target,
        toolbar,
        grid,
        listWrap,
        listBody,
        searchInput,
        perPageSelect,
        viewToggleWrap,
        viewCardsButton: viewCards,
        viewListButton: viewList,
        prevButton: prev,
        nextButton: next,
        pageCounter,
        summary,
    };
}

function currentShapedDevicesPageQuery() {
    const query = {
        page,
        page_size: devicesPerPage,
        kind: shapedDevicesKind,
    };
    if (searchTerm && searchTerm.trim() !== "") {
        query.search = searchTerm;
    }
    return query;
}

function visibleCircuitIds() {
    const seen = new Set();
    const ids = [];
    shapedDevices.forEach((device) => {
        const id = device && device.circuit_id ? String(device.circuit_id).trim() : "";
        if (!id || seen.has(id)) return;
        seen.add(id);
        ids.push(id);
    });
    return ids;
}

function requestCircuitMetricsWatch(force = false) {
    const circuitIds = visibleCircuitIds();
    const signature = JSON.stringify(circuitIds);
    if (!force && circuitMetricsWatchSignature === signature) {
        return;
    }
    circuitMetricsWatchSignature = signature;
    if (circuitIds.length === 0) {
        sendPrivateRequest({StopCircuitMetricsWatch: null});
        return;
    }
    sendPrivateRequest({
        WatchCircuitMetrics: {
            query: {
                circuit_ids: circuitIds,
            },
        },
    });
}

async function requestShapedDevicesPage() {
    try {
        const msg = await sendWsRequest("ShapedDevicesPage", {
            ShapedDevicesPage: {
                query: currentShapedDevicesPageQuery(),
            },
        });
        const data = msg && msg.data ? msg.data : {};
        shapedDevices = Array.isArray(data.rows) ? data.rows : [];
        totalRows = Number.isFinite(Number(data.total_rows)) ? Number(data.total_rows) : shapedDevices.length;
        totalCircuits = Number.isFinite(Number(data.total_circuits)) ? Number(data.total_circuits) : countCircuits();
        planByCircuitId.clear();
        shapedDevices.forEach((device) => {
            if (!device || !device.circuit_id) return;
            const current = planByCircuitId.get(device.circuit_id) || {down: 0, up: 0};
            current.down = Math.max(toNumber(current.down, 0), toNumber(device.download_max_mbps, 0));
            current.up = Math.max(toNumber(current.up, 0), toNumber(device.upload_max_mbps, 0));
            planByCircuitId.set(device.circuit_id, current);
        });
        renderDevices();
        requestCircuitMetricsWatch(true);
        renderCounts();
    } catch (_error) {
        shapedDevices = [];
        totalRows = 0;
        totalCircuits = 0;
        renderDevices();
        renderCounts();
    }
}

function updateViewToggle(layout) {
    const showToggle = shapedDevicesKind === "Dynamic";
    if (layout.viewToggleWrap) {
        layout.viewToggleWrap.classList.toggle("d-none", !showToggle);
        layout.viewToggleWrap.classList.toggle("d-flex", showToggle);
    }
    const cardsActive = !isDynamicListView();
    if (layout.viewCardsButton) {
        layout.viewCardsButton.classList.toggle("active", cardsActive);
        layout.viewCardsButton.setAttribute("aria-pressed", cardsActive.toString());
    }
    if (layout.viewListButton) {
        layout.viewListButton.classList.toggle("active", !cardsActive);
        layout.viewListButton.setAttribute("aria-pressed", (!cardsActive).toString());
    }
}

function renderDevices() {
    const layout = ensureLayout();
    if (!layout) return;

    const totalPages = Math.max(1, Math.ceil(totalRows / devicesPerPage));
    if (page >= totalPages) page = totalPages - 1;
    if (page < 0) page = 0;

    if (layout.perPageSelect && String(devicesPerPage) !== layout.perPageSelect.value) {
        layout.perPageSelect.value = String(devicesPerPage);
    }
    updateViewToggle(layout);
    if (layout.prevButton) layout.prevButton.disabled = page <= 0;
    if (layout.nextButton) layout.nextButton.disabled = page >= totalPages - 1;
    if (layout.pageCounter) layout.pageCounter.innerText = "Page " + (page + 1) + " / " + totalPages;
    if (layout.summary) {
        if (totalRows === 0) {
            layout.summary.innerText = "No matches";
        } else {
            const start = page * devicesPerPage + 1;
            const end = Math.min((page + 1) * devicesPerPage, totalRows);
            let summary = "Showing " + start + "–" + end + " of " + totalRows;
            if (shapedDevicesKind === "Dynamic") {
                summary += " (highest throughput first)";
            }
            layout.summary.innerText = summary;
        }
    }

    metricElsByCircuitId = new Map();
    clearDiv(layout.grid);
    clearDiv(layout.listBody);

    const showList = isDynamicListView();
    layout.grid.classList.toggle("d-none", showList);
    layout.listWrap.classList.toggle("d-none", !showList);

    if (showList) {
        shapedDevices.forEach((device) => {
            layout.listBody.appendChild(buildDeviceListRow(device));
        });
    } else {
        shapedDevices.forEach((device) => {
            const col = document.createElement("div");
            col.classList.add("col");
            col.appendChild(buildDeviceCard(device));
            layout.grid.appendChild(col);
        });
    }

    // Fill visible metrics from cached live data immediately.
    metricElsByCircuitId.forEach((_metrics, circuitId) => {
        const latest = latestByCircuitId.get(circuitId);
        if (latest) {
            applyCircuitUpdate(latest);
        }
    });

    initTooltipsWithin(layout.target);
}

function handleCircuitMetrics(msg) {
    const metrics = msg && Array.isArray(msg.data) ? msg.data : [];
    metrics.forEach((metric) => {
        if (!metric || !metric.circuit_id) return;
        applyCircuitUpdate(metric);
    });
}

wsClient.on("CircuitMetricsSnapshot", handleCircuitMetrics);
wsClient.on("CircuitMetricsUpdate", handleCircuitMetrics);
wsClient.on("join", () => {
    updateKindTabs();
    requestShapedDevicesPage();
});

document.getElementById("sdTabStatic")?.addEventListener("click", () => {
    setShapedDevicesKind("Static");
});
document.getElementById("sdTabDynamic")?.addEventListener("click", () => {
    setShapedDevicesKind("Dynamic");
});

updateKindTabs();
requestShapedDevicesPage();
