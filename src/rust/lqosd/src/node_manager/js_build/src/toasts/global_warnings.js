import {createBootstrapToast} from "../lq_js_common/helpers/toasts";
import {get_ws_client} from "../pubsub/ws";
import {t} from "../i18n/index.js";

// translatable: false — "Info" "Warning" "Error" are the severity tokens the server sends; the switch compares them and the header renders the catalog label

const wsClient = get_ws_client();

const listenOnce = (eventName, handler) => {
    const wrapped = (msg) => {
        wsClient.off(eventName, wrapped);
        handler(msg);
    };
    wsClient.on(eventName, wrapped);
};

function severityLabel(level) {
    switch (level) {
        case "Info": return t("common.severity.info");
        case "Warning": return t("common.severity.warning");
        case "Error": return t("common.severity.error");
        default: return level;
    }
}

export function globalWarningToasts() {
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
            headerSpan.innerHTML = badge + " " + severityLabel(warning[0]);
            let bodyDiv = document.createElement("div");
            bodyDiv.innerHTML = message;
            createBootstrapToast("global-warning-" + i, headerSpan, bodyDiv);
            i++;
        });
    };
    wsClient.on("GlobalWarnings", handler);
    wsClient.send({ GlobalWarnings: {} });
}
