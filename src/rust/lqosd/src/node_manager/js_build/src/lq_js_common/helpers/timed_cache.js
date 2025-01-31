// Keeps items for up to 10 seconds. Items are tagged by a key, which will
// be used to avoid duplicates.
const alpha = 0.3;

export class TimedCache {
    constructor(maxAgeSeconds) {
        this.entries = new Map();
        this.maxAge = maxAgeSeconds;
    }

    addOrUpdate(key, value, score) {
        if (!this.entries.has(key)) {
            // New entry
            this.entries.set(key, { value: value, score: score, lastSeen: Date.now() });
        } else {
            // Update existing entry
            let entry = this.entries.get(key);
            entry.value = value;
            entry.score = alpha * score + (1 - alpha) * entry.score;
            entry.lastSeen = Date.now();
        }
    }

    tick() {
        // Remove older than maxAge seconds
        let now = Date.now();
        this.entries.forEach((v, k) => {
            if (now - v.lastSeen > this.maxAge * 1000) {
                this.entries.delete(k);
            }
        });
    }

    get() {
        // Sort by score, descending
        let entries = Array.from(this.entries.values());
        entries.sort((a, b) => {
            return b.score - a.score;
        });

        // Map to only have the value
        entries = entries.map((v) => {
            return v.value;
        });

        // Return the top 10
        return entries.slice(0, 10);
    }
}