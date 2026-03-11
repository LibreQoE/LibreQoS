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
        this.ringbuffer = new StormguardRingBuffer(RING_SIZE);
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
                const ts = this.ringbuffer.getTimestamp(idx);
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
            this.siteColorMap.set(siteName, window.graphPalette[this.colorIndex % window.graphPalette.length]);
            this.colorIndex++;
        }
        return this.siteColorMap.get(siteName);
    }

    update(sites) {
        this.chart.hideLoading();
        
        // sites is Vec<(String, u64, u64)> = [(siteName, download, upload), ...]
        if (!Array.isArray(sites)) {
            console.warn("StormguardAdjustmentsGraph: Expected array of sites, got:", sites);
            return;
        }

        // Push to ringbuffer
        this.ringbuffer.push(sites, Date.now());

        // Get all unique sites from the ringbuffer
        const allSites = this.ringbuffer.getAllSites();
        
        // Rebuild series based on current sites
        const newSeries = [this.option.series[0]]; // Keep zero line
        const newLegendData = [];

        // Create series for each site
        for (const siteName of allSites) {
            const color = this.getColorForSite(siteName);
            
            // Download series (positive line)
            newSeries.push({
                name: `${siteName} Download`,
                data: this.ringbuffer.getSeriesForSite(siteName, 'download'),
                type: 'line',
                lineStyle: {
                    width: 2,
                    color: color,
                },
                symbol: 'none',
                smooth: true
            });

            // Upload series (negative line)
            newSeries.push({
                name: `${siteName} Upload`,
                data: this.ringbuffer.getSeriesForSite(siteName, 'upload'),
                type: 'line',
                lineStyle: {
                    width: 2,
                    color: color,
                    type: 'dashed'
                },
                symbol: 'none',
                smooth: true
            });

            // Add to legend (one entry per site showing both download and upload)
            if (!newLegendData.find(item => item.name === siteName)) {
                newLegendData.push({
                    name: siteName,
                    icon: 'rect',
                    itemStyle: {
                        color: color
                    }
                });
            }
        }

        this.option.series = newSeries;
        this.option.legend.data = newLegendData;
        this.chart.setOption(this.option);
    }
}

class StormguardRingBuffer {
    constructor(size) {
        this.size = size;
        this.data = [];
        for (let i = 0; i < size; i++) {
            this.data.push({
                sites: new Map(), // Map<siteName, {download: u64, upload: u64}>
                timestamp: 0
            });
        }
        this.head = 0;
        this.allSites = new Set(); // Track all sites we've seen
    }

    push(sites, timestamp) {
        // sites is an array of [siteName, download, upload]
        // Values are in Mbps, need to convert to bps
        const siteMap = new Map();
        
        for (const site of sites) {
            if (Array.isArray(site) && site.length === 3) {
                const [name, downloadMbps, uploadMbps] = site;
                // Convert from Mbps to bps
                siteMap.set(name, { 
                    download: toNumber(downloadMbps, 0) * 1000000, 
                    upload: toNumber(uploadMbps, 0) * 1000000 
                });
                this.allSites.add(name);
            }
        }
        
        this.data[this.head] = {
            sites: siteMap,
            timestamp: timestamp || Date.now()
        };
        
        this.head = (this.head + 1) % this.size;
    }

    getTimestamp(idx) {
        const physical = (this.head + idx) % this.size;
        return this.data[physical].timestamp;
    }

    getDataAt(idx) {
        const physical = (this.head + idx) % this.size;
        return this.data[physical];
    }

    getAllSites() {
        return Array.from(this.allSites).sort();
    }

    getSeriesForSite(siteName, type) {
        const series = [];
        
        // Start from head and wrap around to get chronological order
        for (let i = this.head; i < this.size; i++) {
            const siteData = this.data[i].sites.get(siteName);
            if (siteData) {
                if (type === 'download') {
                    series.push(siteData.download || 0);
                } else if (type === 'upload') {
                    // Invert upload values (negative)
                    series.push(-(siteData.upload || 0));
                }
            } else {
                series.push(0);
            }
        }
        
        // Continue from beginning to head
        for (let i = 0; i < this.head; i++) {
            const siteData = this.data[i].sites.get(siteName);
            if (siteData) {
                if (type === 'download') {
                    series.push(siteData.download || 0);
                } else if (type === 'upload') {
                    // Invert upload values (negative)
                    series.push(-(siteData.upload || 0));
                }
            } else {
                series.push(0);
            }
        }
        
        return series;
    }
}
