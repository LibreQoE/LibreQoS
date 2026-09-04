import assert from "node:assert/strict";
import test from "node:test";
import {
    applyBridgeModePresentation,
    interfaceIsEligible,
    renderInterfaceOptions,
} from "./network_mode_interfaces.mjs";

class FakeElement {
    constructor(ownerDocument) {
        this.ownerDocument = ownerDocument;
        this.checked = false;
        this.disabled = false;
        this.textContent = "";
        this.value = "";
        this.children = [];
    }

    replaceChildren(...children) {
        this.children = children;
    }
}

function fakeDocument() {
    const elements = new Map();
    const root = {
        createElement: () => new FakeElement(root),
        getElementById: (id) => elements.get(id) ?? null,
    };
    for (const id of ["useXdpBridge", "applyButton", "bridgeMtu", "bridgeMtuHelp", "toInternet"]) {
        elements.set(id, new FakeElement(root));
    }
    return root;
}

test("bond masters are selectable only for XDP bridge mode", () => {
    const bond = {
        bridge_eligible: false,
        xdp_bridge_eligible: true,
        single_interface_eligible: false,
    };

    assert.equal(interfaceIsEligible(bond, "xdp"), true);
    assert.equal(interfaceIsEligible(bond, "bridge"), false);
    assert.equal(interfaceIsEligible(bond, "single"), false);
});

test("physical interfaces keep their existing mode eligibility", () => {
    const physical = {
        bridge_eligible: true,
        xdp_bridge_eligible: true,
        single_interface_eligible: true,
    };

    assert.equal(interfaceIsEligible(physical, "xdp"), true);
    assert.equal(interfaceIsEligible(physical, "bridge"), true);
    assert.equal(interfaceIsEligible(physical, "single"), true);
});

test("toggling XDP changes bond options, apply action, and MTU help", () => {
    const root = fakeDocument();
    const candidates = [
        {
            name: "enp1s0",
            bridge_eligible: true,
            xdp_bridge_eligible: true,
            single_interface_eligible: true,
        },
        {
            name: "bond-wan",
            bridge_eligible: false,
            xdp_bridge_eligible: true,
            single_interface_eligible: false,
        },
    ];
    const toggle = root.getElementById("useXdpBridge");
    const select = root.getElementById("toInternet");

    let presentation = applyBridgeModePresentation(root);
    renderInterfaceOptions(select, candidates, "bridge", "enp1s0");
    assert.deepEqual(select.children.map((option) => option.value), ["", "enp1s0"]);
    assert.equal(root.getElementById("applyButton").textContent, "Apply Network Changes");
    assert.equal(root.getElementById("bridgeMtu").disabled, false);
    assert.equal(root.getElementById("bridgeMtuHelp").textContent, "Applies to the bridge members and br0.");
    assert.match(presentation.draftSavedMessage, /Netplan together/);

    toggle.checked = true;
    presentation = applyBridgeModePresentation(root);
    renderInterfaceOptions(select, candidates, "xdp", "bond-wan");
    assert.deepEqual(select.children.map((option) => option.value), ["", "enp1s0", "bond-wan"]);
    assert.equal(root.getElementById("applyButton").textContent, "Save XDP Configuration");
    assert.equal(root.getElementById("bridgeMtu").disabled, true);
    assert.equal(
        root.getElementById("bridgeMtuHelp").textContent,
        "XDP mode keeps this value for later, but LibreQoS will not apply it.",
    );
    assert.match(presentation.draftSavedMessage, /Netplan will not be changed/);
});
