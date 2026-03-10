import {
    loadConfig,
    loadNetworkJson,
    renderConfigMenu,
    saveConfig,
} from "./config/config_helper";

let networkData = null;
let selectedTargets = [];
let excludedSites = [];

function defaultStormguardConfig() {
    return {
        enabled: false,
        dry_run: true,
        log_file: null,
        all_sites: false,
        targets: [],
        exclude_sites: [],
        minimum_download_percentage: 0.5,
        minimum_upload_percentage: 0.5,
        increase_fast_multiplier: 1.30,
        increase_multiplier: 1.15,
        decrease_multiplier: 0.95,
        decrease_fast_multiplier: 0.88,
        increase_fast_cooldown_seconds: 2,
        increase_cooldown_seconds: 1,
        decrease_cooldown_seconds: 3.75,
        decrease_fast_cooldown_seconds: 7.5,
        circuit_fallback_enabled: false,
        circuit_fallback_persist: true,
        circuit_fallback_sqm: "fq_codel",
    };
}

const VALID_FALLBACK_SQMS = ['fq_codel', 'cake'];

function ensureStormguardConfig(config) {
    return {
        ...defaultStormguardConfig(),
        ...(config || {}),
        targets: Array.isArray(config?.targets) ? [...config.targets] : [],
        exclude_sites: Array.isArray(config?.exclude_sites) ? [...config.exclude_sites] : [],
    };
}

function updateTargetsUi() {
    const allSites = document.getElementById("allSites")?.checked ?? false;
    const section = document.getElementById("targetsSection");
    if (section) {
        section.style.display = allSites ? "none" : "";
    }
}

// Load network.json for site dropdown
function loadNetworkData() {
    return new Promise((resolve, reject) => {
        loadNetworkJson(
            (data) => {
                // Check if we got the "Not done yet" response
                if (typeof data === 'string' && data === 'Not done yet') {
                    console.error('Network.json file not found on server');
                    alert('Network configuration not found. Please ensure network.json exists.');
                    resolve();
                    return;
                }
                networkData = data;
                console.log('Network data loaded:', networkData);
                populateSiteSelectors();
                resolve();
            },
            (err) => {
                console.error('Error loading network data:', err);
                alert('Failed to load network sites. Please refresh the page.');
                reject(err);
            },
        );
    });
}

function setSelectorOptions(selectorId) {
    const selector = document.getElementById(selectorId);
    selector.innerHTML = '<option value="">Select a site...</option>';

    function iterate(data, level = 0) {
        if (typeof data !== 'object' || data === null) {
            return;
        }

        for (const [key, value] of Object.entries(data)) {
            const option = document.createElement('option');
            option.value = key;

            let prefix = '';
            for (let i = 0; i < level; i++) {
                prefix += '- ';
            }
            option.textContent = prefix + key;

            selector.appendChild(option);

            if (value && typeof value === 'object' && value.children != null) {
                iterate(value.children, level + 1);
            }
        }
    }

    if (networkData) {
        iterate(networkData);
    }
}

function populateSiteSelectors() {
    setSelectorOptions("targetSiteSelector");
    setSelectorOptions("excludeSiteSelector");
}

function addItemToList(listName, value, duplicateMessage) {
    if (!value) {
        return false;
    }

    const list = listName === "targets" ? selectedTargets : excludedSites;
    if (list.includes(value)) {
        alert(duplicateMessage);
        return false;
    }

    list.push(value);
    list.sort((a, b) => a.localeCompare(b));
    return true;
}

function addTargetFromSelector() {
    const selector = document.getElementById("targetSiteSelector");
    const selectedSite = selector.value;

    if (!selectedSite) {
        alert('Please select a site to add');
        return;
    }

    if (addItemToList("targets", selectedSite, 'This site is already in the allowlist')) {
        selector.value = '';
        updateTargetsList();
    }
}

function addTargetFromManual() {
    const input = document.getElementById("targetSiteManual");
    const siteName = input.value.trim();
    if (!siteName) {
        alert('Please enter a site name');
        return;
    }

    if (addItemToList("targets", siteName, 'This site is already in the allowlist')) {
        input.value = '';
        updateTargetsList();
    }
}

function addExcludeFromSelector() {
    const selector = document.getElementById("excludeSiteSelector");
    const selectedSite = selector.value;
    if (!selectedSite) {
        alert('Please select a site to exclude');
        return;
    }

    if (addItemToList("exclude", selectedSite, 'This site is already excluded')) {
        selector.value = '';
        updateExcludedSitesList();
    }
}

function addExcludeFromManual() {
    const input = document.getElementById("excludeSiteManual");
    const siteName = input.value.trim();
    if (!siteName) {
        alert('Please enter a site name');
        return;
    }

    if (addItemToList("exclude", siteName, 'This site is already excluded')) {
        input.value = '';
        updateExcludedSitesList();
    }
}

function removeItem(listName, itemName) {
    const list = listName === "targets" ? selectedTargets : excludedSites;
    const index = list.indexOf(itemName);
    if (index > -1) {
        list.splice(index, 1);
    }
}

// Add site to targets list
function removeTarget(siteName) {
    removeItem("targets", siteName);
    updateTargetsList();
}

function removeExcludedSite(siteName) {
    removeItem("exclude", siteName);
    updateExcludedSitesList();
}

function updateList(listId, emptyMessage, items, removeHandler) {
    const listContainer = document.getElementById(listId);
    listContainer.innerHTML = '';

    if (items.length === 0) {
        listContainer.innerHTML = `<div class="text-muted">${emptyMessage}</div>`;
        return;
    }

    items.forEach((site) => {
        const listItem = document.createElement('div');
        listItem.className = 'list-group-item d-flex justify-content-between align-items-center';

        const siteName = document.createElement('span');
        siteName.textContent = site;

        const removeBtn = document.createElement('button');
        removeBtn.className = 'btn btn-sm btn-outline-danger';
        removeBtn.innerHTML = '<i class="fa fa-times"></i>';
        removeBtn.onclick = () => removeHandler(site);

        listItem.appendChild(siteName);
        listItem.appendChild(removeBtn);
        listContainer.appendChild(listItem);
    });
}

function updateTargetsList() {
    updateList("selectedTargetsList", "No allowlisted sites", selectedTargets, removeTarget);
}

function updateExcludedSitesList() {
    updateList("excludedSitesList", "No excluded sites", excludedSites, removeExcludedSite);
}

function parseNumber(id) {
    return parseFloat(document.getElementById(id).value);
}

function validatePercent(name, value) {
    if (Number.isNaN(value) || value < 1 || value > 100) {
        alert(`${name} must be between 1 and 100`);
        return false;
    }
    return true;
}

function validatePositiveNumber(name, value, min, relationText) {
    if (Number.isNaN(value) || value < min) {
        alert(`${name} must be ${relationText}`);
        return false;
    }
    return true;
}

// Validate configuration
function validateConfig() {
    const enabled = document.getElementById('enabled').checked;
    const allSites = document.getElementById('allSites').checked;
    const minDownloadPct = parseNumber('minDownloadPct');
    const minUploadPct = parseNumber('minUploadPct');

    if (!validatePercent('Minimum Download Percentage', minDownloadPct)) {
        return false;
    }

    if (!validatePercent('Minimum Upload Percentage', minUploadPct)) {
        return false;
    }

    if (enabled && !allSites && selectedTargets.length === 0) {
        alert('Please select at least one site to monitor when StormGuard is enabled');
        return false;
    }

    const increaseFastMultiplier = parseNumber('increaseFastMultiplier');
    const increaseMultiplier = parseNumber('increaseMultiplier');
    const decreaseMultiplier = parseNumber('decreaseMultiplier');
    const decreaseFastMultiplier = parseNumber('decreaseFastMultiplier');

    if (!validatePositiveNumber('Increase Fast Multiplier', increaseFastMultiplier, 0.01, 'greater than 1.0')) return false;
    if (!validatePositiveNumber('Increase Multiplier', increaseMultiplier, 0.01, 'greater than 1.0')) return false;
    if (!validatePositiveNumber('Decrease Multiplier', decreaseMultiplier, 0.01, 'greater than 0')) return false;
    if (!validatePositiveNumber('Decrease Fast Multiplier', decreaseFastMultiplier, 0.01, 'greater than 0')) return false;

    if (increaseFastMultiplier <= 1) {
        alert('Increase Fast Multiplier must be greater than 1.0');
        return false;
    }

    if (increaseMultiplier <= 1) {
        alert('Increase Multiplier must be greater than 1.0');
        return false;
    }

    if (decreaseMultiplier > 1) {
        alert('Decrease Multiplier must be less than or equal to 1.0');
        return false;
    }

    if (decreaseFastMultiplier > 1) {
        alert('Decrease Fast Multiplier must be less than or equal to 1.0');
        return false;
    }

    const cooldownFields = [
        ['Increase Fast Cooldown', 'increaseFastCooldownSeconds'],
        ['Increase Cooldown', 'increaseCooldownSeconds'],
        ['Decrease Cooldown', 'decreaseCooldownSeconds'],
        ['Decrease Fast Cooldown', 'decreaseFastCooldownSeconds'],
    ];

    for (const [name, id] of cooldownFields) {
        if (!validatePositiveNumber(name, parseNumber(id), 0.01, 'greater than 0 seconds')) {
            return false;
        }
    }

    const fallbackEnabled = document.getElementById('circuitFallbackEnabled').checked;
    const fallbackSqm = document.getElementById('circuitFallbackSqm').value.trim();
    if (fallbackEnabled && !VALID_FALLBACK_SQMS.includes(fallbackSqm)) {
        alert('Circuit Fallback SQM must be one of: fq_codel, cake');
        return false;
    }

    return true;
}

// Update config object
function updateConfig() {
    const logFilePath = document.getElementById('logFile').value.trim();
    
    window.config.stormguard = {
        enabled: document.getElementById('enabled').checked,
        dry_run: document.getElementById('dryRun').checked,
        log_file: logFilePath === '' ? null : logFilePath,
        all_sites: document.getElementById('allSites').checked,
        targets: [...selectedTargets],
        exclude_sites: [...excludedSites],
        minimum_download_percentage: parseNumber('minDownloadPct') / 100,
        minimum_upload_percentage: parseNumber('minUploadPct') / 100,
        increase_fast_multiplier: parseNumber('increaseFastMultiplier'),
        increase_multiplier: parseNumber('increaseMultiplier'),
        decrease_multiplier: parseNumber('decreaseMultiplier'),
        decrease_fast_multiplier: parseNumber('decreaseFastMultiplier'),
        increase_fast_cooldown_seconds: parseNumber('increaseFastCooldownSeconds'),
        increase_cooldown_seconds: parseNumber('increaseCooldownSeconds'),
        decrease_cooldown_seconds: parseNumber('decreaseCooldownSeconds'),
        decrease_fast_cooldown_seconds: parseNumber('decreaseFastCooldownSeconds'),
        circuit_fallback_enabled: document.getElementById('circuitFallbackEnabled').checked,
        circuit_fallback_persist: document.getElementById('circuitFallbackPersist').checked,
        circuit_fallback_sqm: document.getElementById('circuitFallbackSqm').value.trim() || 'fq_codel',
    };
}

// Initialize page
renderConfigMenu('stormguard');

// Load both network data and configuration
Promise.all([
    loadNetworkData(),
    new Promise((resolve) => {
        loadConfig(() => resolve());
    })
]).then(() => {
    console.log('Both network data and config loaded');

    const sg = ensureStormguardConfig(window.config?.stormguard);

    document.getElementById('enabled').checked = sg.enabled;
    document.getElementById('dryRun').checked = sg.dry_run;
    document.getElementById('logFile').value = sg.log_file || '';
    document.getElementById('allSites').checked = sg.all_sites;
    document.getElementById('minDownloadPct').value = Math.round(sg.minimum_download_percentage * 100);
    document.getElementById('minUploadPct').value = Math.round(sg.minimum_upload_percentage * 100);
    document.getElementById('increaseFastMultiplier').value = sg.increase_fast_multiplier;
    document.getElementById('increaseMultiplier').value = sg.increase_multiplier;
    document.getElementById('decreaseMultiplier').value = sg.decrease_multiplier;
    document.getElementById('decreaseFastMultiplier').value = sg.decrease_fast_multiplier;
    document.getElementById('increaseFastCooldownSeconds').value = sg.increase_fast_cooldown_seconds;
    document.getElementById('increaseCooldownSeconds').value = sg.increase_cooldown_seconds;
    document.getElementById('decreaseCooldownSeconds').value = sg.decrease_cooldown_seconds;
    document.getElementById('decreaseFastCooldownSeconds').value = sg.decrease_fast_cooldown_seconds;
    document.getElementById('circuitFallbackEnabled').checked = sg.circuit_fallback_enabled;
    document.getElementById('circuitFallbackPersist').checked = sg.circuit_fallback_persist;
    document.getElementById('circuitFallbackSqm').value = VALID_FALLBACK_SQMS.includes(sg.circuit_fallback_sqm)
        ? sg.circuit_fallback_sqm
        : 'fq_codel';

    selectedTargets = [...sg.targets].sort((a, b) => a.localeCompare(b));
    excludedSites = [...sg.exclude_sites].sort((a, b) => a.localeCompare(b));
    updateTargetsUi();
    updateTargetsList();
    updateExcludedSitesList();

    document.getElementById('allSites').addEventListener('change', updateTargetsUi);
    document.getElementById('addTargetBtn').addEventListener('click', addTargetFromSelector);
    document.getElementById('addTargetManualBtn').addEventListener('click', addTargetFromManual);
    document.getElementById('addExcludeBtn').addEventListener('click', addExcludeFromSelector);
    document.getElementById('addExcludeManualBtn').addEventListener('click', addExcludeFromManual);

    document.getElementById('targetSiteSelector').addEventListener('keypress', (e) => {
        if (e.key === 'Enter') {
            e.preventDefault();
            addTargetFromSelector();
        }
    });

    document.getElementById('targetSiteManual').addEventListener('keypress', (e) => {
        if (e.key === 'Enter') {
            e.preventDefault();
            addTargetFromManual();
        }
    });

    document.getElementById('excludeSiteSelector').addEventListener('keypress', (e) => {
        if (e.key === 'Enter') {
            e.preventDefault();
            addExcludeFromSelector();
        }
    });

    document.getElementById('excludeSiteManual').addEventListener('keypress', (e) => {
        if (e.key === 'Enter') {
            e.preventDefault();
            addExcludeFromManual();
        }
    });

    document.getElementById('saveButton').addEventListener('click', () => {
        if (validateConfig()) {
            updateConfig();
            saveConfig(() => {
                alert('StormGuard configuration saved successfully!');
            });
        }
    });
});
