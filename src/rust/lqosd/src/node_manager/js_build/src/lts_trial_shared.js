import {get_ws_client} from "./pubsub/ws";

const DEFAULT_INSIGHT_BASE_URL = 'https://insight.libreqos.com/';
const wsClient = get_ws_client();

function normalizeInsightBaseUrl(rawBaseUrl) {
    let baseUrl = (rawBaseUrl || DEFAULT_INSIGHT_BASE_URL).trim();

    if (!baseUrl) {
        baseUrl = DEFAULT_INSIGHT_BASE_URL;
    }

    if (!/^https?:\/\//i.test(baseUrl)) {
        baseUrl = `https://${baseUrl}`;
    }

    baseUrl = baseUrl.replace(/\/+$/, '');
    if (baseUrl.endsWith('/signup-api')) {
        baseUrl = baseUrl.slice(0, -'/signup-api'.length);
    }

    return `${baseUrl}/`;
}

function buildInsightUrl(rawBaseUrl, endpoint) {
    const path = String(endpoint || '').replace(/^\/+/, '');
    return `${normalizeInsightBaseUrl(rawBaseUrl)}${path}`;
}

function buildSignupApiUrl(rawBaseUrl, endpoint) {
    const path = String(endpoint || '').replace(/^\/+/, '');
    return buildInsightUrl(rawBaseUrl, `signup-api/${path}`);
}

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

export {buildInsightUrl, buildSignupApiUrl, normalizeInsightBaseUrl, sendWsRequest, wsClient};
