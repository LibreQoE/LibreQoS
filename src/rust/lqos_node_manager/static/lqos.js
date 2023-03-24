function msgPackGet(url, success) {
    var xhr = new XMLHttpRequest();
    xhr.open("GET", url, true);
    xhr.responseType = "arraybuffer";
    xhr.onload = () => {
        var data = xhr.response;
        let decoded = msgpack.decode(new Uint8Array(data));
        success(decoded);
    };
    xhr.send(null);
}

const NetTrans = {
    "name": 0,
    "max_throughput": 1,
    "current_throughput": 2,
    "rtts": 3,
    "parents": 4,
    "immediate_parent": 5
}

const Circuit = {
    "id" : 0,
    "name" : 1,
    "traffic": 2,
    "limit": 3,
}

const IpStats = {
    "ip_address": 0,
    "bits_per_second": 1,
    "packets_per_second": 2,
    "median_tcp_rtt": 3,
    "tc_handle": 4,
    "circuit_id": 5,
    "plan": 6,
}

const FlowTrans = {
    "src": 0,
    "dst": 1,
    "proto": 2,
    "src_port": 3,
    "dst_port": 4,
    "bytes": 5,
    "packets": 6,
    "dscp": 7,
    "ecn": 8
}

function metaverse_color_ramp(n) {
    if (n <= 9) {
        return "#32b08c";
    } else if (n <= 20) {
        return "#ffb94a";
    } else if (n <= 50) {
        return "#f95f53";
    } else if (n <= 70) {
        return "#bf3d5e";
    } else {
        return "#dc4e58";
    }
}

function regular_color_ramp(n) {
    if (n <= 100) {
        return "#aaffaa";
    } else if (n <= 150) {
        return "goldenrod";
    } else {
        return "#ffaaaa";
    }
}

function color_ramp(n) {
    let colorPreference = window.localStorage.getItem("colorPreference");
    if (colorPreference == null) {
        window.localStorage.setItem("colorPreference", 0);
        colorPreference = 0;
    }
    if (colorPreference == 0) {
        return regular_color_ramp(n);
    } else {
        return metaverse_color_ramp(n);
    }
}

function deleteAllCookies() {
    const cookies = document.cookie.split(";");

    for (let i = 0; i < cookies.length; i++) {
        const cookie = cookies[i];
        const eqPos = cookie.indexOf("=");
        const name = eqPos > -1 ? cookie.substr(0, eqPos) : cookie;
        document.cookie = name + "=;expires=Thu, 01 Jan 1970 00:00:00 GMT";
    }
    window.location.reload();
}

function cssrules() {
    var rules = {};
    for (var i = 0; i < document.styleSheets.length; ++i) {
        var cssRules = document.styleSheets[i].cssRules;
        for (var j = 0; j < cssRules.length; ++j)
            rules[cssRules[j].selectorText] = cssRules[j];
    }
    return rules;
}

function css_getclass(name) {
    var rules = cssrules();
    if (!rules.hasOwnProperty(name))
        throw 'TODO: deal_with_notfound_case';
    return rules[name];
}

function updateHostCounts() {
    msgPackGet("/api/host_counts", (hc) => {
        $("#shapedCount").text(hc[0]);
        $("#unshapedCount").text(hc[1]);
        setTimeout(updateHostCounts, 5000);
    });
    $.get("/api/username", (un) => {
        let html = "";
        if (un == "Anonymous") {
            html = "<a class='nav-link' href='/login'><i class='fa fa-user'></i> Login</a>";
        } else {
            html = "<a class='nav-link' onclick='deleteAllCookies();'><i class='fa fa-user'></i> Logout " + un + "</a>";
        }
        $("#currentLogin").html(html);
    });
    $("#startTest").on('click', () => {
        $.get("/api/run_btest", () => { });
    });
}

function colorReloadButton() {
    $("body").append(reloadModal);
    $("#btnReload").on('click', () => {
        $.get("/api/reload_libreqos", (result) => {
            const myModal = new bootstrap.Modal(document.getElementById('reloadModal'), { focus: true });
            $("#reloadLibreResult").text(result);
            myModal.show();
        });
    });
    $.get("/api/reload_required", (req) => {
        if (req) {
            $("#btnReload").addClass('btn-warning');
            $("#btnReload").css('color', 'darkred');
        } else {
            $("#btnReload").addClass('btn-secondary');
        }
    });

    // Redaction
    if (isRedacted()) {
        console.log("Redacting");
        //css_getclass(".redact").style.filter = "blur(4px)";
        css_getclass(".redact").style.fontFamily = "klingon";
    }
}

function isRedacted() {
    let redact = localStorage.getItem("redact");
    if (redact == null) {
        localStorage.setItem("redact", false);
        redact = false;
    }
    if (redact == "false") {
        redact = false;
    } else if (redact == "true") {
        redact = true;
    }
    return redact;
}

const phrases = [
    "quSDaq ba’lu’’a’", // Is this seat taken?
    "vjIjatlh", // speak
    "pe’vIl mu’qaDmey", // curse well
    "nuqDaq ‘oH puchpa’’e’", // where's the bathroom?
    "nuqDaq ‘oH tach’e’", // Where's the bar?
    "tera’ngan Soj lujab’a’", // Do they serve Earth food?
    "qut na’ HInob", // Give me the salty crystals
    "qagh Sopbe’", // He doesn't eat gagh
    "HIja", // Yes
    "ghobe’", // No
    "Dochvetlh vIneH", // I want that thing
    "Hab SoSlI’ Quch", // Your mother has a smooth forehead
    "nuqjatlh", // What did you say?
    "jagh yIbuStaH", // Concentrate on the enemy
    "Heghlu’meH QaQ jajvam", // Today is a good day to die
    "qaStaH nuq jay’", // WTF is happening?
    "wo’ batlhvaD", // For the honor of the empire
    "tlhIngan maH", // We are Klingon!
    "Qapla’", // Success!
]

function redactText(text) {
    if (!isRedacted()) return text;
    let redacted = "";
    let sum = 0;
    for (let i = 0; i < text.length; i++) {
        let code = text.charCodeAt(i);
        sum += code;
    }
    sum = sum % phrases.length;
    return phrases[sum];
}

function scaleNumber(n) {
    if (n > 1000000000000) {
        return (n / 1000000000000).toFixed(2) + "T";
    } else if (n > 1000000000) {
        return (n / 1000000000).toFixed(2) + "G";
    } else if (n > 1000000) {
        return (n / 1000000).toFixed(2) + "M";
    } else if (n > 1000) {
        return (n / 1000).toFixed(2) + "K";
    }
    return n;
}

const reloadModal = `
<div class='modal fade' id='reloadModal' tabindex='-1' aria-labelledby='reloadModalLabel' aria-hidden='true'>
    <div class='modal-dialog modal-fullscreen'>
      <div class='modal-content'>
        <div class='modal-header'>
          <h1 class='modal-title fs-5' id='reloadModalLabel'>LibreQoS Reload Result</h1>
          <button type='button' class='btn-close' data-bs-dismiss='modal' aria-label='Close'></button>
        </div>
        <div class='modal-body'>
          <pre id='reloadLibreResult' style='overflow: vertical; height: 100%; width: 100%;'>
          </pre>
        </div>
        <div class='modal-footer'>
          <button type='button' class='btn btn-secondary' data-bs-dismiss='modal'>Close</button>
        </div>
      </div>
    </div>
  </div>`;

// MultiRingBuffer provides an interface for storing multiple ring-buffers
// of performance data, with a view to them ending up on the same graph.
class MultiRingBuffer {
    constructor(capacity) {
        this.capacity = capacity;
        this.data = {};
    }

    push(id, download, upload) {
        if (!this.data.hasOwnProperty(id)) {
            this.data[id] = new RingBuffer(this.capacity);
        }
        this.data[id].push(download, upload);
    }

    plotStackedBars(target_div, rootName) {
        let graphData = [];
        for (const [k, v] of Object.entries(this.data)) {
            if (k != rootName) {
                let y = v.sortedY;
                let dn = { x: v.x_axis, y: y.down, name: k + "_DL", type: 'scatter', stackgroup: 'dn' };
                let up = { x: v.x_axis, y: y.up, name: k + "_UL", type: 'scatter', stackgroup: 'up' };
                graphData.push(dn);
                graphData.push(up);
            }
        }

        let graph = document.getElementById(target_div);
        Plotly.newPlot(
            graph,
            graphData,
            {
                margin: { l: 0, r: 0, b: 0, t: 0, pad: 4 },
                yaxis: { automargin: true },
                xaxis: { automargin: true, title: "Time since now (seconds)" },
                showlegend: false,
            },
            { responsive: true, displayModeBar: false });
    }

    plotTotalThroughput(target_div) {
        let graph = document.getElementById(target_div);

        let total = this.data['total'].sortedY();
        let shaped = this.data['shaped'].sortedY();

        let x = this.data['total'].x_axis;

        let data = [
            {x: x, y:total.down, name: 'Download', type: 'scatter', marker: {color: 'rgb(255,160,122)'}},
            {x: x, y:total.up, name: 'Upload', type: 'scatter', marker: {color: 'rgb(255,160,122)'}},
            {x: x, y:shaped.down, name: 'Shaped Download', type: 'scatter', fill: 'tozeroy', marker: {color: 'rgb(124,252,0)'}},
            {x: x, y:shaped.up, name: 'Shaped Upload', type: 'scatter', fill: 'tozeroy', marker: {color: 'rgb(124,252,0)'}},
        ];
        Plotly.newPlot(graph, data, { margin: { l:0,r:0,b:0,t:0,pad:4 }, yaxis: { automargin: true, title: "Traffic (bits)" }, xaxis: {automargin: true, title: "Time since now (seconds)"} }, { responsive: true });
    }
}

class RingBuffer {
    constructor(capacity) {
        this.capacity = capacity;
        this.head = capacity - 1;
        this.download = [];
        this.upload = [];
        this.x_axis = [];
        for (var i = 0; i < capacity; ++i) {
            this.download.push(0.0);
            this.upload.push(0.0);
            this.x_axis.push(capacity - i);
        }
    }

    push(download, upload) {
        this.download[this.head] = download;
        this.upload[this.head] = 0.0 - upload;
        this.head += 1;
        this.head %= this.capacity;
    }

    sortedY() {
        let result = {
            down: [],
            up: [],
        };
        for (let i=this.head; i<this.capacity; i++) {
            result.down.push(this.download[i]);
            result.up.push(this.upload[i]);
        }
        for (let i=0; i < this.head; i++) {
            result.down.push(this.download[i]);
            result.up.push(this.upload[i]);
        }
        return result;
    }

    toScatterGraphData() {
        let y = this.sortedY();
        let GraphData = [
            { x: this.x_axis, y: y.down, name: 'Download', type: 'scatter' },
            { x: this.x_axis, y: y.up, name: 'Upload', type: 'scatter' },
        ];
        return GraphData;
    }
}

class RttHistogram {
    constructor() {
        this.entries = []
        this.x = [];
        for (let i = 0; i < 20; ++i) {
            this.entries.push(i);
            this.x.push(i * 10);
        }
    }

    clear() {
        for (let i = 0; i < 20; ++i) {
            this.entries[i] = 0;
        }
    }

    push(rtt) {
        let band = Math.floor(rtt / 10.0);
        if (band > 19) {
            band = 19;
        }
        this.entries[band] += 1;
    }

    pushBand(band, n) {
        this.entries[band] += n;
    }

    plot(target_div) {
        let gData = [
            { x: this.x, y: this.entries, type: 'bar', marker: { color: this.x, colorscale: 'RdBu' } }
        ]
        let graph = document.getElementById(target_div);
        Plotly.newPlot(graph, gData, { margin: { l: 40, r: 0, b: 35, t: 0 }, yaxis: { title: "# Hosts" }, xaxis: { title: 'TCP Round-Trip Time (ms)' } }, { responsive: true });
    }
}

function ecn(n) {
    switch (n) {
        case 0: return "-";
        case 1: return "L4S";
        case 2: return "ECT0";
        case 3: return "CE";
        default: return "???";
    }
}

function zip(a, b) {
    let zipped = [];
    for (let i=0; i<a.length; ++i) {
        zipped.push(a[i]);
        zipped.push(b[i]);
    }
    return zipped;
}

function zero_to_null(array) {
    for (let i=0; i<array.length; ++i) {
        if (array[i] == 0) array[i] = null;
    }
}