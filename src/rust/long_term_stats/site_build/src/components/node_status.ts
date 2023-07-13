import { request_node_status } from "../../wasm/wasm_pipe";
import { Component } from "./component";

export class NodeStatus implements Component {
    wireup(): void {
        
    }

    ontick(): void {
        request_node_status();
    }

    onmessage(event: any): void {
        if (event.msg == "NodeStatus") {
            let status = document.getElementById("nodeStatus");
            let html = "";
            if (status) {
                for (let i = 0; i < event.NodeStatus.nodes.length; i++) {
                    let node = event.NodeStatus.nodes[i];
                    let color = "danger";
                    if (node.last_seen > 86400) {
                        color = "secondary";
                    } 
                    if (node.last_seen < 60) {
                        color = "warning";
                    } 
                    if (node.last_seen < 20) {
                        color = "success";
                    }
                    let url = "\"shaperNode:" + node.node_id + ":" + node.node_name.replace(':', '_') + "\"";
                    html += "<span href='#' onclick='window.router.goto(" + url + ")' class='badge rounded-pill text-bg-" + color + "'><i class='fa-solid fa-server'></i> " + node.node_name + "</span> ";
                }
                status.innerHTML = html;
            }                
        }
    }
}