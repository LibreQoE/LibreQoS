// Theory: maintains a list of objects in a cache.
export class TopXTable {
    constructor(bufferSize = 10, rankedSize = 10) {
        this.dataStore = new Map();
        this.bufferSize = bufferSize;
        this.rankedSize = rankedSize;
    }

    push(key, data, accessFn) {
        let value = accessFn(data);
        if (this.dataStore.has(key)) {
            // Update existing. If there are more than this.bufferSize entries, remove the oldest one.
            let toStore = this.dataStore.get(key);
            toStore.buffer.push({data: data, value: value});
            if (toStore.buffer.length > this.bufferSize) {
                let previous = toStore.buffer.shift();
                toStore.total -= previous.value;
            }
            toStore.total += value;
            this.dataStore.set(key, toStore);
        } else {
            // Add a new one
            let toStore = {
                buffer: [{data: data, value: value}],
                total: value,
            }
            this.dataStore.set(key, toStore);
        }
    }

    // Get the top (rankedSize) entries. Remove any entries that are not in the top (rankedSize) entries.
    // Return an array of {key, value, data} objects.
    getRankedArrayAndClean() {
        let ranked = Array.from(this.dataStore.entries()).map(([key, value]) => {
            return {key: key, value: value.total, data: value.buffer[value.buffer.length-1].data};
        }).sort((a, b) => b.value - a.value).slice(0, this.rankedSize);
        let rankedSet = new Set(ranked.map(entry => entry.key));
        this.dataStore.forEach((value, key) => {
            if (!rankedSet.has(key)) {
                this.dataStore.delete(key);
            }
        });
        return ranked;
    }
}