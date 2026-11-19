// Shared static placeholder teasers for Insight promotions.
// Used by both the standalone Insight trial page and the
// in-app Insight nag modal so the benefits only need to be
// maintained in one place.
//
// Draft teaser copy and image mapping for in-place review. The title and body
// are catalog getters because the same objects feed two surfaces; `id`, `image`
// and `order` stay data. A teaser the Insight service sends at runtime is not
// translated: that copy belongs to the service, not to this bundle.

import {t} from "./i18n/index.js";

export const PLACEHOLDER_TEASERS = [
    {
        id: "executive_summary",
        get title() { return t("dashboard.tab.executive_summary"); },
        get description() { return t("trial.teaser.executive.body"); },
        image: "01_executive_summary.png",
        order: 1,
    },
    {
        id: "site_overview",
        get title() { return t("trial.teaser.site_overview.title"); },
        get description() { return t("trial.teaser.site_overview.body"); },
        image: "02_site_overview.png",
        order: 2,
    },
    {
        id: "site_rankings",
        get title() { return t("trial.teaser.site_rankings.title"); },
        get description() { return t("trial.teaser.site_rankings.body"); },
        image: "03_site_rankings.png",
        order: 3,
    },
    {
        id: "subscriber_view",
        get title() { return t("trial.teaser.subscriber_view.title"); },
        get description() { return t("trial.teaser.subscriber_view.body"); },
        image: "04_subscriber_view.png",
        order: 4,
    },
    {
        id: "issue_triage",
        get title() { return t("trial.teaser.issue_triage.title"); },
        get description() { return t("trial.teaser.issue_triage.body"); },
        image: "05_issue_triage.png",
        order: 5,
    },
    {
        id: "api_only",
        get title() { return t("trial.teaser.api_only.title"); },
        get description() { return t("trial.teaser.api_only.body"); },
        image: "06_api_only.png",
        order: 6,
    },
];
