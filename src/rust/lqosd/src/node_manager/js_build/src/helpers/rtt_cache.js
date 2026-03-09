export class RttCache {
    constructor(maxEntries = 512) {
        this.cache = new Map();
        this.maxEntries = maxEntries;
    }

    set(key, value) {
        if (this.cache.has(key)) {
            this.cache.delete(key);
        }
        this.cache.set(key, value);
        if (this.cache.size > this.maxEntries) {
            const oldestKey = this.cache.keys().next().value;
            this.cache.delete(oldestKey);
        }
    }

    get(key) {
        if (!this.cache.has(key)) {
            return 0;
        }
        const value = this.cache.get(key);
        this.cache.delete(key);
        this.cache.set(key, value);
        return value;
    }
}
