import { Component } from "./component";

export class NodeStatus implements Component {
    wireup(): void {
        
    }

    ontick(): void {
        window.bus.requestNodeStatus();
    }

    onmessage(event: any): void {
        if (event.msg == "nodeStatus") {
            let status = document.getElementById("nodeStatus");
            let html = "";
            if (status) {
                for (let i = 0; i < event.nodes.length; i++) {
                    let node = event.nodes[i];
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
                    html += "<span class='badge rounded-pill text-bg-" + color + "'><i class='fa-solid fa-server'></i> " + node.node_name + "</span> ";
                }
                status.innerHTML = html;
            }                
        }
    }
}