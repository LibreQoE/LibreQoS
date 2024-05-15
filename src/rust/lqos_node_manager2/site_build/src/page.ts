export abstract class Page {
    abstract wireup(): void;
    abstract onmessage(event: any): void;
    abstract ontick(): void;
    abstract anchor(): string;

    fillContent(content: string): void {
        let container = document.getElementById("mainContent");
        if (container) {
            container.innerHTML = content;
        } else {
            console.log("Could not find mainContent");
        }
    }
}