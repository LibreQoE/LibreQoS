import {
    loadConfig,
    loadNetworkJson,
    renderConfigMenu,
    saveConfig,
} from "./config/config_helper";

let networkData = null;
let selectedTargets = [];

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
                populateSiteSelector();
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

// Build dropdown options from network tree
function populateSiteSelector() {
    const selector = document.getElementById('siteSelector');
    selector.innerHTML = '<option value="">Select a site...</option>';
    
    function iterate(data, level = 0) {
        // Handle case where data might be a string or other non-object
        if (typeof data !== 'object' || data === null) {
            console.warn('Data is not an object:', data);
            return;
        }
        
        for (const [key, value] of Object.entries(data)) {
            const option = document.createElement('option');
            option.value = key;
            
            // Add indentation for hierarchy
            let prefix = '';
            for (let i = 0; i < level; i++) {
                prefix += '- ';
            }
            option.textContent = prefix + key;
            
            selector.appendChild(option);
            
            // Recursively add children
            if (value && typeof value === 'object' && value.children != null) {
                iterate(value.children, level + 1);
            }
        }
    }
    
    if (networkData) {
        console.log('Populating site selector with:', networkData);
        iterate(networkData);
    } else {
        console.error('Network data is null or undefined');
    }
}

// Add site to targets list
function addSite() {
    const selector = document.getElementById('siteSelector');
    const selectedSite = selector.value;
    
    if (!selectedSite) {
        alert('Please select a site to add');
        return;
    }
    
    // Check if already in list
    if (selectedTargets.includes(selectedSite)) {
        alert('This site is already being monitored');
        return;
    }
    
    selectedTargets.push(selectedSite);
    updateTargetsList();
    
    // Reset selector
    selector.value = '';
}

// Remove site from targets list
function removeSite(siteName) {
    const index = selectedTargets.indexOf(siteName);
    if (index > -1) {
        selectedTargets.splice(index, 1);
        updateTargetsList();
    }
}

// Update the targets list UI
function updateTargetsList() {
    const listContainer = document.getElementById('selectedSitesList');
    listContainer.innerHTML = '';
    
    if (selectedTargets.length === 0) {
        listContainer.innerHTML = '<div class="text-muted">No sites selected</div>';
        return;
    }
    
    selectedTargets.forEach(site => {
        const listItem = document.createElement('div');
        listItem.className = 'list-group-item d-flex justify-content-between align-items-center';
        
        const siteName = document.createElement('span');
        siteName.textContent = site;
        
        const removeBtn = document.createElement('button');
        removeBtn.className = 'btn btn-sm btn-outline-danger';
        removeBtn.innerHTML = '<i class="fa fa-times"></i>';
        removeBtn.onclick = () => removeSite(site);
        
        listItem.appendChild(siteName);
        listItem.appendChild(removeBtn);
        listContainer.appendChild(listItem);
    });
}

// Validate configuration
function validateConfig() {
    const enabled = document.getElementById('enabled').checked;
    const minDownloadPct = parseInt(document.getElementById('minDownloadPct').value);
    const minUploadPct = parseInt(document.getElementById('minUploadPct').value);
    
    // Validate percentage values
    if (isNaN(minDownloadPct) || minDownloadPct < 1 || minDownloadPct > 100) {
        alert('Minimum Download Percentage must be between 1 and 100');
        return false;
    }
    
    if (isNaN(minUploadPct) || minUploadPct < 1 || minUploadPct > 100) {
        alert('Minimum Upload Percentage must be between 1 and 100');
        return false;
    }
    
    // If enabled, must have at least one target
    if (enabled && selectedTargets.length === 0) {
        alert('Please select at least one site to monitor when StormGuard is enabled');
        return false;
    }
    
    return true;
}

// Update config object
function updateConfig() {
    const logFilePath = document.getElementById('logFile').value.trim();
    
    window.config.stormguard = {
        enabled: document.getElementById('enabled').checked,
        targets: selectedTargets,
        dry_run: document.getElementById('dryRun').checked,
        log_file: logFilePath === '' ? null : logFilePath,
        minimum_download_percentage: parseInt(document.getElementById('minDownloadPct').value) / 100,
        minimum_upload_percentage: parseInt(document.getElementById('minUploadPct').value) / 100
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
    
    // Now populate the UI with config data
    if (window.config && window.config.stormguard) {
        const sg = window.config.stormguard;
        
        // Set form values
        document.getElementById('enabled').checked = sg.enabled || false;
        document.getElementById('dryRun').checked = sg.dry_run || false;
        document.getElementById('logFile').value = sg.log_file || '';
        
        // Convert decimal to percentage for display
        document.getElementById('minDownloadPct').value = Math.round((sg.minimum_download_percentage || 0.5) * 100);
        document.getElementById('minUploadPct').value = Math.round((sg.minimum_upload_percentage || 0.5) * 100);
        
        // Load targets
        selectedTargets = sg.targets || [];
        updateTargetsList();
    }
    
    // Set up event handlers
    document.getElementById('addSiteBtn').addEventListener('click', addSite);
    
    // Allow Enter key in site selector to add site
    document.getElementById('siteSelector').addEventListener('keypress', (e) => {
        if (e.key === 'Enter') {
            e.preventDefault();
            addSite();
        }
    });
    
    // Save button handler
    document.getElementById('saveButton').addEventListener('click', () => {
        if (validateConfig()) {
            updateConfig();
            saveConfig(() => {
                alert('StormGuard configuration saved successfully!');
            });
        }
    });
});
