export class GenericRingBuffer<EntryType> {
    entries: EntryType[];
    head: number;
    size: number;

    constructor(size: number, defaultValue: EntryType) {
        this.size = size;
        this.entries = [];
        for (let i=0; i<this.size; i++) {
            this.entries.push(defaultValue);
        }
        this.head = 0;
    }

    push(item: EntryType): void {
        this.entries[this.head] = item;
        this.head += 1;
        if (this.head > this.size) {
            this.head = 0;
        }
    }

    for_each(functor: (e: EntryType) => void): void {
        for (let i=this.head; i<this.size; i++) {
            functor(this.entries[i]);
        }
        for (let i=0; i<this.head; i++) {
            functor(this.entries[i]);
        }
    }
}