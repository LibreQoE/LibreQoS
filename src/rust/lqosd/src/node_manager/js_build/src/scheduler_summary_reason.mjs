// Pure render mapping for the System Health "Degraded" reason.
//
// The backend serves `summary_reason` (produced TOGETHER with
// `summary_severity`; see local_api/scheduler.rs `compute_summary_severity`).
// The frontend only RENDERS it — it never re-derives the axis (the
// accidental-complexity rule). The import axis and the boot-world axis are
// independent, so the combined reasons render BOTH lines with the boot world
// FIRST (the more actionable), never one silently.

export const BOOTSTRAP_TRUNK_DETAIL =
    "The bridge booted without a configured or last-known-good world; libreqos-bridge serves the bootstrap trunk until restart.";
export const PASS_THROUGH_DETAIL =
    "The bridge booted without a verified world and is passing traffic through unshaped until restart.";

/**
 * Map the served `summary_reason` to a degraded title/subtitle.
 *
 * @param {unknown} summaryReason The `summary_reason` value.
 * @param {unknown} integrationLabel The integration name (for the import line).
 * @param {unknown} importSubtitle The rendered import-failure detail line.
 * @returns {{title: string, subtitle: string}}
 */
export function schedulerDegradedReason(summaryReason, integrationLabel, importSubtitle) {
    const label = String(integrationLabel || "").trim() || "Integration";
    const importLine = String(importSubtitle || "").trim() || `${label} import failing`;
    switch (String(summaryReason ?? "").trim()) {
        case "import_failing":
            return {
                title: `Shaping active on last-known-good topology · ${label} import failing`,
                subtitle: importLine,
            };
        case "boot_world_bootstrap_trunk":
            return {
                title: "Shaping active on a bootstrap trunk · no verified world at boot",
                subtitle: BOOTSTRAP_TRUNK_DETAIL,
            };
        case "boot_world_pass_through":
            return {
                title: "Shaping active (pass-through) · no shaping world verified",
                subtitle: PASS_THROUGH_DETAIL,
            };
        case "import_failing_and_boot_world_bootstrap_trunk":
            return {
                title: "Shaping active on a bootstrap trunk · no verified world at boot",
                subtitle: `${BOOTSTRAP_TRUNK_DETAIL} · ${importLine}`,
            };
        case "import_failing_and_boot_world_pass_through":
            return {
                title: "Shaping active (pass-through) · no shaping world verified",
                subtitle: `${PASS_THROUGH_DETAIL} · ${importLine}`,
            };
        default:
            // No reason served (legacy/unknown): NEVER claim the import axis —
            // that misattribution is the defect this mapping fixes.
            return {
                title: "Shaping active · degraded",
                subtitle: "System Health is degraded; open System Health for details.",
            };
    }
}
