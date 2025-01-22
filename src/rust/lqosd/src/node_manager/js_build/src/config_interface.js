import {loadConfig} from "./config/config_helper";

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
    } else {
        console.error("Configuration not found in window.config");
    }
});
