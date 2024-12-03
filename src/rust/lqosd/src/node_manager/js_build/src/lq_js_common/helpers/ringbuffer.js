export class RingBuffer {
    constructor(size) {
        this.size = size;
        let data = [];
        for (let i=0; i<size; i++) {
            data.push([0, 0]);
        }
        this.head = 0;
        this.data = data;
    }

    push(recent, completed) {
        this.data[this.head] = [recent, completed];
        this.head += 1;
        this.head %= this.size;
    }

    series() {
        let result = [[], []];
        for (let i=this.head; i<this.size; i++) {
            result[0].push(this.data[i][0]);
            result[1].push(this.data[i][1]);
        }
        for (let i=0; i<this.head; i++) {
            result[0].push(this.data[i][0]);
            result[1].push(this.data[i][1]);
        }
        return result;
    }
}