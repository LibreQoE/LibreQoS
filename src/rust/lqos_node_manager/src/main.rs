#[macro_use] extern crate rocket;
use rocket::fairing::AdHoc;
mod static_pages;
mod tracker;
mod shaped_devices;
mod unknown_devices;
mod cache_control;
use rocket_async_compression::Compression;
mod queue_info;

#[launch]
fn rocket() -> _ {
    //tracker::SHAPED_DEVICES.read().write_csv("ShapedDeviceWriteTest.csv").unwrap();
    let server = rocket::build()
        .attach(AdHoc::on_liftoff("Poll lqosd", |_| {
            Box::pin(async move {
                rocket::tokio::spawn(tracker::update_tracking());
            })
        }))
        .mount("/", routes![
            static_pages::index,
            static_pages::shaped_devices_csv_page,
            static_pages::shaped_devices_add_page,
            static_pages::unknown_devices_page,
            static_pages::circuit_queue,

            // Our JS library
            static_pages::lqos_js,

            // API calls
            tracker::current_throughput,
            tracker::throughput_ring,
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
            queue_info::raw_queue_by_circuit,
            queue_info::run_btest,

            // Supporting files
            static_pages::bootsrap_css,
            static_pages::plotly_js,
            static_pages::jquery_js,
            static_pages::bootsrap_js,
            static_pages::tinylogo,
            static_pages::favicon,
            static_pages::fontawesome_solid,
            static_pages::fontawesome_webfont,
            static_pages::fontawesome_woff,
        ]
    );
    
    // Compression is slow in debug builds,
    // so only enable it on release builds.
    if cfg!(debug_assertions) {
        server
    } else {
        server.attach(Compression::fairing())
    }
}