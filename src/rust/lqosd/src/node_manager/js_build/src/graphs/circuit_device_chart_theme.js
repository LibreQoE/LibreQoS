function isLightTheme() {
    return document.documentElement.getAttribute("data-bs-theme") === "light";
}

export function getCircuitDeviceChartTheme() {
    if (!isLightTheme()) {
        return {
            titleText: "rgba(228, 236, 250, 0.9)",
            axisText: "rgba(216, 226, 244, 0.66)",
            axisName: "rgba(216, 226, 244, 0.58)",
            axisLine: "rgba(216, 226, 244, 0.24)",
            axisTick: "rgba(216, 226, 244, 0.18)",
            splitLine: "rgba(216, 226, 244, 0.05)",
            legendText: "rgba(228, 236, 250, 0.74)",
            tooltipBackground: "rgba(12, 19, 30, 0.94)",
            tooltipBorder: "rgba(143, 176, 255, 0.22)",
            tooltipText: "rgba(236, 242, 252, 0.96)",
            tooltipAxisLabelBackground: "rgba(23, 38, 58, 0.96)",
        };
    }

    return {
        titleText: "rgba(34, 48, 69, 0.92)",
        axisText: "rgba(54, 67, 86, 0.72)",
        axisName: "rgba(54, 67, 86, 0.62)",
        axisLine: "rgba(77, 94, 118, 0.26)",
        axisTick: "rgba(77, 94, 118, 0.18)",
        splitLine: "rgba(77, 94, 118, 0.08)",
        legendText: "rgba(34, 48, 69, 0.72)",
        tooltipBackground: "rgba(249, 251, 255, 0.98)",
        tooltipBorder: "rgba(79, 121, 217, 0.18)",
        tooltipText: "rgba(34, 48, 69, 0.94)",
        tooltipAxisLabelBackground: "rgba(232, 239, 248, 0.98)",
    };
}

function applyAxisTheme(axis, theme) {
    if (!axis) {
        return;
    }
    axis.axisLabel = {
        ...(axis.axisLabel || {}),
        color: theme.axisText,
    };
    axis.axisLine = {
        ...(axis.axisLine || {}),
        lineStyle: {
            ...((axis.axisLine && axis.axisLine.lineStyle) || {}),
            color: theme.axisLine,
        },
    };
    axis.axisTick = {
        ...(axis.axisTick || {}),
        lineStyle: {
            ...((axis.axisTick && axis.axisTick.lineStyle) || {}),
            color: theme.axisTick,
        },
    };
    axis.nameTextStyle = {
        ...(axis.nameTextStyle || {}),
        color: theme.axisName,
    };
    if (axis.splitLine !== false) {
        axis.splitLine = {
            ...(axis.splitLine || {}),
            lineStyle: {
                ...((axis.splitLine && axis.splitLine.lineStyle) || {}),
                color: theme.splitLine,
            },
        };
    }
}

export function applyCircuitDeviceChartTheme(option, { hasLegend = false } = {}) {
    if (!option) {
        return option;
    }

    const theme = getCircuitDeviceChartTheme();

    option.title = {
        ...(option.title || {}),
        textStyle: {
            ...((option.title && option.title.textStyle) || {}),
            color: theme.titleText,
            fontWeight: 600,
        },
    };

    if (Array.isArray(option.xAxis)) {
        option.xAxis.forEach((axis) => applyAxisTheme(axis, theme));
    } else {
        applyAxisTheme(option.xAxis, theme);
    }

    if (Array.isArray(option.yAxis)) {
        option.yAxis.forEach((axis) => applyAxisTheme(axis, theme));
    } else {
        applyAxisTheme(option.yAxis, theme);
    }

    if (hasLegend && option.legend) {
        option.legend = {
            ...option.legend,
            textStyle: {
                ...((option.legend && option.legend.textStyle) || {}),
                color: theme.legendText,
            },
        };
    }

    if (option.tooltip) {
        option.tooltip = {
            ...option.tooltip,
            backgroundColor: theme.tooltipBackground,
            borderColor: theme.tooltipBorder,
            borderWidth: 1,
            textStyle: {
                ...((option.tooltip && option.tooltip.textStyle) || {}),
                color: theme.tooltipText,
            },
            axisPointer: option.tooltip.axisPointer
                ? {
                    ...option.tooltip.axisPointer,
                    label: {
                        ...((option.tooltip.axisPointer && option.tooltip.axisPointer.label) || {}),
                        backgroundColor: theme.tooltipAxisLabelBackground,
                        color: theme.tooltipText,
                    },
                }
                : option.tooltip.axisPointer,
        };
    }

    return option;
}
