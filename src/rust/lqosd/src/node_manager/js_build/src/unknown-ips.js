import {clearDiv, formatLastSeen, simpleRow, theading} from "./helpers/builders";
import {scaleNumber} from "./lq_js_common/helpers/scaling";

let button = document.getElementById("btnCsv");
button.onclick = () => {
   window.location.href = "/local-api/unknownIpsCsv";
};

let clearButton = document.getElementById("btnClear");
if (clearButton) {
   clearButton.onclick = async () => {
      try {
         await fetch("/local-api/unknownIps/clear", { method: "POST" });
      } catch (e) {
         console.error("Failed to clear unknown IPs", e);
      } finally {
         // Reload to refresh the list
         window.location.reload();
      }
   };
}

$.get("/local-api/unknownIps", (data) => {
   let target = document.getElementById("unknown");
   clearDiv(target);

   if (!data || data.length === 0) {
      const p = document.createElement('p');
      p.classList.add('text-muted');
      p.textContent = 'No unknown IPs seen in the last 5 minutes.';
      target.appendChild(p);
      return;
   }

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
   target.appendChild(table);
});
