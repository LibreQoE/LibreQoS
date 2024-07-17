import { DashboardGraph } from "./graphs/dashboard_graph";

class AllTreeSankey extends DashboardGraph {
    constructor(id) {
        super(id);
        this.option = {
            series: [
                {
                    type: 'sankey',
                    data: [],
                    links: []
                }
            ]
        };
        this.option && this.chart.setOption(this.option);
        this.chart.hideLoading();
    }

    update(data, links) {
        this.option.series[0].data = data;
        this.option.series[0].links = links;
        this.chart.hideLoading();
        this.chart.setOption(this.option);
    }
}

function start() {
    $.get("/local-api/networkTree", (data) => {
        console.log(data);

        let nodes = [];
        let links = [];

        for (let i=0; i<data.length; i++) {
            nodes.push({
                name: data[i][1].name,
                label: {
                    fontSize: 6,
                    color: "#999"
                }
            });

            if (i > 1) {
                let immediateParent = data[i][1].immediate_parent;
                links.push({
                    source: data[immediateParent][1].name,
                    target: data[i][1].name,
                    value: data[i][1].current_throughput[0] + data[i][1].current_throughput[1]
                });
            }
        }

        graph.update(nodes, links);
        setTimeout(start, 1000);
    });
}

let graph = new AllTreeSankey("sankey");

start();