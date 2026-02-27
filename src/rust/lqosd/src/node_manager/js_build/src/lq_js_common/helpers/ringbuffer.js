export class RingBuffer {
    constructor(size) {
        this.size = size;
        let data = [];
        for (let i = 0; i < size; i++) {
            data.push({ timestamp: 0, recent: 0, completed: 0 });
        }
        this.head = 0;
        this.data = data;

        // Pre-allocate stable series arrays/objects to avoid per-tick allocations.
        // ECharts updates are much more GC-friendly when the same object identities are reused.
        this._seriesCache = [new Array(size), new Array(size)];
        for (let i = 0; i < size; i++) {
            this._seriesCache[0][i] = { timestamp: 0, value: 0 };
            this._seriesCache[1][i] = { timestamp: 0, value: 0 };
        }
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
        const out0 = this._seriesCache[0];
        const out1 = this._seriesCache[1];
        let idx = 0;
        for (let i = this.head; i < this.size; i++) {
            const src = this.data[i];
            out0[idx].timestamp = src.timestamp;
            out0[idx].value = src.recent;
            out1[idx].timestamp = src.timestamp;
            out1[idx].value = src.completed;
            idx++;
        }
        for (let i = 0; i < this.head; i++) {
            const src = this.data[i];
            out0[idx].timestamp = src.timestamp;
            out0[idx].value = src.recent;
            out1[idx].timestamp = src.timestamp;
            out1[idx].value = src.completed;
            idx++;
        }
        return this._seriesCache;
    }
}
