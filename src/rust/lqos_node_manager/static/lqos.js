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
        let html = "";
        if (un == "Anonymous") {
            html = "<a class='nav-link' href='/login'><i class='fa fa-user'></i> Login</a>";
        } else {
            html = "<a class='nav-link' onclick='deleteAllCookies();'><i class='fa fa-user'></i> Logout " + un + "</a>";
        }
        $("#currentLogin").html(html);
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