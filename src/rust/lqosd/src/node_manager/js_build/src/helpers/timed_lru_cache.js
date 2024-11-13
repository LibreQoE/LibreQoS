export class TimedLRUCache {
    constructor(maxCapacity = 10, timeToLive = 300) { // default TTL is 5 minutes (300 seconds)
        this.cache = new Map();
        this.maxCapacity = maxCapacity;
        this.timeToLive = timeToLive; // in seconds
    }

    // Add or update an object in the cache
    set(key, value) {
        if (this.cache.has(key)) {
            this.cache.delete(key); // Reset order
        }
        this.cache.set(key, { value: value, lastAccessed: Date.now() });

        // If capacity exceeds max, remove the oldest item
        if (this.cache.size > this.maxCapacity) {
            const oldestKey = this.cache.keys().next().value;
            this.cache.delete(oldestKey);
        }
    }

    // Get an object from the cache (resets time)
    get(key) {
        if (!this.cache.has(key)) {
            return null;
        }
        const entry = this.cache.get(key);
        entry.lastAccessed = Date.now(); // Update access time
        this.cache.delete(key);
        this.cache.set(key, entry); // Refresh order in the Map
        return entry.value;
    }

    // Tick function to be called externally every second
    tick() {
        const now = Date.now();
        for (const [key, entry] of this.cache) {
            const elapsedTime = (now - entry.lastAccessed) / 1000; // convert ms to seconds
            if (elapsedTime > this.timeToLive) {
                this.cache.delete(key); // Remove expired items
            }
        }
    }

    toArray() {
        return Array.from(this.cache.values());
    }
}