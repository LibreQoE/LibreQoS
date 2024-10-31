// A generic ringbuffer implementation of a fixed size.
export class GenericRingBuffer {
    // Create a new ringbuffer with a fixed size.
    constructor(size) {
        this.size = size;
        this.buffer = new Array(size);
        this.head = 0;
        this.prevHead = 0;
        this.length = 0;
    }

    // Push a value into the ringbuffer.
    push(value) {
        this.buffer[this.head] = value;
        this.prevHead = this.head;
        this.head = (this.head + 1) % this.size;
        this.length = Math.min(this.length + 1, this.size);
    }

    // Iterate over the ringbuffer in order.
    iterate(callback) {
        if (this.length === 0) {
            return;
        }
        // If the length < size, we can just iterate from 0 to head.
        if (this.length < this.size) {
            for (let i=0; i<this.head; i++) {
                callback(this.buffer[i]);
            }
            return;
        }

        for (let i=0; i<this.head; i++) {
            callback(this.buffer[i]);
        }
        for (let i=this.head; i<this.size; i++) {
            callback(this.buffer[i]);
        }
    }

    getHead() {
        return this.buffer[this.prevHead];
    }
}