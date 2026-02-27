import {DashboardGraph} from "./dashboard_graph";
import {GraphOptionsBuilder} from "../lq_js_common/e_charts/chart_builder";
import {scaleNumber, toNumber} from "../lq_js_common/helpers/scaling";

const RING_SIZE = 60 * 5; // 5 Minutes

function formatTime(ts) {
    const d = new Date(ts);
    return d.toLocaleTimeString('en-US', { hour12: false });
}

export class StormguardAdjustmentsGraph extends DashboardGraph {
    constructor(id) {
        super(id);
        this.timestamps = new Array(RING_SIZE).fill(0);
        this.seriesBySite = new Map(); // siteName -> { downloadSeries, uploadSeries }
        this.siteColorMap = new Map(); // Track colors for consistent site coloring
        this.colorIndex = 0;

        this.option = new GraphOptionsBuilder()
            .withSequenceAxis(0, RING_SIZE)
            .withScaledAbsYAxis("Bandwidth Limit (bps)", 40)
            .build();

        // Customize Y-axis to handle both positive and negative values
        this.option.yAxis = {
            type: 'value',
            name: 'Bandwidth Limit (bps)',
            nameLocation: 'middle',
            nameGap: 50,
            axisLabel: {
                formatter: (val) => {
                    if (val === 0) return '0';
                    return scaleNumber(Math.abs(val), 0);
                },
            },
            splitLine: {
                lineStyle: {
                    color: '#333'
                }
            }
        };

        // Initialize with zero line
        this.option.series = [
            {
                name: 'Zero Line',
                type: 'line',
                data: Array(RING_SIZE).fill(0),
                lineStyle: {
                    color: '#666',
                    width: 1,
                    type: 'dashed'
                },
                symbol: 'none',
                silent: true,
                animation: false,
                z: 1
            }
        ];

        this.option.legend = {
            orient: "horizontal",
            right: 10,
            top: "bottom",
            selectMode: false,
            data: [],
            textStyle: {
                color: '#aaa'
            },
        };

        // Add axisPointer and tooltip with time display
        this.option.tooltip = {
            trigger: 'axis',
            axisPointer: {
                type: 'cross',
                link: [{ xAxisIndex: 'all' }],
                label: {
                    backgroundColor: '#6a7985'
                }
            },
            formatter: (params) => {
                if (!params || params.length === 0) return '';
                const idx = params[0].dataIndex;
                const ts = this.getTimestamp(idx);
                let s = `<div><b>Time:</b> ${formatTime(ts)}</div>`;
                
                // Group by site to show download/upload together
                const siteData = new Map();
                for (const p of params) {
                    if (p.seriesName.includes('Zero Line')) continue;
                    
                    const match = p.seriesName.match(/^(.+) (Download|Upload)$/);
                    if (match) {
                        const siteName = match[1];
                        const type = match[2];
                        
                        if (!siteData.has(siteName)) {
                            siteData.set(siteName, {});
                        }
                        siteData.get(siteName)[type.toLowerCase()] = Math.abs(p.value);
                        siteData.get(siteName).color = p.color;
                    }
                }
                
                // Display site data
                for (const [site, data] of siteData) {
                    s += `<div><span style="display:inline-block;margin-right:5px;border-radius:10px;width:9px;height:9px;background-color:${data.color};"></span><b>${site}:</b></div>`;
                    if (data.download !== undefined) {
                        s += `<div style="padding-left:15px;">Download (solid): ${scaleNumber(data.download)}</div>`;
                    }
                    if (data.upload !== undefined) {
                        s += `<div style="padding-left:15px;">Upload (dashed): ${scaleNumber(data.upload)}</div>`;
                    }
                }
                
                return s;
            }
        };

        this.option && this.chart.setOption(this.option);
        this._seriesOnly = { series: this.option.series };
    }

    getTimestamp(idx) {
        const i = Number(idx);
        if (!Number.isFinite(i) || i < 0 || i >= this.timestamps.length) {
            return 0;
        }
        return this.timestamps[i] || 0;
    }

    onThemeChange() {
        super.onThemeChange();
        // Update colors for all series
        for (let i = 1; i < this.option.series.length; i++) {
            const series = this.option.series[i];
            const siteInfo = this.getSiteFromSeriesName(series.name);
            if (siteInfo) {
                const color = this.getColorForSite(siteInfo.site);
                series.lineStyle.color = color;
            }
        }
        
        // Update legend colors
        for (const legendItem of this.option.legend.data) {
            const siteInfo = this.getSiteFromSeriesName(legendItem.name);
            if (siteInfo) {
                legendItem.itemStyle.color = this.getColorForSite(siteInfo.site);
            }
        }
        
        this.chart.setOption(this.option);
    }

    getSiteFromSeriesName(seriesName) {
        const match = seriesName.match(/^(.+) (Download|Upload)$/);
        if (match) {
            return { site: match[1], type: match[2] };
        }
        return null;
    }

    getColorForSite(siteName) {
        if (!this.siteColorMap.has(siteName)) {
            // Store a stable palette index so theme changes can re-map colors cleanly.
            this.siteColorMap.set(siteName, this.colorIndex);
            this.colorIndex++;
        }
        const idx = this.siteColorMap.get(siteName) || 0;
        return window.graphPalette[idx % window.graphPalette.length];
    }

    ensureSiteSeries(siteName) {
        if (this.seriesBySite.has(siteName)) {
            return false;
        }

        const color = this.getColorForSite(siteName);
        const zeros = new Array(RING_SIZE).fill(0);

        const downloadSeries = {
            name: `${siteName} Download`,
            data: zeros.slice(),
            type: 'line',
            lineStyle: {
                width: 2,
                color: color,
            },
            symbol: 'none',
            smooth: true,
            animation: false,
        };

        const uploadSeries = {
            name: `${siteName} Upload`,
            data: zeros.slice(),
            type: 'line',
            lineStyle: {
                width: 2,
                color: color,
                type: 'dashed',
            },
            symbol: 'none',
            smooth: true,
            animation: false,
        };

        this.seriesBySite.set(siteName, { downloadSeries, uploadSeries });
        this.option.series.push(downloadSeries);
        this.option.series.push(uploadSeries);

        // One legend entry per site (visual key); series names include direction.
        this.option.legend.data.push({
            name: siteName,
            icon: 'rect',
            itemStyle: { color: color },
        });

        return true;
    }

    update(sites) {
        this.chart.hideLoading();
        
        // sites is Vec<(String, u64, u64)> = [(siteName, download, upload), ...]
        if (!Array.isArray(sites)) {
            console.warn("StormguardAdjustmentsGraph: Expected array of sites, got:", sites);
            return;
        }

        const now = Date.now();
        // Maintain timestamps as "oldest -> newest" so tooltip index maps naturally.
        this.timestamps.shift();
        this.timestamps.push(now);

        // Build a map of this tick's site values (bps).
        const valuesBySite = new Map();
        for (const site of sites) {
            if (!Array.isArray(site) || site.length !== 3) {
                continue;
            }
            const name = String(site[0] ?? "").trim();
            if (!name) continue;
            valuesBySite.set(name, {
                download: toNumber(site[1], 0) * 1_000_000,
                upload: toNumber(site[2], 0) * 1_000_000,
            });
        }

        // Lazily create series for new sites.
        let addedSeries = false;
        for (const name of valuesBySite.keys()) {
            if (this.ensureSiteSeries(name)) {
                addedSeries = true;
            }
        }

        // Advance each series by 1 tick (fixed window) without allocating new arrays.
        for (const [name, seriesPair] of this.seriesBySite.entries()) {
            const v = valuesBySite.get(name);
            const down = v ? v.download : 0;
            const up = v ? v.upload : 0;

            const downData = seriesPair.downloadSeries.data;
            downData.shift();
            downData.push(down);

            const upData = seriesPair.uploadSeries.data;
            upData.shift();
            upData.push(-(up || 0));
        }

        if (addedSeries) {
            // Structural change (new series): replace to ensure ECharts drops any stale state.
            this.chart.setOption(this.option, true);
        } else {
            // Data-only update: avoid full option merges to reduce memory churn.
            this.chart.setOption(this._seriesOnly, false, true);
        }
    }
}
