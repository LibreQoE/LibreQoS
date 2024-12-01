// Class for helping to build Apache E-Charts in our styles

import {scaleNumber} from "../helpers/scaling";

// Builder for the echarts Option type.
export class GraphOptionsBuilder {
    // Start with defaults for ALL graphs
    constructor() {
        this.option = {};
    }

    // Applies a category x-axis, with a formatter to transform UNIX time to JS time, and then the locale-specific format.
    withTimeAxis() {
        this.option.xAxis = {
            type: 'category',
            data: [],
            axisLabel: {
                formatter: function (val)
                {
                    return new Date(parseInt(val) * 1000).toLocaleString();
                },
                hideOverlap: true
            }
        };
        return this;
    }

    // Provide a positive-number only (suitable for inverted) Y axis
    // that scales the number K/M/G/etc.
    withScaledAbsYAxis() {
        this.option.yAxis = {
            type: 'value',
            axisLabel: {
                formatter: (val) => {
                    return scaleNumber(Math.abs(val), 0);
                },
            }
        };
        return this;
    }

    withScaledAbsYAxisPercent() {
        this.option.yAxis = {
            type: 'value',
            axisLabel: {
                formatter: (val) => {
                    return scaleNumber(Math.abs(val), 0) + "%";
                },
            }
        };
        return this;
    }

    // Adds an empty series array
    withEmptySeries() {
        this.option.series = [];
        return this;
    }

    // Adds an empty legend
    withEmptyLegend() {
        this.option.legend = { data: [] };
        return this;
    }

    // Return the constructed options
    build() {
        return this.option;
    }
}