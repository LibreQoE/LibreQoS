import {isDarkMode} from "../helpers/dark_mode";

function emitDashboardRenderHook(entry) {
    if (typeof window !== "undefined" && typeof window.__lqosDashboardRenderHook === "function") {
        window.__lqosDashboardRenderHook(entry);
    }
}

function domStateSnapshot(dom) {
    if (!dom) {
        return {
            hasDom: false,
        };
    }
    const rect = typeof dom.getBoundingClientRect === "function"
        ? dom.getBoundingClientRect()
        : { width: 0, height: 0 };
    const computed = typeof window !== "undefined" && window.getComputedStyle
        ? window.getComputedStyle(dom)
        : null;
    return {
        hasDom: true,
        clientWidth: dom.clientWidth,
        clientHeight: dom.clientHeight,
        rectWidth: rect.width,
        rectHeight: rect.height,
        isConnected: typeof dom.isConnected === "boolean" ? dom.isConnected : true,
        offsetParentNull: dom.offsetParent === null,
        display: computed ? computed.display : "",
        visibility: computed ? computed.visibility : "",
    };
}

function domCanPaint(dom) {
    const state = domStateSnapshot(dom);
    return state.hasDom && state.clientWidth > 0 && state.clientHeight > 0 && state.display !== "none";
}

function parsePixels(value) {
    if (!value || typeof value !== "string" || !value.endsWith("px")) {
        return 0;
    }
    const numeric = parseFloat(value);
    return Number.isFinite(numeric) ? numeric : 0;
}

function ensureNonZeroChartSize(dom) {
    if (!dom) return () => {};
    const computed = window.getComputedStyle(dom);
    let cleanupWidth = false;
    let cleanupHeight = false;
    if (dom.clientWidth === 0) {
        const parentWidth = dom.parentElement ? dom.parentElement.clientWidth : 0;
        const fallbackWidth = Math.max(parentWidth, parsePixels(computed.width), parsePixels(computed.minWidth), 320);
        dom.style.minWidth = `${fallbackWidth}px`;
        if (!dom.style.width) {
            dom.style.width = `${fallbackWidth}px`;
            cleanupWidth = true;
        }
    }
    if (dom.clientHeight === 0) {
        const parentHeight = dom.parentElement ? dom.parentElement.clientHeight : 0;
        const fallbackHeight = Math.max(
            parentHeight,
            parsePixels(dom.style.height),
            parsePixels(computed.height),
            parsePixels(computed.minHeight),
            180,
        );
        dom.style.minHeight = `${fallbackHeight}px`;
        if (!dom.style.height) {
            dom.style.height = `${fallbackHeight}px`;
            cleanupHeight = true;
        }
    }
    // Force a reflow before echarts.init reads dimensions.
    void dom.offsetWidth;
    return () => {
        dom.style.removeProperty("min-width");
        dom.style.removeProperty("min-height");
        if (cleanupWidth) {
            dom.style.removeProperty("width");
        }
        if (cleanupHeight) {
            dom.style.removeProperty("height");
        }
    };
}

export class DashboardGraph {
    constructor(id) {
        this.id = id;
        this.dom = document.getElementById(id);
        this.pendingSetOptionArgs = null;
        this.pendingSetOptionRaf = null;
        this.pendingSetOptionAttempts = 0;
        if (!this.dom) {
            throw new Error(`DashboardGraph: missing DOM element '${id}'`);
        }
        emitDashboardRenderHook({
            graphId: this.id,
            stage: "graph-constructor-start",
            ...domStateSnapshot(this.dom),
        });
        this.dom.classList.add("muted");
        // Some charts are created while Bootstrap/tab layout is still settling.
        // Ensure ECharts never initializes against a 0x0 container.
        const clearStagingSize = ensureNonZeroChartSize(this.dom);

        // If a chart already exists for this DOM (e.g. time period change, zoom open/close),
        // dispose it before re-initializing to prevent memory growth.
        if (typeof echarts !== "undefined" && echarts.getInstanceByDom) {
            const existing = echarts.getInstanceByDom(this.dom);
            if (existing) {
                existing.dispose();
            }
        }

        if (isDarkMode()) {
            window.graphPalette = [
                '#4992ff',
                '#7cffb2',
                '#fddd60',
                '#ff6e76',
                '#58d9f9',
                '#05c091',
                '#ff8a45',
                '#8d48e3',
                '#dd79ff'
            ];
            this.chart = echarts.init(this.dom, 'dark');
        } else {
            window.graphPalette = [
                '#d87c7c',
                '#919e8b',
                '#d7ab82',
                '#6e7074',
                '#61a0a8',
                '#efa18d',
                '#787464',
                '#cc7e63',
                '#724e58',
                '#4b565b'
            ];
            this.chart = echarts.init(this.dom, 'vintage');
        }
        this.#wrapChartMethod("setOption");
        this.#wrapChartMethod("resize");
        this.#wrapChartMethod("showLoading");
        this.#wrapChartMethod("hideLoading");
        this.chart.showLoading();
        this.option = {};
        window.requestAnimationFrame(() => {
            clearStagingSize();
            if (this.chart && this.chart.resize) {
                this.chart.resize();
            }
        });
        emitDashboardRenderHook({
            graphId: this.id,
            stage: "graph-constructor-complete",
            ...domStateSnapshot(this.dom),
        });

        // Apply to the global list of graphs
        if (window.graphList === undefined) {
            window.graphList = [];
        }
        const domId = this.dom.id;
        // De-dupe by DOM id and remove graphs whose DOM has been detached (e.g. closed zoom).
        window.graphList = window.graphList.filter((g) => {
            if (!g || !g.dom || !g.dom.id) return false;
            if (g.dom.id === domId) return false;
            if (typeof g.dom.isConnected === "boolean") {
                return g.dom.isConnected;
            }
            return document.body && document.body.contains ? document.body.contains(g.dom) : true;
        });
        window.graphList.push(this);
    }

    #scheduleDeferredSetOptionRetry() {
        if (this.pendingSetOptionRaf !== null || this.pendingSetOptionArgs === null) {
            return;
        }
        this.pendingSetOptionRaf = window.requestAnimationFrame(() => {
            this.pendingSetOptionRaf = null;
            if (this.pendingSetOptionArgs === null) {
                return;
            }
            if (domCanPaint(this.dom)) {
                this.#flushPendingSetOption("raf");
                return;
            }
            this.pendingSetOptionAttempts += 1;
            if (this.pendingSetOptionAttempts >= 10) {
                emitDashboardRenderHook({
                    graphId: this.id,
                    stage: "chart-setOption-drop-deferred",
                    attempts: this.pendingSetOptionAttempts,
                    ...domStateSnapshot(this.dom),
                });
                this.pendingSetOptionArgs = null;
                this.pendingSetOptionAttempts = 0;
                return;
            }
            this.#scheduleDeferredSetOptionRetry();
        });
    }

    #flushPendingSetOption(reason) {
        if (this.pendingSetOptionArgs === null) {
            return;
        }
        const args = this.pendingSetOptionArgs;
        this.pendingSetOptionArgs = null;
        this.pendingSetOptionAttempts = 0;
        emitDashboardRenderHook({
            graphId: this.id,
            stage: "chart-setOption-flush-deferred",
            reason,
            ...domStateSnapshot(this.dom),
        });
        this.chart.setOption(...args);
    }

    #wrapChartMethod(methodName) {
        if (!this.chart || typeof this.chart[methodName] !== "function") {
            return;
        }
        const original = this.chart[methodName].bind(this.chart);
        this.chart[methodName] = (...args) => {
            emitDashboardRenderHook({
                graphId: this.id,
                stage: `chart-${methodName}-start`,
                argsCount: args.length,
                ...domStateSnapshot(this.dom),
            });
            try {
                if (methodName === "setOption" && !domCanPaint(this.dom)) {
                    this.pendingSetOptionArgs = args;
                    this.pendingSetOptionAttempts = 0;
                    emitDashboardRenderHook({
                        graphId: this.id,
                        stage: "chart-setOption-deferred",
                        ...domStateSnapshot(this.dom),
                    });
                    this.#scheduleDeferredSetOptionRetry();
                    return undefined;
                }
                const result = original(...args);
                emitDashboardRenderHook({
                    graphId: this.id,
                    stage: `chart-${methodName}-ok`,
                    ...domStateSnapshot(this.dom),
                });
                if (methodName === "resize" && domCanPaint(this.dom)) {
                    this.#flushPendingSetOption("resize");
                }
                return result;
            } catch (err) {
                emitDashboardRenderHook({
                    graphId: this.id,
                    stage: `chart-${methodName}-error`,
                    error: err && err.message ? err.message : String(err),
                    ...domStateSnapshot(this.dom),
                });
                throw err;
            }
        };
    }

    onThemeChange() {
        // Override this if you have to
    }
}
