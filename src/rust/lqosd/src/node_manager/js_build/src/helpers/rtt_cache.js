export class RttCache {
    constructor(maxEntries = 5000) {
        this.maxEntries = maxEntries;
        this.cache = new Map();
    }

    set(key, value) {
        const k = String(key);
        // LRU-ish: move to the end on set.
        if (this.cache.has(k)) {
            this.cache.delete(k);
        }
        this.cache.set(k, value);
        while (this.cache.size > this.maxEntries) {
            const oldest = this.cache.keys().next().value;
            if (oldest === undefined) break;
            this.cache.delete(oldest);
        }
    }

    get(key) {
        const k = String(key);
        if (!this.cache.has(k)) return 0;
        const v = this.cache.get(k);
        // Move to end on get to keep hot keys.
        this.cache.delete(k);
        this.cache.set(k, v);
        return v;
    }
}
