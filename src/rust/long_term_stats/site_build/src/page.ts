export interface Page {
    wireup(): void;
    onmessage(event: any): void;
    ontick(): void;
}