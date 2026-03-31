// Shared static placeholder teasers for Insight promotions.
// Used by both the standalone Insight trial page and the
// in-app Insight nag modal so the benefits only need to be
// maintained in one place.
//
// Draft teaser copy and image mapping for in-place review.

export const PLACEHOLDER_TEASERS = [
    {
        id: "executive_summary",
        title: "Executive Summary",
        description:
            "See network health, capacity, and the biggest problem areas at a glance so you know where to look next.",
        image: "01_executive_summary.png",
        order: 1,
    },
    {
        id: "site_overview",
        title: "Site Overview",
        description:
            "Open a site and compare performance, utilization, and quality over time without jumping between tools.",
        image: "02_site_overview.png",
        order: 2,
    },
    {
        id: "site_rankings",
        title: "Site Rankings",
        description:
            "Rank sites by quality over the selected period so the worst performers surface immediately.",
        image: "03_site_rankings.png",
        order: 3,
    },
    {
        id: "subscriber_view",
        title: "Subscriber View",
        description:
            "Inspect subscriber behavior, usage, and performance from one place when a specific customer needs attention.",
        image: "04_subscriber_view.png",
        order: 4,
    },
    {
        id: "issue_triage",
        title: "Issue Triage",
        description:
            "Turn symptoms into a focused troubleshooting summary with likely causes and recommended next steps.",
        image: "05_issue_triage.png",
        order: 5,
    },
    {
        id: "api_only",
        title: "API-Only Option",
        description:
            "Choose the lower-cost API-only option when you only need provisioning and automation integrations. Hosted dashboards and remote Insight history are not included.",
        image: "06_api_only.png",
        order: 6,
    },
];
