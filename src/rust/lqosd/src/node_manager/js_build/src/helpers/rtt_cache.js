export class RttCache {
    constructor() {
        this.cache = {};
    }

    set(key, value) {
        this.cache[key] = value;
    }

    get(key) {
        if (this.cache[key] === undefined) {
            return 0;
        }
        return this.cache[key];
    }
}