#[macro_use]
extern crate rocket;
use rocket::fairing::AdHoc;
mod cache_control;
mod shaped_devices;
mod static_pages;
mod tracker;
mod unknown_devices;
use rocket_async_compression::Compression;
mod auth_guard;
mod config_control;
mod network_tree;
mod queue_info;
mod toasts;

// Use JemAllocator only on supported platforms
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
use jemallocator::Jemalloc;

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[launch]
fn rocket() -> _ {
  let server = rocket::build()
    .attach(AdHoc::on_liftoff("Poll lqosd", |_| {
      Box::pin(async move {
        rocket::tokio::spawn(tracker::update_tracking());
      })
    }))
    .attach(AdHoc::on_liftoff("Poll throughput", |_| {
      Box::pin(async move {
        rocket::tokio::spawn(tracker::update_total_throughput_buffer());
      })
    }))
    .register("/", catchers![static_pages::login])
    .mount(
      "/",
      routes![
        static_pages::index,
        static_pages::shaped_devices_csv_page,
        static_pages::shaped_devices_add_page,
        static_pages::unknown_devices_page,
        static_pages::circuit_queue,
        config_control::config_page,
        network_tree::tree_page,
        static_pages::ip_dump,
        // Our JS library
        static_pages::lqos_js,
        static_pages::lqos_css,
        static_pages::klingon,
        // API calls
        tracker::current_throughput,
        tracker::throughput_ring_buffer,
        tracker::cpu_usage,
        tracker::ram_usage,
        tracker::top_10_downloaders,
        tracker::worst_10_rtt,
        tracker::rtt_histogram,
        tracker::host_counts,
        shaped_devices::all_shaped_devices,
        shaped_devices::shaped_devices_count,
        shaped_devices::shaped_devices_range,
        shaped_devices::shaped_devices_search,
        shaped_devices::reload_required,
        shaped_devices::reload_libreqos,
        unknown_devices::all_unknown_devices,
        unknown_devices::unknown_devices_count,
        unknown_devices::unknown_devices_range,
        unknown_devices::unknown_devices_csv,
        queue_info::raw_queue_by_circuit,
        queue_info::run_btest,
        queue_info::circuit_info,
        queue_info::current_circuit_throughput,
        queue_info::watch_circuit,
        queue_info::flow_stats,
        queue_info::packet_dump,
        queue_info::pcap,
        queue_info::request_analysis,
        queue_info::dns_query,
        config_control::get_nic_list,
        config_control::get_current_python_config,
        config_control::get_current_lqosd_config,
        config_control::update_python_config,
        config_control::update_lqos_tuning,
        auth_guard::create_first_user,
        auth_guard::login,
        auth_guard::admin_check,
        static_pages::login_page,
        auth_guard::username,
        network_tree::tree_entry,
        network_tree::tree_clients,
        network_tree::network_tree_summary,
        network_tree::node_names,
        network_tree::funnel_for_queue,
        config_control::stats,
        // Supporting files
        static_pages::bootsrap_css,
        static_pages::plotly_js,
        static_pages::jquery_js,
        static_pages::msgpack_js,
        static_pages::bootsrap_js,
        static_pages::tinylogo,
        static_pages::favicon,
        static_pages::fontawesome_solid,
        static_pages::fontawesome_webfont,
        static_pages::fontawesome_woff,
        // Front page toast checks
        toasts::version_check,
        toasts::stats_check,
      ],
    );

  // Compression is slow in debug builds,
  // so only enable it on release builds.
  if cfg!(debug_assertions) {
    server
  } else {
    server.attach(Compression::fairing())
  }
}
