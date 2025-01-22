import {loadConfig} from "./config/config_helper";

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    if (window.config && window.config.tunables) {
        const tunables = window.config.tunables;
        
        // Boolean fields
        document.getElementById("stopIrqBalance").checked = tunables.stop_irq_balance ?? false;
        document.getElementById("disableRxVlan").checked = tunables.disable_rx_vlan_offload ?? false;
        document.getElementById("disableTxVlan").checked = tunables.disable_tx_vlan_offload ?? false;

        // Numeric fields
        if (tunables.netdev_budget_usecs) {
            document.getElementById("netdevBudgetUsecs").value = tunables.netdev_budget_usecs;
        }
        if (tunables.netdev_budget_packets) {
            document.getElementById("netdevBudgetPackets").value = tunables.netdev_budget_packets;
        }
        if (tunables.rx_usecs) {
            document.getElementById("rxUsecs").value = tunables.rx_usecs;
        }
        if (tunables.tx_usecs) {
            document.getElementById("txUsecs").value = tunables.tx_usecs;
        }

        // Array field (convert to space-separated string)
        if (tunables.disable_offload) {
            document.getElementById("disableOffload").value = tunables.disable_offload.join(' ');
        }
    } else {
        console.error("Tuning configuration not found in window.config");
    }
});
