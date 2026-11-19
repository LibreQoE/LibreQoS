import {clearDiv, formatLastSeen, simpleRow, theading} from "./helpers/builders";
import {t} from "./i18n/index.js";
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
         // translatable: false — "Empty unknown IP CSV payload" is a developer console warning, never rendered
         console.warn("Empty unknown IP CSV payload");
         return;
      }
      // translatable: false — "text/csv;charset=utf-8" is the blob's MIME type, not UI text
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
         p.textContent = t("unknown_ips.empty");
         target.appendChild(p);
         return;
      }

      let table = document.createElement("table");
      table.classList.add("lqos-table", "lqos-table-compact");
      let thead = document.createElement("thead");
      thead.appendChild(theading(t("unknown_ips.ip_address")));
      thead.appendChild(theading(t("common.table.last_seen")));
      thead.appendChild(theading(t("unknown_ips.total_bytes"), 2));
      table.appendChild(thead);
      let tbody = document.createElement("tbody");

      data.forEach((row) => {
         let tr = document.createElement("tr");
         tr.appendChild(simpleRow(row.ip, true));
         tr.appendChild(simpleRow(formatLastSeen(row.last_seen_nanos)));
         tr.appendChild(simpleRow(scaleNumber(row.total_bytes.down)));
         tr.appendChild(simpleRow(scaleNumber(row.total_bytes.up)));
         tbody.appendChild(tr);
      });

      table.appendChild(tbody);
      const tableWrap = document.createElement("div");
      tableWrap.classList.add("lqos-table-wrap");
      tableWrap.appendChild(table);
      target.appendChild(tableWrap);
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
