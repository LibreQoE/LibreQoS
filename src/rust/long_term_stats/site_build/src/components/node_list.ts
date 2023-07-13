import { Component } from "./component";
import { request_node_status } from "../../wasm/wasm_pipe";

export class NodeList implements Component {
    wireup(): void {
        
    }

    ontick(): void {
        request_node_status();
    }

    onmessage(event: any): void {
        if (event.msg == "NodeStatus") {
            let status = document.getElementById("nodeList");
            let html = "";
            if (status) {
                html += "<table class='table table-striped'>";
                html += "<thead>";
                html += "<th>Node ID</th><th>Node Name</th><th>Last Seen</th>";
                html += "</thead><tbody>";
                for (let i = 0; i < event.NodeStatus.nodes.length; i++) {
                    let node = event.NodeStatus.nodes[i];
                    let url = "\"shaperNode:" + node.node_id + ":" + node.node_name.replace(':', '_') + "\"";
                    let oc = "onclick='window.router.goto(" + url + ")'";
                    html += "<tr>";
                    html += "<td><span " + oc + ">" + node.node_id + "</span></td>";
                    html += "<td><span " + oc + ">" + node.node_name + "</span></td>";
                    html += "<td><span " + oc + ">" + node.last_seen + " seconds ago</span></td>";
                }
                html += "</tbody></table>";
                status.innerHTML = html;
            }                
        }
    }
}