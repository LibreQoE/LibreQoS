export function interfaceEligibilityField(modeKey) {
    if (modeKey === "single") return "single_interface_eligible";
    if (modeKey === "xdp") return "xdp_bridge_eligible";
    return "bridge_eligible";
}

export function interfaceIsEligible(candidate, modeKey) {
    return Boolean(candidate?.[interfaceEligibilityField(modeKey)]);
}

export function bridgeModePresentation(useXdp, editingLocked = false) {
    const presentation = {
        modeKey: useXdp ? "xdp" : "bridge",
        applyButtonText: useXdp ? "Save XDP Configuration" : "Apply Network Changes",
        draftSavedMessage: useXdp
            ? "Network mode draft saved for this browser tab. Use Save XDP Configuration to update lqos.conf; Netplan will not be changed."
            : "Network mode draft saved for this browser tab. Use Apply Network Changes to commit both lqos.conf and Netplan together.",
    };
    if (editingLocked) {
        return {
            ...presentation,
            bridgeMtuDisabled: true,
            bridgeMtuHelp: "MTU cannot be changed while a network-mode operation is pending.",
        };
    }
    return {
        ...presentation,
        bridgeMtuDisabled: useXdp,
        bridgeMtuHelp: useXdp
            ? "XDP mode keeps this value for later, but LibreQoS will not apply it."
            : "Applies to the bridge members and br0.",
    };
}

export function applyBridgeModePresentation(root, editingLocked = false) {
    const useXdp = Boolean(root.getElementById("useXdpBridge")?.checked);
    const presentation = bridgeModePresentation(useXdp, editingLocked);
    const applyButton = root.getElementById("applyButton");
    const bridgeMtu = root.getElementById("bridgeMtu");
    const bridgeMtuHelp = root.getElementById("bridgeMtuHelp");
    if (applyButton) applyButton.textContent = presentation.applyButtonText;
    if (bridgeMtu) bridgeMtu.disabled = presentation.bridgeMtuDisabled;
    if (bridgeMtuHelp) bridgeMtuHelp.textContent = presentation.bridgeMtuHelp;
    return presentation;
}

function optionDefinitions(candidates, modeKey, selectedValue, excludedValue) {
    const eligibilityField = interfaceEligibilityField(modeKey);
    const options = [{ value: "", label: "Select an eligible interface" }];
    const seen = new Set();

    candidates.forEach((candidate) => {
        const selected = candidate.name === selectedValue;
        const eligible = interfaceIsEligible(candidate, modeKey);
        if (!eligible && !selected) return;
        if (excludedValue && candidate.name === excludedValue && !selected) return;
        let label = candidate.name;
        if (!candidate[eligibilityField]) {
            label = `${candidate.name} (current selection; unavailable)`;
        }
        options.push({ value: candidate.name, label });
        seen.add(candidate.name);
    });

    if (selectedValue && !seen.has(selectedValue)) {
        options.push({
            value: selectedValue,
            label: `${selectedValue} (current selection; unavailable)`,
        });
    }
    return options;
}

export function renderInterfaceOptions(
    selectElement,
    candidates,
    modeKey,
    selectedValue,
    excludedValue = null,
) {
    if (!selectElement) return;
    const ownerDocument = selectElement.ownerDocument ?? globalThis.document;
    const options = optionDefinitions(candidates, modeKey, selectedValue, excludedValue)
        .map(({value, label}) => {
            const option = ownerDocument.createElement("option");
            option.value = value;
            option.textContent = label;
            return option;
        });
    selectElement.replaceChildren(...options);
    selectElement.value = selectedValue || "";
}
