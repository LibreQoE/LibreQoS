export function globalWarningToasts() {
    $.get("/local-api/globalWarnings", (warnings) => {
        let parent = document.getElementById("toasts");
        warnings.forEach(warning => {
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
            parent.appendChild(div);
        });
    })
}