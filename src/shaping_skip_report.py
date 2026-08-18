def _string_value(value):
    if value is None:
        return ""
    return str(value)


def _circuit_parent_values(circuit):
    logical_parent = _string_value(circuit.get("logicalParentNode", circuit.get("ParentNode")))
    effective_parent = _string_value(circuit.get("ParentNode"))
    return logical_parent, effective_parent


def device_shaping_key(circuit, device):
    return (
        _string_value(circuit.get("circuitID", "")),
        _string_value(device.get("deviceID", "")),
        _string_value(device.get("deviceName", "")),
    )


def collect_parent_node_names(network):
    names = set()

    def walk(nodes):
        if not isinstance(nodes, dict):
            return
        for name, details in nodes.items():
            if not isinstance(name, str) or not isinstance(details, dict):
                continue
            names.add(name)
            children = details.get("children")
            if isinstance(children, dict):
                walk(children)

    walk(network)
    return names


def _classify_unshaped_circuit(circuit, valid_parent_nodes, flat_network):
    logical_parent_str, effective_parent_str = _circuit_parent_values(circuit)

    if flat_network:
        return (
            "unattached_flat_network",
            "Circuit is being shaped under generated parent queues in flat-network mode.",
        )

    if logical_parent_str in ("", "none"):
        return (
            "missing_parent",
            "No ParentNode was configured for this circuit, so it could not be attached to the shaping tree.",
        )

    if effective_parent_str not in valid_parent_nodes:
        return (
            "unknown_parent",
            f"ParentNode '{logical_parent_str}' was not found in the shaping tree.",
        )

    return (
        "unattached_circuit",
        f"Circuit was not attached during shaping even though parent node '{effective_parent_str}' exists.",
    )


def format_unshaped_device_line(entry):
    return (
        f"DeviceID: {entry['deviceID']}\t DeviceName: {entry['deviceName']}"
        f"\t CircuitID: {entry['circuitID']}\t CircuitName: {entry['circuitName']}"
        f"\t LogicalParent: {entry['logicalParentNode']}"
        f"\t EffectiveParent: {entry['effectiveParentNode']}"
        f"\t Reason: {entry['reasonText']}"
    )


def build_unshaped_device_report(subscriber_circuits, shaped_device_keys, valid_parent_nodes, flat_network):
    skipped_devices = []
    shaped_key_set = set(shaped_device_keys)

    for circuit in subscriber_circuits:
        logical_parent, effective_parent = _circuit_parent_values(circuit)
        reason_code, reason_text = _classify_unshaped_circuit(
            circuit,
            valid_parent_nodes,
            flat_network,
        )
        for device in circuit.get("devices", []):
            if device_shaping_key(circuit, device) in shaped_key_set:
                continue
            skipped_devices.append(
                {
                    "deviceID": _string_value(device.get("deviceID", "")),
                    "deviceName": _string_value(device.get("deviceName", "")),
                    "circuitID": _string_value(circuit.get("circuitID", "")),
                    "circuitName": _string_value(circuit.get("circuitName", "")),
                    "logicalParentNode": logical_parent,
                    "effectiveParentNode": effective_parent,
                    "reasonCode": reason_code,
                    "reasonText": reason_text,
                }
            )

    skipped_devices.sort(
        key=lambda entry: (
            entry["reasonCode"],
            entry["logicalParentNode"],
            entry["effectiveParentNode"],
            entry["circuitID"],
            entry["deviceID"],
            entry["deviceName"],
        )
    )
    return skipped_devices
