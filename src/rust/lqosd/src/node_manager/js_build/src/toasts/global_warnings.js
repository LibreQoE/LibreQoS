import {createBootstrapToast} from "../lq_js_common/helpers/toasts";
import {PLACEHOLDER_TEASERS} from "../lts_teasers_shared";
import {get_ws_client} from "../pubsub/ws";

let insightModalShown = false;

let modalTeasers = [];
let modalTeasersLoaded = false;
let modalLtsBaseUrl = "https://insight.libreqos.com/";
const wsClient = get_ws_client();

const listenOnce = (eventName, handler) => {
    const wrapped = (msg) => {
        wsClient.off(eventName, wrapped);
        handler(msg);
    };
    wsClient.on(eventName, wrapped);
};

function sendWsRequest(responseEvent, request) {
    return new Promise((resolve, reject) => {
        let done = false;
        const onResponse = (msg) => {
            if (done) return;
            done = true;
            wsClient.off(responseEvent, onResponse);
            wsClient.off("Error", onError);
            resolve(msg);
        };
        const onError = (msg) => {
            if (done) return;
            done = true;
            wsClient.off(responseEvent, onResponse);
            wsClient.off("Error", onError);
            reject(msg);
        };
        wsClient.on(responseEvent, onResponse);
        wsClient.on("Error", onError);
        wsClient.send(request);
    });
}

function getModalLtsUrl(endpoint) {
    let baseUrl = modalLtsBaseUrl;
    if (!/^https?:\/\//i.test(baseUrl)) {
        baseUrl = "https://" + baseUrl;
    }
    let base = baseUrl.endsWith("/") ? baseUrl : baseUrl + "/";
    base += base.endsWith("signup-api/") ? "" : "signup-api/";
    endpoint = endpoint.replace(/^\/+/, "");
    return base + endpoint;
}

async function loadModalTeasers() {
    if (modalTeasersLoaded) {
        renderModalTeasers();
        return;
    }

    try {
        const response = await sendWsRequest("LtsTrialConfigResult", { LtsTrialConfig: {} });
        const data = response && response.data ? response.data : {};
        if (data.lts_url && typeof data.lts_url === "string") {
            modalLtsBaseUrl = data.lts_url;
        }
    } catch (e) {
        // If this fails, we just fall back to the default base URL.
        console.error("Failed to fetch LTS base URL for modal teasers:", e);
    }

    try {
        const response = await $.get(getModalLtsUrl("teasers"));
        if (response && Array.isArray(response.teasers) && response.teasers.length > 0) {
            modalTeasers = response.teasers.map((teaser) => {
                const copy = { ...teaser };
                if (copy.image) {
                    const img = typeof copy.image === "string" ? copy.image : "";
                    const cleaned = img.replace(/^signup-api\//, "");
                    copy.image = getModalLtsUrl(cleaned);
                }
                return copy;
            });
        } else {
            modalTeasers = PLACEHOLDER_TEASERS.slice();
        }
    } catch (e) {
        console.error("Failed to load Insight teasers for modal:", e);
        modalTeasers = PLACEHOLDER_TEASERS.slice();
    }

    modalTeasersLoaded = true;
    renderModalTeasers();
}

function renderModalTeasers() {
    const container = document.getElementById("insightTeaserRow");
    if (!container) {
        return;
    }

    const teasers = (modalTeasers.length ? modalTeasers : PLACEHOLDER_TEASERS).slice();
    teasers.sort((a, b) => (a.order || 0) - (b.order || 0));

    const cardsHtml = teasers
        .map((teaser) => {
            const imageSrc = teaser.image || teaser.imageUrl || "";
            const title = teaser.title || "";
            const description = teaser.description || "";
            return `
                <div class="col-md-4 mb-3">
                    <div class="card h-100 bg-dark border-secondary text-secondary">
                        ${
                            imageSrc
                                ? `<img src="${imageSrc}" class="card-img-top" alt="${title}" style="height: 180px; object-fit: contain; background-color: #1b1e21;">`
                                : ""
                        }
                        <div class="card-body d-flex flex-column">
                            <h5 class="card-title text-secondary">${title}</h5>
                            <p class="card-text text-secondary">${description}</p>
                        </div>
                    </div>
                </div>
            `;
        })
        .join("");

    container.innerHTML = cardsHtml;
}

function showInsightTrialModal() {
    if (insightModalShown) {
        return;
    }
    insightModalShown = true;

    if (!window.bootstrap) {
        return;
    }

    if (!document.getElementById("insightTrialModal")) {
        const modalHtml = `
            <div class="modal fade" id="insightTrialModal" tabindex="-1" aria-labelledby="insightTrialModalLabel" aria-hidden="true">
                <div class="modal-dialog modal-fullscreen">
                    <div class="modal-content bg-dark text-secondary">
                        <div class="modal-header border-secondary">
                            <h2 class="modal-title text-secondary" id="insightTrialModalLabel">
                                <i class="fa fa-line-chart nav-icon"></i>
                                Get The Most Out of LibreQoS with <strong>LibreQoS Insight</strong>
                            </h2>
                            <button type="button" class="btn-close btn-close-white" data-bs-dismiss="modal" aria-label="Close"></button>
                        </div>
                        <div class="modal-body text-secondary">
                            <div class="alert alert-primary text-secondary" role="alert">
                                <p class="mb-0">
                                    You have <strong>{{window.smn}}</strong> shaped devices.
                                    LibreQoS is helping your network; Insight can make it amazing.
                                </p>
                            </div>
                            <p class="mt-3">
                                With Insight, you can:
                            </p>
                            <ul>
                                <li>Ask Libby about your network in natural language.</li>
                                <li>Use CPU load balancing and mitigation features like binpacking and reload reduction.</li>
                                <li>Explore accurate statistics, analytics, and long-term network insights.</li>
                            </ul>
                            <!-- From the Insight Trial -->
                            <div class="mt-4">
                                <div class="row g-3" id="insightTeaserRow"></div>
                            </div>
                        </div>
                        <div class="modal-footer border-secondary">
                            <button type="button" class="btn btn-outline-secondary" data-bs-dismiss="modal">Maybe Later</button>
                            <a href="lts_trial.html" class="btn btn-primary">Start Insight Trial</a>
                        </div>
                    </div>
                </div>
            </div>
        `;
        document.body.insertAdjacentHTML(
            "beforeend",
            modalHtml.replace("{{window.smn}}", window.smn || "?"),
        );
    }

    const modalEl = document.getElementById("insightTrialModal");
    if (!modalEl) {
        return;
    }

    // Load and render teaser cards inside the modal
    loadModalTeasers();

    const modal = new bootstrap.Modal(modalEl, { focus: true });
    modal.show();
}

export function globalWarningToasts() {
    if (window.sm) {
        showInsightTrialModal();
    }
    const handler = (msg) => {
        wsClient.off("GlobalWarnings", handler);
        const warnings = msg && msg.data ? msg.data : [];
        let parent = document.getElementById("toasts");
        let i = 0;
        warnings.forEach(warning => {
            console.log(warning);
            let div = document.createElement("div");
            div.classList.add("alert");
            let message = warning[1];
            let badge = "<fa class='fa fa-exclamation-triangle'></fa>";
            switch (warning[0]) {
                case "Info": {
                    badge = "<fa class='fa fa-info-circle'></fa>";
                    div.classList.add("alert-info");
                } break;
                case "Warning": {
                    badge = "<fa class='fa fa-exclamation-triangle'></fa>";
                    div.classList.add("alert-warning");
                } break;
                case "Error": {
                    badge = "<fa class='fa fa-exclamation-circle'></fa>";
                    div.classList.add("alert-danger");
                } break;
                default: {
                    div.classList.add("alert-warning");
                } break;
            }
            div.innerHTML = badge + " " + message;
            //parent.appendChild(div);
            let headerSpan = document.createElement("span");
            headerSpan.innerHTML = badge + " " + warning[0];
            let bodyDiv = document.createElement("div");
            bodyDiv.innerHTML = message;
            createBootstrapToast("global-warning-" + i, headerSpan, bodyDiv);
            i++;
        });
    };
    wsClient.on("GlobalWarnings", handler);
    wsClient.send({ GlobalWarnings: {} });
}
