// Contract for the migrated operator pages: the S2a dashboard shell (tab bar,
// dashlet menu, editor controls) and four small pages (CPU & Workers, unknown
// IPs, bridge, shaped devices), then the S2b core pages (tree, circuit), and the
// S3 configuration screens, which grow a page at a time (General, Bridge &
// Interface Mode, StormGuard, License & Services, devices, tuning, queues,
// RADIUS, UISP, topology source, dynamic circuits).
//
// `extract.mjs` proves that a key has a reader and that both catalogs define
// it. What it cannot see is whether the *rendered* surface still names the
// thing it is supposed to name: a page that loses its `t()` call, or a catalog
// that ships the English value under `es`, passes the gate. This file pins the
// load-bearing key of every page to the file that renders it, so a deletion
// fails here instead of silently dropping a label.
//
// Three more rules of the campaign are structural, so they are checked from the
// source text rather than by calling the bundles:
//   * no page builds a clock or a relative time by hand — the bundles either
//     call the shared Intl formatters or leave the value to a shared helper;
//   * the dashlet menu carries catalog keys, not English labels, and its key
//     count is the ratchet (a new widget adds a key here, to both catalogs,
//     and to the menu);
//   * the last-seen column reads the shared relative-time helper instead of
//     composing an English suffix of its own.
//
// `lts_trial.js` (section S5a) belongs to the same shell: its four pages carry
// `data-i18n` markup and the bundles fill the rest.
//
// The bundles are page entrypoints that run DOM initialization at import, and
// `dashlet_index.js` imports fifty dashlet modules, so nothing here imports
// them: the assertions read the files as text.

import assert from "node:assert/strict";
import {readFile, readdir} from "node:fs/promises";
import test from "node:test";
import {fileURLToPath} from "node:url";

const PAGES = [
    {
        name: "dashboard",
        markup: [],
        bundles: [
            new URL("../index.js", import.meta.url),
            new URL("../lq_js_common/dashboard/dashboard.js", import.meta.url),
            new URL("../dashlets/dashlet_index.js", import.meta.url),
        ],
        pinned: [
            "dashboard.tab.executive_summary",
            "dashboard.edit",
            "dashboard.refresh_rate",
            "dashboard.add_widget",
            "dashlet.name.throughput_bits_per_second",
            "dashlet.category.throughput",
        ],
    },
    {
        name: "cpu_workers",
        markup: [new URL("../../../static2/cpu_workers.html", import.meta.url)],
        bundles: [new URL("../cpu_workers.js", import.meta.url)],
        pinned: [
            "common.nav.cpu_workers",
            "cpu_workers.fleet_badge",
            "cpu_workers.select_cpu",
            "cpu_workers.state_aria",
            "cpu_workers.fleet_max",
        ],
    },
    {
        name: "unknown_ips",
        markup: [new URL("../../../static2/unknown_ips.html", import.meta.url)],
        bundles: [new URL("../unknown-ips.js", import.meta.url)],
        pinned: [
            "unknown_ips.title",
            "unknown_ips.download_csv",
            "unknown_ips.ip_address",
            "common.table.last_seen",
        ],
    },
    {
        name: "bridge",
        markup: [new URL("../../../static2/bridge.html", import.meta.url)],
        bundles: [new URL("../bridge.js", import.meta.url)],
        pinned: [
            "bridge.page.subtitle",
            "bridge.page.reload_note",
            "common.action.reload",
            "bridge.page.holding",
            "bridge.page.label.healthy",
            "bridge.page.outcome.rejected",
        ],
    },
    {
        name: "shaped_devices",
        markup: [new URL("../../../static2/shaped_devices.html", import.meta.url)],
        bundles: [new URL("../shaped-devices.js", import.meta.url)],
        pinned: [
            "common.table.circuits",
            "shaped_devices.tab_dynamic",
            "shaped_devices.qoo.title",
            "shaped_devices.showing_first",
            "shaped_devices.device_count",
            "shaped_devices.unknown_circuit",
        ],
    },
    {
        name: "tree",
        markup: [new URL("../../../static2/tree.html", import.meta.url)],
        bundles: [
            new URL("../tree.js", import.meta.url),
            // The attached-circuits table and the top-N tables both come from
            // this shared module, so its keys are rendered by this page too.
            new URL("../helpers/builders.js", import.meta.url),
        ],
        pinned: [
            "tree.title",
            "tree.table.effective_limit",
            "tree.override.pending_tooltip",
            "tree.stormguard.bounds",
            "tree.stormguard.passive_flows",
            "common.table.device",
        ],
    },
    {
        name: "topology_manager",
        markup: [new URL("../../../static2/topology_manager.html", import.meta.url)],
        bundles: [
            new URL("../topology_manager.js", import.meta.url),
            // The node-kind vocabulary lives in its own module so a test can
            // render it; the page shows its labels.
            new URL("../topology_labels.mjs", import.meta.url),
        ],
        pinned: [
            "topology_manager.title",
            "topology_manager.hierarchy.canonical",
            "topology_manager.legend.proposed_target",
            "topology_manager.mode.move_mode",
            "topology_manager.manual.explicit_count",
            "topology_manager.details.parent",
            "topology.node_kind.site",
        ],
    },
    {
        name: "topology_probes",
        markup: [new URL("../../../static2/topology_probes.html", import.meta.url)],
        bundles: [new URL("../topology_probes.js", import.meta.url)],
        pinned: [
            "topology_probes.title",
            "topology_probes.filter.note",
            "topology_probes.summary",
            "topology_probes.health.misses",
            "topology_probes.open_in_topology",
            "topology.status.probe_unavailable",
        ],
    },
    {
        name: "circuit",
        markup: [new URL("../../../static2/circuit.html", import.meta.url)],
        bundles: [
            new URL("../circuit.js", import.meta.url),
            new URL("../helpers/builders.js", import.meta.url),
        ],
        pinned: [
            "circuit.tab.queue_stats",
            "circuit.rates.save",
            "circuit.traffic.qoo_columns",
            "circuit.badge.suspended",
            "circuit.queue_tree.upstream_count",
            "common.table.plan_mbps",
        ],
    },
    {
        // Section S3 opens with the two heaviest configuration screens. Their
        // prose splits across the markup (labels, help, buttons) and the bundle
        // (validation, confirmations, the paragraphs that wrap a literal file
        // name in <code> and so render through tHtml rather than data-i18n), so
        // each page pins both halves: a load-bearing label and a help paragraph
        // from the markup, plus the bundle sentence a dropped t() would silence.
        name: "config_general",
        markup: [new URL("../../../static2/config_general.html", import.meta.url)],
        bundles: [new URL("../config_general.js", import.meta.url)],
        pinned: [
            "config.general.title",
            "config.general.save",
            "config.general.runtime.efficiency_help",
            "config.general.qoo.defined_in",
            "config.general.cobrand.pending",
            "config.general.qoo.table.id",
        ],
    },
    {
        name: "config_interface",
        markup: [new URL("../../../static2/config_interface.html", import.meta.url)],
        bundles: [new URL("../config_interface.js", import.meta.url)],
        pinned: [
            "config.interface.title",
            "config.interface.bridge.mtu_help",
            "config.interface.actions.apply",
            "config.interface.confirm.intro",
            "config.interface.review.tabs_aria",
            "config.interface.select.interface",
        ],
    },
    {
        // S3b: the StormGuard strategy page splits its prose the same way — labels
        // and help in the markup, the paragraphs that wrap strategy tokens or a
        // value in <code> rendered by the bundle, plus every validation alert.
        name: "config_stormguard",
        markup: [new URL("../../../static2/config_stormguard.html", import.meta.url)],
        bundles: [new URL("../config_stormguard.js", import.meta.url)],
        pinned: [
            "config.stormguard.advanced_intro",
            "config.stormguard.strategy.title",
            "config.stormguard.delay.min_throughput_help",
            "config.stormguard.strategy.help",
            "config.stormguard.validation.must_be",
        ],
    },
    {
        // S3b: the license screen is half server-driven, so the pin covers the
        // static prose (title, hint), the two <code> paragraphs the bundle writes,
        // and the count and confirm messages that only the bundle renders.
        name: "config_lts",
        markup: [new URL("../../../static2/config_lts.html", import.meta.url)],
        bundles: [new URL("../config_lts.js", import.meta.url)],
        pinned: [
            "config.lts.title",
            "config.lts.status.retry_hint",
            "config.lts.api_keys.subtitle",
            "config.lts.api_keys.count",
            "config.lts.api_keys.revoke_confirm",
        ],
    },
    {
        // S3c: the shaped-device editor is the first config screen whose table
        // is built entirely in the bundle, so the pin covers both halves: the
        // modal labels from the markup, and the summary line, the merge-field
        // attribute label and the validation sentence only the bundle writes.
        name: "config_devices",
        markup: [new URL("../../../static2/config_devices.html", import.meta.url)],
        bundles: [new URL("../config_devices.js", import.meta.url)],
        pinned: [
            "config.devices.title",
            "config.devices.sqm_note",
            "config.devices.save",
            "config.devices.showing_all",
            "config.devices.delete_action",
            "config.devices.validation.sqm",
        ],
    },
    {
        // S3c: the tuning screen's `<code>` prose is empty in the page and
        // rendered by the bundle, so the pin names a paragraph that only exists
        // as a tHtml() call beside the labels the markup still carries.
        name: "config_tuning",
        markup: [new URL("../../../static2/config_tuning.html", import.meta.url)],
        bundles: [new URL("../config_tuning.js", import.meta.url)],
        pinned: [
            "config.tuning.title",
            "config.tuning.netdev.warning",
            "config.tuning.save",
            "config.tuning.dpdk.workers_help",
            "config.tuning.dpdk.worker_capacity_help",
            "config.tuning.validation.rx_polling",
        ],
    },
    {
        name: "config_queues",
        markup: [new URL("../../../static2/config_queues.html", import.meta.url)],
        bundles: [new URL("../config_queues.js", import.meta.url)],
        pinned: [
            "config.queues.title",
            "config.queues.mode_observe",
            "config.queues.default_sqm_help",
            "config.queues.flat.hint",
            "config.queues.readonly_note",
            "config.queues.validation.override_queues",
        ],
    },
    {
        // S3c: the RADIUS screen is half table-driven — the client rows are
        // built in the bundle — so the pin covers the static section labels and
        // the numbered-client validation and aria labels the bundle composes.
        name: "config_radius_accounting",
        markup: [new URL("../../../static2/config_radius_accounting.html", import.meta.url)],
        bundles: [new URL("../config_radius_accounting.js", import.meta.url)],
        pinned: [
            "config.radius_accounting.title",
            "config.radius_accounting.stale_grace",
            "config.radius_accounting.clients.add",
            "config.radius_accounting.clients.name_aria",
            "config.radius_accounting.validation.client_numbered",
            "config.radius_accounting.saved",
        ],
    },
    {
        // S3d: the UISP screen keeps the product name literal but translates
        // everything around it, so the pin names the page title, one <code>
        // paragraph the bundle writes, the shared Clear action and the two
        // messages only the validation path renders.
        name: "config_uisp",
        markup: [new URL("../../../static2/config_uisp.html", import.meta.url)],
        bundles: [new URL("../config_uisp.js", import.meta.url)],
        pinned: [
            "config.uisp.title",
            "config.uisp.compile_mode_help",
            "common.action.clear",
            "config.uisp.site_optional",
            "config.uisp.validation.exception_cpe",
        ],
    },
    {
        // S3d: the topology-source screen is the first page whose list of
        // sources is shared vocabulary, so the pin covers the page chrome, one
        // compile-mode option the bundle re-renders, the composed status line
        // and the mode-mismatch alert.
        name: "config_integration",
        markup: [new URL("../../../static2/config_integration.html", import.meta.url)],
        bundles: [new URL("../config_integration.js", import.meta.url)],
        pinned: [
            "config.integration.topology_source.title",
            "config.integration.mode.full",
            "config.integration.circuit_name_as_address_help",
            "config.integration.status.active_source",
            "config.integration.validation.mode_unsupported",
        ],
    },
    {
        // S3d: the dynamic-circuits table is built in the bundle and its rows
        // are unnamed by default, so the pin covers the page title, the table
        // header the markup carries, the footer note, the numbered-rule label
        // and the per-direction aria label the rate pair composes.
        name: "config_dynamic_circuits",
        markup: [new URL("../../../static2/config_dynamic_circuits.html", import.meta.url)],
        bundles: [new URL("../config_dynamic_circuits.js", import.meta.url)],
        pinned: [
            "config.dynamic_circuits.title",
            "config.dynamic_circuits.ranges.attach_note",
            "config.dynamic_circuits.ranges.down_aria",
            "config.dynamic_circuits.validation.rule_numbered",
            "common.table.actions",
        ],
    },
    {
        // Sonar: the panel header, the recurring-rate table it builds from the
        // catalog (header cells plus the row alerts) and the save alert.
        name: "config_sonar",
        markup: [new URL("../../../static2/config_sonar.html", import.meta.url)],
        bundles: [new URL("../config_sonar.js", import.meta.url)],
        pinned: [
            "config.sonar.title",
            "config.sonar.rates.help",
            "config.sonar.rates.add",
            "config.sonar.validation.download_rate",
            "config.sonar.saved",
        ],
    },
    {
        // SSL: the two `<code>` paragraphs and the hostname rule are rendered by
        // the bundle, so a lost tHtml call empties them without failing a text
        // comparison; the status badge and its yes/no values are pinned too.
        name: "config_ssl",
        markup: [new URL("../../../static2/config_ssl.html", import.meta.url)],
        bundles: [new URL("../config_ssl.js", import.meta.url)],
        pinned: [
            "config.ssl.title",
            "config.ssl.how.hostname",
            "config.ssl.hostname.rule",
            "config.ssl.status.enabled",
            "config.ssl.actions.setup",
        ],
    },
    {
        name: "config_splynx",
        markup: [new URL("../../../static2/config_splynx.html", import.meta.url)],
        bundles: [new URL("../config_splynx.js", import.meta.url)],
        pinned: [
            "config.splynx.title",
            "config.splynx.compile_mode_help",
            "config.splynx.mode.full",
            "config.splynx.saved",
        ],
    },
    {
        name: "config_flows",
        markup: [new URL("../../../static2/config_flows.html", import.meta.url)],
        bundles: [new URL("../config_flows.js", import.meta.url)],
        pinned: [
            "config.flows.title",
            "config.flows.notrack.help",
            "config.flows.netflow.version_9",
            "config.flows.saved",
        ],
    },
    {
        // RTT: the note keeps a `<strong>` and the preview badges are built by
        // the bundle, so both the tHtml key and the per-colour labels are pinned.
        name: "config_rtt",
        markup: [new URL("../../../static2/config_rtt.html", import.meta.url)],
        bundles: [new URL("../config_rtt.js", import.meta.url)],
        pinned: [
            "config.rtt.title",
            "config.rtt.note",
            "config.rtt.preview.yellow",
            "config.rtt.validation.red_positive",
        ],
    },
    {
        name: "config_visp",
        markup: [new URL("../../../static2/config_visp.html", import.meta.url)],
        bundles: [new URL("../config_visp.js", import.meta.url)],
        pinned: [
            "config.visp.title",
            "config.visp.isp_id_help",
            "config.visp.validation.timeout_positive",
            "config.visp.saved",
        ],
    },
    {
        // IP ranges: the placeholder is an attribute-only key, the buttons and
        // the two unknown-IP switches carry the visible text.
        name: "config_iprange",
        markup: [new URL("../../../static2/config_iprange.html", import.meta.url)],
        bundles: [new URL("../config_iprange.js", import.meta.url)],
        pinned: [
            "config.iprange.title",
            "config.iprange.placeholder",
            "config.iprange.unknown.subtitle",
            "config.iprange.validation.duplicate",
        ],
    },
    {
        name: "config_dataplane",
        markup: [new URL("../../../static2/config_dataplane.html", import.meta.url)],
        bundles: [new URL("../config_dataplane.js", import.meta.url)],
        pinned: [
            "config.dataplane.title",
            "config.dataplane.note",
            "config.dataplane.validation.download",
            "config.dataplane.saved",
        ],
    },
    {
        // Users: the role labels are the interesting part — the option text and
        // the table cell read the same key pair, and the role tokens stay literal.
        name: "config_users",
        markup: [new URL("../../../static2/config_users.html", import.meta.url)],
        bundles: [new URL("../config_users.js", import.meta.url)],
        pinned: [
            "config.users.title",
            "config.users.role.read_only",
            "config.users.edit.password_help",
            "config.users.confirm_delete",
            "common.action.close",
        ],
    },
    {
        name: "config_netzur",
        markup: [new URL("../../../static2/config_netzur.html", import.meta.url)],
        bundles: [new URL("../config_netzur.js", import.meta.url)],
        pinned: [
            "config.netzur.title",
            "config.netzur.api_url_help",
            "config.netzur.validation.timeout_positive",
            "config.netzur.saved",
        ],
    },
    {
        name: "config_powercode",
        markup: [new URL("../../../static2/config_powercode.html", import.meta.url)],
        bundles: [new URL("../config_powercode.js", import.meta.url)],
        pinned: [
            "config.powercode.title",
            "config.powercode.api_url_help",
            "config.powercode.saved",
        ],
    },
    {
        name: "config_wispgate",
        markup: [new URL("../../../static2/config_wispgate.html", import.meta.url)],
        bundles: [new URL("../config_wispgate.js", import.meta.url)],
        pinned: [
            "config.wispgate.title",
            "config.wispgate.api_key_help",
            "config.wispgate.saved",
        ],
    },
    {
        // Network: the subtitle wraps `network.json` in `<code>`, and the node
        // cards compose promote/rename/delete labels plus the Mbps badges.
        name: "config_network",
        markup: [new URL("../../../static2/config_network.html", import.meta.url)],
        bundles: [new URL("../config_network.js", import.meta.url)],
        pinned: [
            "config.network.title",
            "config.network.subtitle",
            "config.network.node.download",
            "config.network.confirm_delete",
            "config.network.locked_integrations",
        ],
    },
    {
        // S4a: the Site Map carries its chrome in the markup and every marker
        // popup, details panel and legend label in the bundle. The legend title,
        // the field caveat and the stop formula are bundle-only: each is one
        // message that takes the active metric as a placeholder, so the markup
        // holds no second copy of that prose.
        name: "site_map",
        markup: [new URL("../../../static2/site_map.html", import.meta.url)],
        bundles: [new URL("../site_map.js", import.meta.url)],
        pinned: [
            "common.nav.site_map",
            "site_map.render.heatmap",
            "site_map.legend.title",
            "site_map.legend.halo_caveat",
            "site_map.details.placeholder",
            "site_map.status.tiles_unavailable",
        ],
    },
    {
        // S4a: the Flow Globe is a bundle-only page surface — its chart labels,
        // tooltips and pinned-cell panel are all written after data arrives — so
        // the pin covers the legend label, the cell vocabulary and the waiting
        // state the empty canvas shows.
        name: "flow_map",
        markup: [],
        bundles: [new URL("../flow_map.js", import.meta.url)],
        pinned: [
            "flow_map.legend.title",
            "flow_map.cell.measured",
            "flow_map.tooltip.interpolated",
            "flow_map.details.title_measured",
            "common.state.waiting",
        ],
    },
    {
        // S5a: the Insight trial surface is four pages and two bundles. The
        // teaser page carries the marketing prose, the setup page runs the
        // hosted-signup handoff, and the success and failure pages close it. The
        // country picker's 237 labels and the circuit-count line are pinned in
        // `trial_translations_contract.test.mjs`, which reads them from the
        // source because this bundle cannot be imported.
        name: "lts_trial",
        markup: [
            new URL("../../../static2/lts_trial.html", import.meta.url),
            new URL("../../../static2/lts_trial_setup.html", import.meta.url),
            new URL("../../../static2/lts_trial_success.html", import.meta.url),
            new URL("../../../static2/lts_trial_fail.html", import.meta.url),
        ],
        bundles: [
            new URL("../lts_trial.js", import.meta.url),
            new URL("../lts_trial_setup.js", import.meta.url),
        ],
        pinned: [
            "trial.hero.lead",
            "trial.trial_terms",
            "trial.success.detail",
            "trial.setup.lead",
            "trial.activated.billing",
            "trial.failed.detail",
        ],
    },
    {
        // S5b: the Help page is a support-ticket surface plus the project and
        // community cards. The modals' labels, the empty states and every
        // validation sentence live in the bundle; the markup carries the cards,
        // the two ticket modals and the brand names that stay literal.
        name: "help",
        markup: [new URL("../../../static2/help.html", import.meta.url)],
        bundles: [new URL("../help.js", import.meta.url)],
        pinned: [
            "help.tickets.title",
            "help.tickets.license_required",
            "help.ticket.new_title",
            "help.ticket.details_placeholder",
            "help.docs.action",
            "help.comments.empty",
            "help.tickets.loading",
        ],
    },
    {
        // S5b: setup_runtime steers the operator to a topology source. The
        // server's own onboarding label and summary stay data, exactly as the
        // licence page shows the authority's label; the fallbacks this page
        // supplies and the three runtime file readings are keyed.
        name: "setup_runtime",
        markup: [new URL("../../../static2/setup_runtime.html", import.meta.url)],
        bundles: [new URL("../setup_runtime.js", import.meta.url)],
        pinned: [
            "setup_runtime.title",
            "setup_runtime.custom.step_integration",
            "setup_runtime.manual.edit_devices",
            "setup_runtime.status.setup_required",
            "setup_runtime.summary.choose_source",
            "setup_runtime.none_configured",
        ],
    },
    {
        // S5b: the API overview and the embedded Swagger frame. Endpoint paths,
        // the `x-bearer` header and the OpenAPI URL stay literal; only the
        // prose around them is catalogued. `api_docs.html` is the frame page and
        // carries none of Swagger's own strings.
        name: "api",
        markup: [
            new URL("../../../static2/api.html", import.meta.url),
            new URL("../../../static2/api_docs.html", import.meta.url),
        ],
        bundles: [],
        pinned: [
            "api.title",
            "api.what.title",
            "api.start.part1",
            "api.start.keys_link",
            "api_docs.title",
            "api_docs.frame_title",
        ],
    },
    {
        // S5b: the StormGuard debug page. Its bundle owns the status badge, the
        // empty states and the timestamps; the markup carries the two table
        // headers. Signal and state tokens from the server stay literal.
        name: "stormguard_debug",
        markup: [new URL("../../../static2/stormguard_debug.html", import.meta.url)],
        bundles: [new URL("../stormguard_debug.js", import.meta.url)],
        pinned: [
            "stormguard_debug.title",
            "stormguard_debug.column.queue_range",
            "stormguard_debug.status.live",
            "stormguard_debug.empty_status",
        ],
    },
    {
        // S5b: Ethernet caps lists the circuits and nodes whose rate had to be
        // reduced to the negotiated port speed. Device, interface and ethtool
        // vocabulary stays literal; the labels, help and pager are keyed.
        name: "ethernet_caps",
        markup: [new URL("../../../static2/ethernet_caps.html", import.meta.url)],
        bundles: [new URL("../ethernet_caps.js", import.meta.url)],
        pinned: [
            "ethernet_caps.title",
            "ethernet_caps.column.limiting_device",
            "ethernet_caps.targets_count",
            "ethernet_caps.load_failed",
        ],
    },
    {
        // S5b: the whole-tree Sankey. Chart vocabulary follows the analytics
        // ruling (root, throughput); the overlay's waiting states are the only
        // prose the graph itself renders.
        name: "all_tree_sankey",
        markup: [new URL("../../../static2/all_tree_sankey.html", import.meta.url)],
        bundles: [new URL("../all_tree_sankey.js", import.meta.url)],
        pinned: [
            "sankey.top_node",
            "sankey.max_depth",
            "sankey.overlay.limited_title",
            "sankey.overlay.requesting",
            "graph.sankey.root",
            "dashboard.pause",
        ],
    },
];

const PAGES_DIR = new URL("../../../static2/", import.meta.url);
const DASHLET_INDEX = new URL("../dashlets/dashlet_index.js", import.meta.url);
const CATALOG_DIR = new URL("../../../static2/i18n/", import.meta.url);

const [english, spanish] = await Promise.all(
    ["en.json", "es.json"].map((file) => readFile(new URL(file, CATALOG_DIR), "utf8").then(JSON.parse)),
);
const EN = english.messages;
const ES = spanish.messages;

const MARKUP_KEY = /data-i18n(?:-title|-placeholder|-aria-label|-alt)?\s*=\s*["']([^"']+)["']/g;
const TEXT_KEY = /\b(?:tAttr|tHtml|tn|t)\(\s*["']([a-z][A-Za-z0-9_]*(?:\.[A-Za-z0-9_]+)+)["']/g;
// A data-i18n element's own text is the English fallback the applier replaces;
// an empty one renders nothing until the catalog loads.
const KEYED_ELEMENT = /<([a-z][a-z0-9]*)([^>]*\bdata-i18n(?:-[a-z-]+)?="[^"]+"[^>]*)>([\s\S]*?)<\/\1>/gi;
const TAG = /<[^>]*>/g;
// Display formatting belongs to `i18n/format.js`; `toISOString()` on a debug
// record is a machine timestamp, not a rendered one.
const HAND_ROLLED_CLOCK = /\.toLocale(?:String|TimeString|DateString)\(/;

/** The catalog keys a page file declares. */
function keysOf(text, pattern) {
    return new Set([...text.matchAll(pattern)].map((match) => match[1]));
}

function textOf(html) {
    return html.replace(TAG, " ").replace(/\s+/g, " ").trim();
}

async function readAll(urls) {
    return Promise.all(urls.map((url) => readFile(url, "utf8")));
}

// "Different text" is `catalogs_contract.test.mjs`'s rule and it checks every
// key at once; here the question is only whether the page's key reaches a
// catalog at all.
test("every key a migrated page renders is defined in both locales", async () => {
    for (const page of PAGES) {
        const rendered = new Set();
        for (const html of await readAll(page.markup)) {
            for (const key of keysOf(html, MARKUP_KEY)) rendered.add(key);
        }
        for (const source of await readAll(page.bundles)) {
            for (const key of keysOf(source, TEXT_KEY)) rendered.add(key);
        }
        assert.ok(rendered.size > 0, `${page.name} must render at least one catalog key`);
        for (const key of rendered) {
            assert.ok(key in EN, `${page.name} renders "${key}", which en.json does not define`);
            assert.ok(key in ES, `${page.name} renders "${key}", which es.json does not define`);
        }
    }
});

test("each migrated page keeps its load-bearing key", async () => {
    for (const page of PAGES) {
        const rendered = new Set();
        for (const html of await readAll(page.markup)) {
            for (const key of keysOf(html, MARKUP_KEY)) rendered.add(key);
        }
        for (const source of await readAll(page.bundles)) {
            for (const key of keysOf(source, TEXT_KEY)) rendered.add(key);
        }
        for (const key of page.pinned) {
            assert.ok(rendered.has(key), `${page.name} no longer renders the pinned key "${key}"`);
        }
    }
});

test("every keyed element in the migrated markup keeps an English fallback", async () => {
    for (const page of PAGES) {
        for (const [index, html] of (await readAll(page.markup)).entries()) {
            let keyed = 0;
            for (const match of html.matchAll(KEYED_ELEMENT)) {
                const attributes = match[2];
                const textKey = attributes.match(/\sdata-i18n="([^"]+)"/);
                keyed += 1;
                if (textKey) {
                    assert.notEqual(
                        textOf(match[3]),
                        "",
                        `${page.name} markup ${index}: "${textKey[1]}" has no inline English`,
                    );
                    continue;
                }
                // An attribute-only key has no body text to carry the fallback:
                // the authored attribute (aria-label, title, placeholder) is it.
                for (const [, attribute, label] of attributes.matchAll(/\sdata-i18n-([a-z-]+)="([^"]+)"/g)) {
                    const authored = attributes.match(new RegExp(`\\s${attribute}="([^"]*)"`));
                    assert.ok(
                        authored && authored[1].trim() !== "",
                        `${page.name} markup ${index}: "${label}" has no authored ${attribute} fallback`,
                    );
                }
            }
            assert.ok(keyed > 0, `${page.name} markup ${index} keys no element`);
        }
    }
});

test("no migrated bundle formats a clock or a date by hand", async () => {
    for (const page of PAGES) {
        for (const [index, source] of (await readAll(page.bundles)).entries()) {
            assert.equal(
                HAND_ROLLED_CLOCK.test(source),
                false,
                `${page.name} bundle ${index} calls toLocale*; use i18n/format.js`,
            );
        }
    }
});

test("the dashlet menu carries catalog keys and keeps its key count", async () => {
    const source = await readFile(DASHLET_INDEX, "utf8");
    const menu = source.slice(source.indexOf("export const DashletMenu"));
    const names = keysOf(menu, /name: \(\) => t\("(dashlet\.name\.[a-z0-9_]+)"\)/g);
    const categories = keysOf(menu, /category: \(\) => t\("(dashlet\.category\.[a-z0-9_]+)"\)/g);
    assert.equal(names.size, 50, "the menu's widget-name key count is the ratchet");
    assert.equal(categories.size, 13, "the menu's category key count is the ratchet");
    for (const key of [...names, ...categories]) {
        assert.ok(key in EN && key in ES, `dashlet menu key "${key}" is not in both catalogs`);
    }
    assert.equal(
        /(?:name|category): "(?!dashlet\.)/.test(menu),
        false,
        "the menu must not carry an English label beside the catalog keys",
    );
});

// The config menu is rendered by `config_helper.renderConfigMenu`, which every
// config page imports, so its labels are pinned here instead of on one page.
// Sixteen sections carry catalog keys and the integration product names beside
// them stay literal; the count is the ratchet, so a new section adds a key to
// this list, to both catalogs and to the menu.
test("the config menu carries catalog keys and keeps its key count", async () => {
    const helper = await readFile(new URL("../config/config_helper.js", import.meta.url), "utf8");
    const menu = helper.slice(helper.indexOf("export function renderConfigMenu"));
    const labels = keysOf(menu, /\bt\("(config\.common\.menu\.[a-z_]+)"\)/g);
    assert.equal(labels.size, 16, "the config menu's label key count is the ratchet");
    for (const key of labels) {
        assert.ok(key in EN && key in ES, `config menu key "${key}" is not in both catalogs`);
    }
    // `text:` values that are not a key must be a product name: a new English
    // section label would otherwise slip in beside the translated rows.
    assert.equal(
        /\btext: "(?!StormGuard|Splynx|Netzur|VISP|UISP|Powercode|Sonar|WispGate)/.test(menu),
        false,
        "the config menu must not carry an English label beside the catalog keys",
    );
});

test("the migrated pages are served through the shell that runs the translation pass", async () => {
    // None of these pages has an `applyDomTranslations()` of its own: the shell
    // bundle runs it, and a page moved out of the shell would render the
    // English fallback forever.
    const shell = await readFile(new URL("../../../static2/template.html", import.meta.url), "utf8");
    assert.match(shell, /%%I18N_SCRIPT%%/, "the shell must load the served catalog before the page bundle");
    for (const page of PAGES) {
        for (const url of page.markup) {
            const html = await readFile(url, "utf8");
            assert.equal(
                /applyDomTranslations/.test(html),
                false,
                "a shell page translates through template.js, not its own markup",
            );
        }
    }
});

test("the migrated page list matches the served markup", async () => {
    const served = new Set(await readdir(PAGES_DIR));
    for (const page of PAGES) {
        for (const url of page.markup) {
            const name = url.pathname.split("/").pop();
            assert.ok(served.has(name), `${name} is not in static2/`);
        }
    }
});

// `helpers/builders.js` composed "12s ago" and "> 5 Minutes ago" by hand until
// this unit moved the wording into `helpers/last_seen.mjs`; both pages that show
// a last-seen column call it, so a regression here is visible on the tree, the
// circuit devices, the shaped devices and the unknown-IP pages at once.
test("the last-seen column reads the shared relative-time helper", async () => {
    const builders = await readFile(new URL("../helpers/builders.js", import.meta.url), "utf8");
    const helper = await readFile(new URL("../helpers/last_seen.mjs", import.meta.url), "utf8");
    assert.match(
        builders,
        /export \{formatLastSeen\} from "\.\/last_seen\.mjs"/,
        "builders.js must re-export the moved helper for its existing callers",
    );
    assert.match(
        helper,
        /import \{relativeTimeText\} from "\.\.\/i18n\/relative_time\.mjs"/,
        "the last-seen wording must come from the shared helper",
    );
    for (const source of [builders, helper]) {
        assert.equal(
            /" ago"|` \$\{[^}]+\} ago`/.test(source),
            false,
            "no hand-built English relative-time suffix may remain",
        );
        assert.equal(
            /\.toLocale(?:String|TimeString|DateString)\(/.test(source),
            false,
            "the helper must not format a clock by hand",
        );
    }
});

// S4a: the two map surfaces assembled their tooltips and legend notes by
// concatenating an English label with a value, which hands a translator a
// sentence in fragments. The replacement is one message with named
// placeholders; `extract.mjs` already proves each call site supplies the names
// its message declares, so this pins the shape that made the split impossible:
// two placeholders in both locales and the call that fills them.
test("the map tooltips are whole messages with two placeholders", async () => {
    const CASES = [
        {
            name: "site_map",
            key: "site_map.details.inherit_hint",
            source: new URL("../site_map.js", import.meta.url),
            call: /tn\("site_map\.details\.inherit_hint", attachedAps\.length, \{\s*count: attachedAps\.length,\s*hint: fanoutHint,?\s*\}\)/,
            placeholders: ["count", "hint"],
        },
        {
            name: "flow_map",
            key: "flow_map.details.subtitle",
            source: new URL("../flow_map.js", import.meta.url),
            // The coordinates are formatted by Intl at the call site: a
            // `toFixed(1)` string renders "33.9" in a Spanish sentence.
            call: /t\("flow_map\.details\.subtitle", \{\s*lat: formatNumber\(cell\.lat, ONE_DECIMAL\),\s*lng: formatNumber\(cell\.lng, ONE_DECIMAL\),?\s*\}\)/,
            placeholders: ["lat", "lng"],
        },
    ];
    for (const item of CASES) {
        for (const [locale, catalog] of [["en", EN], ["es", ES]]) {
            const message = catalog[item.key];
            assert.ok(message !== undefined, `${locale}.json must define "${item.key}"`);
            const texts = typeof message === "string" ? [message] : Object.values(message);
            for (const text of texts) {
                assert.deepEqual(
                    [...new Set(text.match(/\{(\w+)\}/g))].sort(),
                    item.placeholders.map((name) => `{${name}}`).sort(),
                    `${locale}.json "${item.key}" must carry exactly ${item.placeholders.join(" and ")}`,
                );
            }
        }
        assert.match(
            await readFile(item.source, "utf8"),
            item.call,
            `${item.name} must fill every placeholder of "${item.key}" at one call site`,
        );
    }
});

// The analytics vocabulary is a project ruling (S4 Oracle batch): the two map
// pages name the same objects the nav and the legend name, and `hexbin` stays a
// technical term in both locales rather than being paraphrased per page.
test("the map vocabulary follows the analytics ruling", () => {
    const VOCABULARY = new Map([
        ["common.nav.site_map", "Mapa de sitios"],
        ["site_map.render.heatmap", "Mapa de calor"],
        ["site_map.render.points", "Puntos"],
        ["site_map.legend.title", "Leyenda"],
        ["flow_map.legend.title", "Latencia de internet (extremo más cercano)"],
    ]);
    for (const [key, expected] of VOCABULARY) {
        assert.equal(ES[key], expected, `es.json "${key}" must use the ruled term`);
    }
    for (const catalog of [EN, ES]) {
        assert.match(
            catalog["site_map.legend.title_hexbin"],
            /hexbin/,
            "the hexbin legend title must keep `hexbin` literal",
        );
    }
});

// §4.2 of the S4 Oracle batch: the Flow Globe's pinned cell detail labelled a byte
// value with the shared throughput message. Throughput is a rate, so the label was
// wrong on the one surface where the number is a volume; the shared message stays
// as it is for the rate widgets.
test("the flow globe labels a cell's bytes as a volume, never as a rate", async () => {
    const bundle = await readFile(new URL("../flow_map.js", import.meta.url), "utf8");
    assert.match(
        bundle,
        /\[t\("flow_map\.cell\.volume"\), scaleNumber\(cell\.bytes \|\| 0\)\]/,
        "the cell detail row must name the volume and leave the unit in the label",
    );
    assert.doesNotMatch(
        bundle,
        /\[t\("common\.table\.throughput"\)/,
        "the cell bytes may not be labelled with the rate message",
    );
    // The scaled number already carries its prefix ("2.5M"); the label carries the unit.
    assert.doesNotMatch(bundle, /scaleNumber\(cell\.bytes \|\| 0\)\}B/);

    assert.equal(EN["flow_map.cell.volume"], "Traffic volume (B)");
    assert.equal(ES["flow_map.cell.volume"], "Volumen de tráfico (B)");
    // The tooltip's English original was dimensionally wrong too: it named a rate
    // for the same byte value, and `{volume}` arrives pre-scaled.
    assert.equal(EN["flow_map.tooltip.throughput"], "Traffic volume (B): {volume}");
    assert.equal(ES["flow_map.tooltip.throughput"], "Volumen de tráfico (B): {volume}");
    // The shared rate label is unchanged: four rate widgets still read it.
    assert.equal(EN["common.table.throughput"], "Throughput");
    assert.equal(ES["common.table.throughput"], "Tasa de transferencia");
});
