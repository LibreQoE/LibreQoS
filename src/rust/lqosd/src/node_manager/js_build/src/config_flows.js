import {saveConfig, loadConfig, renderConfigMenu} from "./config/config_helper";

const urlParams = new URLSearchParams(window.location.search);
const prefillDoNotTrack = String(urlParams.get("prefillDoNotTrack") || "").trim();
let configLoaded = false;
let controlsBound = false;

function isValidIPv4(ip) {
    if (!/^(\d{1,3}\.){3}\d{1,3}$/.test(ip)) return false;
    const parts = ip.split('.').map((p) => parseInt(p, 10));
    return parts.length === 4 && !parts.some((p) => Number.isNaN(p) || p < 0 || p > 255);
}

function isValidIPv6(ip) {
    return ip.includes(':') && /^[0-9a-fA-F:]+$/.test(ip);
}

function normalizeSubnetInput(value) {
    const raw = String(value || "").trim();
    if (!raw) return "";
    if (raw.includes('/')) return raw;
    if (isValidIPv4(raw)) return `${raw}/32`;
    if (isValidIPv6(raw)) return `${raw}/128`;
    return raw;
}

function setDoNotTrackLoadStatus(message, level = "warning") {
    const el = document.getElementById("doNotTrackLoadStatus");
    if (!el) return;
    if (!message) {
        el.textContent = "";
        el.className = "small mt-2";
        return;
    }
    el.textContent = message;
    el.className = `small mt-2 text-${level}`;
}

function isValidCIDR(cidr) {
    try {
        const [ip, mask, extra] = String(cidr).trim().split('/');
        if (!ip || !mask || extra !== undefined) return false;

        // Validate IP address (basic)
        if (ip.includes(':')) {
            if (!isValidIPv6(ip)) return false;
        } else if (!isValidIPv4(ip)) {
            return false;
        }        

        // Validate mask
        const maskNum = parseInt(mask, 10);
        if (Number.isNaN(maskNum)) return false;
        if (ip.includes(':')) {
            if (maskNum < 0 || maskNum > 128) return false;
        } else {
            if (maskNum < 0 || maskNum > 32) return false;
        }

        return true;
    } catch {
        return false;
    }
}

function populateDoNotTrackList(selectId, subnets) {
    const select = document.getElementById(selectId);
    select.innerHTML = '';
    (subnets || []).forEach((subnet) => {
        const option = document.createElement('option');
        option.value = subnet;
        option.text = subnet;
        select.appendChild(option);
    });
}

function addSubnet(listId, inputId) {
    const input = document.getElementById(inputId);
    const cidr = normalizeSubnetInput(input.value);
    if (!isValidCIDR(cidr)) {
        alert('Please enter a valid IP or CIDR notation (e.g. 8.8.8.8, 192.168.1.0/24, or 2001:db8::/32)');
        return;
    }

    const select = document.getElementById(listId);
    for (let i = 0; i < select.options.length; i++) {
        if (select.options[i].value.toLowerCase() === cidr.toLowerCase()) {
            alert('This CIDR is already in the list');
            return;
        }
    }
    const option = document.createElement('option');
    option.value = cidr;
    option.text = cidr;
    select.appendChild(option);
    input.value = '';
}

function removeSubnet(listId) {
    const select = document.getElementById(listId);
    const selected = Array.from(select.selectedOptions);
    selected.forEach(option => select.removeChild(option));
}

function getSubnetsFromList(listId) {
    const select = document.getElementById(listId);
    return Array.from(select.options).map(option => option.value);
}

function validateDoNotTrackList() {
    const items = getSubnetsFromList('doNotTrackSubnets');
    return items.filter((cidr) => !isValidCIDR(cidr));
}

function updateDoNotTrackValidationUi() {
    const invalid = validateDoNotTrackList();
    const holder = document.getElementById("doNotTrackValidation");
    const save = document.getElementById("saveButton");
    if (save) save.disabled = !configLoaded || invalid.length > 0;
    if (!holder) return;

    if (!configLoaded) {
        holder.className = "small mt-3 text-warning";
        holder.innerHTML = "Loading current configuration. Add/remove works now; Save will enable once loading completes.";
        return;
    }

    if (invalid.length === 0) {        
        holder.className = "small mt-3 text-success";
        holder.innerHTML = `All entries look like valid CIDR notation. The flow tracker will honor this ignore list.`;
        return;
    }

    holder.className = "small mt-3 text-danger";
    holder.innerHTML = `
        <div><strong>Invalid CIDR entries detected:</strong></div>
        <ul class="mb-0">${invalid.map(v => `<li><code>${v}</code></li>`).join("")}</ul>
        <div class="mt-1">Fix/remove these entries to enable saving.</div>
    `;
}

function validateConfig() {
    // Validate required fields
    const flowTimeout = parseInt(document.getElementById("flowTimeout").value);
    if (isNaN(flowTimeout) || flowTimeout < 1) {
        alert("Flow Timeout must be a number greater than 0");
        return false;
    }

    // Validate optional fields if provided
    const netflowPort = document.getElementById("netflowPort").value;
    if (netflowPort && (isNaN(netflowPort) || netflowPort < 1 || netflowPort > 65535)) {
        alert("Netflow Port must be a number between 1 and 65535");
        return false;
    }

    const netflowIp = document.getElementById("netflowIP").value.trim();
    if (netflowIp) {
        try {
            new URL(`http://${netflowIp}`);
        } catch {
            alert("Netflow IP must be a valid IP address");
            return false;
        }
    }

    const invalid = validateDoNotTrackList();
    if (invalid.length > 0) {
        alert("Invalid CIDR entries:\n" + invalid.join("\n"));
        return false;
    }

    return true;
}

function updateConfig() {
    if (!window.config || typeof window.config !== "object") {
        return;
    }

    // Update only the flows section
    window.config.flows = {
        flow_timeout_seconds: parseInt(document.getElementById("flowTimeout").value),
        netflow_enabled: document.getElementById("enableNetflow").checked,
        netflow_port: document.getElementById("netflowPort").value ? 
            parseInt(document.getElementById("netflowPort").value) : null,
        netflow_ip: document.getElementById("netflowIP").value.trim() || null,
        netflow_version: document.getElementById("netflowVersion").value ?
            parseInt(document.getElementById("netflowVersion").value) : null,
        do_not_track_subnets: getSubnetsFromList('doNotTrackSubnets'),
    };
}

function bindControls() {
    if (controlsBound) return;
    controlsBound = true;

    const addBtn = document.getElementById('addDoNotTrackSubnet');
    const removeBtn = document.getElementById('removeDoNotTrackSubnet');
    const saveBtn = document.getElementById('saveButton');
    const input = document.getElementById('newDoNotTrackSubnet');

    if (addBtn) {
        addBtn.addEventListener('click', () => {
            addSubnet('doNotTrackSubnets', 'newDoNotTrackSubnet');
            updateDoNotTrackValidationUi();
        });
    }
    if (removeBtn) {
        removeBtn.addEventListener('click', () => {
            removeSubnet('doNotTrackSubnets');
            updateDoNotTrackValidationUi();
        });
    }
    if (input) {
        input.addEventListener('keydown', (e) => {
            if (e.key === "Enter") {
                e.preventDefault();
                addSubnet('doNotTrackSubnets', 'newDoNotTrackSubnet');
                updateDoNotTrackValidationUi();
            }
        });
    }
    if (saveBtn) {
        saveBtn.addEventListener('click', () => {
            if (!configLoaded) {
                alert("Configuration is still loading. Please try again in a moment.");
                return;
            }
            if (validateConfig()) {
                updateConfig();
                saveConfig(() => {
                    alert("Configuration saved successfully!");
                });
            }
        });
    }
}

// Render the configuration menu
renderConfigMenu('flows');
bindControls();
setDoNotTrackLoadStatus("Loading current configuration…", "warning");
updateDoNotTrackValidationUi();

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    if (window.config) {
        const flows = window.config.flows || {
            flow_timeout_seconds: 30,
            netflow_enabled: false,
            netflow_port: null,
            netflow_ip: null,
            netflow_version: null,
            do_not_track_subnets: [],
        };
        
        // Required fields
        document.getElementById("flowTimeout").value = flows.flow_timeout_seconds ?? 30;
        document.getElementById("enableNetflow").checked = flows.netflow_enabled ?? false;

        // Optional fields
        document.getElementById("netflowPort").value = flows.netflow_port ?? "";
        document.getElementById("netflowIP").value = flows.netflow_ip ?? "";
        document.getElementById("netflowVersion").value = flows.netflow_version ?? "5";

        // Populate do not track list
        populateDoNotTrackList('doNotTrackSubnets', flows.do_not_track_subnets || []);
        configLoaded = true;
        setDoNotTrackLoadStatus("");
        updateDoNotTrackValidationUi();

        // Optional prefill from other UI pages (e.g. Circuit/ASN Explorer).
        if (prefillDoNotTrack) {
            const select = document.getElementById("doNotTrackSubnets");
            const input = document.getElementById("newDoNotTrackSubnet");
            const cardBody = select ? select.closest(".card-body") : null;
            const card = cardBody ? cardBody.closest(".card") : null;

            const existingNotice = document.getElementById("doNotTrackPrefillNotice");
            if (existingNotice) existingNotice.remove();

            const valid = isValidCIDR(prefillDoNotTrack);
            const already = getSubnetsFromList("doNotTrackSubnets").includes(prefillDoNotTrack);

            if (cardBody) {
                const notice = document.createElement("div");
                notice.id = "doNotTrackPrefillNotice";
                notice.className = valid ? "alert alert-info py-2 small" : "alert alert-warning py-2 small";

                const line1 = document.createElement("div");
                const strong = document.createElement("strong");
                strong.textContent = "Prefilled exclusion: ";
                const code = document.createElement("code");
                code.textContent = prefillDoNotTrack;
                line1.appendChild(strong);
                line1.appendChild(code);
                notice.appendChild(line1);

                const line2 = document.createElement("div");
                if (!valid) {
                    line2.textContent = "This doesn’t look like valid CIDR notation. Please verify/edit it manually.";
                } else if (already) {
                    line2.textContent = "This entry is already present in the list.";
                } else {
                    line2.textContent = "Click Add, then Save Changes to apply it.";
                }
                notice.appendChild(line2);

                cardBody.insertBefore(notice, cardBody.firstChild);
            }

            if (valid) {
                if (input) {
                    input.value = prefillDoNotTrack;
                    input.focus();
                }
                if (already && select) {
                    select.value = prefillDoNotTrack;
                }
                if (card) {
                    card.scrollIntoView({ behavior: "smooth", block: "center" });
                    card.classList.add("border", "border-info");
                    setTimeout(() => {
                        card.classList.remove("border", "border-info");
                    }, 3000);
                }
            }
        }
    } else {
        setDoNotTrackLoadStatus("Could not load configuration from server. Add/remove still works, but saving is disabled.", "danger");
        console.error("Flows configuration not found in window.config");
    }
}, () => {
    setDoNotTrackLoadStatus("Could not load configuration from server. Add/remove still works, but saving is disabled.", "danger");
    updateDoNotTrackValidationUi();
});
