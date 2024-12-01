// Builder function for building line-series
export function LineSeriesBuilder(name, paletteIndex) {
    return {
        type: 'line',
        data: [],
        name: name,
        smooth: true,
        areaStyle: { opacity: 0 },
        itemStyle: {
            color: window.graphPalette[paletteIndex]
        }
    };
}

// Builder function for named/stacked area types
export function AreaSeriesBulder(name, paletteIndex, stack) {
    return {
        type: 'line',
        data: [],
        name: name,
        smooth: true,
        areaStyle: { },
        itemStyle: {
            color: window.graphPalette[paletteIndex]
        },
        stack: stack
    };
}