import {clearDiv, formatLastSeen, simpleRow, theading} from "./helpers/builders";
import {scaleNumber} from "./lq_js_common/helpers/scaling";

let button = document.getElementById("btnCsv");
button.onclick = () => {
   window.location.href = "/local-api/unknownIpsCsv";
};

$.get("/local-api/unknownIps", (data) => {
   let target = document.getElementById("unknown");
   let table = document.createElement("table");
   table.classList.add("table", "table-striped");
   let thead = document.createElement("thead");
   thead.appendChild(theading("IP Address"));
   thead.appendChild(theading("Last Seen"));
   thead.appendChild(theading("Total Bytes", 2));
   table.appendChild(thead);
   let tbody = document.createElement("tbody");

   data.forEach((row) => {
      let tr = document.createElement("tr");
      tr.appendChild(simpleRow(row.ip));
      tr.appendChild(simpleRow(formatLastSeen(row.last_seen_nanos)));
      tr.appendChild(simpleRow(scaleNumber(row.total_bytes.down)));
      tr.appendChild(simpleRow(scaleNumber(row.total_bytes.up)));
      tbody.appendChild(tr);
   });

   table.appendChild(tbody);
   clearDiv(target);
   target.appendChild(table);

   //console.log(data);
});