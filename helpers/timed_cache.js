// Keeps items for up to 10 seconds. Items are tagged by a key, which will
// be used to avoid duplicates.
export class TimedCache {
    constructor(maxAge = 10) {
        this.cache = new Map();
        this.maxAge = maxAge;
    }

    addOrUpdate(key, value) {
        this.cache.set(key, { value: value, age: 0 });
    }

    tick() {
        let toRemove = [];
        this.cache.forEach((val, key) => {
            val.age += 1;
            if (val.age > this.maxAge) {
                toRemove.push(key);
            }
        });
        toRemove.forEach((key) => {
            this.cache.delete(key)
        });
    }

    get() {
        let result = [];
        this.cache.forEach((val) => {
            result.push(val.value);
        })
        return result;
    }
}
