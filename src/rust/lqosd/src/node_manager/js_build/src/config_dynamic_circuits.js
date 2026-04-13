import {
    loadConfig,
    loadNetworkJson,
    renderConfigMenu,
    saveConfig,
    validNodeList,
} from "./config/config_helper";

const DEFAULT_TTL_SECONDS = 300;

function normalizeIpRangeInput(value) {
    const raw = String(value ?? "").trim();
    if (!raw) return "";
    if (raw === "0.0.0.0") return "0.0.0.0/0";
    if (raw === "::") return "::/0";
    return raw;
}

function isValidIPv4(ip) {
    if (!/^(\d{1,3}\.){3}\d{1,3}$/.test(ip)) return false;
    const parts = ip.split(".").map((p) => parseInt(p, 10));
    return parts.length === 4 && !parts.some((p) => Number.isNaN(p) || p < 0 || p > 255);
}

function isValidIPv6(ip) {
    return ip.includes(":") && /^[0-9a-fA-F:]+$/.test(ip);
}

function isValidCIDR(cidr) {
    try {
        const [ip, mask, extra] = String(cidr).trim().split("/");
        if (!ip || !mask || extra !== undefined) return false;

        if (ip.includes(":")) {
            if (!isValidIPv6(ip)) return false;
        } else if (!isValidIPv4(ip)) {
            return false;
        }

        const maskNum = parseInt(mask, 10);
        if (Number.isNaN(maskNum)) return false;
        if (ip.includes(":")) {
            if (maskNum < 0 || maskNum > 128) return false;
        } else {
            if (maskNum < 0 || maskNum > 32) return false;
        }

        return true;
    } catch {
        return false;
    }
}

function parseFiniteFloat(value) {
    const num = parseFloat(String(value ?? "").trim());
    return Number.isFinite(num) ? num : null;
}

function parsePositiveInt(value) {
    const num = parseInt(String(value ?? "").trim(), 10);
    if (!Number.isFinite(num)) return null;
    if (num <= 0) return null;
    return num;
}

function getDefaultRule() {
    return {
        name: "",
        ip_range: "0.0.0.0/0",
        download_min_mbps: "10.0",
        upload_min_mbps: "10.0",
        download_max_mbps: "100.0",
        upload_max_mbps: "100.0",
        attach_to: "",
    };
}

function ensureDynamicCircuitsConfig(config) {
    if (!config || typeof config !== "object") return;
    if (!config.dynamic_circuits || typeof config.dynamic_circuits !== "object") {
        config.dynamic_circuits = {
            enabled: false,
            ttl_seconds: DEFAULT_TTL_SECONDS,
            enable_unknown_ip_promotion: false,
            ranges: [],
        };
        return;
    }

    const dyn = config.dynamic_circuits;
    if (typeof dyn.enabled !== "boolean") dyn.enabled = false;
    if (!Number.isFinite(Number(dyn.ttl_seconds))) dyn.ttl_seconds = DEFAULT_TTL_SECONDS;
    if (typeof dyn.enable_unknown_ip_promotion !== "boolean") dyn.enable_unknown_ip_promotion = false;
    if (!Array.isArray(dyn.ranges)) dyn.ranges = [];
}

function setNodeDatalist(networkJson) {
    const datalist = document.getElementById("networkNodesDatalist");
    if (!datalist) return;
    datalist.innerHTML = "";

    if (!networkJson || typeof networkJson !== "object") {
        return;
    }

    for (const name of validNodeList(networkJson)) {
        const option = document.createElement("option");
        option.value = name;
        datalist.appendChild(option);
    }
}

function renderRangesTable() {
    const tbody = document.getElementById("dynamicRangesBody");
    if (!tbody) return;
    tbody.innerHTML = "";

    const ranges = window.config?.dynamic_circuits?.ranges;
    if (!Array.isArray(ranges) || ranges.length === 0) {
        const empty = document.createElement("tr");
        const cell = document.createElement("td");
        cell.colSpan = 6;
        cell.className = "text-muted";
        cell.textContent = "No rules configured.";
        empty.appendChild(cell);
        tbody.appendChild(empty);
        return;
    }

    ranges.forEach((rule, index) => {
        const tr = document.createElement("tr");

        const mkText = (value, onChange, placeholder = "") => {
            const input = document.createElement("input");
            input.type = "text";
            input.className = "form-control form-control-sm";
            input.placeholder = placeholder;
            input.value = String(value ?? "");
            input.addEventListener("input", (ev) => onChange(ev.target.value));
            return input;
        };

        const mkNumber = (value, onChange, step = "0.1") => {
            const input = document.createElement("input");
            input.type = "number";
            input.className = "form-control form-control-sm";
            input.step = step;
            input.value = String(value ?? "");
            input.addEventListener("input", (ev) => onChange(ev.target.value));
            return input;
        };

        const mkRatePair = (
            downValue,
            onDownChange,
            upValue,
            onUpChange,
        ) => {
            const container = document.createElement("div");
            container.className = "d-grid gap-1";

            const mkDir = (icon, ariaLabel, value, onChange) => {
                const group = document.createElement("div");
                group.className = "input-group input-group-sm";

                const prefix = document.createElement("span");
                prefix.className = "input-group-text";
                const i = document.createElement("i");
                i.className = `fa ${icon}`;
                prefix.appendChild(i);

                const input = mkNumber(value, onChange);
                input.setAttribute("aria-label", ariaLabel);

                group.appendChild(prefix);
                group.appendChild(input);
                return group;
            };

            container.appendChild(mkDir("fa-arrow-down", "Down Mbps", downValue, onDownChange));
            container.appendChild(mkDir("fa-arrow-up", "Up Mbps", upValue, onUpChange));
            return container;
        };

        const addCell = (label, child) => {
            const td = document.createElement("td");
            td.dataset.label = label;
            td.appendChild(child);
            tr.appendChild(td);
        };

        addCell(
            "Name",
            mkText(rule.name, (v) => {
                window.config.dynamic_circuits.ranges[index].name = v;
            }, "Rule name"),
        );
        addCell(
            "IP Range",
            mkText(rule.ip_range, (v) => {
                window.config.dynamic_circuits.ranges[index].ip_range = v;
            }, "192.168.0.0/24"),
        );
        addCell(
            "Min Mbps",
            mkRatePair(
                rule.download_min_mbps,
                (v) => {
                    window.config.dynamic_circuits.ranges[index].download_min_mbps = v;
                },
                rule.upload_min_mbps,
                (v) => {
                    window.config.dynamic_circuits.ranges[index].upload_min_mbps = v;
                },
            ),
        );
        addCell(
            "Max Mbps",
            mkRatePair(
                rule.download_max_mbps,
                (v) => {
                    window.config.dynamic_circuits.ranges[index].download_max_mbps = v;
                },
                rule.upload_max_mbps,
                (v) => {
                    window.config.dynamic_circuits.ranges[index].upload_max_mbps = v;
                },
            ),
        );
        const attach = mkText(rule.attach_to, (v) => {
            window.config.dynamic_circuits.ranges[index].attach_to = v;
        }, "network.json node (optional)");
        attach.setAttribute("list", "networkNodesDatalist");
        addCell("Attach To", attach);

        const removeTd = document.createElement("td");
        removeTd.dataset.label = "Actions";
        const removeBtn = document.createElement("button");
        removeBtn.type = "button";
        removeBtn.className = "btn btn-sm btn-outline-danger";
        removeBtn.textContent = "Remove";
        removeBtn.addEventListener("click", () => {
            window.config.dynamic_circuits.ranges.splice(index, 1);
            renderRangesTable();
        });
        removeTd.appendChild(removeBtn);
        tr.appendChild(removeTd);

        tbody.appendChild(tr);
    });
}

function validateConfig() {
    const errors = [];

    const dyn = window.config?.dynamic_circuits;
    if (!dyn || typeof dyn !== "object") {
        errors.push("dynamic_circuits section is missing");
    } else {
        const ttl = parsePositiveInt(dyn.ttl_seconds);
        if (ttl === null) {
            errors.push("Time to live must be a positive integer (seconds)");
        }

        const ranges = Array.isArray(dyn.ranges) ? dyn.ranges : [];
        ranges.forEach((rule, index) => {
            const label = (rule?.name ?? "").trim()
                ? `Rule '${String(rule.name).trim()}'`
                : `Rule #${index + 1}`;

            if (!String(rule?.name ?? "").trim()) {
                errors.push(`${label}: Name is required`);
            }

            const ipRange = normalizeIpRangeInput(rule?.ip_range);
            if (!ipRange) {
                errors.push(`${label}: IP Range is required`);
            } else if (!isValidCIDR(ipRange)) {
                errors.push(`${label}: IP Range must be a valid CIDR (e.g. 192.168.1.0/24). 0.0.0.0 and :: are allowed.`);
            }

            const dmin = parseFiniteFloat(rule?.download_min_mbps);
            const umin = parseFiniteFloat(rule?.upload_min_mbps);
            const dmax = parseFiniteFloat(rule?.download_max_mbps);
            const umax = parseFiniteFloat(rule?.upload_max_mbps);

            if (dmin === null || umin === null || dmax === null || umax === null) {
                errors.push(`${label}: All rate fields must be valid numbers`);
                return;
            }

            if (dmin < 0.1) errors.push(`${label}: Min Download must be >= 0.1 Mbps`);
            if (umin < 0.1) errors.push(`${label}: Min Upload must be >= 0.1 Mbps`);
            if (dmax < 0.2) errors.push(`${label}: Max Download must be >= 0.2 Mbps`);
            if (umax < 0.2) errors.push(`${label}: Max Upload must be >= 0.2 Mbps`);
            if (dmin > dmax) errors.push(`${label}: Min Download must be <= Max Download`);
            if (umin > umax) errors.push(`${label}: Min Upload must be <= Max Upload`);
        });
    }

    if (errors.length === 0) {
        return true;
    }

    alert("Validation errors:\n" + errors.join("\n"));
    return false;
}

function updateConfigFromUi() {
    const dyn = window.config.dynamic_circuits;
    dyn.enabled = !!document.getElementById("dynamicCircuitsEnabled")?.checked;
    dyn.enable_unknown_ip_promotion = !!document.getElementById("unknownIpPromotionEnabled")?.checked;

    const ttlValue = document.getElementById("dynamicCircuitsTtlSeconds")?.value;
    dyn.ttl_seconds = parsePositiveInt(ttlValue) ?? DEFAULT_TTL_SECONDS;

    // Normalize IP ranges in-place so saves canonicalize shorthand.
    if (Array.isArray(dyn.ranges)) {
        dyn.ranges = dyn.ranges.map((rule) => ({
            ...rule,
            name: String(rule?.name ?? "").trim(),
            ip_range: normalizeIpRangeInput(rule?.ip_range),
            attach_to: String(rule?.attach_to ?? "").trim(),
        }));
    }
}

function finalizeConfigForSave() {
    const dyn = window.config.dynamic_circuits;
    if (!dyn || typeof dyn !== "object") return;
    if (!Array.isArray(dyn.ranges)) return;

    dyn.ranges = dyn.ranges.map((rule) => ({
        ...rule,
        download_min_mbps: parseFloat(String(rule?.download_min_mbps ?? "").trim()),
        upload_min_mbps: parseFloat(String(rule?.upload_min_mbps ?? "").trim()),
        download_max_mbps: parseFloat(String(rule?.download_max_mbps ?? "").trim()),
        upload_max_mbps: parseFloat(String(rule?.upload_max_mbps ?? "").trim()),
    }));
}

// Render the configuration menu.
renderConfigMenu("dynamic_circuits");

loadConfig(() => {
    ensureDynamicCircuitsConfig(window.config);

    const dyn = window.config.dynamic_circuits;
    document.getElementById("dynamicCircuitsEnabled").checked = dyn.enabled ?? false;
    document.getElementById("dynamicCircuitsTtlSeconds").value = dyn.ttl_seconds ?? DEFAULT_TTL_SECONDS;
    document.getElementById("unknownIpPromotionEnabled").checked = dyn.enable_unknown_ip_promotion ?? false;

    // Load network nodes for the attach_to datalist (UX-only).
    loadNetworkJson(
        (networkJson) => {
            if (typeof networkJson === "object" && networkJson !== null) {
                setNodeDatalist(networkJson);
            }
        },
        () => {},
    );

    renderRangesTable();

    document.getElementById("addDynamicRange").addEventListener("click", () => {
        window.config.dynamic_circuits.ranges.push(getDefaultRule());
        renderRangesTable();
    });

    document.getElementById("saveButton").addEventListener("click", () => {
        updateConfigFromUi();
        if (!validateConfig()) return;
        finalizeConfigForSave();
        saveConfig(() => {
            alert("Configuration saved successfully!");
        });
    });
});
