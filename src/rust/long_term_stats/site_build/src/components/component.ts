export interface Component {
    wireup(): void;
    ontick(): void;
    onmessage(event: any): void;
}