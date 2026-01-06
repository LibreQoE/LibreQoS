import {clearDiv, formatLastSeen, simpleRow, theading} from "./helpers/builders";
import {scaleNumber} from "./lq_js_common/helpers/scaling";
import {get_ws_client} from "./pubsub/ws";

const wsClient = get_ws_client();
const listenOnce = (eventName, handler) => {
   const wrapped = (msg) => {
      wsClient.off(eventName, wrapped);
      handler(msg);
   };
   wsClient.on(eventName, wrapped);
};

function downloadCsv() {
   listenOnce("UnknownIpsCsv", (msg) => {
      const csv = msg && msg.csv ? msg.csv : "";
      if (!csv) {
         console.warn("Empty unknown IP CSV payload");
         return;
      }
      const blob = new Blob([csv], { type: "text/csv;charset=utf-8" });
      const url = URL.createObjectURL(blob);
      const link = document.createElement("a");
      link.href = url;
      link.download = "unknown_ips.csv";
      document.body.appendChild(link);
      link.click();
      link.remove();
      URL.revokeObjectURL(url);
   });
   wsClient.send({ UnknownIpsCsv: {} });
}

function loadUnknownIps() {
   listenOnce("UnknownIps", (msg) => {
      const data = msg && msg.data ? msg.data : [];
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
   wsClient.send({ UnknownIps: {} });
}

let button = document.getElementById("btnCsv");
if (button) {
   button.onclick = () => {
      downloadCsv();
   };
}

let clearButton = document.getElementById("btnClear");
if (clearButton) {
   clearButton.onclick = () => {
      listenOnce("UnknownIpsCleared", () => {
         loadUnknownIps();
      });
      wsClient.send({ UnknownIpsClear: {} });
   };
}

loadUnknownIps();
