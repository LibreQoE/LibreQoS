export class RingBuffer {
    constructor(size) {
        this.size = size;
        let data = [];
        for (let i = 0; i < size; i++) {
            data.push({ timestamp: 0, recent: 0, completed: 0 });
        }
        this.head = 0;
        this.data = data;
    }

    push(recent, completed) {
        this.data[this.head] = {
            timestamp: Date.now(),
            recent: recent,
            completed: completed
        };
        this.head += 1;
        this.head %= this.size;
    }

    /**
     * Returns:
     * [
     *   [{timestamp, value: recent}, ...],
     *   [{timestamp, value: completed}, ...]
     * ]
     */
    series() {
        let result = [[], []];
        for (let i = this.head; i < this.size; i++) {
            result[0].push({ timestamp: this.data[i].timestamp, value: this.data[i].recent });
            result[1].push({ timestamp: this.data[i].timestamp, value: this.data[i].completed });
        }
        for (let i = 0; i < this.head; i++) {
            result[0].push({ timestamp: this.data[i].timestamp, value: this.data[i].recent });
            result[1].push({ timestamp: this.data[i].timestamp, value: this.data[i].completed });
        }
        return result;
    }
}