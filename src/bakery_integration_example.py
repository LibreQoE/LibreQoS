#!/usr/bin/env python3
"""
Example of how to integrate Bakery calls into LibreQoS.py

This shows the pattern for replacing TC shell commands with Bakery API calls.
DO NOT integrate this directly - this is just an example!
"""

import liblqos_python
import hashlib

def example_clear_prior_settings():
    """Example of replacing clearPriorSettings() shell commands with Bakery API"""
    # Original: 
    # if hasMq(interfaceA):
    #     delRootQdisc(interfaceA) 
    # if hasMq(interfaceB):
    #     delRootQdisc(interfaceB)
    
    # Bakery replacement:
    success = liblqos_python.bakery_clear_prior_settings()
    if not success:
        print("WARNING: Failed to clear prior settings via bakery")
    return success

def example_mq_setup():
    """Example of replacing createMq() with Bakery API"""
    # Original createMq() builds complex TC commands
    # Bakery replacement:
    success = liblqos_python.bakery_mq_setup()
    if not success:
        print("WARNING: Failed to setup MQ via bakery")
    return success

def example_add_structural_node(interface, parent, classid, min_mbps, max_mbps, node_name, r2q):
    """Example of adding a structural HTB class (site/AP from network.json)"""
    # Calculate site hash from node name
    site_hash = int(hashlib.sha256(node_name.encode()).hexdigest()[:16], 16)
    if site_hash > 2**63 - 1:  # Convert to signed i64 range
        site_hash = site_hash - 2**64
    
    # Call bakery to add structural HTB class (no qdisc)
    success = liblqos_python.bakery_add_structural_htb_class(
        interface=interface,
        parent=parent,
        classid=classid,
        rate_mbps=float(min_mbps),
        ceil_mbps=float(max_mbps),
        site_hash=site_hash,
        r2q=r2q
    )
    
    if not success:
        print(f"WARNING: Failed to add structural HTB class for {node_name}")
    return success

def example_add_circuit(interface, parent, classid, min_mbps, max_mbps, circuit_id, comment, r2q):
    """Example of adding a circuit HTB class + qdisc"""
    # Calculate circuit hash from circuit ID
    circuit_hash = int(hashlib.sha256(circuit_id.encode()).hexdigest()[:16], 16)
    if circuit_hash > 2**63 - 1:  # Convert to signed i64 range
        circuit_hash = circuit_hash - 2**64
    
    # First add the HTB class
    success = liblqos_python.bakery_add_circuit_htb_class(
        interface=interface,
        parent=parent,
        classid=classid,
        rate_mbps=float(min_mbps),
        ceil_mbps=float(max_mbps),
        circuit_hash=circuit_hash,
        comment=comment,
        r2q=r2q
    )
    
    if not success:
        print(f"WARNING: Failed to add circuit HTB class for {circuit_id}")
        return False
    
    # Then add the qdisc (CAKE or fq_codel)
    # Parse classid to get major:minor
    major, minor = classid.split(':')
    
    # Example SQM params for CAKE
    sqm_params = [
        'cake', 'bandwidth', f'{max_mbps}mbit',
        'rtt', '100ms',
        'besteffort', 'triple-isolate', 'nonat',
        'wash', 'ack-filter'
    ]
    
    success = liblqos_python.bakery_add_circuit_qdisc(
        interface=interface,
        parent_major=int(major),
        parent_minor=int(minor),
        circuit_hash=circuit_hash,
        sqm_params=sqm_params
    )
    
    if not success:
        print(f"WARNING: Failed to add circuit qdisc for {circuit_id}")
    
    return success

def example_bulk_tc_commands():
    """Example of using bulk TC command execution"""
    # Collect all TC commands as strings (without /sbin/tc prefix)
    commands = [
        "qdisc del dev eth0 root",
        "qdisc add dev eth0 root handle 1: mq",
        "qdisc add dev eth0 parent 1:1 handle 2: htb default 2",
        # ... more commands
    ]
    
    # Execute all at once via bakery
    success = liblqos_python.bakery_execute_tc_commands(
        commands=commands,
        force_mode=False  # Set to True to use tc -f flag
    )
    
    if not success:
        print("WARNING: Failed to execute TC commands in bulk")
    return success

# Integration points in LibreQoS.py:
# 
# 1. Line 134 - clearPriorSettings():
#    Replace with: liblqos_python.bakery_clear_prior_settings()
#
# 2. Line 885 - createMq():
#    Replace with: liblqos_python.bakery_mq_setup()
#
# 3. Line 966 - Add structural HTB class:
#    Replace with: liblqos_python.bakery_add_structural_htb_class(...)
#
# 4. Line 995 - Add circuit HTB class + qdisc:
#    Replace with: liblqos_python.bakery_add_circuit_htb_class(...)
#    followed by: liblqos_python.bakery_add_circuit_qdisc(...)
#
# 5. Line 1099 - Execute all commands:
#    Replace with: liblqos_python.bakery_execute_tc_commands(...)

if __name__ == "__main__":
    print("This is just an example - do not run directly!")
    print("The bakery bindings available are:")
    print("  - bakery_clear_prior_settings()")
    print("  - bakery_mq_setup()")
    print("  - bakery_add_structural_htb_class(...)")
    print("  - bakery_add_circuit_htb_class(...)")
    print("  - bakery_add_circuit_qdisc(...)")
    print("  - bakery_execute_tc_commands(...)")