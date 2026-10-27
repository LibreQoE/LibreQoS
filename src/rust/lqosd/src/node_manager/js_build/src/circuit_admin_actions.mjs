// E3 — admin-action affordances for the circuit page (docs/e3_derivation.md).
//
// Thin, testable pieces shared by the page wiring in circuit.js:
//   - role gating (the backend re-checks every write; this only hides
//     affordances from read-only users),
//   - the health gate (bridge-down disables the bar — no action storm),
//   - rate validation (the never-raw-zero rule: the UI never sends a
//     0 rate as an operator edit — 0/0 is the suspend encoding and the
//     CSV cannot re-parse it),
//   - the action lifecycle DOM (optimistic "applying…" → ✓ applied /
//     ✗ + typed reason, sourced from the E3b BridgePublishStatus
//     records — rejects are surfaced HONESTLY and never retried).

export function isAdminRole(role) {
    return String(role ?? "") === "Admin";
}

/// The suspend floor in Mbps — MUST mirror the crate's
/// lqos_overrides::SUSPEND_FLOOR_MBPS (the config suspend encoding:
/// v3 compile rounds it to the 0/0 interim, the broker floors to the
/// trickle). NOTE (driver ruling, 2026-09-10): there is NO UI suspend
/// affordance — the floor is recognized here only so the badge can
/// honestly mark a floor-valued override.
export const SUSPEND_FLOOR_MBPS = 0.01;

/// True when an override carries the full suspend floor (every
/// direction at the floor) — drives the SUSPENDED badge.
export function isSuspendedOverride(speeds) {
    if (!speeds || typeof speeds !== "object") {
        return false;
    }
    return [
        speeds.min_download_bandwidth,
        speeds.max_download_bandwidth,
        speeds.min_upload_bandwidth,
        speeds.max_upload_bandwidth,
    ].every((value) => Number(value) === SUSPEND_FLOOR_MBPS);
}

export function actionsEnabledForHealth(verdict) {
    // "down" = the Supervisor's down verdict; null/pending = unknown.
    // Only a bridge we know is reachable enables actions; degraded
    // and starting still accept publishes (the lifecycle surface
    // shows the pending state honestly if one stalls).
    return verdict === "healthy" || verdict === "degraded" || verdict === "starting";
}

export function validateRateInputs(inputs) {
    const labels = [
        ["download_min_mbps", "Download Min"],
        ["upload_min_mbps", "Upload Min"],
        ["download_max_mbps", "Download Max"],
        ["upload_max_mbps", "Upload Max"],
    ];
    const errors = [];
    for (const [key, label] of labels) {
        const value = Number(inputs?.[key]);
        if (!Number.isFinite(value)) {
            errors.push(`${label} must be a number`);
            continue;
        }
        if (value <= 0) {
            // The never-raw-zero rule: 0 is NOT a valid operator rate
            // (0/0 is the suspend encoding and the CSV cannot re-parse
            // it).
            errors.push(`${label} must be greater than 0`);
        }
    }
    if (errors.length === 0) {
        if (Number(inputs.download_min_mbps) > Number(inputs.download_max_mbps)) {
            errors.push("Download Min must not exceed Download Max");
        }
        if (Number(inputs.upload_min_mbps) > Number(inputs.upload_max_mbps)) {
            errors.push("Upload Min must not exceed Upload Max");
        }
    }
    return errors;
}

/// Locates the publish outcome for one generation in a
/// BridgePublishStatus payload (the broker's own records).
///
/// Wire shapes (serde external tagging, snake_case):
///   { state: "applied" }
///   { state: { pending:  { seq_pos, seq_len } } }
///   { state: { retrying: { attempt, max } } }
///   { state: { rejected: { reason } } }
export function outcomeForGeneration(records, generation) {
    if (!Array.isArray(records)) {
        return null;
    }
    const record = records.find((entry) => entry && entry.generation === generation);
    if (!record) {
        return null;
    }
    const state = record.state;
    if (typeof state === "string") {
        return state === "applied" ? { status: "applied" } : null;
    }
    if (!state || typeof state !== "object") {
        return null;
    }
    if (state.pending) {
        return {
            status: "pending",
            seqPos: state.pending.seq_pos,
            seqLen: state.pending.seq_len,
        };
    }
    if (state.retrying) {
        return {
            status: "pending",
            retrying: true,
            seqPos: state.retrying.attempt,
            seqLen: state.retrying.max,
        };
    }
    if (state.rejected) {
        return { status: "rejected", reason: state.rejected.reason ?? "REJECTED" };
    }
    return null;
}

/// Human text for a settled/pending outcome (typed reasons surface
/// verbatim — honesty over prettiness).
export function describeOutcome(outcome) {
    if (!outcome) {
        return "";
    }
    if (outcome.status === "noop") {
        return "Already in this state";
    }
    if (outcome.status === "pending") {
        if (outcome.seqPos && outcome.seqLen && outcome.seqLen > 1) {
            return outcome.retrying
                ? `Bridge retrying (attempt ${outcome.seqPos}/${outcome.seqLen})…`
                : `Applying to bridge (${outcome.seqPos}/${outcome.seqLen})…`;
        }
        return "Applying to bridge…";
    }
    if (outcome.status === "applied") {
        return "✓ Applied by bridge";
    }
    return `✗ ${outcome.reason}`;
}

const LIFECYCLE_CLASS = "lqos-action-lifecycle";

function clearLifecycle(root) {
    const existing = root.querySelector(`.${LIFECYCLE_CLASS}`);
    if (existing) {
        existing.remove();
    }
}

function lifecycleNode(text, tone) {
    const span = document.createElement("span");
    span.classList.add(LIFECYCLE_CLASS, "ms-2", "small", tone);
    span.textContent = text;
    return span;
}

/// Optimistic pending state: disable the bar's controls, show the
/// spinner + context line.
export function setActionPending(root, contextText) {
    clearLifecycle(root);
    root.querySelectorAll("button").forEach((button) => {
        button.disabled = true;
    });
    root.appendChild(lifecycleNode(contextText || "Applying to bridge…", "text-secondary"));
}

/// Settled state: ✓ applied (success tone) or ✗ + typed reason
/// (danger tone) re-enable the bar; a still-pending outcome keeps it
/// disabled for the next poll. The caller decides whether to reload
/// page data.
export function setActionSettled(root, outcome) {
    clearLifecycle(root);
    const enable = () => {
        root.querySelectorAll("button").forEach((button) => {
            button.disabled = false;
        });
    };
    if (!outcome || outcome.status === "applied") {
        enable();
        root.appendChild(lifecycleNode(describeOutcome(outcome) || "✓ Applied by bridge", "text-success"));
        return "applied";
    }
    if (outcome.status === "pending") {
        root.querySelectorAll("button").forEach((button) => {
            button.disabled = true;
        });
        root.appendChild(lifecycleNode(describeOutcome(outcome), "text-secondary"));
        return "pending";
    }
    if (outcome.status === "noop") {
        // A no-op dispatch (the stored override already equaled the
        // request): no bridge generation will ever appear for it, so
        // settle immediately — waiting would false-NO_ACK.
        enable();
        root.appendChild(lifecycleNode(describeOutcome(outcome), "text-secondary"));
        return "noop";
    }
    enable();
    root.appendChild(lifecycleNode(describeOutcome(outcome), "text-danger"));
    return "rejected";
}
