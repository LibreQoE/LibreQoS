import {saveConfig, loadConfig, renderConfigMenu} from "./config/config_helper";

function validateConfig() {
    if (document.getElementById('bridgeMode').checked) {
        // Validate bridge mode fields
        const toInternet = document.getElementById('toInternet').value.trim();
        const toNetwork = document.getElementById('toNetwork').value.trim();
        
        if (!toInternet || !toNetwork) {
            alert("Both interface names are required in bridge mode");
            return false;
        }

        // Validate sandwich mode if enabled
        const sandwichEnabled = document.getElementById('sandwichEnable')?.checked;
        if (sandwichEnabled) {
            const downStr = document.getElementById('sandwichDownOverride').value.trim();
            const upStr = document.getElementById('sandwichUpOverride').value.trim();
            const qStr = document.getElementById('sandwichQueueOverride').value.trim();
            if (downStr !== '') {
                const down = parseInt(downStr);
                if (isNaN(down) || down <= 0) {
                    alert("Override Download Limit must be a positive integer");
                    return false;
                }
            }
            if (upStr !== '') {
                const up = parseInt(upStr);
                if (isNaN(up) || up <= 0) {
                    alert("Override Upload Limit must be a positive integer");
                    return false;
                }
            }
            if (qStr !== '') {
                const q = parseInt(qStr);
                if (isNaN(q) || q <= 0) {
                    alert("TX Queue Count Override must be a positive integer");
                    return false;
                }
            }
        }
    } else if (document.getElementById('singleInterfaceMode').checked) {
        // Validate single interface mode fields
        const interfaceName = document.getElementById('interface').value.trim();
        const internetVlan = parseInt(document.getElementById('internetVlan').value);
        const networkVlan = parseInt(document.getElementById('networkVlan').value);
        
        if (!interfaceName) {
            alert("Interface name is required in single interface mode");
            return false;
        }
        if (isNaN(internetVlan) || internetVlan < 1 || internetVlan > 4094) {
            alert("Internet VLAN must be between 1 and 4094");
            return false;
        }
        if (isNaN(networkVlan) || networkVlan < 1 || networkVlan > 4094) {
            alert("Network VLAN must be between 1 and 4094");
            return false;
        }
    } else {
        alert("Please select either bridge or single interface mode");
        return false;
    }
    return true;
}

function updateConfig() {
    // Clear both sections first
    window.config.bridge = null;
    window.config.single_interface = null;

    if (document.getElementById('bridgeMode').checked) {
        // Update bridge configuration
        const bridge = {
            use_xdp_bridge: document.getElementById('useXdpBridge').checked,
            to_internet: document.getElementById('toInternet').value.trim(),
            to_network: document.getElementById('toNetwork').value.trim()
        };
        // Sandwich mode
        const sandwichEnabled = document.getElementById('sandwichEnable')?.checked;
        if (sandwichEnabled) {
            const withRate = document.getElementById('sandwichRateLimiter').value;
            const downStr = document.getElementById('sandwichDownOverride').value.trim();
            const upStr = document.getElementById('sandwichUpOverride').value.trim();
            const qStr = document.getElementById('sandwichQueueOverride').value.trim();
            const useFqCodel = document.getElementById('sandwichUseFqCodel')?.checked ?? false;
            const down = downStr === '' ? null : parseInt(downStr);
            const up = upStr === '' ? null : parseInt(upStr);
            const queueOverride = qStr === '' ? null : parseInt(qStr);
            bridge.sandwich = {
                Full: {
                    with_rate_limiter: withRate,
                    rate_override_mbps_down: down,
                    rate_override_mbps_up: up,
                    queue_override: queueOverride,
                    use_fq_codel: useFqCodel,
                }
            };
        } else {
            bridge.sandwich = null;
        }
        window.config.bridge = bridge;
    } else {
        // Update single interface configuration
        window.config.single_interface = {
            interface: document.getElementById('interface').value.trim(),
            internet_vlan: parseInt(document.getElementById('internetVlan').value),
            network_vlan: parseInt(document.getElementById('networkVlan').value)
        };
    }
}

// Render the configuration menu
renderConfigMenu('interface');

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    if (window.config) {
        // Determine which mode is active
        if (window.config.bridge) {
            // Bridge mode
            document.getElementById('bridgeMode').checked = true;
            document.getElementById('useXdpBridge').checked = 
                window.config.bridge.use_xdp_bridge ?? true;
            document.getElementById('toInternet').value = 
                window.config.bridge.to_internet ?? "eth0";
            document.getElementById('toNetwork').value = 
                window.config.bridge.to_network ?? "eth1";

            // Sandwich mode
            const sandwich = window.config.bridge.sandwich;
            const sandEnableEl = document.getElementById('sandwichEnable');
            const sandSectionEl = document.getElementById('sandwichSection');
            const rateEl = document.getElementById('sandwichRateLimiter');
            const downEl = document.getElementById('sandwichDownOverride');
            const upEl = document.getElementById('sandwichUpOverride');
            const qEl = document.getElementById('sandwichQueueOverride');
            const fqEl = document.getElementById('sandwichUseFqCodel');
            // Default UI state
            sandEnableEl.checked = false;
            if (sandSectionEl) sandSectionEl.style.display = 'none';
            if (typeof sandwich === 'object' && sandwich !== null && sandwich.Full) {
                const full = sandwich.Full;
                sandEnableEl.checked = true;
                if (sandSectionEl) sandSectionEl.style.display = 'block';
                if (rateEl) rateEl.value = full.with_rate_limiter ?? 'None';
                if (downEl) downEl.value = (full.rate_override_mbps_down ?? '').toString();
                if (upEl) upEl.value = (full.rate_override_mbps_up ?? '').toString();
                if (qEl) qEl.value = (full.queue_override ?? '').toString();
                if (fqEl) fqEl.checked = !!full.use_fq_codel;
            } else {
                // Handle explicit "None" variant or null/undefined by keeping disabled
                if (rateEl) rateEl.value = 'None';
                if (downEl) downEl.value = '';
                if (upEl) upEl.value = '';
                if (qEl) qEl.value = '';
                if (fqEl) fqEl.checked = false;
            }
        } else if (window.config.single_interface) {
            // Single interface mode
            document.getElementById('singleInterfaceMode').checked = true;
            document.getElementById('interface').value = 
                window.config.single_interface.interface ?? "eth0";
            document.getElementById('internetVlan').value = 
                window.config.single_interface.internet_vlan ?? 2;
            document.getElementById('networkVlan').value = 
                window.config.single_interface.network_vlan ?? 3;
        }
        
        // Trigger form visibility update
        const event = new Event('change');
        document.querySelector('input[name="networkMode"]:checked')?.dispatchEvent(event);

        // Add save button click handler
        document.getElementById('saveButton').addEventListener('click', () => {
            if (validateConfig()) {
                updateConfig();
                saveConfig(() => {
                    alert("Configuration saved successfully!");
                });
            }
        });
    } else {
        console.error("Configuration not found in window.config");
    }
});
