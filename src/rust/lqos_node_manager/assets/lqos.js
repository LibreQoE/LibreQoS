document.addEventListener("DOMContentLoaded", () => {
	let webSocket = new WebSocket('ws://10.1.1.222:3000/ws');
	webSocket.onmessage = function(e) { process_main_ws(e.data) };
	
	var element = document.getElementById('btn-toggle-theme');
	if (document.body.classList.contains('dark-theme')) {
		if (typeof(element) != 'undefined' && element != null) {
			document.getElementById('btn-toggle-theme').checked = true;
		} else {
			document.getElementById('btn-toggle-theme').checked = false;
		}
	} else {
		if (typeof(element) != 'undefined' && element != null) {
			document.getElementById('btn-toggle-theme').checked = false;
		}
	}
});

function process_main_ws(data) {
	let type = JSON.parse(data)[0];
	let message = JSON.parse(data)[1]
	if (type == "SHPDIP") {
		update_shaped(message);
	} else if (type == "UNKNIP") {
		update_unknown(message);
	} else if (type == "DISK") {
		
	} else if (type == "RAM") {
		
	} else if (type == "CPU") {
		
	}
}

function update_shaped(values){
	var spans = document.getElementsByClassName("live-shaped");

	for(i=0;i<spans.length;i++)	{
		spans[i].innerHTML = values;
	}
}

function update_unknown(values){
	var spans = document.getElementsByClassName("live-unknown");

	for(i=0;i<spans.length;i++)	{
		spans[i].innerHTML = values;
	}
}

function update_cpu(values){
	var spans = document.getElementsByClassName("live-cpu");

	for(i=0;i<spans.length;i++)	{
		spans[i].innerHTML = values;
	}
}

function update_disk(values){
	var spans = document.getElementsByClassName("live-disk");

	for(i=0;i<spans.length;i++)	{
		spans[i].innerHTML = values;
	}
}

function update_ram(values){
	var spans = document.getElementsByClassName("live-ram");

	for(i=0;i<spans.length;i++)	{
		spans[i].innerHTML = values;
	}
}

function toggleThemeChange(src) {
	var event = document.createEvent('Event');
	event.initEvent('themeChange', true, true);

	if (document.body.classList.contains('dark-theme')) {
		document.body.classList.remove('dark-theme');
	} else {
		document.body.classList.add('dark-theme');
	}
	document.body.dispatchEvent(event);
}

function metaverse_color_ramp(n) {
    if (n <= 9) {
        return "#32b08c";
    } else if (n <= 20) {
        return "#ffb94a";
    } else if (n <=50) {
        return "#f95f53";
    } else if (n <=70) {
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

function cssrules() {
    var rules = {};
    for (var i=0; i<document.styleSheets.length; ++i) {
        var cssRules = document.styleSheets[i].cssRules;
        for (var j=0; j<cssRules.length; ++j)
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
    $.get("/api/host_counts", (hc) => {
        $("#shapedCount").text(hc[0]);
        $("#unshapedCount").text(hc[1]);
        setTimeout(updateHostCounts, 5000);
    });
    $.get("/api/username", (un) => {
		$("#currentLogin span").text(un);
    });
    $("#startTest").on('click', () => {
        $.get("/api/run_btest", () => {});
    });
}

function colorReloadButton() {
    $("body").append(reloadModal);
    $("#btnReload").on('click', () => {
        $.get("/api/reload_libreqos", (result) => {
            const myModal = new bootstrap.Modal(document.getElementById('reloadModal'), {focus: true});
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
    for(let i = 0; i < text.length; i++){
        let code = text.charCodeAt(i);
        sum += code;
    }
    sum = sum % phrases.length;
    return phrases[sum];
}

function scaleNumber(n) {
    if (n > 1000000000000) {
        return (n/1000000000000).toFixed(2) + "T";
    } else if (n > 1000000000) {
        return (n/1000000000).toFixed(2) + "G";
    } else if (n > 1000000) {
        return (n/1000000).toFixed(2) + "M";
    } else if (n > 1000) {
        return (n/1000).toFixed(2) + "K";
    }
    return n;
}
