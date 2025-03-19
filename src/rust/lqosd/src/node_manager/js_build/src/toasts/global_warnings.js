import {createBootstrapToast} from "../lq_js_common/helpers/toasts";

export function globalWarningToasts() {
    $.get("/local-api/globalWarnings", (warnings) => {
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
    })
}