import { Component } from "./component";

export class RootHeat implements Component {
    wireup(): void {
        
    }

    ontick(): void {
        window.bus.requestSiteRootHeat();
    }

    onmessage(event: any): void {
        if (event.msg == "rootHeat") {
            console.log(event);
        }
    }
}