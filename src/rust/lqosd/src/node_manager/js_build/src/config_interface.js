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
        window.config.bridge = {
            use_xdp_bridge: document.getElementById('useXdpBridge').checked,
            to_internet: document.getElementById('toInternet').value.trim(),
            to_network: document.getElementById('toNetwork').value.trim()
        };
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
