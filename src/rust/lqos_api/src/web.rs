use std::sync::Arc;

use axum::Router;
use utoipa::OpenApi;
use utoipa::openapi::Required;
use utoipa::openapi::path::{Operation, Parameter, ParameterBuilder, ParameterIn, PathItem};
use utoipa::openapi::schema::{Object, Type};
use utoipa::openapi::security::{ApiKey, ApiKeyValue, SecurityScheme};
use utoipa_swagger_ui::SwaggerUi;

use crate::{
    api,
    api::*,
    transit_types::{
        CircuitTransit, DownUpTransit, FlowbeeSummaryDataTransit, IpstatsTransit,
        NetworkJsonTransit,
    },
};

/// Mark the retired topology-write routes deprecated in the OpenAPI document.
struct LegacyTopologyDeprecated;

impl utoipa::Modify for LegacyTopologyDeprecated {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use utoipa::openapi::Deprecated;

        for path in [
            "/network_json/update",
            "/network_json/node",
            "/network_json/node/{node_name}",
            "/network/reparent_site",
        ] {
            let Some(item) = openapi.paths.paths.get_mut(path) else {
                continue;
            };
            if let Some(operation) = item.post.as_mut() {
                operation.deprecated = Some(Deprecated::True);
            }
            if let Some(operation) = item.delete.as_mut() {
                operation.deprecated = Some(Deprecated::True);
            }
        }
    }
}

/// The path prefix the node manager mounts this API on.
///
/// The document's `servers` entry and the daemon's `nest` both carry it, so a
/// client that reads the document applies the prefix exactly once.
pub const API_MOUNT_PREFIX: &str = "/api/v3";

/// The header every request authenticates with.
const BEARER_HEADER: &str = "x-bearer";

/// The document's name for its single security scheme.
const SECURITY_SCHEME: &str = BEARER_HEADER;

/// What a `401` means on this mount: the middleware answers it, and only for the key.
const UNAUTHORISED_DESCRIPTION: &str = "Missing or invalid `x-bearer` API key.";

/// The one API key parameter the document shows, replacing the per-handler copies.
const KEY_PARAMETER_DESCRIPTION: &str = "A named or legacy local API key.";
const KEY_PARAMETER_EXAMPLE: &str = "lqos_api_<uuid>_<secret>";

/// Sentences the handlers inherited from the standalone service's licence model.
///
/// The configuration operations list which secrets survive a write. A licence
/// key is not part of this mount's model, so the clause goes with it.
const RETIRED_SENTENCES: &[(&str, &str)] = &[
    (
        "Local API key digests, the legacy local bearer token, and the Insight/API license key are redacted.",
        "Local API key digests and the legacy local bearer token are redacted.",
    ),
    (
        "while preserving the authoritative local API keys, legacy local bearer token, and Insight/API license key",
        "while preserving the authoritative local API keys and legacy local bearer token",
    ),
];

/// Names the document's single authentication mechanism: an `x-bearer` API key.
///
/// The per-handler attributes still carry the retired licence-service wording
/// (a licence key, a policy-denied `403`, an entitlement). Rewriting those
/// attribute lines across 43 handler modules is not this mount's job: this
/// modifier normalises the generated document instead, and
/// `document_carries_no_licence_or_entitlement_claims` fails if that wording
/// comes back.
struct KeyOnlyAuthDocumentation;

impl utoipa::Modify for KeyOnlyAuthDocumentation {
    fn modify(&self, document: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = document.components.as_mut() {
            components.add_security_scheme(
                SECURITY_SCHEME,
                SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::new(BEARER_HEADER))),
            );
        }
        for item in document.paths.paths.values_mut() {
            for operation in operations_mut(item) {
                document_key_only_auth(operation);
            }
        }
    }
}

/// Every operation on one path item.
fn operations_mut(item: &mut PathItem) -> impl Iterator<Item = &mut Operation> {
    [
        item.get.as_mut(),
        item.put.as_mut(),
        item.post.as_mut(),
        item.delete.as_mut(),
        item.options.as_mut(),
        item.head.as_mut(),
        item.patch.as_mut(),
        item.trace.as_mut(),
    ]
    .into_iter()
    .flatten()
}

/// Rewrites one operation's inherited authentication wording to this mount's
/// single key, and drops the retired policy answer.
///
/// `403` existed only in the standalone service's licence gate. This mount has
/// no `403` producer at all — `require_api_key` answers `401` and the handlers
/// only ever return their own statuses — so the arm is removed rather than
/// reworded: a documented status the mount cannot produce is exactly the kind
/// of untruth this unit exists to delete.
fn document_key_only_auth(operation: &mut Operation) {
    if let Some(utoipa::openapi::RefOr::T(response)) = operation.responses.responses.get_mut("401")
    {
        response.description = UNAUTHORISED_DESCRIPTION.to_string();
    }
    operation.responses.responses.remove("403");

    if let Some(description) = operation.description.as_mut() {
        for (retired, replacement) in RETIRED_SENTENCES {
            *description = description.replace(retired, replacement);
        }
    }

    let Some(parameters) = operation.parameters.as_mut() else {
        return;
    };
    for parameter in parameters.iter_mut() {
        if parameter.name == BEARER_HEADER && parameter.parameter_in == ParameterIn::Header {
            *parameter = key_parameter();
        }
    }
}

/// The `x-bearer` header parameter, documented once instead of per handler.
fn key_parameter() -> Parameter {
    ParameterBuilder::new()
        .name(BEARER_HEADER)
        .parameter_in(ParameterIn::Header)
        .required(Required::True)
        .description(Some(KEY_PARAMETER_DESCRIPTION))
        .schema(Some(Object::with_type(Type::String)))
        .example(Some(serde_json::Value::String(
            KEY_PARAMETER_EXAMPLE.to_string(),
        )))
        .build()
}

/// Test-only incident-matrix companion. Registered always, but answers 404
/// unless `LQOS_API_TEST_HOOKS=1`, so the harness can exercise an unknown-path
/// answer on a route that exists.
async fn test_circuits_hook() -> axum::http::StatusCode {
    if crate::api::test_hooks_enabled() {
        axum::http::StatusCode::OK
    } else {
        axum::http::StatusCode::NOT_FOUND
    }
}

/// OpenAPI document for the LibreQoS Local Node API
///
/// The document describes one mount: `API_MOUNT_PREFIX`, key-only, with the
/// operations kept root-relative so the `servers` entry is the only place the
/// prefix appears.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "LibreQoS API",
        description = "LibreQoS node API. Every request carries an API key in the `x-bearer` header; the node manager creates and revokes keys under Local API Keys.",
        contact(
            name = "LibreQoS Support",
            email = "support@libreqos.io"
        ),
        license(
            name = "GPL-2.0-only",
            url = "https://www.gnu.org/licenses/gpl-2.0.html"
        )
    ),
    servers(
        (url = "/api/v3", description = "This node's embedded API mount.")
    ),
    security(
        ("x-bearer" = [])
    ),
    paths(
        health_check,
        version,
        current_throughput,
        rtt_histogram,
        shaped_unshaped_traffic,
        tree_summary,
        stormguard_status,
        capacity_status,
        retransmit_status,
        protocol_status,
        endpoints_status,
        scheduler_status,
        cpu_usage,
        cpu_workers,
        ram_usage,
        system_stats_health,
        status_snapshot,
        host_counts,
        unknown_ips,
        active_user_count,
        active_user_list,
        missing_users,
        asn_top_download,
        asn_top_upload,
        top_n_downloaders,
        top_n_uploaders,
        worst_rtt,
        worst_retransmits,
        best_rtt,
        get_host_counter,
        reload_libreqos,
        validate_shaped_devices,
        get_network_map,
        get_full_network_map,
        get_network_tree,
        get_network_node_by_name,
        top_map_queues,
        get_node_names_from_ids,
        get_all_circuits,
        get_all_circuits_combined,
        get_circuit_combined,
        get_circuit_stats_by_id,
        get_circuit_stats_by_ip,
        get_funnel,
        get_lqos_stats,
        device_counts,
        circuit_count,
        flow_map,
        flow_explorer_asns,
        flow_explorer_countries,
        flow_explorer_protocols,
        flow_explorer_asn_timeline,
        flow_explorer_country_timeline,
        flow_explorer_protocol_timeline,
        scheduler_details,
        queue_stats_total,
        qoo,
        qoo_site,
        qoo_circuit,
        circuit_capacity,
        tree_capacity,
        retransmit_summary,
        tree_summary_l2,
        search,
        dump_active_flows,
        count_active_flows,
        top_flows,
        flows_by_ip,
        current_endpoints_by_country,
        current_endpoint_latlon,
        flow_duration,
        ether_protocol_summary,
        ip_protocol_summary,
        get_stormguard_stats,
        get_shaped_devices,
        get_shaped_device,
        write_shaped_devices,
        shaped_devices_by_circuit_id,
        identify_ip_address,
        circuits_at_capacity,
        tree_nodes_at_capacity,
        circuits_high_retransmits,
        tree_nodes_high_retransmits,
        ping_circuit,
        ping_ip,
        traceroute_ip,
        traceroute_circuit,
        overrides_persistent_devices_list,
        overrides_persistent_devices_add,
        overrides_persistent_devices_delete_circuit,
        overrides_persistent_devices_delete_device,
        overrides_adjustments_list,
        overrides_adjustments_add_circuit_speed,
        overrides_adjustments_add_device_speed,
        overrides_adjustments_add_remove_circuit,
        overrides_adjustments_add_remove_device,
        overrides_adjustments_add_reparent_circuit,
        overrides_adjustments_delete_index,
        overrides_network_adjustments_list,
        overrides_network_adjustments_add_site_speed,
        overrides_network_adjustments_delete_index,
        overrides_uisp_bandwidth_list,
        overrides_uisp_bandwidth_set,
        overrides_uisp_bandwidth_remove,
        overrides_uisp_route_list,
        overrides_uisp_route_add,
        overrides_uisp_route_remove_index,
        get_config_as_json,
        get_config_as_toml,
        get_config_hash,
        write_config_json,
        write_config_toml,
        get_network_json_as_text,
        get_network_json_as_json,
        legacy_network_json_update_stub,
        legacy_network_json_add_node_stub,
        legacy_network_json_remove_node_stub,
        legacy_network_reparent_site_stub,
        live_upsert_circuit,
        live_remove_circuit,
        warnings,
        urgent_status,
        urgent_list,
        urgent_clear,
        urgent_clear_all,
    ),
    components(
        schemas(CurrentThroughput),
        schemas(RttHistogramData),
        schemas(ShapedUnshapedTraffic),
        schemas(TreeSummaryData),
        schemas(StormguardSiteStatus),
        schemas(RamUsageData),
        schemas(SystemStatsHealth),
        schemas(CpuWorkersResponse),
        schemas(CpuUsageSection),
        schemas(CpuUsageRow),
        schemas(CpuTopologySection),
        schemas(BridgeSection),
        schemas(DeferredMaintenanceSection),
        schemas(AutobalanceSection),
        schemas(WorkersSection),
        schemas(WorkerRow),
        schemas(WorkerCounters),
        schemas(OwnershipSection),
        schemas(CircuitOwner),
        schemas(NodeOwners),
        schemas(MovesSection),
        schemas(MoveRow),
        schemas(StatusSnapshotTransit),
        schemas(HostCountsData),
        schemas(IpstatsTransit),
        schemas(CircuitTransit),
        schemas(crate::transit_types::CircuitRollupTransit),
        schemas(crate::transit_types::DownUpF32Transit),
        schemas(crate::transit_types::DownUpOptionalU64Transit),
        schemas(ActiveUserCountData),
        schemas(DownUpTransit),
        schemas(FlowbeeSummaryDataTransit),
        schemas(crate::transit_types::QooDataTransit),
        schemas(crate::transit_types::QooBlocksTransit),
        schemas(crate::transit_types::QooLatestTransit),
        schemas(NetworkJsonTransit),
        schemas(crate::api::TopAsnRow),
        schemas(crate::api::HealthStatus),
        schemas(crate::api::WarningLevel),
        schemas(crate::api::WarningEntry),
        schemas(crate::api::UrgentStatusData),
        schemas(crate::api::UrgentItem),
        schemas(crate::api::UrgentList),
        schemas(crate::api::UrgentClearResponse),
        schemas(crate::api::DeviceCountsData),
        schemas(crate::api::CircuitCountData),
        schemas(crate::api::FlowMapPointData),
        schemas(crate::api::AsnListEntryData),
        schemas(crate::api::CountryListEntryData),
        schemas(crate::api::ProtocolListEntryData),
        schemas(crate::api::FlowTimelineEntryData),
        schemas(crate::api::SchedulerDetailsData),
        schemas(crate::api::QueueStatsTotalData),
        schemas(crate::api::CircuitCapacityRowData),
        schemas(crate::api::NodeCapacityData),
        schemas(crate::api::RetransmitSummaryData),
        schemas(crate::api::TreeSummaryL2Child),
        schemas(crate::api::TreeSummaryL2Entry),
        schemas(crate::api::SearchResultData),
        schemas(ShapedDeviceTransit),
        schemas(CircuitCapacityData),
        schemas(CircuitsAtCapacityResponse),
        schemas(TreeNodeCapacityData),
        schemas(TreeNodesAtCapacityResponse),
        schemas(CircuitRetransmitData),
        schemas(CircuitsHighRetransmitsResponse),
        schemas(TreeNodeRetransmitData),
        schemas(TreeNodesHighRetransmitsResponse),
        // Overrides API types
        schemas(crate::api::AddPersistentDeviceRequest),
        schemas(crate::api::OverrideActionResponse),
        schemas(crate::api::RemovedCountResponse),
        schemas(crate::api::CircuitAdjustmentTransit),
        schemas(crate::api::NetworkAdjustmentTransit),
        schemas(crate::api::TopologyParentOverrideModeTransit),
        schemas(crate::api::AddCircuitSpeedRequest),
        schemas(crate::api::AddDeviceSpeedRequest),
        schemas(crate::api::RemoveCircuitRequest),
        schemas(crate::api::RemoveDeviceRequest),
        schemas(crate::api::ReparentCircuitRequest),
        schemas(crate::api::AddSiteSpeedRequest),
        schemas(crate::api::UispBandwidthOverrideEntry),
        schemas(crate::api::UispRouteOverrideTransit),
        schemas(crate::api::BandwidthSetRequest),
        schemas(crate::api::RouteAddRequest),
        schemas(crate::api::LiveUpsertCircuitRequest),
        schemas(crate::api::LiveUpdateResponse),
        schemas(crate::api::LiveRemovedCountResponse),
    ),
    modifiers(&KeyOnlyAuthDocumentation, &LegacyTopologyDeprecated),
    tags(
        (name = "Configuration", description = "Read and update LibreQoS configuration."),
        (name = "Topology & Files", description = "Manage network.json and ShapedDevices.csv."),
        (name = "Overrides", description = "Manage persistent overrides and UISP adjustments."),
        (name = "Control & Reload", description = "Trigger reloads and manage scheduler control."),
        (name = "Network Map", description = "Topology maps, node names, and funnel views."),
        (name = "Circuit Analysis", description = "Circuit and node performance analysis."),
        (name = "Monitoring", description = "Targeted circuit metrics and dashboards."),
        (name = "Flow Analysis", description = "Flow-level statistics and rankings."),
        (name = "Endpoint Metrics", description = "Endpoint location and country summaries."),
        (name = "Protocol Metrics", description = "Protocol breakdowns for ethernet and IP."),
        (name = "System Metrics", description = "Host CPU/RAM and throughput metrics."),
        (name = "CPU & Workers", description = "CPU topology, bridge workers, ownership and placements."),
        (name = "Health & Status", description = "Health checks and combined status views."),
        (name = "User Activity", description = "Active/missing users and host counts."),
        (name = "Stormguard", description = "Stormguard status and statistics."),
        (name = "warnings", description = "Operator warnings and urgent issues.")
    )
)]
pub struct ApiDoc;

/// Build the Axum Router with all routes, middleware, and Swagger UI.
/// Builds the API's data routes: no authentication, no docs, no global side
/// effects, so a host process can mount them on its own server.
///
/// Unmatched paths under the mount answer a JSON 404, never the host's HTML
/// fallback: axum's `Router::nest` carries an explicit fallback of the nested
/// router onto the prefix.
///
/// The mount's state is the daemon's handles, never a sampler of its own: the
/// carrier is [`api::ApiState`], built by the process that owns the listeners.
pub fn api_routes(state: Arc<api::ApiState>) -> Router {
    route_table().fallback(api_not_found).with_state(state)
}

/// The Swagger UI's mount path.
const DOCS_UI_PATH: &str = "/api/v3/docs";

/// The path the OpenAPI document is served from.
const DOCUMENT_PATH: &str = "/api/v3/openapi.json";

/// The documentation surface of the embedded mount: the OpenAPI document and
/// the Swagger UI that renders it.
///
/// The paths are absolute from the host server's root, on purpose: the daemon
/// merges this router BESIDE its authenticated `/api/v3` nest, so the
/// documentation answers without a key while the data routes keep theirs.
/// The Swagger UI loads its vendored assets with relative URLs, so the
/// slashless `/docs` redirects to `/docs/`: a page served at `/docs` itself
/// would resolve every asset against `/api/v3/` and render empty.
pub fn docs_router() -> Router {
    Router::from(SwaggerUi::new(DOCS_UI_PATH).url(DOCUMENT_PATH, ApiDoc::openapi()))
}

/// JSON body for an unknown API path.
async fn api_not_found() -> impl axum::response::IntoResponse {
    (
        axum::http::StatusCode::NOT_FOUND,
        axum::Json(serde_json::json!({ "error": "unknown API path" })),
    )
}

fn route_table() -> Router<Arc<api::ApiState>> {
    Router::new()
        .route("/health", axum::routing::get(health_check))
        .route("/version", axum::routing::get(version))
        .route("/circuits", axum::routing::get(test_circuits_hook))
        .route(
            "/current_throughput",
            axum::routing::get(current_throughput),
        )
        .route("/rtt_histogram", axum::routing::get(rtt_histogram))
        .route(
            "/shaped_unshaped_traffic",
            axum::routing::get(shaped_unshaped_traffic),
        )
        .route("/tree_summary", axum::routing::get(tree_summary))
        .route("/stormguard_status", axum::routing::get(stormguard_status))
        .route("/capacity_status", axum::routing::get(capacity_status))
        .route("/retransmit_status", axum::routing::get(retransmit_status))
        .route("/protocol_status", axum::routing::get(protocol_status))
        .route("/endpoints_status", axum::routing::get(endpoints_status))
        .route("/scheduler_status", axum::routing::get(scheduler_status))
        .route("/cpu_usage", axum::routing::get(cpu_usage))
        .route("/cpu_workers", axum::routing::get(cpu_workers))
        .route("/ram_usage", axum::routing::get(ram_usage))
        .route(
            "/system_stats_health",
            axum::routing::get(system_stats_health),
        )
        .route("/status_snapshot", axum::routing::get(status_snapshot))
        .route("/host_counts", axum::routing::get(host_counts))
        .route(
            "/active_user_count/{seconds}",
            axum::routing::get(active_user_count),
        )
        .route(
            "/active_user_list/{seconds}",
            axum::routing::get(active_user_list),
        )
        .route(
            "/missing_users/{seconds}",
            axum::routing::get(missing_users),
        )
        .route(
            "/top_n_downloaders/{start}/{end}",
            axum::routing::get(top_n_downloaders),
        )
        .route(
            "/top_n_uploaders/{start}/{end}",
            axum::routing::get(top_n_uploaders),
        )
        .route("/worst_rtt/{start}/{end}", axum::routing::get(worst_rtt))
        .route(
            "/worst_retransmits/{start}/{end}",
            axum::routing::get(worst_retransmits),
        )
        .route("/best_rtt/{start}/{end}", axum::routing::get(best_rtt))
        .route("/host_counter", axum::routing::get(get_host_counter))
        .route("/reload_libreqos", axum::routing::post(reload_libreqos))
        .route("/qoo", axum::routing::get(qoo))
        .route("/qoo/site/{site_name}", axum::routing::get(qoo_site))
        .route("/qoo/circuit/{circuit_id}", axum::routing::get(qoo_circuit))
        .route("/unknown_ips", axum::routing::get(unknown_ips))
        .route(
            "/validate_shaped_devices",
            axum::routing::get(validate_shaped_devices),
        )
        .route("/network_map/{parent}", axum::routing::get(get_network_map))
        .route(
            "/full_network_map",
            axum::routing::get(get_full_network_map),
        )
        .route("/network_tree", axum::routing::get(get_network_tree))
        .route(
            "/network_node/{node_name}",
            axum::routing::get(get_network_node_by_name),
        )
        .route("/top_map_queues/{n}", axum::routing::get(top_map_queues))
        .route("/node_names", axum::routing::post(get_node_names_from_ids))
        .route("/all_circuits", axum::routing::get(get_all_circuits))
        .route(
            "/all_circuits_combined",
            axum::routing::get(get_all_circuits_combined),
        )
        .route(
            "/circuit_combined/{circuit_id}",
            axum::routing::get(get_circuit_combined),
        )
        .route(
            "/circuit/{circuit_id}",
            axum::routing::get(get_circuit_stats_by_id),
        )
        .route(
            "/circuit/by_ip/{ip_address}",
            axum::routing::get(get_circuit_stats_by_ip),
        )
        .route("/funnel/{target}", axum::routing::get(get_funnel))
        .route("/lqos_stats", axum::routing::get(get_lqos_stats))
        .route("/device_counts", axum::routing::get(device_counts))
        .route("/circuit_count", axum::routing::get(circuit_count))
        .route("/flow_map", axum::routing::get(flow_map))
        .route(
            "/flow_explorer/asns",
            axum::routing::get(flow_explorer_asns),
        )
        .route(
            "/flow_explorer/countries",
            axum::routing::get(flow_explorer_countries),
        )
        .route(
            "/flow_explorer/protocols",
            axum::routing::get(flow_explorer_protocols),
        )
        .route(
            "/flow_explorer/asn/{asn}",
            axum::routing::get(flow_explorer_asn_timeline),
        )
        .route(
            "/flow_explorer/country/{iso}",
            axum::routing::get(flow_explorer_country_timeline),
        )
        .route(
            "/flow_explorer/protocol/{protocol}",
            axum::routing::get(flow_explorer_protocol_timeline),
        )
        .route("/scheduler_details", axum::routing::get(scheduler_details))
        .route("/queue_stats_total", axum::routing::get(queue_stats_total))
        .route("/circuit_capacity", axum::routing::get(circuit_capacity))
        .route("/tree_capacity", axum::routing::get(tree_capacity))
        .route(
            "/retransmit_summary",
            axum::routing::get(retransmit_summary),
        )
        .route("/tree_summary_l2", axum::routing::get(tree_summary_l2))
        .route("/search", axum::routing::get(search))
        .route("/dump_active_flows", axum::routing::get(dump_active_flows))
        .route(
            "/count_active_flows",
            axum::routing::get(count_active_flows),
        )
        .route("/top_flows/{flow_type}/{n}", axum::routing::get(top_flows))
        .route("/flows_by_ip/{ip}", axum::routing::get(flows_by_ip))
        .route("/asn_top_download", axum::routing::get(asn_top_download))
        .route("/asn_top_upload", axum::routing::get(asn_top_upload))
        .route(
            "/current_endpoints_by_country",
            axum::routing::get(current_endpoints_by_country),
        )
        .route(
            "/current_endpoint_latlon",
            axum::routing::get(current_endpoint_latlon),
        )
        .route("/flow_duration", axum::routing::get(flow_duration))
        .route(
            "/ether_protocol_summary",
            axum::routing::get(ether_protocol_summary),
        )
        .route(
            "/ip_protocol_summary",
            axum::routing::get(ip_protocol_summary),
        )
        .route(
            "/stormguard_stats",
            axum::routing::get(get_stormguard_stats),
        )
        .route("/shaped_devices", axum::routing::get(get_shaped_devices))
        .route(
            "/shaped_devices/{device_id}",
            axum::routing::get(get_shaped_device),
        )
        .route(
            "/shaped_devices/update",
            axum::routing::post(write_shaped_devices),
        )
        .route(
            "/shaped_devices/circuit/{circuit_id}",
            axum::routing::get(shaped_devices_by_circuit_id),
        )
        .route(
            "/shaped_devices/circuit/{circuit_id}",
            axum::routing::delete(live_remove_circuit),
        )
        .route(
            "/shaped_devices/circuit",
            axum::routing::post(live_upsert_circuit),
        )
        .route(
            "/shaped_devices_by_circuit/{circuit_id}",
            axum::routing::get(shaped_devices_by_circuit_id),
        )
        .route(
            "/identify_ip/{ip_address}",
            axum::routing::get(identify_ip_address),
        )
        .route(
            "/circuits_at_capacity",
            axum::routing::get(circuits_at_capacity),
        )
        .route(
            "/tree_nodes_at_capacity",
            axum::routing::get(tree_nodes_at_capacity),
        )
        .route(
            "/circuits_high_retransmits",
            axum::routing::get(circuits_high_retransmits),
        )
        .route(
            "/tree_nodes_high_retransmits",
            axum::routing::get(tree_nodes_high_retransmits),
        )
        .route(
            "/ping_circuit/{circuit_id}",
            axum::routing::get(ping_circuit),
        )
        .route(
            "/traceroute_ip/{ip_address}",
            axum::routing::get(traceroute_ip),
        )
        .route(
            "/traceroute_circuit/{circuit_id}",
            axum::routing::get(traceroute_circuit),
        )
        .route("/ping_ip/{ip_address}", axum::routing::get(ping_ip))
        // Overrides API
        .route(
            "/overrides/persistent_devices",
            axum::routing::get(overrides_persistent_devices_list),
        )
        .route(
            "/overrides/persistent_devices",
            axum::routing::post(overrides_persistent_devices_add),
        )
        .route(
            "/overrides/persistent_devices/by_circuit/{circuit_id}",
            axum::routing::delete(overrides_persistent_devices_delete_circuit),
        )
        .route(
            "/overrides/persistent_devices/by_device/{device_id}",
            axum::routing::delete(overrides_persistent_devices_delete_device),
        )
        .route(
            "/overrides/adjustments",
            axum::routing::get(overrides_adjustments_list),
        )
        .route(
            "/overrides/adjustments/circuit_speed",
            axum::routing::post(overrides_adjustments_add_circuit_speed),
        )
        .route(
            "/overrides/adjustments/device_speed",
            axum::routing::post(overrides_adjustments_add_device_speed),
        )
        .route(
            "/overrides/adjustments/remove_circuit",
            axum::routing::post(overrides_adjustments_add_remove_circuit),
        )
        .route(
            "/overrides/adjustments/remove_device",
            axum::routing::post(overrides_adjustments_add_remove_device),
        )
        .route(
            "/overrides/adjustments/reparent_circuit",
            axum::routing::post(overrides_adjustments_add_reparent_circuit),
        )
        .route(
            "/overrides/adjustments/{index}",
            axum::routing::delete(overrides_adjustments_delete_index),
        )
        .route(
            "/overrides/network_adjustments",
            axum::routing::get(overrides_network_adjustments_list),
        )
        .route(
            "/overrides/network_adjustments/site_speed",
            axum::routing::post(overrides_network_adjustments_add_site_speed),
        )
        .route(
            "/overrides/network_adjustments/{index}",
            axum::routing::delete(overrides_network_adjustments_delete_index),
        )
        .route(
            "/overrides/uisp/bandwidth",
            axum::routing::get(overrides_uisp_bandwidth_list),
        )
        .route(
            "/overrides/uisp/bandwidth",
            axum::routing::put(overrides_uisp_bandwidth_set),
        )
        .route(
            "/overrides/uisp/bandwidth/{site_name}",
            axum::routing::delete(overrides_uisp_bandwidth_remove),
        )
        .route(
            "/overrides/uisp/routes",
            axum::routing::get(overrides_uisp_route_list),
        )
        .route(
            "/overrides/uisp/routes",
            axum::routing::post(overrides_uisp_route_add),
        )
        .route(
            "/overrides/uisp/routes/{index}",
            axum::routing::delete(overrides_uisp_route_remove_index),
        )
        .route("/config/json", axum::routing::get(get_config_as_json))
        .route("/config/toml", axum::routing::get(get_config_as_toml))
        .route("/config/hash", axum::routing::get(get_config_hash))
        .route(
            "/config/update_json",
            axum::routing::post(write_config_json),
        )
        .route(
            "/config/update_toml",
            axum::routing::post(write_config_toml),
        )
        .route(
            "/network_json/text",
            axum::routing::get(get_network_json_as_text),
        )
        .route(
            "/network_json/json",
            axum::routing::get(get_network_json_as_json),
        )
        .route(
            "/network_json/update",
            axum::routing::post(legacy_network_json_update_stub),
        )
        .route(
            "/network_json/node",
            axum::routing::post(legacy_network_json_add_node_stub),
        )
        .route(
            "/network_json/node/{node_name}",
            axum::routing::delete(legacy_network_json_remove_node_stub),
        )
        .route(
            "/network/reparent_site",
            axum::routing::post(legacy_network_reparent_site_stub),
        )
        .route("/warnings", axum::routing::get(warnings))
        .route("/urgent/status", axum::routing::get(urgent_status))
        .route("/urgent", axum::routing::get(urgent_list))
        .route("/urgent/clear_all", axum::routing::post(urgent_clear_all))
        .route("/urgent/{id}", axum::routing::delete(urgent_clear))
}

#[cfg(test)]
mod tests {
    use super::{
        API_MOUNT_PREFIX, ApiDoc, DOCS_UI_PATH, DOCUMENT_PATH, KEY_PARAMETER_DESCRIPTION,
        KEY_PARAMETER_EXAMPLE, UNAUTHORISED_DESCRIPTION,
    };
    use utoipa::OpenApi;

    /// Wording that belonged to the standalone service's licence model. None of
    /// it describes what `/api/v3` serves.
    const RETIRED_WORDING: &[&str] = &[
        "license key",
        "licence key",
        "license is invalid",
        "not checked",
        "policy denies",
        "insight",
        "entitlement",
        "all rights reserved",
        "bfnightly",
        "1,000",
    ];

    fn document() -> serde_json::Value {
        serde_json::to_value(ApiDoc::openapi()).expect("the document must serialise")
    }

    /// Collects every string the document shows a reader.
    ///
    /// The OpenAPI `license` object is skipped: the project's GPL-2.0-only
    /// declaration is a licence of the software, not an API access policy.
    fn documented_texts(value: &serde_json::Value, path: &str, out: &mut Vec<(String, String)>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, child) in map {
                    if key == "license" {
                        continue;
                    }
                    documented_texts(child, &format!("{path}/{key}"), out);
                }
            }
            serde_json::Value::Array(items) => {
                for (index, child) in items.iter().enumerate() {
                    documented_texts(child, &format!("{path}/{index}"), out);
                }
            }
            serde_json::Value::String(text) => out.push((path.to_string(), text.clone())),
            _ => {}
        }
    }

    /// The three paths that carry the mount prefix are literals, because both
    /// `#[openapi(servers(...))]` and the Swagger UI mount take string literals.
    /// This pins them to the single constant the daemon nests on.
    #[test]
    fn documentation_paths_agree_with_the_mount_prefix() {
        assert_eq!(
            ApiDoc::openapi()
                .servers
                .expect("the document declares its server")[0]
                .url,
            API_MOUNT_PREFIX
        );
        assert_eq!(DOCS_UI_PATH, format!("{API_MOUNT_PREFIX}/docs"));
        assert_eq!(DOCUMENT_PATH, format!("{API_MOUNT_PREFIX}/openapi.json"));
    }

    /// The prefix appears in `servers` and nowhere else: operations stay
    /// root-relative, so a client applies it exactly once.
    #[test]
    fn the_mount_prefix_is_applied_exactly_once() {
        let document = document();
        let servers = document["servers"]
            .as_array()
            .expect("the document declares servers");
        assert_eq!(servers.len(), 1, "one mount, one server entry");
        assert_eq!(servers[0]["url"], API_MOUNT_PREFIX);

        let paths = document["paths"].as_object().expect("paths");
        assert!(!paths.is_empty());
        for path in paths.keys() {
            assert!(
                path.starts_with('/') && !path.starts_with(API_MOUNT_PREFIX),
                "{path} must stay root-relative; the server entry applies the prefix"
            );
        }
    }

    /// One `apiKey` scheme, in the `x-bearer` header, required by every operation.
    #[test]
    fn document_declares_one_x_bearer_api_key_scheme() {
        let document = document();
        let schemes = document["components"]["securitySchemes"]
            .as_object()
            .expect("the document declares a security scheme");
        assert_eq!(schemes.len(), 1, "the mount accepts one mechanism: a key");

        let scheme = &schemes["x-bearer"];
        assert_eq!(scheme["type"], "apiKey");
        assert_eq!(scheme["in"], "header");
        assert_eq!(scheme["name"], "x-bearer");

        assert_eq!(
            document["security"],
            serde_json::json!([{ "x-bearer": [] }]),
            "the key requirement is global, not per operation"
        );
    }

    /// Every operation documents the key this mount accepts, and a `401` means
    /// exactly one thing.
    #[test]
    fn every_operation_documents_the_key_only_auth() {
        let document = document();
        let mut checked = 0_usize;

        for (path, item) in document["paths"].as_object().expect("paths") {
            for (method, operation) in item.as_object().expect("path item") {
                if let Some(unauthorised) = operation.get("responses").and_then(|r| r.get("401")) {
                    assert_eq!(
                        unauthorised["description"], UNAUTHORISED_DESCRIPTION,
                        "{method} {path} describes a 401 that this mount does not produce"
                    );
                }
                for parameter in operation
                    .get("parameters")
                    .and_then(|p| p.as_array())
                    .into_iter()
                    .flatten()
                {
                    if parameter["name"] != "x-bearer" {
                        continue;
                    }
                    assert_eq!(
                        parameter["description"], KEY_PARAMETER_DESCRIPTION,
                        "{method} {path} documents a key this mount does not accept"
                    );
                    assert_eq!(parameter["example"], KEY_PARAMETER_EXAMPLE);
                    assert_eq!(parameter["schema"]["type"], "string");
                    checked += 1;
                }
            }
        }

        assert!(checked > 0, "no operation documented the x-bearer key");
    }

    /// The mount has no `403` producer: `require_api_key` answers `401` and the
    /// handlers only return their own statuses. A documented `403` would be a
    /// status a client can never receive, so the retired policy arm is gone from
    /// every operation (it was the standalone service's licence gate).
    #[test]
    fn document_documents_no_403_arm() {
        let document = document();
        let mut offenders = Vec::new();

        for (path, item) in document["paths"].as_object().expect("paths") {
            for (method, operation) in item.as_object().expect("path item") {
                if operation
                    .get("responses")
                    .and_then(|responses| responses.get("403"))
                    .is_some()
                {
                    offenders.push(format!("{method} {path}"));
                }
            }
        }

        assert!(
            offenders.is_empty(),
            "these operations still document a 403 this mount cannot produce: {offenders:?}"
        );
    }

    /// The served specification makes no claim about licences, entitlements or
    /// circuit counts, anywhere a reader can see one.
    #[test]
    fn document_carries_no_licence_or_entitlement_claims() {
        let document = document();
        let mut texts = Vec::new();
        documented_texts(&document, "", &mut texts);
        assert!(!texts.is_empty());

        for (path, text) in texts {
            let lowered = text.to_lowercase();
            for retired in RETIRED_WORDING {
                assert!(
                    !lowered.contains(retired),
                    "{path} still carries retired wording {retired:?}: {text}"
                );
            }
        }
    }
}
