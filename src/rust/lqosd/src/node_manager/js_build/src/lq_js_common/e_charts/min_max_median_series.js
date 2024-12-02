// Provides helpers for a 3-entry (min/max/median) series.

// Helper for min/max/median serieses
export class MinMaxSeries {
    // Build the series
    constructor(seriesName, paletteIndex) {
        this.seriesName = seriesName;
        this.paletteIndex = paletteIndex;
        this.series = [
            // Value series
            {
                type: 'line',
                data: [],
                name: this.seriesName,
                smooth: true,
                areaStyle: { opacity: 0 },
                itemStyle: {
                    color: window.graphPalette[paletteIndex]
                }
            },
            // Minimum
            {
                type: 'line',
                data: [],
                name: this.seriesName + " Min",
                smooth: true,
                // Hide the area
                areaStyle: { color: window.graphPalette[paletteIndex], opacity: 0.0 },
                itemStyle: {
                    color: window.graphPalette[paletteIndex]
                },
                lineStyle: { opacity: 0.0 },
                stack: this.seriesName,
            },
            // Maximum
            {
                type: 'line',
                data: [],
                name: this.seriesName + " Max",
                smooth: true,
                areaStyle: { color: window.graphPalette[paletteIndex], opacity: 0.3 },
                itemStyle: {
                    color: window.graphPalette[paletteIndex]
                },
                lineStyle: { opacity: 0.0 },
                stack: this.seriesName,
            }
        ];
    }

    // Clear the stored data
    clear() {
        this.series[0].data = [];
        this.series[1].data = [];
        this.series[2].data = [];
    }

    // Add an "upwards" band. (Typically download)
    pushPositive(median, min, max) {
        this.series[0].data.push(median);
        this.series[1].data.push(min);
        this.series[2].data.push(max - min);
    }

    // Add an inverted "downwards" band. (Typically upload)
    pushNegative(median, min, max) {
        series[0].data.push(0.0 - median);
        series[1].data.push((0.0 - max) - (0.0 - min));
        series[2].data.push(0.0 - min_up);
    }

    addToOptions(option) {
        option.series.push(series[0]);
        option.series.push(series[1]);
        option.series.push(series[2]);
    }
}