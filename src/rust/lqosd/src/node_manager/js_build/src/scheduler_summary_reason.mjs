// Pure render mapping for the System Health "Degraded" reason.
//
// The backend serves `summary_reason` (produced TOGETHER with
// `summary_severity`; see local_api/scheduler.rs `compute_summary_severity`).
// The frontend only RENDERS it — it never re-derives the axis (the
// accidental-complexity rule). The import axis and the boot-world axis are
// independent, so the combined reasons render BOTH lines with the boot world
// FIRST (the more actionable), never one silently.
//
// The served reason is a code: the switch below is the code-to-message seam, so
// a new server reason degrades to the generic degraded line instead of leaking
// the code onto the screen.

import {t} from "./i18n/index.js";

/**
 * Map the served `summary_reason` to a degraded title/subtitle.
 *
 * @param {unknown} summaryReason The `summary_reason` value.
 * @param {unknown} integrationLabel The integration name (for the import line).
 * @param {unknown} importSubtitle The rendered import-failure detail line.
 * @returns {{title: string, subtitle: string}}
 */
export function schedulerDegradedReason(summaryReason, integrationLabel, importSubtitle) {
    const label = String(integrationLabel || "").trim();
    const name = label || t("scheduler.integration.fallback");
    const importLine = String(importSubtitle || "").trim()
        || t("scheduler.integration.import_failing", {name});
    switch (String(summaryReason ?? "").trim()) {
    case "import_failing":
        return {
            title: t("scheduler.degraded.title.import_failing", {integration: name}),
            subtitle: importLine,
        };
    case "boot_world_bootstrap_trunk":
        return {
            title: t("scheduler.degraded.title.bootstrap_trunk"),
            subtitle: t("scheduler.degraded.detail.bootstrap_trunk"),
        };
    case "boot_world_pass_through":
        return {
            title: t("scheduler.degraded.title.pass_through"),
            subtitle: t("scheduler.degraded.detail.pass_through"),
        };
    case "import_failing_and_boot_world_bootstrap_trunk":
        return {
            title: t("scheduler.degraded.title.bootstrap_trunk"),
            subtitle: t("scheduler.degraded.detail.combined", {
                detail: t("scheduler.degraded.detail.bootstrap_trunk"),
                import: importLine,
            }),
        };
    case "import_failing_and_boot_world_pass_through":
        return {
            title: t("scheduler.degraded.title.pass_through"),
            subtitle: t("scheduler.degraded.detail.combined", {
                detail: t("scheduler.degraded.detail.pass_through"),
                import: importLine,
            }),
        };
    default:
        // No reason served (legacy/unknown): NEVER claim the import axis —
        // that misattribution is the defect this mapping fixes.
        return {
            title: t("scheduler.degraded.title.unknown"),
            subtitle: t("scheduler.degraded.detail.unknown"),
        };
    }
}
