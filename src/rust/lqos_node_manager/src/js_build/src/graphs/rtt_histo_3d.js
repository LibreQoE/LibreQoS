import {DashboardGraph} from "./dashboard_graph";
import {lerpGreenToRedViaOrange} from "../helpers/scaling";
import {N_ITEMS} from "./rtt_histo";

export class RttHistogram3D extends DashboardGraph {
    constructor(id) {
        super(id);
        this.ring = new RingBuffer(300);

        let timeAxis = [];
        for (let i=0; i<300; i++) timeAxis.push(i.toString());

        let catAxis = [];
        for (let i=0; i<N_ITEMS; i++) catAxis.push({
            value: (i*10).toString(),
            itemStyle: {color: lerpGreenToRedViaOrange(N_ITEMS-i, N_ITEMS)},
        }
        );

        let data = this.ring.series();

        this.option = {
            tooltip: {},
            xAxis3D: {
                type: 'category',
                data: catAxis,
                name: "RTT"
            },
            yAxis3D: {
                type: 'category',
                data: timeAxis,
                name: "Time"
            },
            zAxis3D: {
                type: 'value',
                name: "Samples"
            },
            grid3D: {
                viewControl: {
                    autoRotate: true,
                },
            },
            series: [{
                type: 'bar3D',
                data: data,
                //shading: 'lambert',
                label: {
                    show: false
                },
            }]
        };
        this.option && this.chart.setOption(this.option);
    }

    update(rtt) {
        this.chart.hideLoading();
        // Uncomment this for test data
        //for (let i=0; i<20; i++) rtt[i] += 20-i;
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
            for (let j=0; j<N_ITEMS; j++) {
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
            for (let j=0; j<N_ITEMS; j++) {
                let val = this.data[i][j];
                let toPush = {
                    value: [j, counter, val],
                    itemStyle: {color: lerpGreenToRedViaOrange(N_ITEMS-j, N_ITEMS)},
                };
                data.push(toPush);
            }
            counter++;
        }
        for (let i=this.head; i<this.size; i++) {
            for (let j=0; j<N_ITEMS; j++) {
                let val = this.data[i][j];
                let toPush = {
                    value: [j, counter, val],
                    itemStyle: {color: lerpGreenToRedViaOrange(N_ITEMS-j, N_ITEMS)},
                };
                data.push(toPush);
            }
            counter++;
        }
        return data.reverse();
    }
}