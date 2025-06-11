import {saveConfig, loadConfig, renderConfigMenu} from "./config/config_helper";

function validateConfig() {
    // Validate numeric fields
    const netdevBudgetUsecs = parseInt(document.getElementById("netdevBudgetUsecs").value);
    if (isNaN(netdevBudgetUsecs) || netdevBudgetUsecs < 0) {
        alert("Netdev Budget (μs) must be a positive number");
        return false;
    }

    const netdevBudgetPackets = parseInt(document.getElementById("netdevBudgetPackets").value);
    if (isNaN(netdevBudgetPackets) || netdevBudgetPackets < 0) {
        alert("Netdev Budget (Packets) must be a positive number");
        return false;
    }

    const rxUsecs = parseInt(document.getElementById("rxUsecs").value);
    if (isNaN(rxUsecs) || rxUsecs < 0) {
        alert("RX Polling Frequency (μs) must be a positive number");
        return false;
    }

    const txUsecs = parseInt(document.getElementById("txUsecs").value);
    if (isNaN(txUsecs) || txUsecs < 0) {
        alert("TX Polling Frequency (μs) must be a positive number");
        return false;
    }

    return true;
}

function updateConfig() {
    // Update only the tuning section
    window.config.tuning = {
        stop_irq_balance: document.getElementById("stopIrqBalance").checked,
        netdev_budget_usecs: parseInt(document.getElementById("netdevBudgetUsecs").value),
        netdev_budget_packets: parseInt(document.getElementById("netdevBudgetPackets").value),
        rx_usecs: parseInt(document.getElementById("rxUsecs").value),
        tx_usecs: parseInt(document.getElementById("txUsecs").value),
        disable_rxvlan: document.getElementById("disableRxVlan").checked,
        disable_txvlan: document.getElementById("disableTxVlan").checked,
        disable_offload: document.getElementById("disableOffload").value
            .split(' ')
            .map(s => s.trim())
            .filter(s => s.length > 0)
    };
}

// Render the configuration menu
renderConfigMenu('tuning');

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    if (window.config && window.config.tuning) {
        const tunables = window.config.tuning;
        
        // Boolean fields
        document.getElementById("stopIrqBalance").checked = tunables.stop_irq_balance ?? false;
        document.getElementById("disableRxVlan").checked = tunables.disable_rxvlan ?? false;
        document.getElementById("disableTxVlan").checked = tunables.disable_txvlan ?? false;

        // Numeric fields
        document.getElementById("netdevBudgetUsecs").value = tunables.netdev_budget_usecs ?? 8000;
        document.getElementById("netdevBudgetPackets").value = tunables.netdev_budget_packets ?? 300;
        document.getElementById("rxUsecs").value = tunables.rx_usecs ?? 8;
        document.getElementById("txUsecs").value = tunables.tx_usecs ?? 8;

        // Array field (convert to space-separated string)
        document.getElementById("disableOffload").value = 
            (tunables.disable_offload ?? ["gso", "tso", "lro", "sg", "gro"]).join(' ');

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
        console.error("Tuning configuration not found in window.config");
    }
});
