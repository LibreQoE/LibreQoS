import { DashboardGraph } from "./graphs/dashboard_graph";

class FlowMap extends DashboardGraph {
    constructor(id) {
        super(id);
        let data = [];
        this.option = {
            geo3D: {
                map: 'world',
                shading: 'realistic',
                silent: true,
                environment: '#333',
                realisticMaterial: {
                    roughness: 0.8,
                    metalness: 0
                },
                postEffect: {
                    enable: true
                },
                groundPlane: {
                    show: false
                },
                light: {
                    main: {
                        intensity: 1,
                        alpha: 30
                    },
                    ambient: {
                        intensity: 0
                    }
                },
                viewControl: {
                    distance: 70,
                    alpha: 89,
                    panMouseButton: 'left',
                    rotateMouseButton: 'right'
                },
                itemStyle: {
                    color: '#000'
                },
                regionHeight: 0.5
            },
            series: [
                {
                    type: 'scatter3D',
                    coordinateSystem: 'geo3D',
                    blendMode: 'lighter',
                    lineStyle: {
                        width: 0.2,
                        opacity: 0.05
                    },
                    data: data
                }
            ]
        };
        this.option && this.chart.setOption(this.option);
    }

    update(data) {
        this.chart.hideLoading();
        this.option.series[0].data = data;
        this.chart.setOption(this.option);
    }
}

let map = new FlowMap("flowMap");
$.get("/local-api/flowMap", (data) => {
    let output = [];
    data.forEach((d) => {
        output.push([d[1], d[0]]); // It wants lon/lat
    });
    map.update(output);
})