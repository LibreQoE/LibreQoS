import {DashboardGraph} from "./dashboard_graph";
import {lerpGreenToRedViaOrange} from "../helpers/scaling";

export class RttHistogram3D extends DashboardGraph {
    constructor(id) {
        super(id);
        this.ring = new RingBuffer(300);

        let timeAxis = [];
        for (let i=0; i<300; i++) timeAxis.push(i.toString());

        let catAxis = [];
        for (let i=0; i<20; i++) catAxis.push(i.toString());

        /*let data = [];
        for (let z=0; z<300; z++) {
            for (let x=0; x<20; x++) {
                data.push([ x, z, 1 ]);
            }
        }*/
        let data = this.ring.series();

        this.option = {
            tooltip: {},
            visualMap: {
                max: 20,
                inRange: {
                    color: ['#313695', '#4575b4', '#74add1', '#abd9e9', '#e0f3f8', '#ffffbf', '#fee090', '#fdae61', '#f46d43', '#d73027', '#a50026']
                }
            },
            xAxis3D: {
                type: 'category',
                data: catAxis
            },
            yAxis3D: {
                type: 'category',
                data: timeAxis
            },
            zAxis3D: {
                type: 'value'
            },
            grid3D: {
                boxWidth: 100,
                boxDepth: 100,
                light: {
                    main: {
                        intensity: 1.2
                    },
                    ambient: {
                        intensity: 0.3
                    }
                }
            },
            series: [{
                type: 'bar3D',
                data: data,
                shading: 'color',
                label: {
                    show: false
                },
            }]
        };
        this.option && this.chart.setOption(this.option);
    }

    update(rtt) {
        this.chart.hideLoading();
        this.ring.push(rtt);
        this.option.series[0].data = this.ring.series();
        this.chart.setOption(this.option);
    }
}

class RingBuffer {
    constructor(size) {
        this.size = size;
        let data = [];
        for (let i=0; i<size; i++) {
            let d = [];
            for (let j=0; j<20; j++) {
                d.push(0);
            }
            data.push(d);
        }
        this.head = 0;
        this.data = data;
    }

    push(histo) {
        this.data[this.head] = histo;
        this.head += 1;
        this.head %= this.size;
    }

    series() {
        let data = [];
        let counter = 0;
        for (let i=0; i<this.head; i++) {
            for (let j=0; j<20; j++) {
                let val = this.data[i][j];
                data.push([j, counter, val]);
            }
            counter++;
        }
        for (let i=this.head; i<this.size; i++) {
            for (let j=0; j<20; j++) {
                let val = this.data[i][j];
                data.push([j, counter, val]);
            }
            counter++;
        }
        return data.reverse();
    }
}