import {simpleRow, theading} from "./helpers/builders";

function loadDevices() {
    $.get("/local-api/devicesAll", (data) => {
        console.log(data);
        let table = document.createElement("table");
        table.classList.add("table", "table-striped");
        let thead = document.createElement("thead");
        thead.appendChild(theading("Circuit"));
        thead.appendChild(theading("Device"));
        thead.appendChild(theading("Plans"));
        thead.appendChild(theading("IP"));
        table.appendChild(thead);
        let tb = document.createElement("tbody");
        data.forEach((d) => {
            let tr = document.createElement("tr");
            tr.appendChild(simpleRow(d.circuit_name));
            tr.appendChild(simpleRow(d.device_name));
            tr.appendChild(simpleRow(d.download_max_mbps + " / " + d.upload_max_mbps));
            tr.appendChild(simpleRow(""));
            tb.append(tr);
        })
        table.appendChild(tb);

        let target = document.getElementById("deviceTable");
        while (target.children.length > 0) {
            target.removeChild(target.lastChild);
        }
        target.appendChild(table);
    })
}

loadDevices();